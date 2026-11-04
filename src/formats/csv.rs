//! CSV/TSV reader and writer.

use std::io::Write;

use clap::ValueEnum;
use csv::{ErrorKind, Terminator};

use crate::error::{Error, Result};
use crate::io::decode_text;
use crate::model::{CellValue, Column, ColumnType, LossEvent, SourceRef, Table};

/// How to handle records whose field count differs from the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RaggedPolicy {
    /// Fail the conversion (default).
    Error,
    /// Pad short rows with empty cells and truncate extra fields with a warning.
    Pad,
    /// Skip the offending row with a warning.
    Skip,
}

/// Output line endings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LineEnding {
    Lf,
    Crlf,
}

/// Input options for delimited text.
#[derive(Debug, Clone)]
pub struct ReadOptions {
    pub delimiter: Option<u8>,
    pub has_header: bool,
    pub ragged: RaggedPolicy,
    pub encoding: Option<String>,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            delimiter: None,
            has_header: true,
            ragged: RaggedPolicy::Error,
            encoding: None,
        }
    }
}

/// Output options for delimited text.
#[derive(Debug, Clone)]
pub struct WriteOptions {
    pub delimiter: u8,
    pub has_header: bool,
    pub line_ending: LineEnding,
    pub null: String,
}

impl WriteOptions {
    pub fn new(delimiter: u8) -> Self {
        Self {
            delimiter,
            has_header: true,
            line_ending: LineEnding::Lf,
            null: String::new(),
        }
    }
}

/// Parse a user-provided delimiter (symbol, name, or escape sequence).
pub fn parse_delimiter(value: &str) -> std::result::Result<u8, String> {
    match value {
        "\\t" | "tab" => Ok(b'\t'),
        "," | "comma" => Ok(b','),
        ";" | "semicolon" => Ok(b';'),
        "|" | "pipe" => Ok(b'|'),
        _ => {
            let bytes = value.as_bytes();
            if bytes.len() == 1 {
                Ok(bytes[0])
            } else {
                Err(format!(
                    "delimiter must be a single byte or one of tab/comma/semicolon/pipe, got '{value}'"
                ))
            }
        }
    }
}

/// Read a CSV/TSV input into a table.
pub fn read(
    source: &SourceRef,
    bytes: &[u8],
    is_tsv: bool,
    options: &ReadOptions,
) -> Result<Table> {
    let decoded = decode_text(bytes, options.encoding.as_deref())?;
    let mut warnings = Vec::new();

    if decoded.had_errors {
        warnings.push(LossEvent::warning(
            "csv.encoding_replaced",
            format!(
                "invalid byte sequences were replaced while decoding as {}",
                decoded.encoding_name()
            ),
        ));
    }

    if has_unterminated_quote(&decoded.text) {
        warnings.push(
            LossEvent::warning(
                "csv.bad_quote",
                "the input ends inside a quoted field; the final record may be truncated or malformed",
            )
            .with_source(source.clone()),
        );
    }

    let delimiter = options.delimiter.unwrap_or_else(|| {
        if is_tsv {
            b'\t'
        } else {
            sniff_delimiter(&decoded.text)
        }
    });

    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(decoded.text.as_bytes());

    let mut records = Vec::new();
    for result in reader.records() {
        let record = result.map_err(|error| csv_error(source, error))?;
        let line = record.position().map_or(0, |position| position.line());
        records.push((record.iter().map(str::to_owned).collect::<Vec<_>>(), line));
    }

    let header = if options.has_header && !records.is_empty() {
        Some(records.remove(0).0)
    } else {
        None
    };

    let expected = header
        .as_ref()
        .map(Vec::len)
        .or_else(|| records.first().map(|(fields, _)| fields.len()))
        .unwrap_or(0);

    let names = match &header {
        Some(fields) => fields
            .iter()
            .enumerate()
            .map(|(index, name)| {
                if name.is_empty() {
                    format!("column_{}", index + 1)
                } else {
                    name.clone()
                }
            })
            .collect::<Vec<_>>(),
        None => (1..=expected)
            .map(|index| format!("column_{index}"))
            .collect::<Vec<_>>(),
    };

    let mut rows = Vec::with_capacity(records.len());
    for (index, (mut fields, line)) in records.into_iter().enumerate() {
        let row_number = index as u64 + 1;
        let actual = fields.len();

        if actual == expected {
            rows.push(fields.into_iter().map(CellValue::Text).collect());
            continue;
        }

        if options.ragged == RaggedPolicy::Error {
            let column = if actual < expected {
                names
                    .get(actual)
                    .map(|name| format!(" (missing column '{name}')"))
                    .unwrap_or_default()
            } else {
                names
                    .last()
                    .map(|name| format!(" (extra fields after column '{name}')"))
                    .unwrap_or_default()
            };
            return Err(Error::data(format!(
                "{}: row {row_number} (line {line}) has {actual} fields, expected {expected}{column}",
                source.path.display()
            )));
        }

        let message = if actual < expected {
            format!(
                "row {row_number} (line {line}) has {actual} fields, expected {expected}; padded with {} empty cells",
                expected - actual
            )
        } else {
            format!(
                "row {row_number} (line {line}) has {actual} fields, expected {expected}; dropped {} extra cells",
                actual - expected
            )
        };
        warnings.push(
            LossEvent::warning("csv.ragged_row", message)
                .with_source(source.clone().with_row(row_number)),
        );

        if options.ragged == RaggedPolicy::Skip {
            continue;
        }

        if actual < expected {
            fields.resize(expected, String::new());
        } else {
            fields.truncate(expected);
        }
        rows.push(fields.into_iter().map(CellValue::Text).collect());
    }

    let columns = names
        .into_iter()
        .map(|name| Column::new(name, ColumnType::Text))
        .collect();

    Ok(Table {
        name: table_name(source),
        source: source.clone(),
        columns,
        rows,
        warnings,
        first_data_row: 1,
    })
}

