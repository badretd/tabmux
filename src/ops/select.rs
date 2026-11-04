//! Column selection.

use crate::error::{Error, Result};
use crate::model::{CellValue, Column, Row, Table};

/// Select columns by name or 1-based index and return a derived table.
pub fn select_columns(table: &Table, specification: &str) -> Result<Table> {
    let specifications = specification
        .split(',')
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    select_columns_named(table, &specifications)
}

/// Select columns from an already-split list of names or indices.
pub fn select_columns_named(table: &Table, specifications: &[String]) -> Result<Table> {
    let mut indices = Vec::new();

    for raw in specifications {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }

        let index = match raw.parse::<usize>() {
            Ok(0) => {
                return Err(Error::usage("column indices are 1-based; 0 is not valid"));
            }
            Ok(number) => {
                let index = number - 1;
                if index >= table.width() {
                    return Err(Error::data(format!(
                        "column index {number} is out of range; the table has {} columns",
                        table.width()
                    )));
                }
                index
            }
            Err(_) => table.column_index(raw).ok_or_else(|| {
                Error::data(format!(
                    "unknown column '{raw}'; available columns: {}",
                    table.column_names().join(", ")
                ))
            })?,
        };

        indices.push(index);
    }

    if indices.is_empty() {
        return Err(Error::usage(
            "select --columns must name at least one column",
        ));
    }

    let columns = indices
        .iter()
        .map(|&index| table.columns[index].clone())
        .collect::<Vec<Column>>();
    let rows = table
        .rows
        .iter()
        .map(|row| {
            indices
                .iter()
                .map(|&index| row.get(index).cloned().unwrap_or(CellValue::Null))
                .collect::<Row>()
        })
        .collect::<Vec<Row>>();

    Ok(Table {
        name: table.name.clone(),
        source: table.source.clone(),
        columns,
        rows,
        warnings: table.warnings.clone(),
        first_data_row: table.first_data_row,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::FormatId;
    use crate::model::{CellValue, ColumnType, SourceRef};

    fn table() -> Table {
        Table::with_columns(
            "data",
            SourceRef::new("data.csv", FormatId::Csv),
            vec![
                Column::new("name", ColumnType::Text),
                Column::new("amount", ColumnType::Int),
            ],
            vec![
                vec![CellValue::Text("a".into()), CellValue::Int(1)],
                vec![CellValue::Text("b".into()), CellValue::Int(2)],
            ],
        )
    }

    #[test]
    fn select_accepts_names_and_indices() {
        let selected = select_columns(&table(), "amount").unwrap();
        assert_eq!(selected.column_names(), ["amount"]);

        let selected = select_columns(&table(), "2,1").unwrap();
        assert_eq!(selected.column_names(), ["amount", "name"]);
        assert_eq!(selected.rows[0][0], CellValue::Int(1));
        assert_eq!(selected.rows[0][1], CellValue::Text("a".into()));
    }

    #[test]
    fn select_reports_unknown_columns_with_available_names() {
        let error = select_columns(&table(), "missing").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("missing"));
        assert!(message.contains("name"));
    }

    #[test]
    fn select_rejects_zero_indices() {
        let error = select_columns(&table(), "0").unwrap_err();
        assert_eq!(error.exit_code(), crate::error::ExitCode::Usage);
    }
}
