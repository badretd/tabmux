//! Command-line definitions and dispatch.

pub mod args;
mod batch;
pub(crate) mod reporting;
mod run;

use std::collections::HashSet;
use std::io::{self, Write as _};
use std::path::{Component, Path, PathBuf};

use clap::{Parser, Subcommand};

use crate::diagnostics::{DiagnosticsPolicy, DiagnosticsScope};
use crate::error::{Error, Result};
use crate::formats::{self, FormatId};
use crate::io::SafeWriter;
use crate::model::{CellValue, Column, ColumnType, LossEvent, SourceRef, Table, TableSet};
use crate::ops::{dedupe, filter, merge, rename, schema, select, sort};
use crate::report::{self, Artifact};

use self::args::{
    CellArgs, ConvertArgs, CountArgs, DedupeArgs, DiagnosticsArgs, FilterArgs, HeadArgs, MergeArgs,
    MergeModeArg, QueryArgs, ReadArgs, RenameArgs, RowArgs, RunArgs, SelectArgs, SheetsArgs,
    SortArgs, ToArgs, ToDuckdbArgs, ToTarget,
};
use self::reporting::{ReportContext, input_artifacts, output_artifact, table_events};

/// Top-level command-line interface.
#[derive(Debug, Parser)]
#[command(
    name = "tabmux",
    version,
    about = "Convert messy tabular sources into clean tables and workbooks",
    long_about = "tabmux converts messy tabular sources into clean tables and workbooks with \
                  safe writes, a replayable pipeline, and an honest loss report.",
    propagate_version = true
)]
pub struct Cli {
    /// Operation to run.
    #[command(subcommand)]
    pub command: Command,

    #[command(flatten)]
    pub diagnostics: DiagnosticsArgs,
}

/// Available operations.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show the supported format matrix.
    Formats,
    /// Convert one or more inputs to one output.
    Convert(ConvertArgs),
    /// List the sheets in a workbook.
    Sheets(SheetsArgs),
    /// Count data rows in a table.
    Count(CountArgs),
    /// Print the first rows of a table.
    Head(HeadArgs),
    /// Select columns and print them to stdout.
    Select(SelectArgs),
    /// Print one data row by number or a simple equality filter.
    Row(RowArgs),
    /// Print one cell by row number and column.
    Cell(CellArgs),
    /// Merge or concatenate several tables into one output.
    #[command(visible_alias = "concat")]
    Merge(MergeArgs),
    /// Filter rows with a simple comparison expression.
    Filter(FilterArgs),
    /// Sort rows by one or more columns.
    Sort(SortArgs),
    /// Rename columns.
    Rename(RenameArgs),
    /// Drop exact duplicate rows.
    Dedupe(DedupeArgs),
    /// Export a table to a SQL database.
    To(ToArgs),
    /// Run a read-only SQL query against a database.
    Query(QueryArgs),
    /// Run a reproducible pipeline manifest.
    Run(RunArgs),
}

/// Execute a parsed command.
pub fn dispatch(cli: Cli) -> Result<()> {
    let diagnostics = cli.diagnostics.policy();
    match cli.command {
        Command::Formats => print_matrix(),
        Command::Convert(args) => convert(args, diagnostics),
        Command::Sheets(args) => sheets(args),
        Command::Count(args) => count(args, diagnostics),
        Command::Head(args) => head(args, diagnostics),
        Command::Select(args) => select(args, diagnostics),
        Command::Row(args) => row(args, diagnostics),
        Command::Cell(args) => cell(args, diagnostics),
        Command::Merge(args) => merge_tables(args, diagnostics),
        Command::Filter(args) => filter_rows(args, diagnostics),
        Command::Sort(args) => sort_rows(args, diagnostics),
        Command::Rename(args) => rename_columns(args, diagnostics),
        Command::Dedupe(args) => dedupe_rows(args, diagnostics),
        Command::To(args) => to_command(args, diagnostics),
        Command::Query(args) => query_command(args),
        Command::Run(args) => run::run_pipeline(args, diagnostics),
    }
}

fn to_command(args: ToArgs, diagnostics: DiagnosticsPolicy) -> Result<()> {
    match args.target {
        ToTarget::Duckdb(args) => to_duckdb(args, diagnostics),
    }
}

