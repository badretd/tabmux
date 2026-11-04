//! Format registry and capability matrix.
//!
//! This is intentionally the living scope guard for v1: if a format is not in
//! [`FORMATS`], no command may read or write it. New formats arrive only with a
//! reader/writer, a loss policy, and checked-in fixtures.

use std::fmt::Write as _;
use std::io::{Seek, Write};
use std::path::Path;

use crate::error::{Error, Result};
use crate::model::{SourceRef, Table, TableSet};

pub mod archive;
pub mod csv;
#[cfg(feature = "sql-duckdb")]
pub mod duckdb;
#[cfg(feature = "html")]
pub mod html;
pub mod json;
#[cfg(feature = "excel")]
pub mod ods;
#[cfg(feature = "excel")]
pub mod workbook;
#[cfg(feature = "excel")]
pub mod workbook_write;
#[cfg(feature = "excel")]
pub mod xlsx;
#[cfg(all(feature = "excel", not(feature = "rust_xlsxwriter")))]
pub mod xlsx_manual;
#[cfg(all(feature = "excel", feature = "rust_xlsxwriter"))]
pub mod xlsx_writer;

/// Stable identifier for a supported input/output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FormatId {
    Csv,
    Tsv,
    Xlsx,
    Ods,
    Html,
    Json,
    /// Not a table format. Used only as the source format on loss events for
    /// batch files whose extension is not supported.
    Unknown,
}

impl FormatId {
    /// Canonical lowercase name used by the CLI and reports.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Tsv => "tsv",
            Self::Xlsx => "xlsx",
            Self::Ods => "ods",
            Self::Html => "html",
            Self::Json => "json",
            Self::Unknown => "unknown",
        }
    }

    /// Detect a format from a filename extension.
    ///
    /// `jsonl`/`ndjson` are treated as [`FormatId::Json`] because they share
    /// the JSON reader. XLS/XLSB are deliberately absent until their phase
    /// lands; callers must report `format.unsupported` for them.
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension
            .trim_start_matches('.')
            .to_ascii_lowercase()
            .as_str()
        {
            "csv" => Some(Self::Csv),
            "tsv" | "tab" => Some(Self::Tsv),
            // Legacy Excel containers are read through the same capability row
            // because the registry is about the conversion format, not the
            // on-disk container version.
            "xlsx" | "xlsm" | "xls" | "xlsb" => Some(Self::Xlsx),
            "ods" => Some(Self::Ods),
            "html" | "htm" => Some(Self::Html),
            "json" | "jsonl" | "ndjson" => Some(Self::Json),
            _ => None,
        }
    }
}

/// What a format can do in this build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capability {
    /// Can be read as an input.
    pub read: bool,
    /// Can be written as an output.
    pub write: bool,
    /// Can be processed without materializing the whole table (where the
    /// reader/writer supports it).
    pub stream: bool,
    /// Can contain more than one table (workbook sheets, HTML tables, archive
    /// members).
    pub multi_table: bool,
}

impl Capability {
    const fn new(read: bool, write: bool, stream: bool, multi_table: bool) -> Self {
        Self {
            read,
            write,
            stream,
            multi_table,
        }
    }
}

/// A row in the format matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatInfo {
    /// Stable identifier.
    pub id: FormatId,
    /// Name printed in the matrix.
    pub name: &'static str,
    /// Capabilities in this build.
    pub capability: Capability,
    /// Short human-readable notes.
    pub notes: &'static str,
}

