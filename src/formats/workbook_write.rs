//! Shared helpers for the XLSX and ODS writers.
//!
//! Both writers accept the same [`Table`] model and need the same conservative
//! type inference, sheet-name sanitisation, column-width calculation, and XML
//! escaping. Keeping those rules in one place makes the two output formats
//! behave predictably and keeps the loss policy identical.
//!
//! Delimited sources are kept byte-faithful when they are written back as
//! delimited text. Only workbook output applies type inference, and it never
//! reinterprets values that could lose meaning (for example `007` stays text).

use std::collections::HashSet;

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike};

use crate::error::{Error, Result};
use crate::formats::FormatId;
use crate::model::{CellValue, ColumnType, SourceRef, Table};

/// Excel's hard limit for worksheet names.
pub const MAX_SHEET_NAME: usize = 31;

/// Prepare a table for a workbook writer.
///
/// Tables read from delimited sources carry text cells so CSV/TSV conversion
/// stays byte-faithful. Workbook output is different: native integers, floats,
/// booleans, and dates make the generated workbook useful in a spreadsheet
/// application. Every other source already carries types and is only cloned.
pub fn prepare_table(table: &Table) -> Table {
    let mut prepared = table.clone();
    if matches!(prepared.source.format, FormatId::Csv | FormatId::Tsv) {
        infer_text_types(&mut prepared);
    }
    prepared
}

/// Infer conservative types for an all-text table and convert the cells.
pub fn infer_text_types(table: &mut Table) {
    if table.width() == 0 {
        return;
    }

    let mut inferred = vec![ColumnType::Null; table.width()];
    for row in &table.rows {
        for (index, cell) in row.iter().enumerate().take(inferred.len()) {
            inferred[index] = inferred[index].merge(infer_type(cell));
        }
    }

    for (index, inferred_type) in inferred.into_iter().enumerate() {
        let current = table.columns[index].ty;
        let target = if matches!(current, ColumnType::Text | ColumnType::Null) {
            inferred_type
        } else {
            current
        };
        table.columns[index].ty = target;
    }

    for row in &mut table.rows {
        for (index, cell) in row.iter_mut().enumerate().take(table.columns.len()) {
            let target = table.columns[index].ty;
            *cell = convert_cell(cell, target);
        }
    }
}

fn infer_type(cell: &CellValue) -> ColumnType {
    match cell {
        CellValue::Text(text) if text.is_empty() => ColumnType::Null,
        CellValue::Text(text) => {
            if matches!(text.as_str(), "true" | "false") {
                ColumnType::Bool
            } else if parse_safe_int(text).is_some() {
                ColumnType::Int
            } else if parse_safe_float(text).is_some() {
                ColumnType::Float
            } else if parse_date(text).is_some() {
                ColumnType::Date
            } else if parse_datetime(text).is_some() {
                ColumnType::DateTime
            } else if parse_time(text).is_some() {
                ColumnType::Time
            } else {
                ColumnType::Text
            }
        }
        other => other.type_of(),
    }
}

fn convert_cell(cell: &CellValue, target: ColumnType) -> CellValue {
    let CellValue::Text(text) = cell else {
        return cell.clone();
    };
    if text.is_empty() {
        return CellValue::Null;
    }

    match target {
        ColumnType::Null => CellValue::Null,
        ColumnType::Bool => match text.as_str() {
            "true" => CellValue::Bool(true),
            "false" => CellValue::Bool(false),
            _ => cell.clone(),
        },
        ColumnType::Int => parse_safe_int(text)
            .map(CellValue::Int)
            .unwrap_or_else(|| cell.clone()),
        ColumnType::Float => parse_safe_float(text)
            .map(CellValue::Float)
            .or_else(|| parse_safe_int(text).map(|value| CellValue::Float(value as f64)))
            .unwrap_or_else(|| cell.clone()),
        ColumnType::Date => parse_date(text)
            .map(CellValue::Date)
            .unwrap_or_else(|| cell.clone()),
        ColumnType::DateTime => parse_datetime(text)
            .map(CellValue::DateTime)
            .unwrap_or_else(|| cell.clone()),
        ColumnType::Time => parse_time(text)
            .map(CellValue::Time)
            .unwrap_or_else(|| cell.clone()),
        ColumnType::Decimal | ColumnType::Text | ColumnType::Error => cell.clone(),
    }
}

