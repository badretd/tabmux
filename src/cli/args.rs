//! Shared command-line argument groups.

use clap::Args;

use crate::error::{Error, Result};
use crate::formats;
use crate::formats::csv::{LineEnding, RaggedPolicy, parse_delimiter};

/// Options shared by every command that reads a table.
#[derive(Debug, Clone, Args)]
pub struct ReadArgs {
    /// Input delimiter (symbol or tab/comma/semicolon/pipe).
    #[arg(long, value_name = "CHAR", value_parser = parse_delimiter)]
    pub delimiter: Option<u8>,

    /// Override input encoding (for example latin1 or utf-16le).
    #[arg(long, value_name = "LABEL")]
    pub encoding: Option<String>,

    /// Treat the first row as data and omit the output header.
    #[arg(long)]
    pub no_header: bool,

    /// Policy for rows with the wrong number of fields.
    #[arg(long, value_enum, default_value_t = RaggedPolicy::Error)]
    pub ragged: RaggedPolicy,
}

impl ReadArgs {
    pub fn options(&self) -> formats::ReadOptions {
        formats::ReadOptions {
            delimiter: self.delimiter,
            has_header: !self.no_header,
            ragged: self.ragged,
            encoding: self.encoding.clone(),
        }
    }
}

/// Options shared by commands that write files.
#[derive(Debug, Clone, Args)]
pub struct WriteArgs {
    /// Output line endings.
    #[arg(long, value_enum, default_value_t = LineEnding::Lf)]
    pub line_ending: LineEnding,

    /// Text used for null values in delimited output.
    #[arg(long, value_name = "TEXT", default_value = "")]
    pub null: String,

    /// Overwrite an existing output file.
    #[arg(short = 'y', long)]
    pub force: bool,

    /// Copy an existing output to a timestamped .bak before replacing it.
    #[arg(long)]
    pub backup: bool,

    /// Validate and describe the conversion without writing anything.
    #[arg(long)]
    pub dry_run: bool,
}

/// Convert one input to one output.
#[derive(Debug, Clone, Args)]
pub struct ConvertArgs {
    /// Input path, or - for stdin.
    #[arg(value_name = "INPUT")]
    pub input: String,

    /// Output path, or - for stdout. May also be supplied with -o.
    #[arg(value_name = "OUTPUT")]
    pub positional_output: Option<String>,

    /// Output path, or - for stdout.
    #[arg(short = 'o', long = "output", value_name = "PATH")]
    pub output: Option<String>,

    #[command(flatten)]
    pub read: ReadArgs,

    #[command(flatten)]
    pub write: WriteArgs,
}

impl ConvertArgs {
    pub fn resolve_output(&self) -> Result<&str> {
        match (&self.positional_output, &self.output) {
            (Some(_), Some(_)) => Err(Error::usage(
                "output was given both positionally and with --output",
            )),
            (Some(output), None) | (None, Some(output)) => Ok(output),
            (None, None) => Err(Error::usage(
                "missing output path (pass it positionally or with -o/--output)",
            )),
        }
    }
}

/// Count data rows.
#[derive(Debug, Clone, Args)]
pub struct CountArgs {
    /// Input path, or - for stdin.
    #[arg(value_name = "INPUT")]
    pub input: String,

    #[command(flatten)]
    pub read: ReadArgs,
}

/// Print the first rows of a table.
#[derive(Debug, Clone, Args)]
pub struct HeadArgs {
    /// Input path, or - for stdin.
    #[arg(value_name = "INPUT")]
    pub input: String,

    /// Number of data rows to print.
    #[arg(short = 'n', long, default_value_t = 10, value_name = "N")]
    pub rows: usize,

    #[command(flatten)]
    pub read: ReadArgs,
}

/// Select columns and print them to stdout.
#[derive(Debug, Clone, Args)]
pub struct SelectArgs {
    /// Input path, or - for stdin.
    #[arg(value_name = "INPUT")]
    pub input: String,

    /// Comma-separated column names or 1-based indices.
    #[arg(long, value_name = "COLUMNS", required = true)]
    pub columns: String,

    #[command(flatten)]
    pub read: ReadArgs,
}