#[cfg(feature = "sql-duckdb")]
fn to_duckdb(args: ToDuckdbArgs, diagnostics: DiagnosticsPolicy) -> Result<()> {
    let (mut tables, _) = load_tables(&args.input, &args.read)?;
    if tables.tables.len() != 1 {
        let available = tables
            .tables
            .iter()
            .map(|table| table.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(Error::usage(format!(
            "to duckdb imports exactly one table, but {} were selected from '{}'{}",
            tables.tables.len(),
            args.input,
            if available.is_empty() {
                String::new()
            } else {
                format!(" (available: {available})")
            }
        )));
    }

    let table = tables.tables.remove(0);
    print_warnings(
        &table,
        &diagnostics,
        DiagnosticsScope::Write,
        diagnostics.fidelity_report,
    );

    if args.dry_run {
        eprintln!(
            "tabmux: dry-run: would import {} into table '{}' in {} ({} rows x {} columns)",
            table.source.context(),
            args.table,
            args.output,
            table.row_count(),
            table.width()
        );
        return Ok(());
    }

    let report = formats::duckdb::import_table(
        Path::new(&args.output),
        &args.table,
        &table,
        args.force,
        args.verify,
    )?;

    if diagnostics.enabled() {
        let mut suffix = String::new();
        if report.replaced {
            suffix.push_str("; replaced existing table");
        }
        if report.verified {
            suffix.push_str("; verified");
        }
        suffix.push_str(&format!("; sha256:{}", &report.checksum[..16]));
        eprintln!(
            "tabmux: imported {} row(s) x {} column(s) from {} into table '{}' in {}{}",
            report.rows,
            report.columns,
            table.source.context(),
            report.table,
            args.output,
            suffix
        );
    }
    Ok(())
}

#[cfg(not(feature = "sql-duckdb"))]
fn to_duckdb(_args: ToDuckdbArgs, _diagnostics: DiagnosticsPolicy) -> Result<()> {
    Err(Error::unsupported(
        "DuckDB export is not compiled in (enable the sql-duckdb feature)",
    ))
}

#[cfg(feature = "sql-duckdb")]
fn query_command(args: QueryArgs) -> Result<()> {
    use self::args::QueryFormat;

    let result = formats::duckdb::query(Path::new(&args.database), &args.sql)?;

    match args.format {
        QueryFormat::Table => {
            let table = result.to_table(&args.database, FormatId::Unknown);
            write_stdout_pretty(&table, true)
        }
        QueryFormat::Csv | QueryFormat::Tsv => {
            let format = if args.format == QueryFormat::Csv {
                FormatId::Csv
            } else {
                FormatId::Tsv
            };
            let table = result.to_table(&args.database, format);
            let options = formats::WriteOptions::for_format(format);
            let mut buffer = Vec::new();
            formats::write_table(&mut buffer, &table, format, &options)?;
            crate::io::write_stdout(&buffer)
        }
        QueryFormat::Json => {
            let mut rendered = serde_json::to_string_pretty(&result.to_json())
                .map_err(|error| Error::data(format!("cannot serialize query result: {error}")))?;
            rendered.push('\n');
            crate::io::write_stdout(rendered.as_bytes())
        }
    }
}

#[cfg(not(feature = "sql-duckdb"))]
fn query_command(_args: QueryArgs) -> Result<()> {
    Err(Error::unsupported(
        "SQL query support is not compiled in (enable the sql-duckdb feature)",
    ))
}

fn print_matrix() -> Result<()> {
    let matrix = formats::render_matrix();
    let mut stdout = io::stdout().lock();
    stdout.write_all(matrix.as_bytes()).map_err(Error::Io)?;
    stdout.flush().map_err(Error::Io)
}

fn convert(args: ConvertArgs, diagnostics: DiagnosticsPolicy) -> Result<()> {
    let (inputs, output) = args.resolve()?;
    reject_self_overwrite(&inputs, &output)?;

    if inputs.len() == 1 && formats::archive::is_batch_input(Path::new(&inputs[0])) {
        return batch::convert_batch(&inputs[0], &output, &args, &diagnostics);
    }

    if args.recursive || args.flatten || !args.include.is_empty() || !args.exclude.is_empty() {
        return Err(Error::usage(
            "--recursive, --include, --exclude, and --flatten require a directory or ZIP batch input",
        ));
    }

    if args.split_sheets && output == "-" {
        return Err(Error::usage(
            "--split-sheets requires an output directory, not stdout",
        ));
    }
    if args.name_template.is_some() && !args.split_sheets {
        return Err(Error::usage("--name-template requires --split-sheets"));
    }
    if args.add_source_column.is_some() && !args.stack {
        return Err(Error::usage("--add-source-column requires --stack"));
    }
    if args.sheet_per_table {
        if args.stack {
            return Err(Error::usage(
                "--sheet-per-table cannot be combined with --stack",
            ));
        }
        if args.split_sheets
            || output == "-"
            || !is_workbook_output(detect_supported_output(&output)?)
        {
            return Err(Error::usage(
                "--sheet-per-table requires a workbook output (.xlsx or .ods)",
            ));
        }
    }
    if args.index_sheet {
        if args.split_sheets || output == "-" {
            return Err(Error::usage(
                "--index-sheet requires a workbook output (.xlsx or .ods)",
            ));
        }
        if !is_workbook_output(detect_supported_output(&output)?) {
            return Err(Error::usage(
                "--index-sheet requires a workbook output (.xlsx or .ods)",
            ));
        }
        if args.stack {
            return Err(Error::usage(
                "--index-sheet cannot be combined with --stack",
            ));
        }
        if args.sheet_name.is_some() || args.sheet_name_template.is_some() {
            return Err(Error::usage(
                "--index-sheet cannot be combined with --sheet-name or --sheet-name-template",
            ));
        }
    }
    if args.split_sheets
        && let Some(format) = formats::detect_path(Path::new(&output))
        && is_workbook_output(format)
    {
        return Err(Error::usage(
            "--split-sheets writes delimited files; it cannot write a workbook",
        ));
    }

    let requested_format = if args.split_sheets || output == "-" {
        None
    } else {
        Some(detect_supported_output(&output)?)
    };

    if let Some(format) = requested_format {
        if is_workbook_output(format) {
            return convert_workbook(&inputs, &output, format, &args, &diagnostics);
        }

        if inputs.len() > 1 {
            return Err(Error::usage(
                "multiple inputs require a workbook output (.xlsx or .ods); merge/concat arrives in Phase 5",
            ));
        }
        if args.sheet_per_input || args.sheet_name_template.is_some() || args.sheet_name.is_some() {
            return Err(Error::usage(
                "--sheet-per-input, --sheet-name-template, and --sheet-name require a workbook output",
            ));
        }
        if args.read.sheets.all_sheets && !args.split_sheets && !args.stack {
            return Err(Error::usage(
                "--all-sheets requires --split-sheets, --stack, or a workbook output",
            ));
        }

        return convert_delimited(&inputs[0], &output, Some(format), &args, &diagnostics);
    }

    if inputs.len() > 1 {
        return Err(Error::usage(
            "multiple inputs require a workbook output (.xlsx or .ods); merge/concat arrives in Phase 5",
        ));
    }
    if args.sheet_per_input || args.sheet_name_template.is_some() || args.sheet_name.is_some() {
        return Err(Error::usage(
            "--sheet-per-input, --sheet-name-template, and --sheet-name require a workbook output",
        ));
    }
    if args.read.sheets.all_sheets && !args.split_sheets && !args.stack {
        return Err(Error::usage(
            "--all-sheets requires --split-sheets, --stack, or a workbook output",
        ));
    }

    convert_delimited(&inputs[0], &output, None, &args, &diagnostics)
}

fn convert_delimited(
    input: &str,
    output: &str,
    requested_format: Option<FormatId>,
    args: &ConvertArgs,
    diagnostics: &DiagnosticsPolicy,
) -> Result<()> {
    if !args.split_sheets && output != "-" {
        SafeWriter::ensure_writable(Path::new(output), args.write.force)?;
    }

    let (mut tables, input_format) = load_tables(input, &args.read)?;
    let context = ReportContext::new(
        &args.reporting,
        input_artifacts(&[input.to_owned()])?,
        args.write.force,
    );

    if args.split_sheets {
        return write_split(
            &mut tables,
            Path::new(output),
            args,
            input,
            diagnostics,
            context,
        );
    }

    let mut table = if args.stack {
        stack_tables(
            std::mem::take(&mut tables.tables),
            args.add_source_column.as_deref(),
        )?
    } else {
        tables.into_single()?
    };
    let output_format = delimited_output_format(input_format, output, requested_format)?;
    report::push_null_rendering_warning(&mut table, output_format, &args.write.null);
    let events = table_events(std::slice::from_ref(&table));
    let protected = if output == "-" {
        Vec::new()
    } else {
        vec![("output", Path::new(output))]
    };
    context.validate_paths(&protected)?;

    let include_fidelity =
        diagnostics.fidelity_report || is_workbook_output(output_format) || args.reporting.strict;
    if context.aggregates_diagnostics(diagnostics) {
        context.print_event_summary(
            &events,
            diagnostics,
            DiagnosticsScope::Write,
            include_fidelity,
        );
    } else {
        print_warnings(
            &table,
            diagnostics,
            DiagnosticsScope::Write,
            include_fidelity,
        );
    }
    context.check_strict(&events)?;

    let mut options = formats::WriteOptions::for_format(output_format);
    options.has_header = !args.read.no_header;
    options.line_ending = args.write.line_ending;
    options.null = args.write.null.clone();
    let read_options = args.read.options()?;

    if output == "-" {
        if args.write.dry_run {
            print_dry_run("stdout", &table, output_format);
            context.write_loss(&events, &[])?;
            context.print_dry_run_reports(&events, diagnostics);
            return Ok(());
        }
        let mut buffer = Vec::new();
        formats::write_table(&mut buffer, &table, output_format, &options)?;
        context.verify_bytes(
            "stdout",
            output_format,
            std::slice::from_ref(&table),
            &read_options,
            &buffer,
            &args.write.null,
        )?;
        let outputs = vec![Artifact::from_bytes("-", output_format, &buffer)];
        crate::io::write_stdout(&buffer)?;
        context.write_loss(&events, &outputs)?;
        context.write_manifest(
            &events,
            &outputs,
            convert_manifest_options(args, output_format),
        )?;
        if diagnostics.enabled() {
            eprintln!(
                "tabmux: wrote 1 table ({} rows x {} columns) to stdout{}",
                table.row_count(),
                table.width(),
                context.summary_suffix(&events)
            );
        }
        return Ok(());
    }

    let destination = Path::new(output);
    if args.write.dry_run {
        print_dry_run(output, &table, output_format);
        context.write_loss(&events, &[])?;
        context.print_dry_run_reports(&events, diagnostics);
        return Ok(());
    }

    let mut writer = SafeWriter::create(destination, args.write.force, args.write.backup)?;
    formats::write_table(&mut writer, &table, output_format, &options)?;
    writer.commit()?;

    let outputs = vec![output_artifact(destination, output_format)?];
    context.verify_file(
        destination,
        output_format,
        std::slice::from_ref(&table),
        &read_options,
        &args.write.null,
    )?;
    context.write_loss(&events, &outputs)?;
    context.write_manifest(
        &events,
        &outputs,
        convert_manifest_options(args, output_format),
    )?;
    if diagnostics.enabled() {
        eprintln!(
            "tabmux: wrote 1 table ({} rows x {} columns) to {}{}",
            table.row_count(),
            table.width(),
            destination.display(),
            context.summary_suffix(&events)
        );
    }
    Ok(())
}

#[cfg(feature = "excel")]
fn convert_workbook(
    inputs: &[String],
    output: &str,
    format: FormatId,
    args: &ConvertArgs,
    diagnostics: &DiagnosticsPolicy,
) -> Result<()> {
    if args.split_sheets {
        return Err(Error::usage(
            "--split-sheets writes delimited files; it cannot write a workbook",
        ));
    }

    SafeWriter::ensure_writable(Path::new(output), args.write.force)?;

    let mut selected = Vec::new();
    let mut skipped = Vec::new();
    let mut index_entries = Vec::new();
    for input in inputs {
        let (set, _) = load_tables(input, &args.read)?;
        index_entries.extend(index_entries_for_set(&set));
        selected.extend(set.tables);
        skipped.extend(set.skipped);
    }

    if selected.is_empty() {
        return Err(Error::data("no tables were selected to write"));
    }

    let mut tables = if args.stack {
        vec![stack_tables(selected, args.add_source_column.as_deref())?]
    } else {
        selected
    };

    name_workbook_sheets(&mut tables, args)?;

    if args.index_sheet {
        let index = build_index_table(&tables, &index_entries, output, format);
        tables.insert(0, index);
    }

    let context = ReportContext::new(&args.reporting, input_artifacts(inputs)?, args.write.force);
    context.validate_paths(&[("output", Path::new(output))])?;
    let events = table_events(&tables);

    // Writing a workbook always includes source-fidelity events.
    if context.aggregates_diagnostics(diagnostics) {
        context.print_event_summary(&events, diagnostics, DiagnosticsScope::Write, true);
    } else {
        for table in &tables {
            print_warnings(table, diagnostics, DiagnosticsScope::Write, true);
        }
    }
    context.check_strict(&events)?;

    if args.write.dry_run {
        eprintln!(
            "tabmux: dry-run: would write {output} as {} with {} sheet(s)",
            format.label(),
            tables.len()
        );
        for table in &tables {
            eprintln!(
                "tabmux: dry-run: sheet '{}' ({} rows x {} columns)",
                table.name,
                table.row_count(),
                table.width()
            );
        }
        if !skipped.is_empty() && diagnostics.enabled() {
            eprintln!(
                "tabmux: dry-run: {}",
                workbook_summary(&tables, &skipped, true)
            );
        }
        context.write_loss(&events, &[])?;
        context.print_dry_run_reports(&events, diagnostics);
        return Ok(());
    }

    let mut options = formats::WriteOptions::for_format(format);
    options.has_header = !args.read.no_header;
    options.line_ending = args.write.line_ending;
    options.null = args.write.null.clone();

    let destination = Path::new(output);
    let mut writer = SafeWriter::create(destination, args.write.force, args.write.backup)?;
    formats::write_tables(writer.file_mut(), &tables, format, &options)?;
    writer.commit()?;

    let outputs = vec![output_artifact(destination, format)?];
    context.verify_file(
        destination,
        format,
        &tables,
        &args.read.options()?,
        &args.write.null,
    )?;
    context.write_loss(&events, &outputs)?;
    context.write_manifest(&events, &outputs, convert_manifest_options(args, format))?;

    if diagnostics.enabled() {
        eprintln!(
            "tabmux: {}{}",
            workbook_summary(&tables, &skipped, false),
            context.summary_suffix(&events)
        );
    }
    Ok(())
}

#[cfg(not(feature = "excel"))]
fn convert_workbook(
    _inputs: &[String],
    _output: &str,
    _format: FormatId,
    _args: &ConvertArgs,
    _diagnostics: &DiagnosticsPolicy,
) -> Result<()> {
    Err(Error::unsupported(
        "workbook writing is not compiled in (enable the excel feature)",
    ))
}

/// One row in the `_index` worksheet.
#[cfg(feature = "excel")]
#[derive(Debug, Clone)]
struct IndexEntry {
    source: String,
    table: usize,
    caption: String,
    rows: usize,
    columns: usize,
    status: &'static str,
    reason: String,
}

/// Build index entries for one input, including tables that were skipped.
#[cfg(feature = "excel")]
fn index_entries_for_set(set: &TableSet) -> Vec<IndexEntry> {
    let mut entries = Vec::with_capacity(set.tables.len() + set.skipped.len());
    let mut per_source = std::collections::HashMap::<String, usize>::new();

    for table in &set.tables {
        let source_path = table.source.path.display().to_string();
        let fallback = {
            let number = per_source.entry(source_path.clone()).or_insert(0);
            *number += 1;
            *number
        };
        let number = table
            .source
            .table
            .as_deref()
            .and_then(|table| table.strip_prefix("table-"))
            .and_then(|number| number.parse::<usize>().ok())
            .unwrap_or(fallback);
        let reason = table
            .source
            .nested_in
            .as_ref()
            .map(|parent| {
                format!(
                    "nested table extracted from {parent}; outer cell replaced with marker '[table {number}]'"
                )
            })
            .unwrap_or_default();

        entries.push(IndexEntry {
            source: source_path,
            table: number,
            caption: table.source.caption.clone().unwrap_or_default(),
            rows: table.row_count(),
            columns: table.width(),
            status: "included",
            reason,
        });
    }

    for skipped in &set.skipped {
        entries.push(IndexEntry {
            source: skipped.source.path.display().to_string(),
            table: skipped.table,
            caption: skipped.caption.clone().unwrap_or_default(),
            rows: skipped.rows,
            columns: skipped.columns,
            status: "skipped",
            reason: skipped.reason.clone(),
        });
    }

    entries.sort_by_key(|entry| entry.table);
    entries
}

/// Build the `_index` worksheet inserted by `--index-sheet`.
#[cfg(feature = "excel")]
fn build_index_table(
    tables: &[Table],
    entries: &[IndexEntry],
    output: &str,
    format: FormatId,
) -> Table {
    let rows = entries
        .iter()
        .map(|entry| {
            vec![
                CellValue::Text(entry.source.clone()),
                CellValue::Int(entry.table as i64),
                CellValue::Text(entry.caption.clone()),
                CellValue::Int(entry.rows as i64),
                CellValue::Int(entry.columns as i64),
                CellValue::Text(entry.status.to_owned()),
                CellValue::Text(entry.reason.clone()),
            ]
        })
        .collect::<Vec<_>>();

    let columns = vec![
        Column::new("source", ColumnType::Text),
        Column::new("table", ColumnType::Int),
        Column::new("caption", ColumnType::Text),
        Column::new("rows", ColumnType::Int),
        Column::new("columns", ColumnType::Int),
        Column::new("status", ColumnType::Text),
        Column::new("reason", ColumnType::Text),
    ];

    let source = SourceRef::new(output, format).with_sheet("_index");
    let mut used = tables
        .iter()
        .map(|table| table.name.to_lowercase())
        .collect::<HashSet<_>>();
    let name = formats::workbook_write::unique_sheet_name("_index", &mut used);

    let mut index = Table::with_columns(name.clone(), source, columns, rows);
    if name != "_index" {
        index.push_warning(LossEvent::info(
            "output.sheet_name_sanitized",
            format!("index sheet name '_index' collided; wrote '{name}'"),
        ));
    }
    index
}

/// A concise post-write inventory for workbook output.
#[cfg(feature = "excel")]
fn workbook_summary(
    tables: &[Table],
    skipped: &[crate::model::SkippedTable],
    dry_run: bool,
) -> String {
    let names = tables
        .iter()
        .map(|table| table.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let verb = if dry_run { "would write" } else { "wrote" };
    let mut summary = format!("{verb} {} sheet(s): {names}", tables.len());

    if !skipped.is_empty() {
        let details = skipped
            .iter()
            .map(|table| {
                let label = table
                    .caption
                    .clone()
                    .filter(|caption| !caption.trim().is_empty())
                    .unwrap_or_else(|| format!("table-{}", table.table));
                format!("{label} ({})", table.reason)
            })
            .collect::<Vec<_>>()
            .join("; ");
        summary.push_str(&format!("; skipped {details}"));
    }

    summary
}

/// Apply `--sheet-name`/`--sheet-name-template`, then sanitise and deduplicate
/// every worksheet name. Naming events are attached to the affected table so
/// they are visible in normal write diagnostics.
#[cfg(feature = "excel")]
fn name_workbook_sheets(tables: &mut [Table], args: &ConvertArgs) -> Result<()> {
    if let Some(name) = &args.sheet_name {
        if tables.len() != 1 {
            return Err(Error::usage(format!(
                "--sheet-name requires exactly one output sheet, but {} were selected",
                tables.len()
            )));
        }
        tables[0].name = name.clone();
    } else if let Some(template) = &args.sheet_name_template {
        for (index, table) in tables.iter_mut().enumerate() {
            let source = table.source.path.to_string_lossy();
            let stem = input_stem(&source);
            table.name = render_sheet_name_template(template, &stem, &table.name, index + 1)?;
        }
    }

    let mut used = HashSet::<String>::new();
    for table in tables.iter_mut() {
        let original = table.name.clone();
        let sanitized = formats::workbook_write::sanitize_sheet_name(&original);
        let final_name = formats::workbook_write::unique_sheet_name(&sanitized, &mut used);
        if final_name != original {
            let message = if final_name == sanitized {
                format!(
                    "sheet name '{original}' was sanitized to '{final_name}' for workbook output"
                )
            } else {
                format!(
                    "sheet name '{original}' collided or exceeded 31 characters; wrote '{final_name}'"
                )
            };
            table.push_warning(
                LossEvent::info("output.sheet_name_sanitized", message)
                    .with_source(table.source.clone()),
            );
        }
        table.name = final_name;
    }

    Ok(())
}

#[cfg(feature = "excel")]
fn render_sheet_name_template(
    template: &str,
    stem: &str,
    sheet: &str,
    index: usize,
) -> Result<String> {
    let rendered = template
        .replace("{stem}", stem)
        .replace("{sheet}", sheet)
        .replace("{name}", sheet)
        .replace("{index}", &index.to_string());

    if rendered.contains('{') || rendered.contains('}') {
        return Err(Error::usage(format!(
            "unknown placeholder in --sheet-name-template '{template}' (supported: {{stem}}, {{sheet}}, {{name}}, {{index}})"
        )));
    }
    if rendered.trim().is_empty() {
        return Err(Error::usage(
            "--sheet-name-template produced an empty sheet name",
        ));
    }

    Ok(rendered)
}

fn write_split(
    tables: &mut TableSet,
    output: &Path,
    args: &ConvertArgs,
    input: &str,
    diagnostics: &DiagnosticsPolicy,
    context: ReportContext,
) -> Result<()> {
    if tables.tables.is_empty() {
        return Err(Error::data("no tables were selected to write"));
    }

    let template = args
        .name_template
        .as_deref()
        .unwrap_or("{stem}.{sheet}.{format}");
    let format = split_output_format(template)?;
    let stem = input_stem(input);
    let mut used = HashSet::<String>::new();
    let mut plans = Vec::with_capacity(tables.tables.len());

    for (index, table) in tables.tables.iter_mut().enumerate() {
        let sanitized = sanitize_sheet_name(&table.name);
        if sanitized != table.name {
            table.push_warning(
                crate::model::LossEvent::info(
                    "output.sheet_name_sanitized",
                    format!(
                        "sheet name '{}' was sanitized to '{}' for the output filename",
                        table.name, sanitized
                    ),
                )
                .with_source(table.source.clone()),
            );
        }

        let rendered = render_name_template(template, &stem, &sanitized, format, index + 1)?;
        let relative = unique_relative_path(Path::new(&rendered), &mut used);
        if relative != Path::new(&rendered) {
            table.push_warning(
                crate::model::LossEvent::info(
                    "output.sheet_name_sanitized",
                    format!(
                        "sheet name '{}' collided with an earlier sheet; wrote '{}'",
                        table.name,
                        relative.display()
                    ),
                )
                .with_source(table.source.clone()),
            );
        }

        plans.push((index, output.join(relative)));
    }

    let protected = plans
        .iter()
        .map(|(_, destination)| ("output", destination.as_path()))
        .collect::<Vec<_>>();
    context.validate_paths(&protected)?;

    for table in &mut tables.tables {
        report::push_null_rendering_warning(table, format, &args.write.null);
    }
    let events = table_events(&tables.tables);
    let include_fidelity =
        diagnostics.fidelity_report || is_workbook_output(format) || args.reporting.strict;
    if context.aggregates_diagnostics(diagnostics) {
        context.print_event_summary(
            &events,
            diagnostics,
            DiagnosticsScope::Write,
            include_fidelity,
        );
    } else {
        for table in &tables.tables {
            print_warnings(
                table,
                diagnostics,
                DiagnosticsScope::Write,
                include_fidelity,
            );
        }
    }
    context.check_strict(&events)?;

    let mut options = formats::WriteOptions::for_format(format);
    options.has_header = !args.read.no_header;
    options.line_ending = args.write.line_ending;
    options.null = args.write.null.clone();
    let read_options = args.read.options()?;

    if args.write.dry_run {
        for (index, destination) in &plans {
            let table = &tables.tables[*index];
            print_dry_run(&destination.display().to_string(), table, format);
        }
        context.write_loss(&events, &[])?;
        context.print_dry_run_reports(&events, diagnostics);
        return Ok(());
    }

    // Stage every output before committing any of them. A failure while writing
    // one file drops the other staged files instead of leaving a partial set.
    let mut staged = Vec::with_capacity(plans.len());
    for (index, destination) in &plans {
        let table = &tables.tables[*index];
        let mut writer = SafeWriter::create(destination, args.write.force, args.write.backup)?;
        formats::write_table(&mut writer, table, format, &options)?;
        staged.push((destination.clone(), writer));
    }

    let mut written = Vec::with_capacity(staged.len());
    for (destination, writer) in staged {
        match writer.commit() {
            Ok(()) => written.push(destination),
            Err(error) => {
                let completed = written
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                let completed = if completed.is_empty() {
                    "none".to_owned()
                } else {
                    completed
                };
                return Err(Error::Partial(format!(
                    "split commit failed for '{}': {error}; committed outputs: {completed}",
                    destination.display()
                )));
            }
        }
    }

    let mut outputs = Vec::with_capacity(plans.len());
    for (index, destination) in &plans {
        let table = &tables.tables[*index];
        context.verify_file(
            destination,
            format,
            std::slice::from_ref(table),
            &read_options,
            &args.write.null,
        )?;
        outputs.push(output_artifact(destination, format)?);
    }
    context.write_loss(&events, &outputs)?;
    context.write_manifest(&events, &outputs, convert_manifest_options(args, format))?;
    if diagnostics.enabled() {
        let paths = written
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!(
            "tabmux: wrote {} output(s): {}{}",
            written.len(),
            paths,
            context.summary_suffix(&events)
        );
    }
    Ok(())
}

/// Determine the split-output format from the filename template.
fn split_output_format(template: &str) -> Result<FormatId> {
    let extension = Path::new(template)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();

    if extension.is_empty() || extension.contains('{') {
        return Ok(FormatId::Csv);
    }

    match FormatId::from_extension(extension) {
        Some(format @ (FormatId::Csv | FormatId::Tsv)) => Ok(format),
        Some(format) => Err(Error::unsupported(format!(
            "writing {} from --split-sheets is not implemented yet",
            format.label()
        ))),
        None => Err(Error::unsupported(format!(
            "cannot detect a split-output format from '{template}' (use .csv or .tsv)"
        ))),
    }
}

/// Expand the supported placeholders in a split-output filename template.
fn render_name_template(
    template: &str,
    stem: &str,
    sheet: &str,
    format: FormatId,
    index: usize,
) -> Result<String> {
    let rendered = template
        .replace("{stem}", stem)
        .replace("{sheet}", sheet)
        .replace("{name}", sheet)
        .replace("{format}", format.label())
        .replace("{ext}", format.label())
        .replace("{index}", &index.to_string());

    if rendered.contains('{') || rendered.contains('}') {
        return Err(Error::usage(format!(
            "unknown placeholder in --name-template '{template}' (supported: {{stem}}, {{sheet}}, {{format}}, {{ext}}, {{index}})"
        )));
    }
    if rendered.trim().is_empty() {
        return Err(Error::usage("--name-template produced an empty filename"));
    }

    let path = Path::new(&rendered);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::RootDir | Component::ParentDir | Component::Prefix(_)
            )
        })
    {
        return Err(Error::usage(format!(
            "--name-template must produce a relative path inside the output directory, got '{rendered}'"
        )));
    }

    Ok(rendered)
}