/// Parse an integer only when doing so cannot silently change a leading zero.
fn parse_safe_int(text: &str) -> Option<i64> {
    if text.is_empty() {
        return None;
    }

    let digits = text
        .strip_prefix('-')
        .or_else(|| text.strip_prefix('+'))
        .unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if digits.len() > 1 && digits.starts_with('0') {
        return None;
    }
    text.parse::<i64>().ok()
}

/// Parse a float only when the text clearly carries a fractional/exponent form.
fn parse_safe_float(text: &str) -> Option<f64> {
    if !text.contains('.') && !text.contains('e') && !text.contains('E') {
        return None;
    }

    let value = text.parse::<f64>().ok()?;
    if !value.is_finite() {
        return None;
    }

    let body = text
        .strip_prefix('-')
        .or_else(|| text.strip_prefix('+'))
        .unwrap_or(text);
    let integer_part = body.split(['.', 'e', 'E']).next().unwrap_or(body);
    if integer_part.len() > 1 && integer_part.starts_with('0') {
        return None;
    }
    Some(value)
}

fn parse_date(text: &str) -> Option<NaiveDate> {
    if text.len() != 10 {
        return None;
    }
    NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
}

fn parse_datetime(text: &str) -> Option<NaiveDateTime> {
    if let Ok(value) = chrono::DateTime::parse_from_rfc3339(text) {
        return Some(value.naive_utc());
    }
    for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"] {
        if let Ok(value) = NaiveDateTime::parse_from_str(text, format) {
            return Some(value);
        }
    }
    None
}

fn parse_time(text: &str) -> Option<NaiveTime> {
    for format in ["%H:%M:%S%.f", "%H:%M"] {
        if let Ok(value) = NaiveTime::parse_from_str(text, format) {
            return Some(value);
        }
    }
    None
}

