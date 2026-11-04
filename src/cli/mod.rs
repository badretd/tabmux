//! Command-line definitions and dispatch.

pub mod args;

use std::collections::HashSet;
use std::io::{self, Write as _};
use std::path::{Component, Path, PathBuf};

use clap::{Parser, Subcommand};

use crate::diagnostics::{DiagnosticsPolicy, DiagnosticsScope};
use crate::error::{Error, Result};
use crate::formats::{self, FormatId};
use crate::io::SafeWriter;
use crate::model::{CellValue, Column, ColumnType, LossEvent, Row, SourceRef, Table, TableSet};

use self::args::{
    CellArgs, ConvertArgs, CountArgs, DiagnosticsArgs, HeadArgs, ReadArgs, RowArgs, SelectArgs,
    SheetsArgs,
};

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
    /// Convert one input to one output.
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
    }
}

fn print_matrix() -> Result<()> {
    let matrix = formats::render_matrix();
    let mut stdout = io::stdout().lock();
    stdout.write_all(matrix.as_bytes()).map_err(Error::Io)?;
    stdout.flush().map_err(Error::Io)
}

fn convert(args: ConvertArgs, diagnostics: DiagnosticsPolicy) -> Result<()> {
    let output = args.resolve_output()?.to_owned();
    reject_self_overwrite(&args.input, &output)?;

    if args.read.sheets.all_sheets && !args.split_sheets && !args.stack {
        return Err(Error::usage(
            "--all-sheets requires --split-sheets or --stack",
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

    if !args.split_sheets && output != "-" {
        SafeWriter::ensure_writable(Path::new(&output), args.write.force)?;
    }

    let (mut tables, input_format) = load_tables(&args.input, &args.read)?;

    if args.split_sheets {
        return write_split(&mut tables, Path::new(&output), &args, &diagnostics);
    }

    let table = if args.stack {
        stack_tables(
            std::mem::take(&mut tables.tables),
            args.add_source_column.as_deref(),
        )?
    } else {
        tables.into_single()?
    };
    let output_format = output_format(input_format, &output)?;
    let include_fidelity = diagnostics.fidelity_report || is_workbook_output(output_format);
    print_warnings(
        &table,
        &diagnostics,
        DiagnosticsScope::Write,
        include_fidelity,
    );

    let mut options = formats::WriteOptions::for_format(output_format);
    options.has_header = !args.read.no_header;
    options.line_ending = args.write.line_ending;
    options.null = args.write.null.clone();

    if output == "-" {
        if args.write.dry_run {
            print_dry_run("stdout", &table, output_format);
            return Ok(());
        }
        let mut buffer = Vec::new();
        formats::write_table(&mut buffer, &table, output_format, &options)?;
        return crate::io::write_stdout(&buffer);
    }

    let destination = Path::new(&output);
    if args.write.dry_run {
        print_dry_run(&output, &table, output_format);
        return Ok(());
    }

    let mut writer = SafeWriter::create(destination, args.write.force, args.write.backup)?;
    formats::write_table(&mut writer, &table, output_format, &options)?;
    writer.commit()
}

fn write_split(
    tables: &mut TableSet,
    output: &Path,
    args: &ConvertArgs,
    diagnostics: &DiagnosticsPolicy,
) -> Result<()> {
    if tables.tables.is_empty() {
        return Err(Error::data("no tables were selected to write"));
    }

    let template = args
        .name_template
        .as_deref()
        .unwrap_or("{stem}.{sheet}.{format}");
    let format = split_output_format(template)?;
    let stem = input_stem(&args.input);
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

    for (_, destination) in &plans {
        SafeWriter::ensure_writable(destination, args.write.force)?;
    }

    let mut options = formats::WriteOptions::for_format(format);
    options.has_header = !args.read.no_header;
    options.line_ending = args.write.line_ending;
    options.null = args.write.null.clone();
    let include_fidelity = diagnostics.fidelity_report || is_workbook_output(format);

    if args.write.dry_run {
        for (index, destination) in &plans {
            let table = &tables.tables[*index];
            print_dry_run(&destination.display().to_string(), table, format);
        }
        for table in &tables.tables {
            print_warnings(
                table,
                diagnostics,
                DiagnosticsScope::Write,
                include_fidelity,
            );
        }
        return Ok(());
    }

    for (index, destination) in &plans {
        let table = &tables.tables[*index];
        let mut writer = SafeWriter::create(destination, args.write.force, args.write.backup)?;
        formats::write_table(&mut writer, table, format, &options)?;
        writer.commit()?;
    }

    for table in &tables.tables {
        print_warnings(
            table,
            diagnostics,
            DiagnosticsScope::Write,
            include_fidelity,
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
    let selected = select_columns(&table, &args.columns)?;
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
        let predicate = parse_where(expression)?;
        if table.column_index(&predicate.column).is_none() {
            return Err(Error::data(format!(
                "unknown column '{}' in --where; available columns: {}",
                predicate.column,
                table.column_names().join(", ")
            )));
        }
        let mut indices = Vec::new();
        for (index, row) in table.rows.iter().enumerate() {
            if predicate.matches(&table, row) {
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

/// A parsed simple row filter (`column == value` or `column != value`).
struct WherePredicate {
    column: String,
    equals: bool,
    value: String,
}

impl WherePredicate {
    fn matches(&self, table: &Table, row: &Row) -> bool {
        let Some(index) = table.column_index(&self.column) else {
            return false;
        };
        let actual = row
            .get(index)
            .map(|cell| cell.to_text(""))
            .unwrap_or_default();
        let matched = actual == self.value;
        if self.equals { matched } else { !matched }
    }
}

fn parse_where(expression: &str) -> Result<WherePredicate> {
    let (column, equals, raw_value) = if let Some((column, value)) = expression.split_once("!=") {
        (column, false, value)
    } else if let Some((column, value)) = expression.split_once("==") {
        (column, true, value)
    } else if let Some((column, value)) = expression.split_once('=') {
        (column, true, value)
    } else {
        return Err(Error::usage(format!(
            "invalid --where '{expression}'; expected 'column == value' or 'column != value'"
        )));
    };

    let column = column.trim();
    if column.is_empty() {
        return Err(Error::usage(format!(
            "invalid --where '{expression}'; the column name is empty"
        )));
    }

    Ok(WherePredicate {
        column: column.to_owned(),
        equals,
        value: unquote(raw_value.trim()),
    })
}

fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
            return value[1..value.len() - 1].to_owned();
        }
    }
    value.to_owned()
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
    let tables = formats::read_tables(&source, &bytes, format, &read.options())?;
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

fn output_format(input_format: FormatId, output: &str) -> Result<FormatId> {
    if output == "-" {
        return Ok(match input_format {
            FormatId::Csv | FormatId::Tsv => input_format,
            _ => FormatId::Csv,
        });
    }

    let format = formats::detect_path(Path::new(output)).ok_or_else(|| {
        Error::unsupported(format!(
            "cannot detect output format for '{output}' (supported outputs: csv, tsv)"
        ))
    })?;

    if matches!(format, FormatId::Csv | FormatId::Tsv) {
        Ok(format)
    } else {
        Err(Error::unsupported(format!(
            "writing {} is not implemented yet",
            format.label()
        )))
    }
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
    let rendered = render_pretty(table, has_header);
    crate::io::write_stdout(rendered.as_bytes())
}

/// Render an aligned terminal table. Embedded newlines in cells become
/// additional physical lines so multiline values stay readable and aligned.
fn render_pretty(table: &Table, has_header: bool) -> String {
    let width = table.width();
    if width == 0 {
        return String::new();
    }

    let header = table
        .columns
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let rows = table
        .rows
        .iter()
        .map(|row| {
            (0..width)
                .map(|column| {
                    row.get(column)
                        .unwrap_or(&CellValue::Null)
                        .to_text("")
                        .split('\n')
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let mut widths = vec![1usize; width];
    if has_header {
        for (column, name) in header.iter().enumerate() {
            widths[column] = widths[column].max(name.chars().count());
        }
    }
    for row in &rows {
        for (column, lines) in row.iter().enumerate() {
            for line in lines {
                widths[column] = widths[column].max(line.chars().count());
            }
        }
    }

    let mut output = String::new();
    if has_header {
        let cells = header.iter().map(String::as_str).collect::<Vec<_>>();
        push_pretty_line(&mut output, &cells, &widths);
        let separator = widths
            .iter()
            .map(|width| "-".repeat(*width))
            .collect::<Vec<_>>()
            .join("-+-");
        output.push_str(&separator);
        output.push('\n');
    }

    for row in &rows {
        let height = row.iter().map(Vec::len).max().unwrap_or(1);
        for line_index in 0..height {
            let cells = row
                .iter()
                .map(|lines| lines.get(line_index).map(String::as_str).unwrap_or(""))
                .collect::<Vec<_>>();
            push_pretty_line(&mut output, &cells, &widths);
        }
    }

    output
}

fn push_pretty_line(output: &mut String, cells: &[&str], widths: &[usize]) {
    let mut line = String::new();
    for (column, cell) in cells.iter().enumerate() {
        if column > 0 {
            line.push_str(" | ");
        }
        line.push_str(cell);
        let padding = widths[column].saturating_sub(cell.chars().count());
        line.push_str(&" ".repeat(padding));
    }
    output.push_str(line.trim_end());
    output.push('\n');
}

fn select_columns(table: &Table, specification: &str) -> Result<Table> {
    let mut indices = Vec::new();

    for raw in specification.split(',') {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }

        let index = match raw.parse::<usize>() {
            Ok(0) => {
                return Err(Error::usage("column indices are 1-based; 0 is not valid"));
            }
            Ok(number) => {
                let index = number - 1;
                if index >= table.width() {
                    return Err(Error::data(format!(
                        "column index {number} is out of range; the table has {} columns",
                        table.width()
                    )));
                }
                index
            }
            Err(_) => table.column_index(raw).ok_or_else(|| {
                Error::data(format!(
                    "unknown column '{raw}'; available columns: {}",
                    table.column_names().join(", ")
                ))
            })?,
        };

        indices.push(index);
    }

    if indices.is_empty() {
        return Err(Error::usage(
            "select --columns must name at least one column",
        ));
    }

    let columns = indices
        .iter()
        .map(|&index| table.columns[index].clone())
        .collect::<Vec<Column>>();
    let rows = table
        .rows
        .iter()
        .map(|row| {
            indices
                .iter()
                .map(|&index| row.get(index).cloned().unwrap_or(CellValue::Null))
                .collect::<Row>()
        })
        .collect::<Vec<Row>>();

    Ok(Table {
        name: table.name.clone(),
        source: table.source.clone(),
        columns,
        rows,
        warnings: table.warnings.clone(),
        first_data_row: table.first_data_row,
    })
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

fn print_event(warning: &LossEvent) {
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

fn reject_self_overwrite(input: &str, output: &str) -> Result<()> {
    if input == "-" || output == "-" {
        return Ok(());
    }

    let input = std::fs::canonicalize(input).ok();
    let output = std::fs::canonicalize(output).ok();
    if let (Some(input), Some(output)) = (input, output)
        && input == output
    {
        return Err(Error::usage(format!(
            "input and output are the same file: {}",
            input.display()
        )));
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
        let selected = select_columns(&table(), "amount").unwrap();
        assert_eq!(selected.column_names(), ["amount"]);

        let selected = select_columns(&table(), "2,1").unwrap();
        assert_eq!(selected.column_names(), ["amount", "name"]);
        assert_eq!(selected.rows[0][0], CellValue::Int(1));
        assert_eq!(selected.rows[0][1], CellValue::Text("a".into()));
    }

    #[test]
    fn select_reports_unknown_columns_with_available_names() {
        let error = select_columns(&table(), "missing").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("missing"));
        assert!(message.contains("name"));
    }

    #[test]
    fn select_rejects_zero_indices() {
        let error = select_columns(&table(), "0").unwrap_err();
        assert_eq!(error.exit_code(), crate::error::ExitCode::Usage);
    }
}
