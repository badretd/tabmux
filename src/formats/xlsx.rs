//! XLSX/XLS/XLSB reader.
//!
//! The implementation is shared with ODS in [`super::workbook`]; this module is
//! the stable format-specific entry point used by the registry.

use crate::error::Result;
use crate::formats::{FormatId, workbook};
use crate::model::{SourceRef, TableSet};

pub use workbook::ReadOptions;

/// Return the sheet names in workbook order.
pub fn sheet_names(source: &SourceRef, bytes: &[u8]) -> Result<Vec<String>> {
    workbook::sheet_names(source, bytes)
}

/// Read the requested sheet or sheets.
pub fn read(source: &SourceRef, bytes: &[u8], options: &ReadOptions) -> Result<TableSet> {
    workbook::read(source, bytes, FormatId::Xlsx, options)
}
