//! Core table model shared by readers, operations, and writers.

pub mod types;

use std::fmt;
use std::path::PathBuf;

use crate::formats::FormatId;

pub use types::{CellValue, ColumnType};

/// One row of cells.
pub type Row = Vec<CellValue>;

/// Materialized rows for v1. CSV streaming will use a separate path once the
/// conversion core is stable.
pub type RowSet = Vec<Row>;

/// Location of data for diagnostics and loss reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRef {
    pub path: PathBuf,
    pub format: FormatId,
    pub sheet: Option<String>,
    pub table: Option<String>,
    /// Caption text for sources that have one (currently HTML tables).
    pub caption: Option<String>,
    /// Parent table marker when this table was extracted from a nested HTML
    /// table (for example `table-2`).
    pub nested_in: Option<String>,
    pub row: Option<u64>,
    pub column: Option<String>,
    pub cell: Option<String>,
}

impl SourceRef {
    /// Create a source reference for a whole file or stream.
    pub fn new(path: impl Into<PathBuf>, format: FormatId) -> Self {
        Self {
            path: path.into(),
            format,
            sheet: None,
            table: None,
            caption: None,
            nested_in: None,
            row: None,
            column: None,
            cell: None,
        }
    }

    /// Attach a sheet name.
    pub fn with_sheet(mut self, sheet: impl Into<String>) -> Self {
        self.sheet = Some(sheet.into());
        self
    }

    /// Attach an HTML table name or index.
    pub fn with_table(mut self, table: impl Into<String>) -> Self {
        self.table = Some(table.into());
        self
    }

    /// Attach a caption (HTML table captions).
    pub fn with_caption(mut self, caption: impl Into<String>) -> Self {
        self.caption = Some(caption.into());
        self
    }

    /// Attach a caption if one is present.
    pub fn with_caption_opt(mut self, caption: Option<String>) -> Self {
        self.caption = caption;
        self
    }

    /// Record the parent table when this table came from a nested table.
    pub fn with_nested_in(mut self, parent: impl Into<String>) -> Self {
        self.nested_in = Some(parent.into());
        self
    }

    /// Attach a 1-based data row.
    pub fn with_row(mut self, row: u64) -> Self {
        self.row = Some(row);
        self
    }

    /// Attach a column name.
    pub fn with_column(mut self, column: impl Into<String>) -> Self {
        self.column = Some(column.into());
        self
    }

    /// Attach an A1-style cell reference where the source has one.
    pub fn with_cell(mut self, cell: impl Into<String>) -> Self {
        self.cell = Some(cell.into());
        self
    }

    /// Short, single-line context for warnings and errors.
    pub fn context(&self) -> String {
        let mut out = self.path.display().to_string();
        if let Some(sheet) = &self.sheet {
            out.push_str(&format!("#{sheet}"));
        }
        if let Some(table) = &self.table {
            out.push_str(&format!("#{table}"));
        }
        if let Some(row) = self.row {
            out.push_str(&format!(" row {row}"));
        }
        if let Some(column) = &self.column {
            out.push_str(&format!(" column '{column}'"));
        }
        if let Some(cell) = &self.cell {
            out.push_str(&format!(" cell {cell}"));
        }
        out
    }
}

impl fmt::Display for SourceRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.context())
    }
}

/// Severity of a loss or quality event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// A machine-readable warning attached to a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LossEvent {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    pub source: Option<SourceRef>,
    pub count: u64,
}

impl LossEvent {
    pub fn new(severity: Severity, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity,
            code,
            message: message.into(),
            source: None,
            count: 1,
        }
    }

    pub fn info(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(Severity::Info, code, message)
    }

    pub fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, code, message)
    }

    pub fn with_source(mut self, source: SourceRef) -> Self {
        self.source = Some(source);
        self
    }

    /// Set the occurrence count for an aggregated event.
    pub fn with_count(mut self, count: u64) -> Self {
        self.count = count.max(1);
        self
    }
}

/// A named column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub ty: ColumnType,
    pub source: Option<SourceRef>,
}

impl Column {
    pub fn new(name: impl Into<String>, ty: ColumnType) -> Self {
        Self {
            name: name.into(),
            ty,
            source: None,
        }
    }
}

