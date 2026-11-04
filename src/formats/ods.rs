//! ODS reader and writer.
//!
//! The reader is shared with XLSX in [`super::workbook`]. ODS merged cells are
//! expanded by inspecting the package because `calamine` does not expose ODS
//! merge ranges. The writer emits a minimal, valid ODF spreadsheet package with
//! native cell types, a bold header, and readable column widths.

use std::io::{Seek, Write};

use chrono::{NaiveDateTime, NaiveTime, Timelike};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::error::{Error, Result};
use crate::formats::{FormatId, WriteOptions, workbook, workbook_write};
use crate::model::{CellValue, SourceRef, Table, TableSet};

pub use workbook::ReadOptions;

/// Return the sheet names in workbook order.
pub fn sheet_names(source: &SourceRef, bytes: &[u8]) -> Result<Vec<String>> {
    workbook::sheet_names(source, bytes)
}

/// Read the requested sheet or sheets.
pub fn read(source: &SourceRef, bytes: &[u8], options: &ReadOptions) -> Result<TableSet> {
    workbook::read(source, bytes, FormatId::Ods, options)
}

const MIMETYPE: &str = "application/vnd.oasis.opendocument.spreadsheet";

/// Write one or more tables as an ODS workbook.
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
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    // ODF requires the mimetype entry to be first and uncompressed.
    zip.start_file("mimetype", stored).map_err(zip_error)?;
    zip.write_all(MIMETYPE.as_bytes())?;

    let content = content_xml(&prepared, &sheet_names, options.has_header)?;
    write_entry(&mut zip, deflated, "content.xml", &content)?;
    write_entry(&mut zip, deflated, "styles.xml", STYLES)?;
    write_entry(&mut zip, deflated, "meta.xml", META)?;
    write_entry(&mut zip, deflated, "settings.xml", SETTINGS)?;
    write_entry(&mut zip, deflated, "META-INF/manifest.xml", MANIFEST)?;

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
        other => Error::data(format!("cannot write ODS package: {other}")),
    }
}

const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-styles
  xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
  xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"
  office:version="1.2">
  <office:styles>
    <style:style style:name="Default" style:family="table-cell"/>
  </office:styles>
</office:document-styles>
"#;

const SETTINGS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-settings
  xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  office:version="1.2">
  <office:settings/>
</office:document-settings>
"#;

const META: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-meta
  xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0"
  office:version="1.2">
  <office:meta><meta:generator>tabmux</meta:generator></office:meta>
</office:document-meta>
"#;

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest
  xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"
  manifest:version="1.2">
  <manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/>
  <manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
  <manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/>
  <manifest:file-entry manifest:full-path="meta.xml" manifest:media-type="text/xml"/>
  <manifest:file-entry manifest:full-path="settings.xml" manifest:media-type="text/xml"/>
</manifest:manifest>
"#;

fn content_xml(tables: &[Table], sheet_names: &[String], has_header: bool) -> Result<String> {
    let mut xml = String::with_capacity(2048);
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str(
        "<office:document-content \
         xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
         xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" \
         xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" \
         xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" \
         xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" \
         office:version=\"1.2\">\n",
    );

    xml.push_str("  <office:automatic-styles>\n");
    xml.push_str(
        "    <style:style style:name=\"ce_header\" style:family=\"table-cell\">\
         <style:text-properties fo:font-weight=\"bold\"/></style:style>\n",
    );

    for (table_index, table) in tables.iter().enumerate() {
        let widths = workbook_write::column_widths(table, has_header);
        for (column_index, width) in widths.iter().enumerate() {
            let centimeters = ((*width as f64) * 0.18 + 0.4).clamp(1.5, 12.0);
            xml.push_str(&format!(
                "    <style:style style:name=\"co{}_{}\" style:family=\"table-column\">\
                 <style:table-column-properties style:column-width=\"{centimeters:.2}cm\"/>\
                 </style:style>\n",
                table_index + 1,
                column_index + 1
            ));
        }
        xml.push_str(&format!(
            "    <style:style style:name=\"ta{}\" style:family=\"table\">\
             <style:table-properties table:display=\"true\"/></style:style>\n",
            table_index + 1
        ));
    }
    xml.push_str("  </office:automatic-styles>\n");
    xml.push_str("  <office:body><office:spreadsheet>\n");

    for (table_index, table) in tables.iter().enumerate() {
        let name = sheet_names
            .get(table_index)
            .map(String::as_str)
            .unwrap_or(&table.name);
        let source = table.source.clone().with_sheet(name.to_owned());
        let escaped_name = workbook_write::escape_xml_attr(name, &source)?;
        xml.push_str(&format!(
            "    <table:table table:name=\"{escaped_name}\" table:style-name=\"ta{}\">\n",
            table_index + 1
        ));

        for column_index in 0..table.width() {
            xml.push_str(&format!(
                "      <table:table-column table:style-name=\"co{}_{}\"/>\n",
                table_index + 1,
                column_index + 1
            ));
        }

        if has_header && table.width() > 0 {
            xml.push_str("      <table:table-row>");
            for definition in &table.columns {
                let text = workbook_write::ods_text_element(&definition.name, &source)?;
                xml.push_str(&format!(
                    "<table:table-cell table:style-name=\"ce_header\" office:value-type=\"string\">{text}</table:table-cell>"
                ));
            }
            xml.push_str("</table:table-row>\n");
        }

        for row in &table.rows {
            xml.push_str("      <table:table-row>");
            for value in row.iter().take(table.width()) {
                push_cell(&mut xml, value, &source)?;
            }
            xml.push_str("</table:table-row>\n");
        }

        xml.push_str("    </table:table>\n");
    }

    xml.push_str("  </office:spreadsheet></office:body>\n</office:document-content>\n");
    Ok(xml)
}