/// Add a stable numeric suffix when two sheets render to the same path.
fn unique_relative_path(candidate: &Path, used: &mut HashSet<String>) -> PathBuf {
    let mut candidate = candidate.to_path_buf();
    let mut suffix = 2;
    loop {
        let key = candidate.to_string_lossy().to_lowercase();
        if used.insert(key) {
            return candidate;
        }

        candidate = path_with_suffix(candidate, suffix);
        suffix += 1;
    }
}

fn path_with_suffix(path: PathBuf, suffix: usize) -> PathBuf {
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("sheet");
    let extension = path.extension().and_then(|ext| ext.to_str());
    let stem = extension
        .map(|ext| file_name.trim_end_matches(&format!(".{ext}")))
        .unwrap_or(file_name);

    match extension {
        Some(extension) => parent.join(format!("{stem}-{suffix}.{extension}")),
        None => parent.join(format!("{stem}-{suffix}")),
    }
}

/// Concatenate selected sheets into one table, unioning columns by name.
fn stack_tables(tables: Vec<Table>, source_column: Option<&str>) -> Result<Table> {
    let Some(first) = tables.first() else {
        return Err(Error::data("no tables were selected to stack"));
    };

    let mut names = Vec::<String>::new();
    for table in &tables {
        for column in &table.columns {
            if !names.iter().any(|name| name == &column.name) {
                names.push(column.name.clone());
            }
        }
    }

    if let Some(source_column) = source_column
        && names.iter().any(|name| name == source_column)
    {
        return Err(Error::usage(format!(
            "--add-source-column '{source_column}' conflicts with an existing column"
        )));
    }

    let mut types = vec![ColumnType::Null; names.len()];
    for table in &tables {
        for (index, name) in names.iter().enumerate() {
            if let Some(column) = table.column_index(name) {
                types[index] = types[index].merge(table.columns[column].ty);
            }
        }
    }

    let mut rows = Vec::new();
    for table in &tables {
        for row in &table.rows {
            let mut stacked =
                Vec::with_capacity(names.len() + usize::from(source_column.is_some()));
            for name in &names {
                stacked.push(
                    table
                        .column_index(name)
                        .and_then(|column| row.get(column))
                        .cloned()
                        .unwrap_or(CellValue::Null),
                );
            }
            if source_column.is_some() {
                stacked.push(CellValue::Text(table.name.clone()));
            }
            rows.push(stacked);
        }
    }

    let mut columns = names
        .into_iter()
        .zip(types)
        .map(|(name, ty)| Column::new(name, ty))
        .collect::<Vec<_>>();
    if let Some(source_column) = source_column {
        columns.push(Column::new(source_column, ColumnType::Text));
    }

    let mut warnings = Vec::new();
    for table in &tables {
        warnings.extend(table.warnings.iter().cloned());
    }

    Ok(Table {
        name: first.name.clone(),
        source: first.source.clone(),
        columns,
        rows,
        warnings,
        first_data_row: first.first_data_row,
    })
}

