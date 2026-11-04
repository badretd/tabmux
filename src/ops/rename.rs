//! Column renaming.

use std::collections::HashSet;

use crate::error::{Error, Result};
use crate::model::Table;

/// One `old=new` column mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
    pub from: String,
    pub to: String,
}

/// Parse a comma-separated mapping specification.
///
/// Both `old=new` and `old->new` are accepted so the verb form and the
/// pipeline notation stay close to each other.
pub fn parse_mappings(specification: &str) -> Result<Vec<Mapping>> {
    let mut mappings = Vec::new();
    for raw in specification.split(',') {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }

        let (from, to) = if let Some((from, to)) = raw.split_once("->") {
            (from, to)
        } else if let Some((from, to)) = raw.split_once('=') {
            (from, to)
        } else {
            return Err(Error::usage(format!(
                "invalid rename mapping '{raw}'; expected old=new or old->new"
            )));
        };

        let from = from.trim();
        let to = to.trim();
        if from.is_empty() {
            return Err(Error::usage(format!(
                "invalid rename mapping '{raw}'; the source column is empty"
            )));
        }
        if to.is_empty() {
            return Err(Error::usage(format!(
                "invalid rename mapping '{raw}'; the target column is empty"
            )));
        }

        mappings.push(Mapping {
            from: from.to_owned(),
            to: to.to_owned(),
        });
    }

    if mappings.is_empty() {
        return Err(Error::usage(
            "rename --map must contain at least one old=new mapping",
        ));
    }
    Ok(mappings)
}

/// Rename columns in place while preserving their order and rows.
pub fn rename(table: &Table, mappings: &[Mapping]) -> Result<Table> {
    let mut names = table
        .columns
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();

    for mapping in mappings {
        let index = names
            .iter()
            .position(|name| name == &mapping.from)
            .ok_or_else(|| {
                Error::data(format!(
                    "unknown column '{}' in --map; available columns: {}",
                    mapping.from,
                    table.column_names().join(", ")
                ))
            })?;
        names[index] = mapping.to.clone();
    }

    let mut seen = HashSet::new();
    for name in &names {
        if !seen.insert(name.clone()) {
            return Err(Error::usage(format!(
                "rename would produce duplicate column '{name}'"
            )));
        }
    }

    let mut columns = table.columns.clone();
    for (column, name) in columns.iter_mut().zip(names) {
        column.name = name;
    }

    Ok(Table {
        name: table.name.clone(),
        source: table.source.clone(),
        columns,
        rows: table.rows.clone(),
        warnings: table.warnings.clone(),
        first_data_row: table.first_data_row,
    })
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
            vec![
                Column::new("id", ColumnType::Text),
                Column::new("amount", ColumnType::Text),
            ],
            vec![vec![
                CellValue::Text("1".into()),
                CellValue::Text("10".into()),
            ]],
        )
    }

    #[test]
    fn parses_both_mapping_separators() {
        let mappings = parse_mappings("amount=total, id->key").unwrap();
        assert_eq!(mappings[0].to, "total");
        assert_eq!(mappings[1].from, "id");
        assert!(parse_mappings("amount").is_err());
    }

    #[test]
    fn renames_in_place_and_keeps_order() {
        let renamed = rename(&table(), &parse_mappings("amount=total").unwrap()).unwrap();
        assert_eq!(renamed.column_names(), ["id", "total"]);
        assert_eq!(renamed.rows, table().rows);
    }

    #[test]
    fn rejects_unknown_and_duplicate_names() {
        let error = rename(&table(), &parse_mappings("missing=x").unwrap()).unwrap_err();
        assert!(error.to_string().contains("unknown column 'missing'"));

        let error = rename(&table(), &parse_mappings("amount=id").unwrap()).unwrap_err();
        assert!(error.to_string().contains("duplicate column 'id'"));
    }
}
