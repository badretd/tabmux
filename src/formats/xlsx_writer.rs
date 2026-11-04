//! XLSX writer backed by the `rust_xlsxwriter` crate.
//!
//! Enabled with the `rust_xlsxwriter` Cargo feature. The manual OOXML writer
//! remains the default fallback so the repository still builds in the offline
//! registry. Both implementations expose the same `formats::write_tables`
//! contract and produce native cell types, a bold header, a frozen top row, an
//! autofilter, and readable column widths.

use std::io::{Seek, Write};

use rust_xlsxwriter::{Format, Workbook, Worksheet, XlsxError};

use crate::error::{Error, Result};
use crate::formats::{WriteOptions, workbook_write};
use crate::model::{CellValue, SourceRef, Table};

const SAFE_NUMBER_LIMIT: i64 = 9_007_199_254_740_992;
const DATE_FORMAT: &str = "yyyy-mm-dd";
const DATETIME_FORMAT: &str = "yyyy-mm-dd hh:mm:ss";
const TIME_FORMAT: &str = "[h]:mm:ss";

/// Write one or more tables as an XLSX workbook.
pub fn write<W: Write + Seek>(
    mut output: W,
    tables: &[Table],
    options: &WriteOptions,
) -> Result<()> {
    if tables.is_empty() {
        return Err(Error::data("no tables to write"));
    }

    let prepared = tables
        .iter()
        .map(workbook_write::prepare_table)
        .collect::<Vec<_>>();
    let sheet_names = workbook_write::normalize_sheet_names(&prepared);

    let mut workbook = Workbook::new();
    let header_format = Format::new().set_bold();
    let date_format = Format::new().set_num_format(DATE_FORMAT);
    let datetime_format = Format::new().set_num_format(DATETIME_FORMAT);
    let time_format = Format::new().set_num_format(TIME_FORMAT);

    for (table_index, table) in prepared.iter().enumerate() {
        let sheet_name = sheet_names
            .get(table_index)
            .map(String::as_str)
            .unwrap_or("Sheet");
        let source = table.source.clone().with_sheet(sheet_name.to_owned());
        let width = table.width();

        let worksheet = workbook.add_worksheet();
        worksheet.set_name(sheet_name).map_err(xlsx_error)?;

        if width > 0 {
            for (column, width) in workbook_write::column_widths(table, options.has_header)
                .iter()
                .enumerate()
            {
                worksheet
                    .set_column_width(column as u16, *width as f64)
                    .map_err(xlsx_error)?;
            }
        }

        if options.has_header && width > 0 {
            for (column, definition) in table.columns.iter().enumerate() {
                workbook_write::escape_xml_text(&definition.name, &source)?;
                worksheet
                    .write_string_with_format(
                        0,
                        column as u16,
                        definition.name.as_str(),
                        &header_format,
                    )
                    .map_err(xlsx_error)?;
            }
            worksheet.set_freeze_panes(1, 0).map_err(xlsx_error)?;

            if !table.rows.is_empty() {
                worksheet
                    .autofilter(0, 0, table.rows.len() as u32, (width - 1) as u16)
                    .map_err(xlsx_error)?;
            }
        }

        let row_offset = usize::from(options.has_header && width > 0);
        for (row_index, row) in table.rows.iter().enumerate() {
            let row_number = (row_index + row_offset) as u32;
            for (column, value) in row.iter().enumerate().take(width) {
                write_cell(
                    worksheet,
                    row_number,
                    column as u16,
                    value,
                    &source,
                    &date_format,
                    &datetime_format,
                    &time_format,
                )?;
            }
        }
    }

    let buffer = workbook.save_to_buffer().map_err(xlsx_error)?;
    output.write_all(&buffer)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_cell(
    worksheet: &mut Worksheet,
    row: u32,
    column: u16,
    value: &CellValue,
    source: &SourceRef,
    date_format: &Format,
    datetime_format: &Format,
    time_format: &Format,
) -> Result<()> {
    match value {
        CellValue::Null => {}
        CellValue::Bool(value) => {
            worksheet
                .write_boolean(row, column, *value)
                .map_err(xlsx_error)?;
        }
        CellValue::Int(value) => {
            if (-SAFE_NUMBER_LIMIT..=SAFE_NUMBER_LIMIT).contains(value) {
                worksheet
                    .write_number(row, column, *value as f64)
                    .map_err(xlsx_error)?;
            } else {
                write_string(worksheet, row, column, &value.to_string(), source)?;
            }
        }
        CellValue::Float(value) if value.is_finite() => {
            worksheet
                .write_number(row, column, *value)
                .map_err(xlsx_error)?;
        }
        CellValue::Float(value) => {
            write_string(worksheet, row, column, &value.to_string(), source)?;
        }
        CellValue::Date(value) if workbook_write::is_excel_date_supported(*value) => {
            worksheet
                .write_datetime_with_format(row, column, *value, date_format)
                .map_err(xlsx_error)?;
        }
        CellValue::DateTime(value) if workbook_write::is_excel_date_supported(value.date()) => {
            worksheet
                .write_datetime_with_format(row, column, *value, datetime_format)
                .map_err(xlsx_error)?;
        }
        CellValue::Time(value) => {
            worksheet
                .write_datetime_with_format(row, column, *value, time_format)
                .map_err(xlsx_error)?;
        }
        CellValue::Date(value) => {
            write_string(
                worksheet,
                row,
                column,
                &value.format("%Y-%m-%d").to_string(),
                source,
            )?;
        }
        CellValue::DateTime(value) => {
            write_string(
                worksheet,
                row,
                column,
                &value.format("%Y-%m-%dT%H:%M:%S").to_string(),
                source,
            )?;
        }
        CellValue::Decimal(value) | CellValue::Text(value) | CellValue::Error(value) => {
            write_string(worksheet, row, column, value, source)?;
        }
    }
    Ok(())
}

fn write_string(
    worksheet: &mut Worksheet,
    row: u32,
    column: u16,
    value: &str,
    source: &SourceRef,
) -> Result<()> {
    // Reuse the manual writer's XML validation so control characters are
    // rejected with the same file/sheet/cell context.
    workbook_write::escape_xml_text(value, source)?;
    worksheet
        .write_string(row, column, value)
        .map_err(xlsx_error)?;
    Ok(())
}

fn xlsx_error(error: XlsxError) -> Error {
    match error {
        XlsxError::IoError(error) => Error::Io(error),
        other => Error::data(format!("cannot write XLSX workbook: {other}")),
    }
}
