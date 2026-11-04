//! Schema inference and schema-diff reporting.
//!
//! Delimited sources keep their cells as text so CSV/TSV conversion stays
//! byte-faithful. Merge diagnostics still need a useful notion of column type,
//! so the schema layer infers a conservative type from the values without
//! rewriting the table.

use std::collections::{HashMap, HashSet};

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use serde_json::{Value, json};

use crate::formats::FormatId;
use crate::model::{CellValue, ColumnType, Table};

/// A column as it appears in a schema diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaColumn {
    pub name: String,
    pub ty: ColumnType,
}

/// A column that exists in one schema but not the other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDifference {
    pub name: String,
    pub ty: ColumnType,
}

/// Columns that exist in both schemas but in a different order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderDifference {
    pub expected: Vec<String>,
    pub actual: Vec<String>,
}

/// Two columns with the same name but incompatible types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeConflict {
    pub column: String,
    pub reference: ColumnType,
    pub candidate: ColumnType,
}

/// A complete, actionable comparison of two table schemas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaDiff {
    pub reference: String,
    pub candidate: String,
    pub missing: Vec<ColumnDifference>,
    pub extra: Vec<ColumnDifference>,
    pub order: Option<OrderDifference>,
    pub type_conflicts: Vec<TypeConflict>,
}

impl SchemaDiff {
    /// Whether the schemas are structurally compatible.
    pub fn is_empty(&self) -> bool {
        self.missing.is_empty()
            && self.extra.is_empty()
            && self.order.is_none()
            && self.type_conflicts.is_empty()
    }

    /// Render a human-readable diff for stderr.
    pub fn render_text(&self) -> String {
        let mut out = format!("schema diff: {} -> {}\n", self.reference, self.candidate);

        for column in &self.missing {
            out.push_str(&format!(
                "  missing column '{}' ({})\n",
                column.name,
                column.ty.as_str()
            ));
        }
        for column in &self.extra {
            out.push_str(&format!(
                "  extra column '{}' ({})\n",
                column.name,
                column.ty.as_str()
            ));
        }
        if let Some(order) = &self.order {
            out.push_str(&format!(
                "  column order differs: expected [{}], found [{}]\n",
                order.expected.join(", "),
                order.actual.join(", ")
            ));
        }
        for conflict in &self.type_conflicts {
            out.push_str(&format!(
                "  type conflict on '{}': {} vs {}\n",
                conflict.column,
                conflict.reference.as_str(),
                conflict.candidate.as_str()
            ));
        }

        if self.is_empty() {
            out.push_str("  schemas match\n");
        }
        out
    }

    /// Stable machine-readable representation.
    pub fn to_json(&self) -> Value {
        json!({
            "version": 1,
            "reference": self.reference,
            "candidate": self.candidate,
            "missing": self.missing.iter().map(column_json).collect::<Vec<_>>(),
            "extra": self.extra.iter().map(column_json).collect::<Vec<_>>(),
            "order": self.order.as_ref().map(|order| json!({
                "expected": order.expected,
                "actual": order.actual,
            })),
            "type_conflicts": self.type_conflicts.iter().map(|conflict| json!({
                "column": conflict.column,
                "reference": conflict.reference.as_str(),
                "candidate": conflict.candidate.as_str(),
            })).collect::<Vec<_>>(),
        })
    }

    /// Render a stable machine-readable diff.
    pub fn render_json(&self) -> String {
        serde_json::to_string_pretty(&self.to_json()).unwrap_or_else(|_| "{}".to_owned())
    }
}

fn column_json(column: &ColumnDifference) -> Value {
    json!({ "name": column.name, "type": column.ty.as_str() })
}

/// Normalise a header for case-insensitive matching.
pub fn normalize_header(name: &str, case_insensitive: bool) -> String {
    if case_insensitive {
        name.to_lowercase()
    } else {
        name.to_owned()
    }
}

/// Return the effective schema of a table.
///
/// Workbooks and JSON already carry types. Delimited sources are all text, so
/// their types are inferred from the values for comparison and reporting.
pub fn effective_schema(table: &Table) -> Vec<SchemaColumn> {
    table
        .columns
        .iter()
        .enumerate()
        .map(|(index, column)| SchemaColumn {
            name: column.name.clone(),
            ty: effective_column_type(table, index),
        })
        .collect()
}

