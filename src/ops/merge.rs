//! Merge/concatenate tables by rows.

use std::collections::{HashMap, HashSet};

use crate::error::{Error, Result};
use crate::model::{CellValue, Column, ColumnType, LossEvent, Table};
use crate::ops::{dedupe, schema};

/// Schema policy for a merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeMode {
    /// Column names, order, and types must match.
    Strict,
    /// Union columns by name and fill missing cells with null.
    Loose,
}

/// Options that control how tables are concatenated.
#[derive(Debug, Clone, Default)]
pub struct MergeOptions {
    /// Name of the provenance column appended after the merged data.
    pub source_column: Option<String>,
    /// Drop exact duplicate data rows, keeping the first occurrence.
    pub dedup: bool,
    /// Match column names without regard to case.
    pub case_insensitive_headers: bool,
}

/// Compare every input against the first and return non-empty schema diffs.
pub fn strict_diffs(tables: &[Table], case_insensitive: bool) -> Vec<schema::SchemaDiff> {
    let Some(reference) = tables.first() else {
        return Vec::new();
    };

    tables
        .iter()
        .skip(1)
        .map(|table| schema::diff(reference, table, case_insensitive))
        .filter(|diff| !diff.is_empty())
        .collect()
}

/// Merge tables into one table.
///
/// Loose merging is used for the actual assembly. Strict-mode validation is a
/// separate step ([`strict_diffs`]) so a mismatch can be reported before any
/// output writer is opened.
pub fn merge(tables: Vec<Table>, options: &MergeOptions) -> Result<Table> {
    let Some(first) = tables.first() else {
        return Err(Error::data("no tables were selected to merge"));
    };

    let mut union = Vec::<UnionColumn>::new();
    let mut union_index = HashMap::<String, usize>::new();
    let mut mappings = Vec::<Vec<usize>>::with_capacity(tables.len());

    for table in &tables {
        let columns = schema::effective_schema(table);
        let mut local_keys = HashSet::new();
        let mut mapping = Vec::with_capacity(columns.len());

        for column in &columns {
            let key = schema::normalize_header(&column.name, options.case_insensitive_headers);
            if !local_keys.insert(key.clone()) {
                return Err(Error::data(format!(
                    "{} has more than one column named '{}'; merge needs unique headers",
                    table.source.context(),
                    column.name
                )));
            }

            let target = if let Some(&index) = union_index.get(&key) {
                union[index].ty = union[index].ty.merge(column.ty);
                index
            } else {
                let index = union.len();
                union_index.insert(key.clone(), index);
                union.push(UnionColumn {
                    name: column.name.clone(),
                    ty: column.ty,
                });
                index
            };
            mapping.push(target);
        }

        mappings.push(mapping);
    }

    if let Some(source_column) = &options.source_column {
        let key = schema::normalize_header(source_column, options.case_insensitive_headers);
        if union_index.contains_key(&key) {
            return Err(Error::usage(format!(
                "--add-source-column '{source_column}' conflicts with an existing column"
            )));
        }
    }

    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    for (table, mapping) in tables.iter().zip(mappings) {
        let label = source_label(table);
        for row in &table.rows {
            let mut merged = vec![CellValue::Null; union.len()];
            for (source_index, &target_index) in mapping.iter().enumerate() {
                if let Some(cell) = row.get(source_index) {
                    merged[target_index] = cell.clone();
                }
            }

            if options.dedup && !seen.insert(dedupe::row_key(&merged)) {
                continue;
            }

            if options.source_column.is_some() {
                merged.push(CellValue::Text(label.clone()));
            }
            rows.push(merged);
        }
    }

    let mut columns = union
        .into_iter()
        .map(|column| Column::new(column.name, column.ty))
        .collect::<Vec<_>>();
    if let Some(source_column) = &options.source_column {
        columns.push(Column::new(source_column.clone(), ColumnType::Text));
    }

    let warnings = tables
        .iter()
        .flat_map(|table| table.warnings.iter().cloned())
        .collect::<Vec<LossEvent>>();

    Ok(Table {
        name: first.name.clone(),
        source: first.source.clone(),
        columns,
        rows,
        warnings,
        first_data_row: 1,
    })
}

