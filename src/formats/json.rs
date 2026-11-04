//! JSON and JSONL reader.
//!
//! Objects are flattened with dot-separated keys (`user.name`). Arrays are
//! preserved as JSON text so no information is silently lost.

use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use crate::error::{Error, Result};
use crate::io::decode_text;
use crate::model::{CellValue, Column, ColumnType, LossEvent, Row, SourceRef, Table};

/// Input options for JSON.
#[derive(Debug, Clone, Default)]
pub struct ReadOptions {
    pub encoding: Option<String>,
}

/// Read a JSON or JSONL input into one table.
pub fn read(source: &SourceRef, bytes: &[u8], options: &ReadOptions) -> Result<Table> {
    let decoded = decode_text(bytes, options.encoding.as_deref())?;
    let text = decoded.text;
    let is_jsonl = has_jsonl_extension(&source.path)
        || (serde_json::from_str::<Value>(&text).is_err() && looks_like_jsonl(&text));

    let roots = if is_jsonl {
        parse_jsonl(source, &text)?
    } else {
        parse_document(source, &text)?
    };

    let mut warnings = Vec::new();
    let (names, rows) = if !roots.is_empty() && roots.iter().all(Value::is_array) {
        build_array_rows(&roots)
    } else {
        build_object_rows(source, roots, &mut warnings)?
    };

    let columns = columns_from_values(names, &rows);

    Ok(Table {
        name: table_name(source),
        source: source.clone(),
        columns,
        rows,
        warnings,
        first_data_row: 1,
    })
}

fn parse_document(source: &SourceRef, text: &str) -> Result<Vec<Value>> {
    let value: Value = serde_json::from_str(text).map_err(|error| {
        Error::data(format!(
            "{}: invalid JSON at line {}, column {}: {error}",
            source.path.display(),
            error.line(),
            error.column()
        ))
    })?;

    Ok(match value {
        Value::Array(items) => items,
        other => vec![other],
    })
}

fn parse_jsonl(source: &SourceRef, text: &str) -> Result<Vec<Value>> {
    let mut values = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value = serde_json::from_str(line).map_err(|error| {
            Error::data(format!(
                "{}: invalid JSONL at line {}: {error}",
                source.path.display(),
                index + 1
            ))
        })?;
        values.push(value);
    }
    Ok(values)
}

fn looks_like_jsonl(text: &str) -> bool {
    let lines = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    lines.len() > 1
        && lines
            .iter()
            .all(|line| serde_json::from_str::<Value>(line).is_ok())
}

fn has_jsonl_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(extension.to_ascii_lowercase().as_str(), "jsonl" | "ndjson")
        })
}

#[derive(Default)]
struct Builder {
    names: Vec<String>,
    index: HashMap<String, usize>,
    rows: Vec<Row>,
}

impl Builder {
    fn add_record(&mut self, fields: Vec<(String, CellValue)>) {
        for (name, _) in &fields {
            if !self.index.contains_key(name) {
                let index = self.names.len();
                self.names.push(name.clone());
                self.index.insert(name.clone(), index);
                for row in &mut self.rows {
                    row.push(CellValue::Null);
                }
            }
        }

        let mut row = vec![CellValue::Null; self.names.len()];
        for (name, value) in fields {
            if let Some(&index) = self.index.get(&name) {
                row[index] = value;
            }
        }
        self.rows.push(row);
    }
}

fn build_object_rows(
    source: &SourceRef,
    roots: Vec<Value>,
    warnings: &mut Vec<LossEvent>,
) -> Result<(Vec<String>, Vec<Row>)> {
    let mut builder = Builder::default();

    for (index, value) in roots.into_iter().enumerate() {
        let row_number = index as u64 + 1;
        let fields = match value {
            Value::Object(object) => {
                let mut fields = Vec::new();
                flatten_object("", &object, &mut fields, warnings, source, row_number);
                fields
            }
            other => vec![("value".to_owned(), value_to_cell(&other))],
        };
        builder.add_record(fields);
    }

    Ok((builder.names, builder.rows))
}

fn flatten_object(
    prefix: &str,
    object: &serde_json::Map<String, Value>,
    fields: &mut Vec<(String, CellValue)>,
    warnings: &mut Vec<LossEvent>,
    source: &SourceRef,
    row_number: u64,
) {
    for (key, value) in object {
        let name = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };

        match value {
            Value::Object(inner) => {
                warnings.push(
                    LossEvent::warning(
                        "json.flattened",
                        format!("flattened nested object '{name}'"),
                    )
                    .with_source(source.clone().with_row(row_number)),
                );
                if inner.is_empty() {
                    fields.push((name, CellValue::Text("{}".to_owned())));
                } else {
                    flatten_object(&name, inner, fields, warnings, source, row_number);
                }
            }
            other => fields.push((name, value_to_cell(other))),
        }
    }
}