/// Return the effective type of one column.
pub fn effective_column_type(table: &Table, index: usize) -> ColumnType {
    let declared = table
        .columns
        .get(index)
        .map(|column| column.ty)
        .unwrap_or(ColumnType::Null);

    if !matches!(table.source.format, FormatId::Csv | FormatId::Tsv) {
        return declared;
    }

    let mut inferred = ColumnType::Null;
    for row in &table.rows {
        if let Some(cell) = row.get(index) {
            inferred = inferred.merge(infer_cell_type(cell));
        }
    }

    if matches!(declared, ColumnType::Null | ColumnType::Text) {
        inferred
    } else {
        declared.merge(inferred)
    }
}

/// Infer a conservative type from one cell.
pub fn infer_cell_type(cell: &CellValue) -> ColumnType {
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

/// Whether two non-null types cannot be reconciled without falling back to text.
pub fn types_conflict(reference: ColumnType, candidate: ColumnType) -> bool {
    if matches!(reference, ColumnType::Null) || matches!(candidate, ColumnType::Null) {
        return false;
    }
    if reference == candidate {
        return false;
    }
    reference.merge(candidate) == ColumnType::Text
}

/// Compare two tables column by column.
pub fn diff(reference: &Table, candidate: &Table, case_insensitive: bool) -> SchemaDiff {
    let reference_schema = effective_schema(reference);
    let candidate_schema = effective_schema(candidate);

    let mut candidate_by_key = HashMap::new();
    for column in &candidate_schema {
        candidate_by_key
            .entry(normalize_header(&column.name, case_insensitive))
            .or_insert(column);
    }
    let mut reference_keys = HashSet::new();
    let mut reference_names = HashMap::new();
    for column in &reference_schema {
        let key = normalize_header(&column.name, case_insensitive);
        reference_keys.insert(key.clone());
        reference_names
            .entry(key)
            .or_insert_with(|| column.name.clone());
    }

    let missing = reference_schema
        .iter()
        .filter(|column| {
            !candidate_by_key.contains_key(&normalize_header(&column.name, case_insensitive))
        })
        .map(|column| ColumnDifference {
            name: column.name.clone(),
            ty: column.ty,
        })
        .collect::<Vec<_>>();

    let extra = candidate_schema
        .iter()
        .filter(|column| {
            !reference_keys.contains(&normalize_header(&column.name, case_insensitive))
        })
        .map(|column| ColumnDifference {
            name: column.name.clone(),
            ty: column.ty,
        })
        .collect::<Vec<_>>();

    let expected = reference_schema
        .iter()
        .filter(|column| {
            candidate_by_key.contains_key(&normalize_header(&column.name, case_insensitive))
        })
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let actual = candidate_schema
        .iter()
        .filter(|column| reference_keys.contains(&normalize_header(&column.name, case_insensitive)))
        .map(|column| {
            let key = normalize_header(&column.name, case_insensitive);
            reference_names
                .get(&key)
                .cloned()
                .unwrap_or_else(|| column.name.clone())
        })
        .collect::<Vec<_>>();
    let order = if expected != actual {
        Some(OrderDifference { expected, actual })
    } else {
        None
    };

    let mut type_conflicts = Vec::new();
    for column in &reference_schema {
        let key = normalize_header(&column.name, case_insensitive);
        if let Some(candidate_column) = candidate_by_key.get(&key)
            && types_conflict(column.ty, candidate_column.ty)
        {
            type_conflicts.push(TypeConflict {
                column: column.name.clone(),
                reference: column.ty,
                candidate: candidate_column.ty,
            });
        }
    }

    SchemaDiff {
        reference: reference.source.context(),
        candidate: candidate.source.context(),
        missing,
        extra,
        order,
        type_conflicts,
    }
}

/// Parse an integer only when doing so cannot silently change a leading zero.
pub(crate) fn parse_safe_int(text: &str) -> Option<i64> {
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
pub(crate) fn parse_safe_float(text: &str) -> Option<f64> {
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

pub(crate) fn parse_date(text: &str) -> Option<NaiveDate> {
    if text.len() != 10 {
        return None;
    }
    NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
}

pub(crate) fn parse_datetime(text: &str) -> Option<NaiveDateTime> {
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

pub(crate) fn parse_time(text: &str) -> Option<NaiveTime> {
    for format in ["%H:%M:%S%.f", "%H:%M"] {
        if let Ok(value) = NaiveTime::parse_from_str(text, format) {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Column, SourceRef};

    fn table(name: &str, columns: &[(&str, ColumnType)], rows: Vec<Vec<CellValue>>) -> Table {
        Table::with_columns(
            name,
            SourceRef::new(format!("{name}.csv"), FormatId::Csv),
            columns
                .iter()
                .map(|(name, ty)| Column::new(*name, *ty))
                .collect(),
            rows,
        )
    }

    #[test]
    fn inference_recognises_scalar_types_but_keeps_leading_zeroes_text() {
        assert_eq!(
            infer_cell_type(&CellValue::Text("12".into())),
            ColumnType::Int
        );
        assert_eq!(
            infer_cell_type(&CellValue::Text("1.5".into())),
            ColumnType::Float
        );
        assert_eq!(
            infer_cell_type(&CellValue::Text("true".into())),
            ColumnType::Bool
        );
        assert_eq!(
            infer_cell_type(&CellValue::Text("2026-01-15".into())),
            ColumnType::Date
        );
        assert_eq!(
            infer_cell_type(&CellValue::Text("2026-01-15T10:30:00".into())),
            ColumnType::DateTime
        );
        assert_eq!(
            infer_cell_type(&CellValue::Text("10:30:00".into())),
            ColumnType::Time
        );
        assert_eq!(
            infer_cell_type(&CellValue::Text("007".into())),
            ColumnType::Text
        );
        assert_eq!(
            infer_cell_type(&CellValue::Text("hello".into())),
            ColumnType::Text
        );
        assert_eq!(infer_cell_type(&CellValue::Null), ColumnType::Null);
    }

    #[test]
    fn diff_reports_missing_extra_order_and_type_conflicts() {
        let reference = table(
            "a",
            &[("id", ColumnType::Text), ("amount", ColumnType::Text)],
            vec![
                vec![CellValue::Text("1".into()), CellValue::Text("10.5".into())],
                vec![CellValue::Text("2".into()), CellValue::Text("20".into())],
            ],
        );
        let candidate = table(
            "d",
            &[("amount", ColumnType::Text), ("id", ColumnType::Text)],
            vec![vec![
                CellValue::Text("unknown".into()),
                CellValue::Text("3".into()),
            ]],
        );

        let diff = diff(&reference, &candidate, false);
        assert_eq!(
            diff.order.as_ref().unwrap().expected,
            ["id", "amount"],
            "expected order"
        );
        assert!(diff.missing.is_empty());
        assert!(diff.extra.is_empty());
        assert_eq!(diff.type_conflicts.len(), 1);
        assert_eq!(diff.type_conflicts[0].column, "amount");
        assert_eq!(diff.type_conflicts[0].reference, ColumnType::Float);
        assert_eq!(diff.type_conflicts[0].candidate, ColumnType::Text);

        let text = diff.render_text();
        assert!(text.contains("column order differs"));
        assert!(text.contains("type conflict on 'amount'"));
        let json = diff.render_json();
        assert!(json.contains("\"type_conflicts\""));
        assert!(json.contains("\"candidate\": \"text\""));
    }

    #[test]
    fn case_insensitive_matching_ignores_case_only() {
        let reference = table(
            "a",
            &[("ID", ColumnType::Text)],
            vec![vec![CellValue::Text("1".into())]],
        );
        let candidate = table(
            "b",
            &[("id", ColumnType::Text)],
            vec![vec![CellValue::Text("2".into())]],
        );

        assert!(diff(&reference, &candidate, true).is_empty());
        let case_sensitive = diff(&reference, &candidate, false);
        assert_eq!(case_sensitive.missing.len(), 1);
        assert_eq!(case_sensitive.extra.len(), 1);

        let case_insensitive = diff(&reference, &candidate, true);
        assert!(case_insensitive.is_empty());
    }
}