fn sheets(args: SheetsArgs) -> Result<()> {
    let bytes = crate::io::read_input(&args.input)?;
    let format = detect_input_format(&args.input, &bytes)?;
    let source = SourceRef::new(&args.input, format);
    let names = formats::sheet_names(&source, &bytes, format)?;
    for name in names {
        println!("{name}");
    }
    Ok(())
}

fn count(args: CountArgs, diagnostics: DiagnosticsPolicy) -> Result<()> {
    let (table, _) = load_table(&args.input, &args.read)?;
    print_warnings(
        &table,
        &diagnostics,
        DiagnosticsScope::ReadOnly,
        diagnostics.fidelity_report,
    );
    println!("{}", table.row_count());
    Ok(())
}

fn head(args: HeadArgs, diagnostics: DiagnosticsPolicy) -> Result<()> {
    let (mut table, _) = load_table(&args.input, &args.read)?;
    table.rows.truncate(args.rows);
    let rows = (0..table.row_count()).collect::<Vec<_>>();
    print_warnings_for_rows(
        &table,
        &rows,
        &diagnostics,
        DiagnosticsScope::ReadOnly,
        diagnostics.fidelity_report,
    );
    write_stdout_table(&table, !args.read.no_header, args.display.is_pretty())
}

fn select(args: SelectArgs, diagnostics: DiagnosticsPolicy) -> Result<()> {
    let (table, _) = load_table(&args.input, &args.read)?;
    let selected = select::select_columns(&table, &args.columns)?;
    print_warnings(
        &selected,
        &diagnostics,
        DiagnosticsScope::ReadOnly,
        diagnostics.fidelity_report,
    );
    write_stdout_table(&selected, !args.read.no_header, args.display.is_pretty())
}