/// The complete v1 conversion matrix.
///
/// Only in-scope formats belong here. Directories and ZIP archives are input
/// containers handled by the batch phase; they are not table formats and get
/// their own registry once that phase lands.
pub const FORMATS: &[FormatInfo] = &[
    FormatInfo {
        id: FormatId::Csv,
        name: "csv",
        capability: Capability::new(true, true, true, false),
        notes: "RFC 4180 + dialects",
    },
    FormatInfo {
        id: FormatId::Tsv,
        name: "tsv",
        capability: Capability::new(true, true, true, false),
        notes: "",
    },
    FormatInfo {
        id: FormatId::Xlsx,
        name: "xlsx",
        capability: Capability::new(true, true, false, true),
        notes: "basic fidelity",
    },
    FormatInfo {
        id: FormatId::Ods,
        name: "ods",
        capability: Capability::new(true, true, false, true),
        notes: "basic fidelity",
    },
    FormatInfo {
        id: FormatId::Html,
        name: "html",
        capability: Capability::new(true, false, false, true),
        notes: "table extraction",
    },
    FormatInfo {
        id: FormatId::Json,
        name: "json",
        capability: Capability::new(true, false, true, false),
        notes: "JSON/JSONL input",
    },
];

/// Return the format registry.
pub const fn registry() -> &'static [FormatInfo] {
    FORMATS
}

/// Look up a format by its stable identifier.
pub fn find(id: FormatId) -> Option<&'static FormatInfo> {
    FORMATS.iter().find(|info| info.id == id)
}

/// Render the capability matrix shown by `tabmux formats`.
pub fn render_matrix() -> String {
    let mut out = String::with_capacity(512);
    let _ = writeln!(
        out,
        "{:<8} {:<5} {:<6} {:<12} NOTES",
        "FORMAT", "READ", "WRITE", "MULTI-TABLE"
    );

    for info in FORMATS {
        let line = format!(
            "{:<8} {:<5} {:<6} {:<12} {}",
            info.name,
            yes_no(info.capability.read),
            yes_no(info.capability.write),
            yes_no(info.capability.multi_table),
            info.notes
        );
        let _ = writeln!(out, "{}", line.trim_end());
    }

    out
}

const fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

/// Which table or tables to read from a multi-table source.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SheetSelection {
    /// The format default: the first sheet for workbooks.
    #[default]
    Default,
    /// Every sheet in workbook order.
    All,
    /// A sheet selected by name.
    Named(String),
    /// A sheet selected by 1-based index.
    Index(usize),
}

/// How HTML `colspan`/`rowspan` cells are represented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MergePolicy {
    /// Copy the value into every covered cell (the default).
    #[default]
    Expand,
    /// Keep the value in the top-left cell only.
    First,
    /// Reject the input when a merged cell is present.
    Error,
}

/// Options shared by all readers.
#[derive(Debug, Clone)]
pub struct ReadOptions {
    pub delimiter: Option<u8>,
    pub has_header: bool,
    pub ragged: csv::RaggedPolicy,
    pub encoding: Option<String>,
    pub sheets: SheetSelection,
    pub merge_policy: MergePolicy,
    /// Explicit 1-based header row for HTML tables.
    pub header_row: Option<usize>,
    /// Prefer HTML `<caption>` text for table names.
    pub use_captions: bool,
    /// Layout-table filter for HTML input.
    pub min_rows: usize,
    pub min_cols: usize,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            delimiter: None,
            has_header: true,
            ragged: csv::RaggedPolicy::Error,
            encoding: None,
            sheets: SheetSelection::Default,
            merge_policy: MergePolicy::Expand,
            header_row: None,
            use_captions: false,
            min_rows: 2,
            min_cols: 1,
        }
    }
}

/// Options shared by all writers.
#[derive(Debug, Clone)]
pub struct WriteOptions {
    /// Override the format's default delimiter.
    pub delimiter: Option<u8>,
    pub has_header: bool,
    pub line_ending: csv::LineEnding,
    pub null: String,
}

impl WriteOptions {
    pub fn for_format(format: FormatId) -> Self {
        Self {
            delimiter: default_delimiter(format),
            has_header: true,
            line_ending: csv::LineEnding::Lf,
            null: String::new(),
        }
    }
}

/// Default delimiter for a delimited format.
pub const fn default_delimiter(format: FormatId) -> Option<u8> {
    match format {
        FormatId::Csv => Some(b','),
        FormatId::Tsv => Some(b'\t'),
        _ => None,
    }
}

