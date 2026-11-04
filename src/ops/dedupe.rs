//! Exact-row deduplication.

use std::collections::HashSet;

use crate::model::{Row, Table};

/// Build a collision-safe key for an exact row.
///
/// The cell type and value length are part of the key so two different values
/// cannot collide even when they contain the separator characters.
pub fn row_key(row: &Row) -> String {
    let mut key = String::new();
    for cell in row {
        let rendered = cell.to_text("\u{0}");
        key.push_str(cell.type_of().as_str());
        key.push(':');
        key.push_str(&rendered.len().to_string());
        key.push(':');
        key.push_str(&rendered);
        key.push('\u{1e}');
    }
    key
}

/// Keep the first occurrence of each exact row.
pub fn dedupe_rows(rows: &[Row]) -> (Vec<Row>, Vec<usize>) {
    let mut seen = HashSet::new();
    let mut kept = Vec::new();
    let mut indices = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        if seen.insert(row_key(row)) {
            kept.push(row.clone());
            indices.push(index);
        }
    }
    (kept, indices)
}

/// Deduplicate a table while preserving the first occurrence order.
pub fn dedupe(table: &Table) -> Table {
    let (rows, kept_indices) = dedupe_rows(&table.rows);
    let kept_source_rows = kept_indices
        .into_iter()
        .map(|index| table.source_row(index))
        .collect::<HashSet<_>>();

    let warnings = table
        .warnings
        .iter()
        .filter(|warning| {
            warning
                .source
                .as_ref()
                .and_then(|source| source.row)
                .is_none_or(|row| kept_source_rows.contains(&row))
        })
        .cloned()
        .collect();

    Table {
        name: table.name.clone(),
        source: table.source.clone(),
        columns: table.columns.clone(),
        rows,
        warnings,
        first_data_row: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::FormatId;
    use crate::model::{CellValue, Column, ColumnType, SourceRef};

    fn table() -> Table {
        Table::with_columns(
            "data",
            SourceRef::new("data.csv", FormatId::Csv),
            vec![Column::new("id", ColumnType::Text)],
            vec![
                vec![CellValue::Text("2".into())],
                vec![CellValue::Text("1".into())],
                vec![CellValue::Text("2".into())],
                vec![CellValue::Text("1".into())],
            ],
        )
    }

    #[test]
    fn dedupe_is_stable_and_keeps_first_occurrence() {
        let deduped = dedupe(&table());
        assert_eq!(
            deduped.rows,
            vec![
                vec![CellValue::Text("2".into())],
                vec![CellValue::Text("1".into())]
            ]
        );
    }

    #[test]
    fn row_keys_distinguish_types_and_separator_content() {
        assert_ne!(
            row_key(&vec![CellValue::Text("1".into())]),
            row_key(&vec![CellValue::Int(1)])
        );
        assert_ne!(
            row_key(&vec![
                CellValue::Text("a".into()),
                CellValue::Text("b".into())
            ]),
            row_key(&vec![CellValue::Text("a\u{1e}b".into())])
        );
    }
}
