//! Shared command-line argument groups.

use clap::{ArgGroup, Args, ValueEnum};

use crate::diagnostics::{DiagnosticsLevel, DiagnosticsPolicy};
use crate::error::{Error, Result};
use crate::formats;
use crate::formats::csv::{LineEnding, RaggedPolicy, parse_delimiter};
use crate::ops::merge::MergeMode;

/// Sheet/table selection shared by commands that can read workbooks.
#[derive(Debug, Clone, Args)]
pub struct SheetArgs {
    /// Read one sheet by name.
    #[arg(long, value_name = "NAME", conflicts_with_all = ["sheet_index", "all_sheets", "tables"])]
    pub sheet: Option<String>,

    /// Read one sheet by 1-based index.
    #[arg(long, value_name = "N", conflicts_with_all = ["sheet", "all_sheets", "tables"])]
    pub sheet_index: Option<usize>,

    /// Read every sheet in workbook order.
    #[arg(long, conflicts_with_all = ["sheet", "sheet_index", "tables"])]
    pub all_sheets: bool,

    /// Select HTML tables: all, a 1-based index, or a caption/name.
    #[arg(long, value_name = "SPEC", conflicts_with_all = ["sheet", "sheet_index", "all_sheets"])]
    pub tables: Option<String>,
}

impl SheetArgs {
    pub fn selection(&self) -> Result<formats::SheetSelection> {
        if self.all_sheets {
            return Ok(formats::SheetSelection::All);
        }
        if let Some(index) = self.sheet_index {
            if index == 0 {
                return Err(Error::usage("sheet indices are 1-based; 0 is not valid"));
            }
            return Ok(formats::SheetSelection::Index(index));
        }
        if let Some(sheet) = &self.sheet {
            return Ok(formats::SheetSelection::Named(sheet.clone()));
        }
        if let Some(specification) = &self.tables {
            let specification = specification.trim();
            if specification.is_empty() {
                return Err(Error::usage("--tables requires all, an index, or a name"));
            }
            if specification.eq_ignore_ascii_case("all") {
                return Ok(formats::SheetSelection::All);
            }
            if let Ok(index) = specification.parse::<usize>() {
                if index == 0 {
                    return Err(Error::usage("table indices are 1-based; 0 is not valid"));
                }
                return Ok(formats::SheetSelection::Index(index));
            }
            return Ok(formats::SheetSelection::Named(specification.to_owned()));
        }
        Ok(formats::SheetSelection::Default)
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

    /// HTML header row (1-based). Rows before it are discarded.
    #[arg(long, value_name = "N", conflicts_with = "no_header")]
    pub header_row: Option<usize>,

    /// How HTML colspan/rowspan cells are represented.
    #[arg(long, value_enum, default_value_t = MergePolicyArg::Expand)]
    pub merge_policy: MergePolicyArg,

    /// Prefer HTML table captions for output sheet names.
    #[arg(long)]
    pub use_captions: bool,

    /// Skip HTML tables with fewer rows than this.
    #[arg(long, value_name = "N", default_value_t = 2)]
    pub min_rows: usize,

    /// Skip HTML tables with fewer columns than this.
    #[arg(long, value_name = "N", default_value_t = 1)]
    pub min_cols: usize,

    #[command(flatten)]
    pub sheets: SheetArgs,
}

impl ReadArgs {
    pub fn options(&self) -> Result<formats::ReadOptions> {
        Ok(formats::ReadOptions {
            delimiter: self.delimiter,
            has_header: !self.no_header,
            ragged: self.ragged,
            encoding: self.encoding.clone(),
            sheets: self.sheets.selection()?,
            merge_policy: self.merge_policy.into(),
            header_row: self.header_row,
            use_captions: self.use_captions,
            min_rows: self.min_rows,
            min_cols: self.min_cols,
        })
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

/// Convert one or more inputs to one output.
#[derive(Debug, Clone, Args)]
pub struct ConvertArgs {
    /// Input path(s). Use - for stdin. With -o, every positional is an input;
    /// without -o, the final positional is the output path.
    #[arg(value_name = "INPUT", required = true, num_args = 1..)]
    pub inputs: Vec<String>,

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

    /// Explicitly assemble one worksheet per input file (the default for
    /// multi-input workbook output).
    #[arg(long)]
    pub sheet_per_input: bool,

    /// Assemble one worksheet per selected HTML table.
    #[arg(long)]
    pub sheet_per_table: bool,

    /// Add an _index worksheet describing the selected tables.
    #[arg(long)]
    pub index_sheet: bool,

    /// Worksheet-name template for workbook output. Supports {stem}, {sheet},
    /// {name}, and {index}.
    #[arg(long, value_name = "TEMPLATE", conflicts_with = "sheet_name")]
    pub sheet_name_template: Option<String>,

    /// Name the single output worksheet.
    #[arg(
        long = "sheet-name",
        visible_alias = "output-sheet",
        value_name = "NAME",
        conflicts_with = "sheet_name_template"
    )]
    pub sheet_name: Option<String>,
}

impl ConvertArgs {
    /// Resolve the input list and output path.
    ///
    /// With `-o`, all positionals are inputs. Without `-o`, the last positional
    /// is the output so the Phase 0/1 `convert INPUT OUTPUT` form keeps working.
    pub fn resolve(&self) -> Result<(Vec<String>, String)> {
        if let Some(output) = &self.output {
            if self.inputs.is_empty() {
                return Err(Error::usage("missing input path"));
            }
            return Ok((self.inputs.clone(), output.clone()));
        }

        if self.inputs.len() < 2 {
            return Err(Error::usage(
                "missing output path (pass it positionally or with -o/--output)",
            ));
        }

        let output = self.inputs.last().cloned().unwrap();
        let inputs = self.inputs[..self.inputs.len() - 1].to_vec();
        Ok((inputs, output))
    }
}

/// HTML merged-cell policy exposed on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum MergePolicyArg {
    /// Copy the value into every covered cell.
    Expand,
    /// Keep the value in the first covered cell only.
    First,
    /// Reject tables that contain merged cells.
    Error,
}