fn row(args: RowArgs, diagnostics: DiagnosticsPolicy) -> Result<()> {
    let (table, _) = load_table(&args.input, &args.read)?;

    let (indices, rows) = if let Some(number) = args.number {
        let index = row_index(number, table.row_count())?;
        (vec![index], vec![table.rows[index].clone()])
    } else {
        let expression = args.where_expr.as_deref().unwrap_or_default();
        let predicate = filter::parse(expression)?;
        if !predicate.is_valid_for(&table) {
            return Err(Error::data(format!(
                "unknown column '{}' in --where; available columns: {}",
                predicate.column,
                table.column_names().join(", ")
            )));
        }
        let mut indices = Vec::new();
        for (index, row) in table.rows.iter().enumerate() {
            if predicate.matches(&table, row, table.source_row(index))? {
                indices.push(index);
            }
        }
        if indices.is_empty() {
            return Err(Error::data(format!(
                "no row in {} matches '{expression}'",
                table.source.path.display()
            )));
        }
        let rows = indices
            .iter()
            .map(|&index| table.rows[index].clone())
            .collect();
        (indices, rows)
    };

    print_warnings_for_rows(
        &table,
        &indices,
        &diagnostics,
        DiagnosticsScope::ReadOnly,
        diagnostics.fidelity_report,
    );

    let selected = Table {
        name: table.name.clone(),
        source: table.source.clone(),
        columns: table.columns.clone(),
        rows,
        warnings: Vec::new(),
        first_data_row: table.first_data_row,
    };
    if args.display.is_pretty() {
        write_stdout_pretty(&selected, !args.read.no_header)
    } else {
        write_stdout_records(&selected)
    }
}