/// Detect a format from a path extension.
pub fn detect_path(path: &Path) -> Option<FormatId> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .and_then(FormatId::from_extension)
}

/// List the tables in a multi-table input.
pub fn sheet_names(source: &SourceRef, bytes: &[u8], format: FormatId) -> Result<Vec<String>> {
    let _ = (source, bytes);
    match format {
        #[cfg(feature = "excel")]
        FormatId::Xlsx => xlsx::sheet_names(source, bytes),
        #[cfg(feature = "excel")]
        FormatId::Ods => ods::sheet_names(source, bytes),
        #[cfg(not(feature = "excel"))]
        FormatId::Xlsx | FormatId::Ods => Err(Error::unsupported(format!(
            "{} support is not compiled in (enable the excel feature)",
            format.label()
        ))),
        #[cfg(feature = "html")]
        FormatId::Html => html::table_names(source, bytes),
        #[cfg(not(feature = "html"))]
        FormatId::Html => Err(Error::unsupported(
            "html support is not compiled in (enable the html feature)",
        )),
        _ => Err(Error::unsupported(format!(
            "{} is not a multi-table workbook",
            format.label()
        ))),
    }
}

/// Read a single table from already-loaded bytes.
///
/// For a multi-table input this reads the format default (the first sheet for
/// workbooks). Use [`read_tables`] when the caller controls sheet selection.
pub fn read_table(
    source: &SourceRef,
    bytes: &[u8],
    format: FormatId,
    options: &ReadOptions,
) -> Result<Table> {
    read_tables(source, bytes, format, options)?.into_single()
}

/// Read one or more tables from already-loaded bytes.
pub fn read_tables(
    source: &SourceRef,
    bytes: &[u8],
    format: FormatId,
    options: &ReadOptions,
) -> Result<TableSet> {
    match format {
        FormatId::Csv | FormatId::Tsv => {
            let table = csv::read(
                source,
                bytes,
                format == FormatId::Tsv,
                &csv::ReadOptions {
                    delimiter: options.delimiter,
                    has_header: options.has_header,
                    ragged: options.ragged,
                    encoding: options.encoding.clone(),
                },
            )?;
            let mut set = TableSet::new(source.clone());
            set.push(table);
            Ok(set)
        }
        FormatId::Json => {
            let table = json::read(
                source,
                bytes,
                &json::ReadOptions {
                    encoding: options.encoding.clone(),
                },
            )?;
            let mut set = TableSet::new(source.clone());
            set.push(table);
            Ok(set)
        }
        #[cfg(feature = "excel")]
        FormatId::Xlsx => xlsx::read(
            source,
            bytes,
            &xlsx::ReadOptions {
                has_header: options.has_header,
                sheets: options.sheets.clone(),
            },
        ),
        #[cfg(feature = "excel")]
        FormatId::Ods => ods::read(
            source,
            bytes,
            &ods::ReadOptions {
                has_header: options.has_header,
                sheets: options.sheets.clone(),
            },
        ),
        #[cfg(not(feature = "excel"))]
        FormatId::Xlsx | FormatId::Ods => Err(Error::unsupported(format!(
            "{} support is not compiled in (enable the excel feature)",
            format.label()
        ))),
        #[cfg(feature = "html")]
        FormatId::Html => html::read(
            source,
            bytes,
            &html::ReadOptions {
                encoding: options.encoding.clone(),
                has_header: options.has_header,
                sheets: options.sheets.clone(),
                merge_policy: options.merge_policy,
                header_row: options.header_row,
                use_captions: options.use_captions,
                min_rows: options.min_rows,
                min_cols: options.min_cols,
            },
        ),
        #[cfg(not(feature = "html"))]
        FormatId::Html => Err(Error::unsupported(
            "html support is not compiled in (enable the html feature)",
        )),
        FormatId::Unknown => Err(Error::unsupported("unknown input format")),
    }
}

