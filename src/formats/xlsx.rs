//! XLSX/XLS/XLSB reader and XLSX writer dispatch.
//!
//! The reader is shared with ODS in [`super::workbook`]; this module is the
//! stable format-specific entry point used by the registry. The writer uses
//! `rust_xlsxwriter` when the `rust_xlsxwriter` feature is enabled and falls
//! back to the built-in OOXML writer otherwise.

use std::io::{Seek, Write};

use crate::error::Result;
use crate::formats::{FormatId, WriteOptions, workbook};
use crate::model::{SourceRef, Table, TableSet};

pub use workbook::ReadOptions;

/// Return the sheet names in workbook order.
pub fn sheet_names(source: &SourceRef, bytes: &[u8]) -> Result<Vec<String>> {
    workbook::sheet_names(source, bytes)
}

/// Read the requested sheet or sheets.
pub fn read(source: &SourceRef, bytes: &[u8], options: &ReadOptions) -> Result<TableSet> {
    workbook::read(source, bytes, FormatId::Xlsx, options)
}

/// Write one or more tables as an XLSX workbook.
#[cfg(feature = "rust_xlsxwriter")]
pub fn write<W: Write + Seek>(output: W, tables: &[Table], options: &WriteOptions) -> Result<()> {
    super::xlsx_writer::write(output, tables, options)
}

/// Write one or more tables as an XLSX workbook.
#[cfg(not(feature = "rust_xlsxwriter"))]
pub fn write<W: Write + Seek>(output: W, tables: &[Table], options: &WriteOptions) -> Result<()> {
    super::xlsx_manual::write(output, tables, options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::SheetSelection;
    use crate::model::{CellValue, Column, ColumnType};
    use std::io::Cursor;

    fn table(name: &str, columns: Vec<(&str, ColumnType)>, rows: Vec<Vec<CellValue>>) -> Table {
        let source = SourceRef::new("input.csv", FormatId::Csv);
        Table::with_columns(
            name,
            source,
            columns
                .into_iter()
                .map(|(name, ty)| Column::new(name, ty))
                .collect(),
            rows,
        )
    }

    #[test]
    fn writes_and_rereads_native_types() {
        let tables = vec![
            table(
                "Sales",
                vec![
                    ("id", ColumnType::Int),
                    ("name", ColumnType::Text),
                    ("amount", ColumnType::Float),
                    ("active", ColumnType::Bool),
                    ("sold_on", ColumnType::Date),
                ],
                vec![vec![
                    CellValue::Int(1),
                    CellValue::Text("Alice".into()),
                    CellValue::Float(120.5),
                    CellValue::Bool(true),
                    CellValue::Date(chrono::NaiveDate::from_ymd_opt(2026, 1, 15).unwrap()),
                ]],
            ),
            table(
                "Regions",
                vec![("region", ColumnType::Text)],
                vec![vec![CellValue::Text("North".into())]],
            ),
        ];

        let mut output = Cursor::new(Vec::new());
        write(
            &mut output,
            &tables,
            &WriteOptions::for_format(FormatId::Xlsx),
        )
        .unwrap();

        let bytes = output.into_inner();
        let source = SourceRef::new("out.xlsx", FormatId::Xlsx);
        let set = read(
            &source,
            &bytes,
            &ReadOptions {
                has_header: true,
                sheets: SheetSelection::All,
            },
        )
        .unwrap();
        assert_eq!(set.len(), 2);
        assert_eq!(set.tables[0].name, "Sales");
        assert_eq!(set.tables[0].rows[0][0], CellValue::Int(1));
        assert_eq!(set.tables[0].rows[0][1], CellValue::Text("Alice".into()));
        assert_eq!(set.tables[0].rows[0][2], CellValue::Float(120.5));
        assert_eq!(set.tables[0].rows[0][3], CellValue::Bool(true));
        assert!(matches!(set.tables[0].rows[0][4], CellValue::Date(_)));
    }

    #[test]
    fn writes_a_valid_package_with_expected_parts() {
        let tables = vec![table(
            "data",
            vec![("name", ColumnType::Text)],
            vec![vec![CellValue::Text("a".into())]],
        )];
        let mut output = Cursor::new(Vec::new());
        write(
            &mut output,
            &tables,
            &WriteOptions::for_format(FormatId::Xlsx),
        )
        .unwrap();
        let bytes = output.into_inner();

        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        for part in [
            "[Content_Types].xml",
            "_rels/.rels",
            "xl/workbook.xml",
            "xl/_rels/workbook.xml.rels",
            "xl/styles.xml",
            "xl/worksheets/sheet1.xml",
        ] {
            assert!(archive.by_name(part).is_ok(), "missing part {part}");
        }
    }

    #[test]
    fn control_characters_fail_with_cell_context() {
        let tables = vec![table(
            "data",
            vec![("name", ColumnType::Text)],
            vec![vec![CellValue::Text("bad\u{0001}".into())]],
        )];
        let mut output = Cursor::new(Vec::new());
        let error = write(
            &mut output,
            &tables,
            &WriteOptions::for_format(FormatId::Xlsx),
        )
        .unwrap_err();
        assert!(error.to_string().contains("control character"));
    }
}