fn cell(args: CellArgs, diagnostics: DiagnosticsPolicy) -> Result<()> {
    let (table, _) = load_table(&args.input, &args.read)?;
    let row = row_index(args.row, table.row_count())?;
    let column = resolve_column(&table, &args.column)?;
    let value = table.rows[row]
        .get(column)
        .cloned()
        .unwrap_or(CellValue::Null);

    print_warnings_for_rows(
        &table,
        &[row],
        &diagnostics,
        DiagnosticsScope::ReadOnly,
        diagnostics.fidelity_report,
    );

    if args.display.is_pretty() {
        let name = table.columns[column].name.clone();
        let ty = table.columns[column].ty;
        let selected = Table::with_columns(
            "cell",
            table.source.clone(),
            vec![Column::new(name, ty)],
            vec![vec![value]],
        );
        return write_stdout_pretty(&selected, true);
    }

    let mut rendered = value.to_text("");
    rendered.push('\n');
    crate::io::write_stdout(rendered.as_bytes())
}

fn merge_tables(args: MergeArgs, diagnostics: DiagnosticsPolicy) -> Result<()> {
    reject_self_overwrite(&args.inputs, &args.output)?;

    let mut tables = Vec::with_capacity(args.inputs.len());
    for input in &args.inputs {
        let (set, _) = load_tables(input, &args.read)?;
        if set.tables.len() != 1 {
            return Err(Error::usage(format!(
                "merge input '{}' selected {} tables; select exactly one sheet with --sheet",
                input,
                set.tables.len()
            )));
        }
        tables.push(set.into_single()?);
    }

    if args.mode == MergeModeArg::Strict {
        let diffs = merge::strict_diffs(&tables, args.case_insensitive_headers);
        if !diffs.is_empty() {
            if args.json {
                let value = serde_json::json!({
                    "version": 1,
                    "mode": "strict",
                    "diffs": diffs.iter().map(schema::SchemaDiff::to_json).collect::<Vec<_>>(),
                });
                let rendered = serde_json::to_string_pretty(&value)
                    .unwrap_or_else(|_| "{\"version\":1}".to_owned());
                crate::io::write_stdout(format!("{rendered}\n").as_bytes())?;
            } else {
                for diff in &diffs {
                    eprint!("{}", diff.render_text());
                }
            }
            return Err(Error::data(format!(
                "strict merge found schema differences in {} input(s); no output was written",
                diffs.len()
            )));
        }
    }

    let first_format = tables
        .first()
        .map(|table| table.source.format)
        .unwrap_or(FormatId::Csv);
    let mut merged = merge::merge(
        tables,
        &merge::MergeOptions {
            source_column: args.add_source_column.clone(),
            dedup: args.dedup,
            case_insensitive_headers: args.case_insensitive_headers,
        },
    )?;

    let output_format = if args.output == "-" {
        match first_format {
            FormatId::Csv | FormatId::Tsv => first_format,
            _ => FormatId::Csv,
        }
    } else {
        detect_supported_output(&args.output)?
    };

    let context = ReportContext::new(
        &args.reporting,
        input_artifacts(&args.inputs)?,
        args.write.force,
    );
    report::push_null_rendering_warning(&mut merged, output_format, &args.write.null);
    let events = table_events(std::slice::from_ref(&merged));
    let protected = if args.output == "-" {
        Vec::new()
    } else {
        vec![("output", Path::new(&args.output))]
    };
    context.validate_paths(&protected)?;

    let include_fidelity =
        diagnostics.fidelity_report || is_workbook_output(output_format) || args.reporting.strict;
    if context.aggregates_diagnostics(&diagnostics) {
        context.print_event_summary(
            &events,
            &diagnostics,
            DiagnosticsScope::Write,
            include_fidelity,
        );
    } else {
        print_warnings(
            &merged,
            &diagnostics,
            DiagnosticsScope::Write,
            include_fidelity,
        );
    }
    context.check_strict(&events)?;

    let mut options = formats::WriteOptions::for_format(output_format);
    options.has_header = !args.read.no_header;
    options.line_ending = args.write.line_ending;
    options.null = args.write.null.clone();
    let read_options = args.read.options()?;

    if args.output == "-" {
        if args.write.dry_run {
            print_dry_run("stdout", &merged, output_format);
            context.write_loss(&events, &[])?;
            context.print_dry_run_reports(&events, &diagnostics);
            return Ok(());
        }
        let mut buffer = Vec::new();
        formats::write_table(&mut buffer, &merged, output_format, &options)?;
        context.verify_bytes(
            "stdout",
            output_format,
            std::slice::from_ref(&merged),
            &read_options,
            &buffer,
            &args.write.null,
        )?;
        let outputs = vec![Artifact::from_bytes("-", output_format, &buffer)];
        crate::io::write_stdout(&buffer)?;
        context.write_loss(&events, &outputs)?;
        context.write_manifest(
            &events,
            &outputs,
            merge_manifest_options(&args, output_format),
        )?;
        if diagnostics.enabled() {
            eprintln!(
                "tabmux: wrote 1 table ({} rows x {} columns) to stdout{}",
                merged.row_count(),
                merged.width(),
                context.summary_suffix(&events)
            );
        }
        return Ok(());
    }

    let destination = Path::new(&args.output);
    if args.write.dry_run {
        print_dry_run(&args.output, &merged, output_format);
        context.write_loss(&events, &[])?;
        context.print_dry_run_reports(&events, &diagnostics);
        return Ok(());
    }

    let mut writer = SafeWriter::create(destination, args.write.force, args.write.backup)?;
    if is_workbook_output(output_format) {
        formats::write_tables(
            writer.file_mut(),
            std::slice::from_ref(&merged),
            output_format,
            &options,
        )?;
    } else {
        formats::write_table(&mut writer, &merged, output_format, &options)?;
    }
    writer.commit()?;

    let outputs = vec![output_artifact(destination, output_format)?];
    context.verify_file(
        destination,
        output_format,
        std::slice::from_ref(&merged),
        &read_options,
        &args.write.null,
    )?;
    context.write_loss(&events, &outputs)?;
    context.write_manifest(
        &events,
        &outputs,
        merge_manifest_options(&args, output_format),
    )?;
    if diagnostics.enabled() {
        eprintln!(
            "tabmux: wrote 1 table ({} rows x {} columns) to {}{}",
            merged.row_count(),
            merged.width(),
            destination.display(),
            context.summary_suffix(&events)
        );
    }
    Ok(())
}