/// Write a table to a delimited output.
pub fn write_table<W: Write>(
    output: W,
    table: &Table,
    format: FormatId,
    options: &WriteOptions,
) -> Result<()> {
    let delimiter = match format {
        FormatId::Csv | FormatId::Tsv => options
            .delimiter
            .or_else(|| default_delimiter(format))
            .unwrap_or(b','),
        _ => {
            return Err(Error::unsupported(format!(
                "writing {} is not implemented yet",
                format.label()
            )));
        }
    };

    csv::write(
        output,
        table,
        &csv::WriteOptions {
            delimiter,
            has_header: options.has_header,
            line_ending: options.line_ending,
            null: options.null.clone(),
        },
    )
}

/// Write one or more tables to a multi-table output.
///
/// Delimited formats hold exactly one table; workbooks hold one sheet per table.
pub fn write_tables<W: Write + Seek>(
    output: W,
    tables: &[Table],
    format: FormatId,
    options: &WriteOptions,
) -> Result<()> {
    if tables.is_empty() {
        return Err(Error::data("no tables to write"));
    }

    match format {
        FormatId::Csv | FormatId::Tsv => {
            if tables.len() > 1 {
                return Err(Error::data(format!(
                    "{} output holds one table but {} were selected",
                    format.label(),
                    tables.len()
                )));
            }
            write_table(output, &tables[0], format, options)
        }
        #[cfg(feature = "excel")]
        FormatId::Xlsx => xlsx::write(output, tables, options),
        #[cfg(feature = "excel")]
        FormatId::Ods => ods::write(output, tables, options),
        #[cfg(not(feature = "excel"))]
        FormatId::Xlsx | FormatId::Ods => Err(Error::unsupported(format!(
            "{} support is not compiled in (enable the excel feature)",
            format.label()
        ))),
        FormatId::Html | FormatId::Json => Err(Error::unsupported(format!(
            "writing {} is not implemented yet",
            format.label()
        ))),
        FormatId::Unknown => Err(Error::unsupported("unknown output format")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_only_v1_formats_in_a_stable_order() {
        let names: Vec<_> = registry().iter().map(|info| info.name).collect();
        assert_eq!(names, ["csv", "tsv", "xlsx", "ods", "html", "json"]);
    }

    #[test]
    fn matrix_has_the_documented_header_and_rows() {
        let matrix = render_matrix();
        let lines: Vec<_> = matrix.lines().collect();

        assert_eq!(
            lines[0], "FORMAT   READ  WRITE  MULTI-TABLE  NOTES",
            "matrix header changed:\n{matrix}"
        );
        assert_eq!(lines.len(), registry().len() + 1);
        assert_eq!(
            lines[1],
            "csv      yes   yes    no           RFC 4180 + dialects"
        );
        assert_eq!(
            lines[5],
            "html     yes   no     yes          table extraction"
        );
        assert_eq!(
            lines[6],
            "json     yes   no     no           JSON/JSONL input"
        );
    }

    #[test]
    fn extension_detection_is_case_insensitive() {
        assert_eq!(FormatId::from_extension("CSV"), Some(FormatId::Csv));
        assert_eq!(FormatId::from_extension(".xlsx"), Some(FormatId::Xlsx));
        assert_eq!(FormatId::from_extension("jsonl"), Some(FormatId::Json));
        assert_eq!(FormatId::from_extension("txt"), None);
    }

    #[test]
    fn capabilities_match_the_scope() {
        let csv = find(FormatId::Csv).expect("csv is registered");
        assert!(csv.capability.read && csv.capability.write);
        assert!(!csv.capability.multi_table);

        let xlsx = find(FormatId::Xlsx).expect("xlsx is registered");
        assert!(xlsx.capability.multi_table);

        let html = find(FormatId::Html).expect("html is registered");
        assert!(html.capability.read);
        assert!(!html.capability.write);

        let json = find(FormatId::Json).expect("json is registered");
        assert!(!json.capability.multi_table);
    }
}