/// Detect a quoted field that is still open at end of input.
///
/// This is a deliberately simple RFC 4180 balance check: doubled quotes inside
/// a quoted field are escaped, and every other quote toggles the quoted state.
fn has_unterminated_quote(text: &str) -> bool {
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();

    while let Some(character) = chars.next() {
        if character != '"' {
            continue;
        }

        if in_quotes {
            if chars.peek() == Some(&'"') {
                chars.next();
            } else {
                in_quotes = false;
            }
        } else {
            in_quotes = true;
        }
    }

    in_quotes
}

/// Score each candidate delimiter and return the most plausible one.
pub fn sniff_delimiter(text: &str) -> u8 {
    const CANDIDATES: [u8; 4] = *b",;\t|";
    let mut best = (i64::MIN, b',');

    for delimiter in CANDIDATES {
        let score = delimiter_score(text, delimiter);
        if score > best.0 {
            best = (score, delimiter);
        }
    }

    best.1
}

fn delimiter_score(text: &str, delimiter: u8) -> i64 {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());

    let mut counts = Vec::new();
    for record in reader.records().take(64) {
        match record {
            Ok(record) => counts.push(record.len()),
            Err(_) => return -10_000,
        }
    }

    let Some(&first) = counts.first() else {
        return 0;
    };
    let consistent = counts.iter().filter(|&&count| count == first).count();
    let mut score = first as i64 + consistent as i64 * 10;
    if first <= 1 {
        score -= 1_000;
    }
    if consistent != counts.len() {
        score -= 100;
    }
    score
}

/// Write a table as CSV/TSV.
pub fn write<W: Write>(output: W, table: &Table, options: &WriteOptions) -> Result<()> {
    let terminator = match options.line_ending {
        LineEnding::Lf => Terminator::Any(b'\n'),
        LineEnding::Crlf => Terminator::CRLF,
    };

    let mut writer = csv::WriterBuilder::new()
        .delimiter(options.delimiter)
        .terminator(terminator)
        .from_writer(output);

    if options.has_header {
        let headers = table
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect::<Vec<_>>();
        writer
            .write_record(&headers)
            .map_err(|error| csv_error(&table.source, error))?;
    }

    let width = table.width();
    for (index, row) in table.rows.iter().enumerate() {
        if row.len() > width {
            return Err(Error::data(format!(
                "{}: row {} has {} cells but the table has {width} columns",
                table.source.path.display(),
                index + 1,
                row.len()
            )));
        }

        let record = (0..width)
            .map(|column| {
                row.get(column)
                    .unwrap_or(&CellValue::Null)
                    .to_text(&options.null)
            })
            .collect::<Vec<_>>();
        writer
            .write_record(record)
            .map_err(|error| csv_error(&table.source, error))?;
    }

    writer.flush()?;
    Ok(())
}

fn table_name(source: &SourceRef) -> String {
    if source.path.as_os_str() == "-" {
        return "stdin".to_owned();
    }
    source
        .path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("table")
        .to_owned()
}