fn build_array_rows(roots: &[Value]) -> (Vec<String>, Vec<Row>) {
    let width = roots
        .iter()
        .map(|value| value.as_array().map_or(0, Vec::len))
        .max()
        .unwrap_or(0);

    let names = (1..=width)
        .map(|index| format!("column_{index}"))
        .collect::<Vec<_>>();

    let rows = roots
        .iter()
        .map(|value| {
            let items = value.as_array().expect("caller checked array roots");
            let mut row = vec![CellValue::Null; width];
            for (index, item) in items.iter().enumerate() {
                row[index] = value_to_cell(item);
            }
            row
        })
        .collect();

    (names, rows)
}

fn value_to_cell(value: &Value) -> CellValue {
    match value {
        Value::Null => CellValue::Null,
        Value::Bool(value) => CellValue::Bool(*value),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                CellValue::Int(value)
            } else if let Some(value) = number.as_u64() {
                CellValue::Decimal(value.to_string())
            } else if let Some(value) = number.as_f64() {
                CellValue::Float(value)
            } else {
                CellValue::Decimal(number.to_string())
            }
        }
        Value::String(value) => CellValue::Text(value.clone()),
        Value::Array(_) | Value::Object(_) => {
            CellValue::Text(serde_json::to_string(value).unwrap_or_default())
        }
    }
}

fn columns_from_values(names: Vec<String>, rows: &[Row]) -> Vec<Column> {
    let mut types = vec![ColumnType::Null; names.len()];
    for row in rows {
        for (index, value) in row.iter().enumerate() {
            if let Some(ty) = types.get_mut(index) {
                *ty = ty.merge(value.type_of());
            }
        }
    }

    names
        .into_iter()
        .zip(types)
        .map(|(name, ty)| Column::new(name, ty))
        .collect()
}

fn table_name(source: &SourceRef) -> String {
    if source.path.as_os_str() == "-" {
        return "stdin".to_owned();
    }
    source
        .path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("table")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::FormatId;

    fn source(name: &str) -> SourceRef {
        SourceRef::new(name, FormatId::Json)
    }

    #[test]
    fn flattens_objects_and_keeps_arrays_as_text() {
        let input = br#"[
            {"id": 1, "user": {"name": "Alice", "tags": ["x", "y"]}},
            {"id": 2, "user": {"name": "Bob"}}
        ]"#;
        let table = read(&source("data.json"), input, &ReadOptions::default()).unwrap();

        assert_eq!(table.column_names(), ["id", "user.name", "user.tags"]);
        assert_eq!(table.columns[0].ty, ColumnType::Int);
        assert_eq!(table.columns[1].ty, ColumnType::Text);
        assert_eq!(table.rows[0][0], CellValue::Int(1));
        assert_eq!(table.rows[0][1], CellValue::Text("Alice".into()));
        assert_eq!(table.rows[0][2], CellValue::Text("[\"x\",\"y\"]".into()));
        assert_eq!(table.rows[1][2], CellValue::Null);
        assert_eq!(table.warnings.len(), 2);
        assert_eq!(table.warnings[0].code, "json.flattened");
    }

    #[test]
    fn reads_jsonl_as_multiple_rows() {
        let input = b"{\"id\":1}\n\n{\"id\":2,\"ok\":true}\n";
        let table = read(&source("data.jsonl"), input, &ReadOptions::default()).unwrap();
        assert_eq!(table.column_names(), ["id", "ok"]);
        assert_eq!(table.row_count(), 2);
        assert_eq!(table.rows[1][1], CellValue::Bool(true));
        assert_eq!(table.columns[1].ty, ColumnType::Bool);
    }

    #[test]
    fn reads_a_single_object_as_one_row() {
        let table = read(
            &source("data.json"),
            br#"{"name":"Alice","amount":12.5}"#,
            &ReadOptions::default(),
        )
        .unwrap();
        assert_eq!(table.row_count(), 1);
        assert_eq!(table.columns[0].ty, ColumnType::Text);
        assert_eq!(table.columns[1].ty, ColumnType::Float);
    }

    #[test]
    fn preserves_large_unsigned_integers_as_decimal_text() {
        let table = read(
            &source("data.json"),
            br#"{"id": 18446744073709551615}"#,
            &ReadOptions::default(),
        )
        .unwrap();
        assert_eq!(
            table.rows[0][0],
            CellValue::Decimal("18446744073709551615".into())
        );
    }

    #[test]
    fn reports_invalid_json_with_a_location() {
        let error = read(
            &source("bad.json"),
            b"{\n  \"a\": }",
            &ReadOptions::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("invalid JSON"));
        assert!(error.to_string().contains("line 2"));
    }

    #[test]
    fn reports_invalid_jsonl_with_a_line() {
        let error = read(
            &source("bad.jsonl"),
            b"{\"a\":1}\nnot-json\n",
            &ReadOptions::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("line 2"));
    }
}