/// Sanitise a worksheet name for Excel's rules. ODS uses the same rules so a
/// workbook can move between the two formats without a second round of loss.
pub fn sanitize_sheet_name(name: &str) -> String {
    let mut sanitized = String::with_capacity(name.len());
    for character in name.chars() {
        if character.is_control()
            || !is_xml_character(character)
            || matches!(character, '[' | ']' | ':' | '*' | '?' | '/' | '\\')
        {
            sanitized.push('_');
        } else {
            sanitized.push(character);
        }
    }

    let trimmed = sanitized.trim().trim_matches('\'').trim();
    if trimmed.is_empty() {
        "Sheet".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Truncate a name so a stable suffix still fits inside the 31-character limit.
fn fit_sheet_name(name: &str, suffix_len: usize) -> String {
    let max = MAX_SHEET_NAME.saturating_sub(suffix_len);
    let truncated = name.chars().take(max).collect::<String>();
    let trimmed = truncated.trim().trim_matches('\'').trim();
    if trimmed.is_empty() {
        "Sheet".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Add a stable `-2`, `-3`, ... suffix when a name collides case-insensitively.
pub fn unique_sheet_name(candidate: &str, used: &mut HashSet<String>) -> String {
    let mut suffix = 1usize;
    loop {
        let suffix_text = if suffix == 1 {
            String::new()
        } else {
            format!("-{suffix}")
        };
        let mut name = fit_sheet_name(candidate, suffix_text.chars().count());
        if suffix > 1 {
            name.push_str(&suffix_text);
        }
        if used.insert(name.to_lowercase()) {
            return name;
        }
        suffix += 1;
    }
}

/// Normalise every table name and make the set unique.
///
/// The CLI does this first so it can attach loss events, but the writers run the
/// same normalisation as a safety net for direct library callers.
pub fn normalize_sheet_names(tables: &[Table]) -> Vec<String> {
    let mut used = HashSet::new();
    tables
        .iter()
        .map(|table| unique_sheet_name(&sanitize_sheet_name(&table.name), &mut used))
        .collect()
}

/// Maximum rendered width per column, clamped to a readable range.
pub fn column_widths(table: &Table, has_header: bool) -> Vec<usize> {
    let mut widths = vec![1usize; table.width()];

    if has_header {
        for (index, column) in table.columns.iter().enumerate() {
            widths[index] = widths[index].max(column.name.chars().count());
        }
    }

    for row in &table.rows {
        for (index, cell) in row.iter().enumerate().take(widths.len()) {
            let text = cell.to_text("");
            for line in text.split('\n') {
                widths[index] = widths[index].max(line.chars().count());
            }
        }
    }

    widths.into_iter().map(|width| width.clamp(8, 60)).collect()
}

/// Escape text content for XML and reject characters XML 1.0 cannot represent.
pub fn escape_xml_text(value: &str, source: &SourceRef) -> Result<String> {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            '\t' | '\n' | '\r' => escaped.push(character),
            other if is_xml_character(other) => escaped.push(other),
            other => {
                return Err(Error::data(format!(
                    "{}: text contains control character U+{:04X}, which XML cannot represent",
                    source.context(),
                    other as u32
                )));
            }
        }
    }
    Ok(escaped)
}

/// Escape text for an XML attribute.
pub fn escape_xml_attr(value: &str, source: &SourceRef) -> Result<String> {
    escape_xml_text(value, source)
}

/// Render ODF text content as one `<text:p>` element per source line.
///
/// Multiple paragraphs are the portable way to keep embedded newlines: readers
/// join paragraphs with a newline, whereas some ODF readers ignore
/// `<text:line-break/>`. Tabs stay literal because XML preserves them.
pub fn ods_text_element(value: &str, source: &SourceRef) -> Result<String> {
    let normalized = value.replace("\r\n", "\n").replace('\r', "\n");
    let mut rendered = String::with_capacity(normalized.len() + 16);

    for line in normalized.split('\n') {
        rendered.push_str("<text:p>");
        for character in line.chars() {
            match character {
                '&' => rendered.push_str("&amp;"),
                '<' => rendered.push_str("&lt;"),
                '>' => rendered.push_str("&gt;"),
                '\t' => rendered.push('\t'),
                other if is_xml_character(other) => rendered.push(other),
                other => {
                    return Err(Error::data(format!(
                        "{}: text contains control character U+{:04X}, which XML cannot represent",
                        source.context(),
                        other as u32
                    )));
                }
            }
        }
        rendered.push_str("</text:p>");
    }

    Ok(rendered)
}

pub fn is_xml_character(character: char) -> bool {
    matches!(character, '\t' | '\n' | '\r')
        || ('\u{20}'..='\u{D7FF}').contains(&character)
        || ('\u{E000}'..='\u{FFFD}').contains(&character)
        || character >= '\u{10000}'
}

/// Days since 1899-12-30 for an Excel serial date.
pub fn excel_date_serial(date: NaiveDate) -> i64 {
    let epoch = NaiveDate::from_ymd_opt(1899, 12, 30).expect("the Excel epoch is a valid date");
    i64::from(date.num_days_from_ce()) - i64::from(epoch.num_days_from_ce())
}

/// Excel serial date-time, including the fractional day.
pub fn excel_datetime_serial(value: NaiveDateTime) -> f64 {
    excel_date_serial(value.date()) as f64 + excel_time_fraction(value.time())
}

/// Fraction of a day for an Excel time value.
pub fn excel_time_fraction(value: NaiveTime) -> f64 {
    let seconds =
        value.num_seconds_from_midnight() as f64 + value.nanosecond() as f64 / 1_000_000_000.0;
    seconds / 86_400.0
}

/// Excel cannot display dates before 1900-01-01 reliably.
pub fn is_excel_date_supported(date: NaiveDate) -> bool {
    date >= NaiveDate::from_ymd_opt(1900, 1, 1).unwrap()
}

/// A1-style column label (0 -> A).
pub fn column_label(index: usize) -> String {
    let mut number = index as u64 + 1;
    let mut letters = String::new();
    while number > 0 {
        let remainder = ((number - 1) % 26) as u8;
        letters.insert(0, (b'A' + remainder) as char);
        number = (number - 1) / 26;
    }
    letters
}

/// A1-style cell reference.
pub fn cell_reference(row: usize, column: usize) -> String {
    format!("{}{}", column_label(column), row + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Column;

    fn table(columns: &[(&str, ColumnType)], rows: Vec<Vec<CellValue>>) -> Table {
        let source = SourceRef::new("data.csv", FormatId::Csv);
        Table::with_columns(
            "data",
            source,
            columns
                .iter()
                .map(|(name, ty)| Column::new(*name, *ty))
                .collect(),
            rows,
        )
    }

    #[test]
    fn inference_preserves_leading_zeroes_and_plain_text() {
        let mut input = table(
            &[("code", ColumnType::Text), ("n", ColumnType::Text)],
            vec![vec![
                CellValue::Text("007".into()),
                CellValue::Text("12".into()),
            ]],
        );
        infer_text_types(&mut input);
        assert_eq!(input.columns[0].ty, ColumnType::Text);
        assert_eq!(input.rows[0][0], CellValue::Text("007".into()));
        assert_eq!(input.columns[1].ty, ColumnType::Int);
        assert_eq!(input.rows[0][1], CellValue::Int(12));
    }

    #[test]
    fn inference_handles_bool_float_date_datetime_and_time() {
        let mut input = table(
            &[
                ("flag", ColumnType::Text),
                ("ratio", ColumnType::Text),
                ("day", ColumnType::Text),
                ("stamp", ColumnType::Text),
                ("at", ColumnType::Text),
            ],
            vec![vec![
                CellValue::Text("true".into()),
                CellValue::Text("1.5".into()),
                CellValue::Text("2026-01-15".into()),
                CellValue::Text("2026-01-15T10:30:00".into()),
                CellValue::Text("10:30:00".into()),
            ]],
        );
        infer_text_types(&mut input);
        assert_eq!(input.rows[0][0], CellValue::Bool(true));
        assert_eq!(input.rows[0][1], CellValue::Float(1.5));
        assert!(matches!(input.rows[0][2], CellValue::Date(_)));
        assert!(matches!(input.rows[0][3], CellValue::DateTime(_)));
        assert!(matches!(input.rows[0][4], CellValue::Time(_)));
    }

    #[test]
    fn sheet_names_are_sanitised_truncated_and_unique() {
        assert_eq!(sanitize_sheet_name("Sales/Q1"), "Sales_Q1");
        assert_eq!(sanitize_sheet_name("   "), "Sheet");

        let mut used = HashSet::new();
        assert_eq!(unique_sheet_name("data", &mut used), "data");
        assert_eq!(unique_sheet_name("data", &mut used), "data-2");
        assert_eq!(unique_sheet_name("DATA", &mut used), "DATA-3");

        let long = "x".repeat(40);
        let mut used = HashSet::new();
        let first = unique_sheet_name(&long, &mut used);
        let second = unique_sheet_name(&long, &mut used);
        assert_eq!(first.chars().count(), MAX_SHEET_NAME);
        assert!(second.chars().count() <= MAX_SHEET_NAME);
        assert!(second.ends_with("-2"));
    }

    #[test]
    fn excel_serials_use_the_1900_date_system() {
        assert_eq!(
            excel_date_serial(NaiveDate::from_ymd_opt(1900, 1, 1).unwrap()),
            2
        );
        assert_eq!(
            excel_date_serial(NaiveDate::from_ymd_opt(2026, 1, 15).unwrap()),
            46_037
        );
        assert!(
            (excel_time_fraction(NaiveTime::from_hms_opt(12, 0, 0).unwrap()) - 0.5).abs() < 1e-12
        );
    }

    #[test]
    fn column_labels_share_a1_notation() {
        assert_eq!(column_label(0), "A");
        assert_eq!(column_label(25), "Z");
        assert_eq!(column_label(26), "AA");
        assert_eq!(cell_reference(0, 0), "A1");
        assert_eq!(cell_reference(4, 1), "B5");
    }

    #[test]
    fn invalid_xml_control_characters_are_rejected() {
        let source = SourceRef::new("data.csv", FormatId::Csv);
        assert!(escape_xml_text("ok\n", &source).is_ok());
        let error = escape_xml_text("bad\u{0001}", &source).unwrap_err();
        assert!(error.to_string().contains("control character"));
    }
}