impl From<MergePolicyArg> for formats::MergePolicy {
    fn from(policy: MergePolicyArg) -> Self {
        match policy {
            MergePolicyArg::Expand => Self::Expand,
            MergePolicyArg::First => Self::First,
            MergePolicyArg::Error => Self::Error,
        }
    }
}

/// Merge mode exposed on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum MergeModeArg {
    /// Column names, order, and types must match.
    Strict,
    /// Union columns by name and fill missing cells with null.
    Loose,
}

impl From<MergeModeArg> for MergeMode {
    fn from(mode: MergeModeArg) -> Self {
        match mode {
            MergeModeArg::Strict => Self::Strict,
            MergeModeArg::Loose => Self::Loose,
        }
    }
}

/// Merge/concatenate several inputs into one output.
#[derive(Debug, Clone, Args)]
pub struct MergeArgs {
    /// Input paths. Each input must select exactly one table.
    #[arg(value_name = "INPUT", required = true, num_args = 2..)]
    pub inputs: Vec<String>,

    /// Output path, or - for stdout.
    #[arg(short = 'o', long = "output", value_name = "PATH", default_value = "-")]
    pub output: String,

    /// Schema policy: strict requires identical headers and compatible types.
    #[arg(long, value_enum, default_value_t = MergeModeArg::Strict)]
    pub mode: MergeModeArg,

    /// Append a provenance column with this name (default: source).
    #[arg(long, value_name = "NAME", num_args = 0..=1, default_missing_value = "source")]
    pub add_source_column: Option<String>,

    /// Drop exact duplicate data rows, keeping the first occurrence.
    #[arg(long)]
    pub dedup: bool,

    /// Match column names without regard to case.
    #[arg(long)]
    pub case_insensitive_headers: bool,

    /// Print a machine-readable schema diff when strict mode fails.
    #[arg(long)]
    pub json: bool,

    #[command(flatten)]
    pub read: ReadArgs,

    #[command(flatten)]
    pub write: WriteArgs,
}

/// Filter rows with a simple comparison expression.
#[derive(Debug, Clone, Args)]
pub struct FilterArgs {
    /// Input path, or - for stdin.
    #[arg(value_name = "INPUT")]
    pub input: String,

    /// Row filter such as 'amount > 100' or 'status != closed'.
    #[arg(long = "where", value_name = "EXPR")]
    pub where_expr: String,

    #[command(flatten)]
    pub read: ReadArgs,

    #[command(flatten)]
    pub display: DisplayArgs,
}

/// Sort rows by one or more columns.
#[derive(Debug, Clone, Args)]
pub struct SortArgs {
    /// Input path, or - for stdin.
    #[arg(value_name = "INPUT")]
    pub input: String,

    /// Comma-separated sort keys such as name or amount:desc.
    #[arg(long = "by", value_name = "KEYS")]
    pub by: String,

    #[command(flatten)]
    pub read: ReadArgs,

    #[command(flatten)]
    pub display: DisplayArgs,
}

/// Rename one or more columns.
#[derive(Debug, Clone, Args)]
pub struct RenameArgs {
    /// Input path, or - for stdin.
    #[arg(value_name = "INPUT")]
    pub input: String,

    /// Comma-separated mappings such as amount=total,old_id=id.
    #[arg(long = "map", value_name = "MAPPINGS")]
    pub map: String,

    #[command(flatten)]
    pub read: ReadArgs,

    #[command(flatten)]
    pub display: DisplayArgs,
}

/// Drop exact duplicate rows.
#[derive(Debug, Clone, Args)]
pub struct DedupeArgs {
    /// Input path, or - for stdin.
    #[arg(value_name = "INPUT")]
    pub input: String,

    #[command(flatten)]
    pub read: ReadArgs,

    #[command(flatten)]
    pub display: DisplayArgs,
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