fn filter_rows(args: FilterArgs, diagnostics: DiagnosticsPolicy) -> Result<()> {
    let (table, _) = load_table(&args.input, &args.read)?;
    let predicate = filter::parse(&args.where_expr)?;
    let filtered = filter::filter(&table, &predicate)?;
    print_warnings(
        &filtered,
        &diagnostics,
        DiagnosticsScope::ReadOnly,
        diagnostics.fidelity_report,
    );
    write_stdout_table(&filtered, !args.read.no_header, args.display.is_pretty())
}

fn sort_rows(args: SortArgs, diagnostics: DiagnosticsPolicy) -> Result<()> {
    let (table, _) = load_table(&args.input, &args.read)?;
    let keys = sort::parse_keys(&args.by)?;
    let sorted = sort::sort(&table, &keys)?;
    print_warnings(
        &sorted,
        &diagnostics,
        DiagnosticsScope::ReadOnly,
        diagnostics.fidelity_report,
    );
    write_stdout_table(&sorted, !args.read.no_header, args.display.is_pretty())
}

fn rename_columns(args: RenameArgs, diagnostics: DiagnosticsPolicy) -> Result<()> {
    let (table, _) = load_table(&args.input, &args.read)?;
    let mappings = rename::parse_mappings(&args.map)?;
    let renamed = rename::rename(&table, &mappings)?;
    print_warnings(
        &renamed,
        &diagnostics,
        DiagnosticsScope::ReadOnly,
        diagnostics.fidelity_report,
    );
    write_stdout_table(&renamed, !args.read.no_header, args.display.is_pretty())
}

fn dedupe_rows(args: DedupeArgs, diagnostics: DiagnosticsPolicy) -> Result<()> {
    let (table, _) = load_table(&args.input, &args.read)?;
    let deduped = dedupe::dedupe(&table);
    print_warnings(
        &deduped,
        &diagnostics,
        DiagnosticsScope::ReadOnly,
        diagnostics.fidelity_report,
    );
    write_stdout_table(&deduped, !args.read.no_header, args.display.is_pretty())
}

fn row_index(number: usize, row_count: usize) -> Result<usize> {
    if number == 0 {
        return Err(Error::usage("row numbers are 1-based; 0 is not valid"));
    }
    if number > row_count {
        return Err(Error::data(format!(
            "row {number} is out of range; the table has {row_count} data row(s)"
        )));
    }
    Ok(number - 1)
}

fn resolve_column(table: &Table, specification: &str) -> Result<usize> {
    if let Ok(number) = specification.parse::<usize>() {
        if number == 0 {
            return Err(Error::usage("column indices are 1-based; 0 is not valid"));
        }
        if number > table.width() {
            return Err(Error::data(format!(
                "column index {number} is out of range; the table has {} columns",
                table.width()
            )));
        }
        return Ok(number - 1);
    }

    table.column_index(specification).ok_or_else(|| {
        Error::data(format!(
            "unknown column '{specification}'; available columns: {}",
            table.column_names().join(", ")
        ))
    })
}

fn load_tables(path: &str, read: &ReadArgs) -> Result<(TableSet, FormatId)> {
    let bytes = crate::io::read_input(path)?;
    let format = detect_input_format(path, &bytes)?;
    let source = SourceRef::new(path, format);
    let tables = formats::read_tables(&source, &bytes, format, &read.options()?)?;
    Ok((tables, format))
}

fn load_table(path: &str, read: &ReadArgs) -> Result<(Table, FormatId)> {
    let (tables, format) = load_tables(path, read)?;
    let table = tables.into_single()?;
    Ok((table, format))
}

fn is_workbook_output(format: FormatId) -> bool {
    matches!(format, FormatId::Xlsx | FormatId::Ods)
}

fn input_stem(input: &str) -> String {
    if input == "-" {
        return "stdin".to_owned();
    }
    Path::new(input)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("table")
        .to_owned()
}

