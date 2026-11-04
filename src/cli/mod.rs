//! Command-line definitions and dispatch.

pub mod args;

use std::io::{self, Write as _};
use std::path::Path;

use clap::{Parser, Subcommand};

use crate::error::{Error, Result};
use crate::formats::{self, FormatId};
use crate::io::SafeWriter;
use crate::model::{CellValue, Column, Row, SourceRef, Table};

use self::args::{ConvertArgs, CountArgs, HeadArgs, ReadArgs, SelectArgs};

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
}

/// Available operations.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show the supported format matrix.
    Formats,
    /// Convert one input to one output.
    Convert(ConvertArgs),
    /// Count data rows in a table.
    Count(CountArgs),
    /// Print the first rows of a table.
    Head(HeadArgs),
    /// Select columns and print them to stdout.
    Select(SelectArgs),
}

/// Execute a parsed command.
pub fn dispatch(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Formats => print_matrix(),
        Command::Convert(args) => convert(args),
        Command::Count(args) => count(args),
        Command::Head(args) => head(args),
        Command::Select(args) => select(args),
    }
}

fn print_matrix() -> Result<()> {
    let matrix = formats::render_matrix();
    let mut stdout = io::stdout().lock();
    stdout.write_all(matrix.as_bytes()).map_err(Error::Io)?;
    stdout.flush().map_err(Error::Io)
}

fn convert(args: ConvertArgs) -> Result<()> {
    let output = args.resolve_output()?.to_owned();
    reject_self_overwrite(&args.input, &output)?;
    if output != "-" {
        SafeWriter::ensure_writable(Path::new(&output), args.write.force)?;
    }

    let (table, input_format) = load_table(&args.input, &args.read)?;
    let output_format = output_format(input_format, &output)?;

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
        SafeWriter::ensure_writable(destination, args.write.force)?;
        print_dry_run(&output, &table, output_format);
        return Ok(());
    }

    let mut writer = SafeWriter::create(destination, args.write.force, args.write.backup)?;
    formats::write_table(&mut writer, &table, output_format, &options)?;
    writer.commit()
}

fn count(args: CountArgs) -> Result<()> {
    let (table, _) = load_table(&args.input, &args.read)?;
    println!("{}", table.row_count());
    Ok(())
}

fn head(args: HeadArgs) -> Result<()> {
    let (mut table, _) = load_table(&args.input, &args.read)?;
    table.rows.truncate(args.rows);
    write_stdout_table(&table, !args.read.no_header)
}

fn select(args: SelectArgs) -> Result<()> {
    let (table, _) = load_table(&args.input, &args.read)?;
    let selected = select_columns(&table, &args.columns)?;
    write_stdout_table(&selected, !args.read.no_header)
}

fn load_table(path: &str, read: &ReadArgs) -> Result<(Table, FormatId)> {
    let bytes = crate::io::read_input(path)?;
    let format = detect_input_format(path, &bytes)?;
    let source = SourceRef::new(path, format);
    let table = formats::read_table(&source, &bytes, format, &read.options())?;
    print_warnings(&table);
    Ok((table, format))
}

fn detect_input_format(path: &str, bytes: &[u8]) -> Result<FormatId> {
    if let Some(format) = formats::detect_path(Path::new(path)) {
        return Ok(format);
    }

    if path == "-" {
        return Ok(sniff_content(bytes));
    }

    Err(Error::unsupported(format!(
        "cannot detect input format for '{path}' (supported: csv, tsv, json, jsonl)"
    )))
}

fn sniff_content(bytes: &[u8]) -> FormatId {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match bytes
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace())
    {
        Some(b'{') | Some(b'[') => FormatId::Json,
        _ => FormatId::Csv,
    }
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

fn write_stdout_table(table: &Table, has_header: bool) -> Result<()> {
    let format = match table.source.format {
        FormatId::Csv | FormatId::Tsv => table.source.format,
        _ => FormatId::Csv,
    };
    let mut options = formats::WriteOptions::for_format(format);
    options.has_header = has_header;

    let mut buffer = Vec::new();
    formats::write_table(&mut buffer, table, format, &options)?;
    crate::io::write_stdout(&buffer)
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
    })
}

fn print_warnings(table: &Table) {
    for warning in &table.warnings {
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
