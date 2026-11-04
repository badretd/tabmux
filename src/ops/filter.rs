//! Simple row predicates (`column OP value`) shared by `filter` and `row`.

use std::cmp::Ordering;

use crate::error::{Error, Result};
use crate::model::{CellValue, Row, Table};

/// Comparison operator in a simple filter expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
}

impl Operator {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::Lt => "<",
            Self::Le => "<=",
        }
    }

    fn apply(self, ordering: Ordering) -> bool {
        match self {
            Self::Eq => ordering == Ordering::Equal,
            Self::Ne => ordering != Ordering::Equal,
            Self::Gt => ordering == Ordering::Greater,
            Self::Ge => ordering != Ordering::Less,
            Self::Lt => ordering == Ordering::Less,
            Self::Le => ordering != Ordering::Greater,
        }
    }
}

/// A parsed simple predicate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Predicate {
    pub column: String,
    pub operator: Operator,
    pub operand: String,
    expression: String,
}

impl Predicate {
    /// Whether this predicate names a column in `table`.
    pub fn is_valid_for(&self, table: &Table) -> bool {
        table.column_index(&self.column).is_some()
    }

    /// Evaluate the predicate for one row.
    ///
    /// Equality is textual, matching the byte-faithful delimited model.
    /// Ordering comparisons are numeric when the right-hand operand is a
    /// number; otherwise they compare the rendered text (which also makes
    /// ISO-8601 dates sort correctly).
    pub fn matches(&self, table: &Table, row: &Row, source_row: u64) -> Result<bool> {
        let index = table.column_index(&self.column).ok_or_else(|| {
            Error::data(format!(
                "unknown column '{}'; available columns: {}",
                self.column,
                table.column_names().join(", ")
            ))
        })?;
        let cell = row.get(index).unwrap_or(&CellValue::Null);
        let rendered = cell.to_text("");

        if matches!(self.operator, Operator::Eq | Operator::Ne) {
            let equal = rendered == self.operand;
            return Ok(if self.operator == Operator::Eq {
                equal
            } else {
                !equal
            });
        }

        if cell.is_null() || rendered.is_empty() {
            return Ok(false);
        }

        let ordering = if let Some(operand) = parse_number(&self.operand) {
            let Some(value) = numeric_value(cell) else {
                return Err(Error::data(format!(
                    "{} row {source_row}: column '{}' contains '{}', which is not numeric, in filter '{}'",
                    table.source.path.display(),
                    self.column,
                    rendered,
                    self.expression
                )));
            };
            value.partial_cmp(&operand).unwrap_or(Ordering::Equal)
        } else {
            rendered.as_str().cmp(self.operand.as_str())
        };

        Ok(self.operator.apply(ordering))
    }
}

/// Parse `column == value`, `column != value`, or a numeric/text comparison.
pub fn parse(expression: &str) -> Result<Predicate> {
    // Two-character tokens come first so `>=` is not parsed as `>`.
    const TOKENS: [(&str, Operator); 7] = [
        ("!=", Operator::Ne),
        (">=", Operator::Ge),
        ("<=", Operator::Le),
        ("==", Operator::Eq),
        (">", Operator::Gt),
        ("<", Operator::Lt),
        ("=", Operator::Eq),
    ];

    let mut found: Option<(usize, &str, Operator)> = None;
    for (token, operator) in TOKENS {
        if let Some(position) = expression.find(token)
            && found.is_none_or(|(best, _, _)| position < best)
        {
            found = Some((position, token, operator));
        }
    }

    let Some((position, token, operator)) = found else {
        return Err(Error::usage(format!(
            "invalid filter '{expression}'; expected 'column OP value' with OP one of ==, !=, >, >=, <, <="
        )));
    };

    let column = expression[..position].trim();
    if column.is_empty() {
        return Err(Error::usage(format!(
            "invalid filter '{expression}'; the column name is empty"
        )));
    }

    let operand = unquote(expression[position + token.len()..].trim());
    Ok(Predicate {
        column: column.to_owned(),
        operator,
        operand,
        expression: expression.to_owned(),
    })
}

/// Filter a table, retaining only matching rows.
pub fn filter(table: &Table, predicate: &Predicate) -> Result<Table> {
    if !predicate.is_valid_for(table) {
        return Err(Error::data(format!(
            "unknown column '{}'; available columns: {}",
            predicate.column,
            table.column_names().join(", ")
        )));
    }

    let mut rows = Vec::new();
    let mut kept_source_rows = std::collections::HashSet::new();
    for (index, row) in table.rows.iter().enumerate() {
        let source_row = table.source_row(index);
        if predicate.matches(table, row, source_row)? {
            kept_source_rows.insert(source_row);
            rows.push(row.clone());
        }
    }

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

    Ok(Table {
        name: table.name.clone(),
        source: table.source.clone(),
        columns: table.columns.clone(),
        rows,
        warnings,
        first_data_row: 1,
    })
}

fn numeric_value(cell: &CellValue) -> Option<f64> {
    match cell {
        CellValue::Int(value) => Some(*value as f64),
        CellValue::Float(value) => Some(*value),
        CellValue::Decimal(value) | CellValue::Text(value) => parse_number(value),
        _ => None,
    }
}

fn parse_number(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let value = trimmed.parse::<f64>().ok()?;
    value.is_finite().then_some(value)
}

fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
            return value[1..value.len() - 1].to_owned();
        }
    }
    value.to_owned()
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
                    CellValue::Text("120".into()),
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
    fn parses_equality_inequality_and_numeric_comparisons() {
        assert_eq!(parse("id == 3").unwrap().operator, Operator::Eq);
        assert_eq!(parse("id != 3").unwrap().operator, Operator::Ne);
        assert_eq!(parse("amount >= 100").unwrap().operator, Operator::Ge);
        assert_eq!(parse("amount < 100").unwrap().operator, Operator::Lt);
        assert_eq!(parse("amount > 100").unwrap().operand, "100");
        assert_eq!(
            parse("name == 'Ada Lovelace'").unwrap().operand,
            "Ada Lovelace"
        );
    }

    #[test]
    fn numeric_comparisons_select_the_expected_rows() {
        let table = table();
        let filtered = filter(&table, &parse("amount > 100").unwrap()).unwrap();
        assert_eq!(filtered.row_count(), 2);
        assert_eq!(
            filtered.rows[0][0],
            CellValue::Text("Alice".into()),
            "row order must be stable"
        );

        let filtered = filter(&table, &parse("amount <= 120").unwrap()).unwrap();
        assert_eq!(filtered.row_count(), 2);
        let filtered = filter(&table, &parse("name != Bob").unwrap()).unwrap();
        assert_eq!(filtered.row_count(), 2);
    }

    #[test]
    fn non_numeric_ordering_reports_the_row() {
        let table = Table::with_columns(
            "data",
            SourceRef::new("data.csv", FormatId::Csv),
            vec![Column::new("amount", ColumnType::Text)],
            vec![vec![CellValue::Text("unknown".into())]],
        );
        let error = filter(&table, &parse("amount > 100").unwrap()).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("row 1"));
        assert!(message.contains("not numeric"));
    }

    #[test]
    fn empty_values_do_not_match_ordering_comparisons() {
        let table = Table::with_columns(
            "data",
            SourceRef::new("data.csv", FormatId::Csv),
            vec![Column::new("amount", ColumnType::Text)],
            vec![vec![CellValue::Null], vec![CellValue::Text("5".into())]],
        );
        let filtered = filter(&table, &parse("amount > 1").unwrap()).unwrap();
        assert_eq!(filtered.row_count(), 1);
    }
}