/// A single table (a sheet, an HTML table, an archive member, ...).
#[derive(Debug, Clone)]
pub struct Table {
    pub name: String,
    pub source: SourceRef,
    pub columns: Vec<Column>,
    pub rows: RowSet,
    pub warnings: Vec<LossEvent>,
    /// 1-based source row number of the first data row. Readers keep this
    /// consistent with the row numbers used in diagnostics so filtered
    /// commands can tell whether an event belongs to an output row.
    pub first_data_row: u64,
}

impl Table {
    pub fn new(name: impl Into<String>, source: SourceRef) -> Self {
        Self {
            name: name.into(),
            source,
            columns: Vec::new(),
            rows: Vec::new(),
            warnings: Vec::new(),
            first_data_row: 1,
        }
    }

    pub fn with_columns(
        name: impl Into<String>,
        source: SourceRef,
        columns: Vec<Column>,
        rows: RowSet,
    ) -> Self {
        Self {
            name: name.into(),
            source,
            columns,
            rows,
            warnings: Vec::new(),
            first_data_row: 1,
        }
    }

    /// Source row number for a zero-based data-row index.
    pub fn source_row(&self, index: usize) -> u64 {
        self.first_data_row.saturating_add(index as u64)
    }

    pub fn width(&self) -> usize {
        self.columns.len()
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn column_names(&self) -> Vec<&str> {
        self.columns
            .iter()
            .map(|column| column.name.as_str())
            .collect()
    }

    /// Find a column by exact name.
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|column| column.name == name)
    }

    pub fn push_warning(&mut self, warning: LossEvent) {
        self.warnings.push(warning);
    }
}

/// A table that was deliberately not selected because it failed a reader
/// filter (currently the HTML `--min-rows`/`--min-cols` layout-table filter).
#[derive(Debug, Clone)]
pub struct SkippedTable {
    /// Source location, including the `table-N` marker where available.
    pub source: SourceRef,
    /// 1-based table number within the source.
    pub table: usize,
    pub caption: Option<String>,
    pub rows: usize,
    pub columns: usize,
    /// Human-readable reason, including the failing threshold.
    pub reason: String,
}

/// A collection of tables produced by one input (workbook sheets, HTML tables,
/// archive members).
#[derive(Debug, Clone)]
pub struct TableSet {
    pub source: SourceRef,
    pub tables: Vec<Table>,
    /// Tables the reader intentionally skipped, in document order.
    pub skipped: Vec<SkippedTable>,
}

impl TableSet {
    pub fn new(source: SourceRef) -> Self {
        Self {
            source,
            tables: Vec::new(),
            skipped: Vec::new(),
        }
    }

    pub fn push(&mut self, table: Table) {
        self.tables.push(table);
    }

    pub fn len(&self) -> usize {
        self.tables.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    pub fn into_single(mut self) -> crate::error::Result<Table> {
        match self.tables.len() {
            0 => Err(crate::error::Error::data(format!(
                "no tables found in {}",
                self.source.path.display()
            ))),
            1 => Ok(self.tables.remove(0)),
            count => Err(crate::error::Error::data(format!(
                "expected one table in {}, found {count}",
                self.source.path.display()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> SourceRef {
        SourceRef::new("data.csv", FormatId::Csv)
    }

    #[test]
    fn source_context_includes_row_and_column() {
        let source = source().with_row(4).with_column("email");
        assert_eq!(source.context(), "data.csv row 4 column 'email'");
    }

    #[test]
    fn column_lookup_is_exact_and_ordered() {
        let table = Table::with_columns(
            "data",
            source(),
            vec![
                Column::new("name", ColumnType::Text),
                Column::new("amount", ColumnType::Int),
            ],
            vec![vec![CellValue::Text("a".into()), CellValue::Int(1)]],
        );
        assert_eq!(table.column_names(), ["name", "amount"]);
        assert_eq!(table.column_index("amount"), Some(1));
        assert_eq!(table.column_index("Amount"), None);
        assert_eq!(table.width(), 2);
        assert_eq!(table.row_count(), 1);
    }
}