/// Provenance label for the `--add-source-column` feature.
pub fn source_label(table: &Table) -> String {
    let path = &table.source.path;
    let base = if path.as_os_str() == "-" {
        "stdin".to_owned()
    } else {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .unwrap_or_else(|| path.display().to_string())
    };

    match &table.source.sheet {
        Some(sheet) => format!("{base}#{sheet}"),
        None => base,
    }
}

struct UnionColumn {
    name: String,
    ty: ColumnType,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::FormatId;
    use crate::model::SourceRef;

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

    fn compatible() -> (Table, Table) {
        let a = table(
            "a",
            &[("id", ColumnType::Text), ("name", ColumnType::Text)],
            vec![
                vec![CellValue::Text("1".into()), CellValue::Text("Alice".into())],
                vec![CellValue::Text("2".into()), CellValue::Text("Bob".into())],
            ],
        );
        let b = table(
            "b",
            &[("id", ColumnType::Text), ("name", ColumnType::Text)],
            vec![vec![
                CellValue::Text("3".into()),
                CellValue::Text("Carol".into()),
            ]],
        );
        (a, b)
    }

    #[test]
    fn strict_compatible_tables_have_no_diff() {
        let (a, b) = compatible();
        assert!(strict_diffs(&[a, b], false).is_empty());
    }

    #[test]
    fn strict_reports_missing_and_reordered_columns() {
        let (a, _) = compatible();
        let c = table(
            "c",
            &[("name", ColumnType::Text), ("id", ColumnType::Text)],
            vec![vec![
                CellValue::Text("Zoe".into()),
                CellValue::Text("9".into()),
            ]],
        );
        let diffs = strict_diffs(&[a, c], false);
        assert_eq!(diffs.len(), 1);
        assert!(diffs[0].order.is_some());
    }

    #[test]
    fn loose_merge_unions_columns_and_fills_nulls() {
        let (a, b) = compatible();
        let c = table(
            "c",
            &[("id", ColumnType::Text), ("note", ColumnType::Text)],
            vec![vec![
                CellValue::Text("4".into()),
                CellValue::Text("hello".into()),
            ]],
        );
        let merged = merge(
            vec![a, b, c],
            &MergeOptions {
                case_insensitive_headers: false,
                ..MergeOptions::default()
            },
        )
        .unwrap();

        assert_eq!(merged.column_names(), ["id", "name", "note"]);
        assert_eq!(merged.row_count(), 4);
        assert_eq!(merged.rows[3][1], CellValue::Null);
        assert_eq!(merged.rows[3][2], CellValue::Text("hello".into()));
    }

    #[test]
    fn source_column_and_dedup_are_deterministic() {
        let (a, b) = compatible();
        let merged = merge(
            vec![a.clone(), a, b],
            &MergeOptions {
                source_column: Some("source".into()),
                dedup: true,
                case_insensitive_headers: false,
            },
        )
        .unwrap();

        assert_eq!(merged.column_names(), ["id", "name", "source"]);
        assert_eq!(merged.row_count(), 3);
        assert_eq!(merged.rows[0][2], CellValue::Text("a.csv".into()));
        assert_eq!(merged.rows[2][2], CellValue::Text("b.csv".into()));
    }

    #[test]
    fn case_insensitive_headers_match_by_lowercase_name() {
        let a = table(
            "a",
            &[("ID", ColumnType::Text), ("Name", ColumnType::Text)],
            vec![vec![
                CellValue::Text("1".into()),
                CellValue::Text("Alice".into()),
            ]],
        );
        let b = table(
            "b",
            &[("id", ColumnType::Text), ("name", ColumnType::Text)],
            vec![vec![
                CellValue::Text("2".into()),
                CellValue::Text("Bob".into()),
            ]],
        );

        assert_eq!(strict_diffs(&[a.clone(), b.clone()], false).len(), 1);
        assert!(strict_diffs(&[a.clone(), b.clone()], true).is_empty());

        let merged = merge(
            vec![a, b],
            &MergeOptions {
                case_insensitive_headers: true,
                ..MergeOptions::default()
            },
        )
        .unwrap();
        assert_eq!(merged.column_names(), ["ID", "Name"]);
        assert_eq!(merged.row_count(), 2);
    }
}
