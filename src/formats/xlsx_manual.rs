//! Manual XLSX writer used when the `rust_xlsxwriter` feature is disabled.
//!
//! It emits a small, standards-compliant OOXML package with values as native
//! types, a bold header, a frozen top row, an autofilter, and readable column
//! widths. The `rust_xlsxwriter` feature swaps this implementation for the
//! planned library-backed writer without changing the public API.

use std::io::{Seek, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::error::{Error, Result};
use crate::formats::{FormatId, WriteOptions, workbook_write};
use crate::model::{CellValue, SourceRef, Table};

/// Write one or more tables as an XLSX workbook.
pub fn write<W: Write + Seek>(output: W, tables: &[Table], options: &WriteOptions) -> Result<()> {
    if tables.is_empty() {
        return Err(Error::data("no tables to write"));
    }

    let prepared = tables
        .iter()
        .map(workbook_write::prepare_table)
        .collect::<Vec<_>>();
    let sheet_names = workbook_write::normalize_sheet_names(&prepared);

    let mut zip = ZipWriter::new(output);
    let file_options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    write_entry(
        &mut zip,
        file_options,
        "[Content_Types].xml",
        &content_types(prepared.len()),
    )?;
    write_entry(&mut zip, file_options, "_rels/.rels", ROOT_RELS)?;
    write_entry(
        &mut zip,
        file_options,
        "xl/workbook.xml",
        &workbook_xml(&sheet_names)?,
    )?;
    write_entry(
        &mut zip,
        file_options,
        "xl/_rels/workbook.xml.rels",
        &workbook_rels(prepared.len()),
    )?;
    write_entry(&mut zip, file_options, "xl/styles.xml", STYLES)?;

    for (index, table) in prepared.iter().enumerate() {
        let xml = sheet_xml(table, options.has_header)?;
        write_entry(
            &mut zip,
            file_options,
            &format!("xl/worksheets/sheet{}.xml", index + 1),
            &xml,
        )?;
    }

    zip.finish().map_err(zip_error)?;
    Ok(())
}

fn write_entry(
    zip: &mut ZipWriter<impl Write + Seek>,
    options: SimpleFileOptions,
    name: &str,
    content: &str,
) -> Result<()> {
    zip.start_file(name, options).map_err(zip_error)?;
    zip.write_all(content.as_bytes())?;
    Ok(())
}

fn zip_error(error: zip::result::ZipError) -> Error {
    match error {
        zip::result::ZipError::Io(error) => Error::Io(error),
        other => Error::data(format!("cannot write XLSX package: {other}")),
    }
}

const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>
"#;

const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <numFmts count="3">
    <numFmt numFmtId="164" formatCode="yyyy-mm-dd"/>
    <numFmt numFmtId="165" formatCode="yyyy-mm-dd hh:mm:ss"/>
    <numFmt numFmtId="166" formatCode="[h]:mm:ss"/>
  </numFmts>
  <fonts count="2">
    <font><sz val="11"/><name val="Calibri"/><family val="2"/></font>
    <font><b/><sz val="11"/><name val="Calibri"/><family val="2"/></font>
  </fonts>
  <fills count="2">
    <fill><patternFill patternType="none"/></fill>
    <fill><patternFill patternType="gray125"/></fill>
  </fills>
  <borders count="1">
    <border><left/><right/><top/><bottom/><diagonal/></border>
  </borders>
  <cellStyleXfs count="1">
    <xf numFmtId="0" fontId="0" fillId="0" borderId="0"/>
  </cellStyleXfs>
  <cellXfs count="5">
    <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
    <xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/>
    <xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
    <xf numFmtId="165" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
    <xf numFmtId="166" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
  </cellXfs>
  <cellStyles count="1">
    <cellStyle name="Normal" xfId="0" builtinId="0"/>
  </cellStyles>
</styleSheet>
"#;

fn content_types(sheet_count: usize) -> String {
    let mut overrides = String::new();
    for index in 1..=sheet_count {
        overrides.push_str(&format!(
            "  <Override PartName=\"/xl/worksheets/sheet{index}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\n"
        ));
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
{overrides}</Types>
"#
    )
}

fn workbook_xml(sheet_names: &[String]) -> Result<String> {
    let source = SourceRef::new("workbook.xml", FormatId::Xlsx);
    let mut sheets = String::new();
    for (index, name) in sheet_names.iter().enumerate() {
        let escaped = workbook_write::escape_xml_attr(name, &source)?;
        sheets.push_str(&format!(
            "    <sheet name=\"{escaped}\" sheetId=\"{}\" r:id=\"rId{}\"/>\n",
            index + 1,
            index + 1
        ));
    }

    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
{sheets}  </sheets>
</workbook>
"#
    ))
}

fn workbook_rels(sheet_count: usize) -> String {
    let mut relationships = String::new();
    for index in 1..=sheet_count {
        relationships.push_str(&format!(
            "  <Relationship Id=\"rId{index}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet{index}.xml\"/>\n"
        ));
    }
    relationships.push_str(&format!(
        "  <Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\n",
        sheet_count + 1
    ));

    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
{relationships}</Relationships>
"#
    )
}

