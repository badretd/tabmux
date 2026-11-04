//! Shared command-line argument groups.

use clap::{ArgGroup, Args};

use crate::diagnostics::{DiagnosticsLevel, DiagnosticsPolicy};
use crate::error::{Error, Result};
use crate::formats;
use crate::formats::csv::{LineEnding, RaggedPolicy, parse_delimiter};

/// Sheet/table selection shared by commands that can read workbooks.
#[derive(Debug, Clone, Args)]
pub struct SheetArgs {
    /// Read one sheet by name.
    #[arg(long, value_name = "NAME", conflicts_with_all = ["sheet_index", "all_sheets"])]
    pub sheet: Option<String>,

    /// Read one sheet by 1-based index.
    #[arg(long, value_name = "N", conflicts_with_all = ["sheet", "all_sheets"])]
    pub sheet_index: Option<usize>,

    /// Read every sheet in workbook order.
    #[arg(long, conflicts_with_all = ["sheet", "sheet_index"])]
    pub all_sheets: bool,
}

impl SheetArgs {
    pub fn selection(&self) -> formats::SheetSelection {
        if self.all_sheets {
            formats::SheetSelection::All
        } else if let Some(index) = self.sheet_index {
            formats::SheetSelection::Index(index)
        } else if let Some(sheet) = &self.sheet {
            formats::SheetSelection::Named(sheet.clone())
        } else {
            formats::SheetSelection::Default
        }
    }
}

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

    #[command(flatten)]
    pub sheets: SheetArgs,
}

impl ReadArgs {
    pub fn options(&self) -> formats::ReadOptions {
        formats::ReadOptions {
            delimiter: self.delimiter,
            has_header: !self.no_header,
            ragged: self.ragged,
            encoding: self.encoding.clone(),
            sheets: self.sheets.selection(),
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

/// Diagnostic verbosity shared by every command.
#[derive(Debug, Clone, Args)]
pub struct DiagnosticsArgs {
    /// Diagnostic detail: summary, full, or none.
    #[arg(long, global = true, value_enum, value_name = "LEVEL")]
    pub diagnostics: Option<DiagnosticsLevel>,

    /// Suppress diagnostics entirely.
    #[arg(long, global = true, conflicts_with_all = ["verbose", "debug"])]
    pub quiet: bool,

    /// Print every diagnostic, including source-fidelity events.
    #[arg(long, global = true, conflicts_with = "quiet")]
    pub verbose: bool,

    /// Include filtered-out per-row diagnostics and imply --verbose.
    #[arg(long, global = true, conflicts_with = "quiet")]
    pub debug: bool,

    /// Show workbook fidelity events even for read-only commands.
    #[arg(long = "fidelity-report", global = true)]
    pub fidelity_report: bool,
}

impl DiagnosticsArgs {
    pub fn policy(&self) -> DiagnosticsPolicy {
        DiagnosticsPolicy::resolve(
            self.diagnostics,
            self.quiet,
            self.verbose,
            self.debug,
            self.fidelity_report,
        )
    }
}

/// Terminal display mode for inspection commands.
#[derive(Debug, Clone, Args)]
pub struct DisplayArgs {
    /// Print machine-friendly CSV/TSV without decorative formatting.
    #[arg(long, conflicts_with = "pretty")]
    pub raw: bool,

    /// Print an aligned terminal table.
    #[arg(long, conflicts_with = "raw")]
    pub pretty: bool,
}

impl DisplayArgs {
    pub fn is_pretty(&self) -> bool {
        self.pretty
    }
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

    /// Write each selected sheet/table to its own file in the output directory.
    #[arg(long)]
    pub split_sheets: bool,

    /// Filename template for --split-sheets. Supports {stem}, {sheet}, {format},
    /// {ext}, and {index}.
    #[arg(long, value_name = "TEMPLATE")]
    pub name_template: Option<String>,

    /// Concatenate every selected sheet into one output table.
    #[arg(long, conflicts_with = "split_sheets", requires = "all_sheets")]
    pub stack: bool,

    /// Add a provenance column with the source sheet name when stacking.
    #[arg(long, value_name = "NAME", requires = "stack")]
    pub add_source_column: Option<String>,
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

/// List the sheets in a workbook.
#[derive(Debug, Clone, Args)]
pub struct SheetsArgs {
    /// Workbook path.
    #[arg(value_name = "INPUT")]
    pub input: String,
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

    #[command(flatten)]
    pub display: DisplayArgs,
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

    #[command(flatten)]
    pub display: DisplayArgs,
}

/// Print one data row by number or by a simple equality filter.
#[derive(Debug, Clone, Args)]
#[command(group(
    ArgGroup::new("row_selector")
        .required(true)
        .args(["number", "where_expr"])
))]
pub struct RowArgs {
    /// Input path, or - for stdin.
    #[arg(value_name = "INPUT")]
    pub input: String,

    /// 1-based data row number (the header is not counted).
    #[arg(long, value_name = "N", conflicts_with = "where_expr")]
    pub number: Option<usize>,

    /// Row filter such as 'id == 1234' or 'status != closed'.
    #[arg(long = "where", value_name = "EXPR", conflicts_with = "number")]
    pub where_expr: Option<String>,

    #[command(flatten)]
    pub read: ReadArgs,

    #[command(flatten)]
    pub display: DisplayArgs,
}

/// Print one cell by row number and column name.
#[derive(Debug, Clone, Args)]
pub struct CellArgs {
    /// Input path, or - for stdin.
    #[arg(value_name = "INPUT")]
    pub input: String,

    /// 1-based data row number (the header is not counted).
    #[arg(long, value_name = "N")]
    pub row: usize,

    /// Column name or 1-based column index.
    #[arg(long, value_name = "COLUMN")]
    pub column: String,

    #[command(flatten)]
    pub read: ReadArgs,

    #[command(flatten)]
    pub display: DisplayArgs,
}