fn push_cell(xml: &mut String, value: &CellValue, source: &SourceRef) -> Result<()> {
    match value {
        CellValue::Null => xml.push_str("<table:table-cell/>"),
        CellValue::Bool(value) => {
            xml.push_str(&format!(
                "<table:table-cell office:value-type=\"boolean\" office:boolean-value=\"{value}\"><text:p>{value}</text:p></table:table-cell>"
            ));
        }
        CellValue::Int(value) => {
            xml.push_str(&format!(
                "<table:table-cell office:value-type=\"float\" office:value=\"{value}\"><text:p>{value}</text:p></table:table-cell>"
            ));
        }
        CellValue::Float(value) if value.is_finite() => {
            xml.push_str(&format!(
                "<table:table-cell office:value-type=\"float\" office:value=\"{value}\"><text:p>{value}</text:p></table:table-cell>"
            ));
        }
        CellValue::Date(value) => {
            let rendered = value.format("%Y-%m-%d").to_string();
            xml.push_str(&format!(
                "<table:table-cell office:value-type=\"date\" office:date-value=\"{rendered}\"><text:p>{rendered}</text:p></table:table-cell>"
            ));
        }
        CellValue::DateTime(value) => {
            let value_text = iso_datetime(*value);
            xml.push_str(&format!(
                "<table:table-cell office:value-type=\"date\" office:date-value=\"{value_text}\"><text:p>{value_text}</text:p></table:table-cell>"
            ));
        }
        CellValue::Time(value) => {
            let duration = iso_duration(*value);
            let text = value.format("%H:%M:%S").to_string();
            xml.push_str(&format!(
                "<table:table-cell office:value-type=\"time\" office:time-value=\"{duration}\"><text:p>{text}</text:p></table:table-cell>"
            ));
        }
        CellValue::Float(value) => push_string_cell(xml, &value.to_string(), source)?,
        CellValue::Decimal(value) | CellValue::Text(value) | CellValue::Error(value) => {
            push_string_cell(xml, value, source)?;
        }
    }
    Ok(())
}

fn push_string_cell(xml: &mut String, value: &str, source: &SourceRef) -> Result<()> {
    let text = workbook_write::ods_text_element(value, source)?;
    xml.push_str(&format!(
        "<table:table-cell office:value-type=\"string\">{text}</table:table-cell>"
    ));
    Ok(())
}

fn iso_datetime(value: NaiveDateTime) -> String {
    let mut rendered = value.format("%Y-%m-%dT%H:%M:%S").to_string();
    let nanos = value.and_utc().timestamp_subsec_nanos();
    if nanos > 0 {
        let fraction = format!("{nanos:09}");
        rendered.push('.');
        rendered.push_str(fraction.trim_end_matches('0'));
    }
    rendered
}

fn iso_duration(value: NaiveTime) -> String {
    let mut rendered = format!(
        "PT{:02}H{:02}M{:02}",
        value.hour(),
        value.minute(),
        value.second()
    );
    let nanos = value.nanosecond();
    if nanos > 0 {
        let fraction = format!("{nanos:09}");
        rendered.push('.');
        rendered.push_str(fraction.trim_end_matches('0'));
    }
    rendered.push('S');
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::SheetSelection;
    use crate::model::{Column, ColumnType};
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
            &WriteOptions::for_format(FormatId::Ods),
        )
        .unwrap();

        let bytes = output.into_inner();
        let source = SourceRef::new("out.ods", FormatId::Ods);
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
    fn writes_a_valid_odf_package() {
        let tables = vec![table(
            "data",
            vec![("name", ColumnType::Text)],
            vec![vec![CellValue::Text("a".into())]],
        )];
        let mut output = Cursor::new(Vec::new());
        write(
            &mut output,
            &tables,
            &WriteOptions::for_format(FormatId::Ods),
        )
        .unwrap();
        let bytes = output.into_inner();

        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        for part in [
            "mimetype",
            "content.xml",
            "styles.xml",
            "meta.xml",
            "settings.xml",
            "META-INF/manifest.xml",
        ] {
            assert!(archive.by_name(part).is_ok(), "missing part {part}");
        }
    }

    #[test]
    fn time_values_use_iso_durations() {
        assert_eq!(
            iso_duration(NaiveTime::from_hms_opt(12, 30, 0).unwrap()),
            "PT12H30M00S"
        );
        assert_eq!(
            iso_duration(NaiveTime::from_hms_nano_opt(8, 5, 30, 500_000_000).unwrap()),
            "PT08H05M30.5S"
        );
    }
}