fn sheet_xml(table: &Table, has_header: bool) -> Result<String> {
    let width = table.width();
    let data_rows = table.rows.len();
    let row_count = data_rows + usize::from(has_header && width > 0);
    let last_row = row_count.max(1);
    let last_column = if width == 0 {
        "A".to_owned()
    } else {
        workbook_write::column_label(width - 1)
    };
    let dimension = format!("A1:{last_column}{last_row}");
    let source = table.source.clone().with_sheet(table.name.clone());

    let mut xml = String::with_capacity(1024 + data_rows * width * 32);
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
    xml.push_str(
        "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\n",
    );
    xml.push_str(&format!("  <dimension ref=\"{dimension}\"/>\n"));

    if has_header && width > 0 {
        xml.push_str(
            "  <sheetViews><sheetView workbookViewId=\"0\"><pane ySplit=\"1\" topLeftCell=\"A2\" activePane=\"bottomLeft\" state=\"frozen\"/></sheetView></sheetViews>\n",
        );
    }
    xml.push_str("  <sheetFormatPr defaultRowHeight=\"15\"/>\n");

    if width > 0 {
        let widths = workbook_write::column_widths(table, has_header);
        xml.push_str("  <cols>\n");
        for (index, width) in widths.iter().enumerate() {
            xml.push_str(&format!(
                "    <col min=\"{}\" max=\"{}\" width=\"{width}\" customWidth=\"1\"/>\n",
                index + 1,
                index + 1
            ));
        }
        xml.push_str("  </cols>\n");
    }

    xml.push_str("  <sheetData>\n");

    if has_header && width > 0 {
        xml.push_str("    <row r=\"1\">\n");
        for (column, definition) in table.columns.iter().enumerate() {
            let reference = workbook_write::cell_reference(0, column);
            let escaped = workbook_write::escape_xml_text(&definition.name, &source)?;
            xml.push_str(&format!(
                "      <c r=\"{reference}\" s=\"1\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{escaped}</t></is></c>\n"
            ));
        }
        xml.push_str("    </row>\n");
    }

    let row_offset = usize::from(has_header && width > 0);
    for (row_index, row) in table.rows.iter().enumerate() {
        let excel_row = row_index + row_offset;
        let row_number = excel_row + 1;
        xml.push_str(&format!("    <row r=\"{row_number}\">\n"));
        for (column, value) in row.iter().enumerate().take(width) {
            if value.is_null() {
                continue;
            }
            let reference = workbook_write::cell_reference(excel_row, column);
            push_cell(&mut xml, &reference, value, &source)?;
        }
        xml.push_str("    </row>\n");
    }

    xml.push_str("  </sheetData>\n");

    if has_header && width > 0 && !table.rows.is_empty() {
        let filter_last = workbook_write::column_label(width - 1);
        xml.push_str(&format!(
            "  <autoFilter ref=\"A1:{filter_last}{}\"/>\n",
            table.rows.len() + 1
        ));
    }

    xml.push_str("</worksheet>\n");
    Ok(xml)
}

fn push_cell(
    xml: &mut String,
    reference: &str,
    value: &CellValue,
    source: &SourceRef,
) -> Result<()> {
    match value {
        CellValue::Null => {}
        CellValue::Bool(value) => {
            xml.push_str(&format!(
                "      <c r=\"{reference}\" t=\"b\"><v>{}</v></c>\n",
                u8::from(*value)
            ));
        }
        CellValue::Int(value) => {
            xml.push_str(&format!("      <c r=\"{reference}\"><v>{value}</v></c>\n"));
        }
        CellValue::Float(value) if value.is_finite() => {
            xml.push_str(&format!("      <c r=\"{reference}\"><v>{value}</v></c>\n"));
        }
        CellValue::Date(value) if workbook_write::is_excel_date_supported(*value) => {
            xml.push_str(&format!(
                "      <c r=\"{reference}\" s=\"2\"><v>{}</v></c>\n",
                workbook_write::excel_date_serial(*value)
            ));
        }
        CellValue::DateTime(value) if workbook_write::is_excel_date_supported(value.date()) => {
            xml.push_str(&format!(
                "      <c r=\"{reference}\" s=\"3\"><v>{:.10}</v></c>\n",
                workbook_write::excel_datetime_serial(*value)
            ));
        }
        CellValue::Time(value) => {
            xml.push_str(&format!(
                "      <c r=\"{reference}\" s=\"4\"><v>{:.10}</v></c>\n",
                workbook_write::excel_time_fraction(*value)
            ));
        }
        CellValue::Float(value) => {
            push_inline_string(xml, reference, &value.to_string(), source)?;
        }
        CellValue::Decimal(value) | CellValue::Text(value) | CellValue::Error(value) => {
            push_inline_string(xml, reference, value, source)?;
        }
        CellValue::Date(value) => {
            push_inline_string(
                xml,
                reference,
                &value.format("%Y-%m-%d").to_string(),
                source,
            )?;
        }
        CellValue::DateTime(value) => {
            push_inline_string(
                xml,
                reference,
                &value.format("%Y-%m-%dT%H:%M:%S").to_string(),
                source,
            )?;
        }
    }
    Ok(())
}

fn push_inline_string(
    xml: &mut String,
    reference: &str,
    value: &str,
    source: &SourceRef,
) -> Result<()> {
    let escaped = workbook_write::escape_xml_text(value, source)?;
    xml.push_str(&format!(
        "      <c r=\"{reference}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{escaped}</t></is></c>\n"
    ));
    Ok(())
}