fn csv_error(source: &SourceRef, error: csv::Error) -> Error {
    if error.is_io_error() {
        return match error.into_kind() {
            ErrorKind::Io(io) => Error::Io(io),
            _ => unreachable!("is_io_error guarantees the Io kind"),
        };
    }
    Error::data(format!("{}: {error}", source.path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::FormatId;

    fn source() -> SourceRef {
        SourceRef::new("data.csv", FormatId::Csv)
    }

    fn read_bytes(bytes: &[u8], options: &ReadOptions) -> Table {
        read(&source(), bytes, false, options).unwrap()
    }

    #[test]
    fn parses_quoted_commas_quotes_and_newlines() {
        let input = b"name,note\r\nAlice,\"likes commas, quotes \"\"and\"\"\r\nnewlines\"\r\n";
        let table = read_bytes(input, &ReadOptions::default());

        assert_eq!(table.column_names(), ["name", "note"]);
        assert_eq!(table.row_count(), 1);
        assert_eq!(
            table.rows[0][1],
            CellValue::Text("likes commas, quotes \"and\"\r\nnewlines".into())
        );
    }

    #[test]
    fn detects_unterminated_quotes_without_flagging_escaped_quotes() {
        assert!(has_unterminated_quote("a,b\n\"unterminated,2\n"));
        assert!(!has_unterminated_quote("a,b\n\"quoted, value\",2\n"));
        assert!(!has_unterminated_quote("a,b\n\"quoted \"\"value\"\"\",2\n"));
    }

    #[test]
    fn sniffs_semicolon_and_pipe_delimiters() {
        assert_eq!(sniff_delimiter("a;b;c\n1;2;3\n"), b';');
        assert_eq!(sniff_delimiter("a|b|c\n1|2|3\n"), b'|');
        assert_eq!(sniff_delimiter("a,b,c\n1,2,3\n"), b',');
        assert_eq!(sniff_delimiter("a\tb\tc\n1\t2\t3\n"), b'\t');
    }

    #[test]
    fn generates_names_without_a_header() {
        let options = ReadOptions {
            has_header: false,
            ..ReadOptions::default()
        };
        let table = read_bytes(b"Alice,1\nBob,2\n", &options);
        assert_eq!(table.column_names(), ["column_1", "column_2"]);
        assert_eq!(table.row_count(), 2);
    }

    #[test]
    fn ragged_rows_error_by_default() {
        let error = read(&source(), b"a,b\n1\n", false, &ReadOptions::default()).unwrap_err();
        assert_eq!(error.exit_code(), crate::error::ExitCode::UserData);
        assert!(error.to_string().contains("expected 2"));
    }

    #[test]
    fn ragged_rows_can_be_padded_or_skipped() {
        let pad = ReadOptions {
            ragged: RaggedPolicy::Pad,
            ..ReadOptions::default()
        };
        let table = read(&source(), b"a,b\n1\n2,3,4\n", false, &pad).unwrap();
        assert_eq!(table.rows[0][1], CellValue::Text(String::new()));
        assert_eq!(table.rows[1].len(), 2);
        assert_eq!(table.warnings.len(), 2);

        let skip = ReadOptions {
            ragged: RaggedPolicy::Skip,
            ..ReadOptions::default()
        };
        let table = read(&source(), b"a,b\n1\n2,3\n", false, &skip).unwrap();
        assert_eq!(table.row_count(), 1);
        assert_eq!(table.warnings.len(), 1);
    }

    #[test]
    fn round_trips_delimited_output() {
        let input = b"name,note\nAlice,\"a,b\"\nBob,\"line 1\nline 2\"\n";
        let table = read_bytes(input, &ReadOptions::default());
        let mut output = Vec::new();
        write(&mut output, &table, &WriteOptions::new(b',')).unwrap();

        let reparsed = read_bytes(&output, &ReadOptions::default());
        assert_eq!(reparsed.column_names(), table.column_names());
        assert_eq!(reparsed.rows, table.rows);
    }

    #[test]
    fn writes_crlf_and_null_text() {
        let table = Table::with_columns(
            "data",
            source(),
            vec![
                Column::new("a", ColumnType::Text),
                Column::new("b", ColumnType::Text),
            ],
            vec![vec![CellValue::Text("x".into()), CellValue::Null]],
        );
        let mut options = WriteOptions::new(b',');
        options.line_ending = LineEnding::Crlf;
        options.null = "NULL".into();

        let mut output = Vec::new();
        write(&mut output, &table, &options).unwrap();
        assert_eq!(output, b"a,b\r\nx,NULL\r\n");
    }

    #[test]
    fn delimiter_parser_accepts_symbols_and_names() {
        assert_eq!(parse_delimiter(";").unwrap(), b';');
        assert_eq!(parse_delimiter("tab").unwrap(), b'\t');
        assert_eq!(parse_delimiter("\\t").unwrap(), b'\t');
        assert!(parse_delimiter("nope").is_err());
    }
}