fn sanitize_sheet_name(name: &str) -> String {
    let mut sanitized = String::with_capacity(name.len());
    for character in name.chars() {
        if character.is_control()
            || matches!(
                character,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            )
        {
            sanitized.push('_');
        } else {
            sanitized.push(character);
        }
    }
    let trimmed = sanitized.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        "sheet".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn detect_input_format(path: &str, bytes: &[u8]) -> Result<FormatId> {
    if let Some(format) = formats::detect_path(Path::new(path)) {
        return Ok(format);
    }

    if path == "-" {
        return sniff_content(bytes);
    }

    Err(Error::unsupported(format!(
        "cannot detect input format for '{path}' (supported: csv, tsv, xlsx, xls, xlsb, ods, html, json, jsonl)"
    )))
}

fn sniff_content(bytes: &[u8]) -> Result<FormatId> {
    if bytes.starts_with(b"PK\x03\x04") {
        return Err(Error::unsupported(
            "workbook input from stdin is not supported; pass a file path so the container can be identified",
        ));
    }

    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if let Ok(text) = std::str::from_utf8(bytes) {
        let trimmed = text.trim_start();
        let lower = trimmed.to_ascii_lowercase();
        if trimmed.starts_with('<')
            && (lower.starts_with("<!doctype html")
                || lower.starts_with("<html")
                || lower.contains("<table"))
        {
            return Ok(FormatId::Html);
        }
    }

    Ok(
        match bytes
            .iter()
            .copied()
            .find(|byte| !byte.is_ascii_whitespace())
        {
            Some(b'{') | Some(b'[') => FormatId::Json,
            _ => FormatId::Csv,
        },
    )
}

fn detect_supported_output(output: &str) -> Result<FormatId> {
    let extension = Path::new(output)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match extension.as_str() {
        "csv" => Ok(FormatId::Csv),
        "tsv" | "tab" => Ok(FormatId::Tsv),
        "xlsx" => Ok(FormatId::Xlsx),
        "ods" => Ok(FormatId::Ods),
        "json" | "jsonl" | "ndjson" | "html" | "htm" => Err(Error::unsupported(format!(
            "writing {extension} is not implemented yet"
        ))),
        "xls" | "xlsb" | "xlsm" => Err(Error::unsupported(
            "writing legacy or macro-enabled Excel containers is not supported; use .xlsx",
        )),
        _ => Err(Error::unsupported(format!(
            "cannot detect output format for '{output}' (supported outputs: csv, tsv, xlsx, ods)"
        ))),
    }
}

fn delimited_output_format(
    input_format: FormatId,
    output: &str,
    requested_format: Option<FormatId>,
) -> Result<FormatId> {
    if let Some(format) = requested_format {
        return Ok(format);
    }

    if output == "-" {
        return Ok(match input_format {
            FormatId::Csv | FormatId::Tsv => input_format,
            _ => FormatId::Csv,
        });
    }

    Err(Error::unsupported(format!(
        "cannot detect output format for '{output}' (supported outputs: csv, tsv, xlsx, ods)"
    )))
}

fn write_stdout_table(table: &Table, has_header: bool, pretty: bool) -> Result<()> {
    if pretty {
        return write_stdout_pretty(table, has_header);
    }

    let format = delimited_format(table);
    let mut options = formats::WriteOptions::for_format(format);
    options.has_header = has_header;

    let mut buffer = Vec::new();
    formats::write_table(&mut buffer, table, format, &options)?;
    crate::io::write_stdout(&buffer)
}

/// Write data records without a header (used by `row` for piping).
fn write_stdout_records(table: &Table) -> Result<()> {
    let format = delimited_format(table);
    let mut options = formats::WriteOptions::for_format(format);
    options.has_header = false;

    let mut buffer = Vec::new();
    formats::write_table(&mut buffer, table, format, &options)?;
    crate::io::write_stdout(&buffer)
}

fn delimited_format(table: &Table) -> FormatId {
    match table.source.format {
        FormatId::Csv | FormatId::Tsv => table.source.format,
        _ => FormatId::Csv,
    }
}

fn write_stdout_pretty(table: &Table, has_header: bool) -> Result<()> {
    let rendered = crate::render::render_pretty(table, has_header);
    crate::io::write_stdout(rendered.as_bytes())
}

fn print_warnings(
    table: &Table,
    diagnostics: &DiagnosticsPolicy,
    scope: DiagnosticsScope,
    include_fidelity: bool,
) {
    print_warnings_filtered(table, None, diagnostics, scope, include_fidelity);
}

/// Print diagnostics for a row-filtered command. Per-row events for rows that
/// are not part of the output are hidden unless `--debug` is active.
fn print_warnings_for_rows(
    table: &Table,
    rows: &[usize],
    diagnostics: &DiagnosticsPolicy,
    scope: DiagnosticsScope,
    include_fidelity: bool,
) {
    print_warnings_filtered(table, Some(rows), diagnostics, scope, include_fidelity);
}

fn print_warnings_filtered(
    table: &Table,
    rows: Option<&[usize]>,
    diagnostics: &DiagnosticsPolicy,
    scope: DiagnosticsScope,
    include_fidelity: bool,
) {
    for warning in &table.warnings {
        if !diagnostics.shows(warning, scope, include_fidelity) {
            continue;
        }

        if let Some(rows) = rows
            && !diagnostics.debug
            && let Some(source_row) = warning.source.as_ref().and_then(|source| source.row)
            && !rows
                .iter()
                .any(|&index| table.source_row(index) == source_row)
        {
            continue;
        }

        print_event(warning);
    }
}

pub(crate) fn print_event(warning: &LossEvent) {
    match &warning.source {
        Some(source) => eprintln!(
            "tabmux: {}: {}: {} [{}]",
            warning.severity.as_str(),
            source.context(),
            warning.message,
            warning.code
        ),
        None => eprintln!(
            "tabmux: {}: {} [{}]",
            warning.severity.as_str(),
            warning.message,
            warning.code
        ),
    }
}

fn print_dry_run(destination: &str, table: &Table, format: FormatId) {
    eprintln!(
        "tabmux: dry-run: would write {destination} ({} rows x {} columns) as {}",
        table.row_count(),
        table.width(),
        format.label()
    );
}

fn convert_manifest_options(args: &ConvertArgs, format: FormatId) -> serde_json::Value {
    serde_json::json!({
        "command": "convert",
        "output_format": format.label(),
        "strict": args.reporting.strict,
        "lossy": args.reporting.lossy,
        "max_warnings": args.reporting.max_warnings,
        "verify": args.reporting.verify,
        "force": args.write.force,
        "backup": args.write.backup,
        "dry_run": args.write.dry_run,
        "no_header": args.read.no_header,
        "split_sheets": args.split_sheets,
        "stack": args.stack,
        "all_sheets": args.read.sheets.all_sheets,
        "sheet": args.read.sheets.sheet.as_deref(),
        "sheet_index": args.read.sheets.sheet_index,
        "tables": args.read.sheets.tables.as_deref(),
    })
}

fn merge_manifest_options(args: &MergeArgs, format: FormatId) -> serde_json::Value {
    serde_json::json!({
        "command": "merge",
        "output_format": format.label(),
        "mode": match args.mode {
            MergeModeArg::Strict => "strict",
            MergeModeArg::Loose => "loose",
        },
        "strict": args.reporting.strict,
        "lossy": args.reporting.lossy,
        "max_warnings": args.reporting.max_warnings,
        "verify": args.reporting.verify,
        "force": args.write.force,
        "backup": args.write.backup,
        "dry_run": args.write.dry_run,
        "dedup": args.dedup,
        "add_source_column": args.add_source_column.as_deref(),
    })
}

fn reject_self_overwrite(inputs: &[String], output: &str) -> Result<()> {
    if output == "-" {
        return Ok(());
    }

    let Some(output) = std::fs::canonicalize(output).ok() else {
        return Ok(());
    };

    for input in inputs {
        if input == "-" {
            continue;
        }
        if let Some(input) = std::fs::canonicalize(input).ok()
            && input == output
        {
            return Err(Error::usage(format!(
                "input and output are the same file: {}",
                input.display()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ColumnType;

    fn table() -> Table {
        let source = SourceRef::new("data.csv", FormatId::Csv);
        Table::with_columns(
            "data",
            source,
            vec![
                Column::new("name", ColumnType::Text),
                Column::new("amount", ColumnType::Int),
            ],
            vec![
                vec![CellValue::Text("a".into()), CellValue::Int(1)],
                vec![CellValue::Text("b".into()), CellValue::Int(2)],
            ],
        )
    }

    #[test]
    fn cli_definition_is_valid() {
        use clap::CommandFactory;

        Cli::command().debug_assert();
    }

    #[test]
    fn select_accepts_names_and_indices() {
        let selected = select::select_columns(&table(), "amount").unwrap();
        assert_eq!(selected.column_names(), ["amount"]);

        let selected = select::select_columns(&table(), "2,1").unwrap();
        assert_eq!(selected.column_names(), ["amount", "name"]);
        assert_eq!(selected.rows[0][0], CellValue::Int(1));
        assert_eq!(selected.rows[0][1], CellValue::Text("a".into()));
    }

    #[test]
    fn select_reports_unknown_columns_with_available_names() {
        let error = select::select_columns(&table(), "missing").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("missing"));
        assert!(message.contains("name"));
    }

    #[test]
    fn select_rejects_zero_indices() {
        let error = select::select_columns(&table(), "0").unwrap_err();
        assert_eq!(error.exit_code(), crate::error::ExitCode::Usage);
    }
}
