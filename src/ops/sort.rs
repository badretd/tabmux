//! Stable multi-column sorting.

use std::cmp::Ordering;

use crate::error::{Error, Result};
use crate::model::{CellValue, Table};

/// One sort key, parsed from `column` or `column:desc`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortKey {
    pub column: String,
    pub descending: bool,
}

/// Parse a comma-separated sort specification such as `name,amount:desc`.
pub fn parse_keys(specification: &str) -> Result<Vec<SortKey>> {
    let mut keys = Vec::new();
    for raw in specification.split(',') {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let (column, descending) = match raw.split_once(':') {
            Some((column, direction)) => {
                let descending = match direction.trim().to_ascii_lowercase().as_str() {
                    "asc" | "ascending" => false,
                    "desc" | "descending" => true,
                    other => {
                        return Err(Error::usage(format!(
                            "invalid sort direction '{other}' in '{raw}' (use asc or desc)"
                        )));
                    }
                };
                (column.trim(), descending)
            }
            None => (raw, false),
        };
        if column.is_empty() {
            return Err(Error::usage(format!(
                "invalid sort key '{raw}'; the column name is empty"
            )));
        }
        keys.push(SortKey {
            column: column.to_owned(),
            descending,
        });
    }

    if keys.is_empty() {
        return Err(Error::usage(
            "sort --by must name at least one column (for example amount:desc)",
        ));
    }
    Ok(keys)
}

/// Sort a table by one or more keys, preserving the input order for ties.
pub fn sort(table: &Table, keys: &[SortKey]) -> Result<Table> {
    let indices = keys
        .iter()
        .map(|key| {
            table.column_index(&key.column).ok_or_else(|| {
                Error::data(format!(
                    "unknown column '{}' in --by; available columns: {}",
                    key.column,
                    table.column_names().join(", ")
                ))
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let mut order = (0..table.rows.len()).collect::<Vec<_>>();
    order.sort_by(|&left, &right| compare_rows(table, &indices, keys, left, right));

    let rows = order
        .into_iter()
        .map(|index| table.rows[index].clone())
        .collect();

    Ok(Table {
        name: table.name.clone(),
        source: table.source.clone(),
        columns: table.columns.clone(),
        rows,
        warnings: table.warnings.clone(),
        first_data_row: 1,
    })
}

fn compare_rows(
    table: &Table,
    indices: &[usize],
    keys: &[SortKey],
    left: usize,
    right: usize,
) -> Ordering {
    let left_row = &table.rows[left];
    let right_row = &table.rows[right];

    for (&index, key) in indices.iter().zip(keys) {
        let left_cell = left_row.get(index).unwrap_or(&CellValue::Null);
        let right_cell = right_row.get(index).unwrap_or(&CellValue::Null);
        let left_null = left_cell.is_null();
        let right_null = right_cell.is_null();

        // Nulls sort last regardless of direction.
        if left_null || right_null {
            if left_null && right_null {
                continue;
            }
            return left_null.cmp(&right_null);
        }

        let mut ordering = compare_values(left_cell, right_cell);
        if key.descending {
            ordering = ordering.reverse();
        }
        if ordering != Ordering::Equal {
            return ordering;
        }
    }

    Ordering::Equal
}

fn compare_values(left: &CellValue, right: &CellValue) -> Ordering {
    if let (Some(left), Some(right)) = (numeric_value(left), numeric_value(right)) {
        return left.partial_cmp(&right).unwrap_or(Ordering::Equal);
    }
    left.to_text("").cmp(&right.to_text(""))
}

fn numeric_value(cell: &CellValue) -> Option<f64> {
    match cell {
        CellValue::Int(value) => Some(*value as f64),
        CellValue::Float(value) => Some(*value),
        CellValue::Decimal(value) | CellValue::Text(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return None;
            }
            let value = trimmed.parse::<f64>().ok()?;
            value.is_finite().then_some(value)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::FormatId;
    use crate::model::{Column, ColumnType, SourceRef};

    fn table() -> Table {
        Table::with_columns(
            "data",
            SourceRef::new("data.csv", FormatId::Csv),
            vec![
                Column::new("name", ColumnType::Text),
                Column::new("amount", ColumnType::Text),
            ],
            vec![
                vec![
                    CellValue::Text("Alice".into()),
                    CellValue::Text("120.5".into()),
                ],
                vec![CellValue::Text("Bob".into()), CellValue::Text("80".into())],
                vec![
                    CellValue::Text("Carol".into()),
                    CellValue::Text("200".into()),
                ],
            ],
        )
    }

    #[test]
    fn parses_multiple_keys_and_directions() {
        let keys = parse_keys("name, amount:desc").unwrap();
        assert_eq!(
            keys,
            vec![
                SortKey {
                    column: "name".into(),
                    descending: false
                },
                SortKey {
                    column: "amount".into(),
                    descending: true
                }
            ]
        );
        assert!(parse_keys("").is_err());
        assert!(parse_keys("amount:sideways").is_err());
    }

    #[test]
    fn descending_numeric_sort_is_stable_and_numeric() {
        let sorted = sort(&table(), &parse_keys("amount:desc").unwrap()).unwrap();
        let names = sorted
            .rows
            .iter()
            .map(|row| row[0].to_text(""))
            .collect::<Vec<_>>();
        assert_eq!(names, ["Carol", "Alice", "Bob"]);
    }

    #[test]
    fn nulls_sort_last_in_both_directions() {
        let table = Table::with_columns(
            "data",
            SourceRef::new("data.csv", FormatId::Csv),
            vec![Column::new("amount", ColumnType::Text)],
            vec![
                vec![CellValue::Null],
                vec![CellValue::Text("2".into())],
                vec![CellValue::Text("1".into())],
            ],
        );
        for key in ["amount", "amount:desc"] {
            let sorted = sort(&table, &parse_keys(key).unwrap()).unwrap();
            assert!(sorted.rows.last().unwrap()[0].is_null(), "key {key}");
        }
    }

    #[test]
    fn unknown_columns_are_rejected() {
        let error = sort(&table(), &parse_keys("missing").unwrap()).unwrap_err();
        assert!(error.to_string().contains("unknown column 'missing'"));
    }
}
