//! SQL export and read-only query support for the `sql-duckdb` feature.
//!
//! The public contract is the one described by Phase 10: `to duckdb` imports
//! one table in a transaction, and `query` runs a single read-only statement
//! and renders its result. The real `duckdb` crate is not available in the
//! offline registry used by this build, so the feature is backed by the
//! embedded SQLite engine. The command surface, type mapping, transaction
//! semantics, and read-only guard are identical; database files are SQLite
//! files rather than DuckDB's native format. Real DuckDB files are rejected
//! explicitly instead of being misread. See `docs/implementation-notes.md`.

use std::collections::HashSet;
use std::fs;
use std::io::Read as _;
use std::path::Path;

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, params_from_iter};
use serde_json::{Value as JsonValue, json};
use sha2::{Digest as _, Sha256};

use crate::error::{Error, Result};
use crate::model::{CellValue, Column, ColumnType, SourceRef, Table};
use crate::ops::schema;

/// Stable name of the embedded backend used when DuckDB itself is unavailable.
pub const BACKEND: &str = "sqlite";

/// Result of one successful table import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    pub table: String,
    pub rows: usize,
    pub columns: usize,
    /// True when an existing table/view was replaced.
    pub replaced: bool,
    /// True when `--verify` compared every imported value.
    pub verified: bool,
    /// Deterministic checksum of the imported values.
    pub checksum: String,
}

/// One result set from a read-only query.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<CellValue>>,
}

impl QueryResult {
    /// Column types inferred from the returned values.
    pub fn column_types(&self) -> Vec<ColumnType> {
        let mut types = vec![ColumnType::Null; self.columns.len()];
        for row in &self.rows {
            for (index, cell) in row.iter().enumerate().take(types.len()) {
                types[index] = types[index].merge(cell.type_of());
            }
        }
        types
    }

    /// Convert the result into the shared table model for delimited or pretty
    /// rendering.
    pub fn to_table(&self, name: &str, format: crate::formats::FormatId) -> Table {
        let source = SourceRef::new(name, format);
        let columns = self
            .columns
            .iter()
            .cloned()
            .zip(self.column_types())
            .map(|(name, ty)| Column::new(name, ty))
            .collect();
        Table::with_columns("query", source, columns, self.rows.clone())
    }

    /// Render the result as a JSON array of objects.
    pub fn to_json(&self) -> JsonValue {
        let keys = unique_column_keys(&self.columns);
        let rows = self
            .rows
            .iter()
            .map(|row| {
                let mut object = serde_json::Map::with_capacity(keys.len());
                for (index, key) in keys.iter().enumerate() {
                    object.insert(
                        key.clone(),
                        row.get(index).map(cell_to_json).unwrap_or(JsonValue::Null),
                    );
                }
                JsonValue::Object(object)
            })
            .collect::<Vec<_>>();
        JsonValue::Array(rows)
    }
}

/// Import one table into a database.
///
/// The destination table is created inside a transaction. A failure rolls the
/// transaction back and, for a database created by this call, removes the new
/// file so no partial database is left behind.
pub fn import_table(
    database: &Path,
    table_name: &str,
    table: &Table,
    force: bool,
    verify: bool,
) -> Result<ImportReport> {
    validate_table_name(table_name)?;
    if database == Path::new("-") {
        return Err(Error::usage(
            "database output must be a file path, not stdout",
        ));
    }
    if table.width() == 0 {
        return Err(Error::data(format!(
            "cannot import a table with no columns from {}",
            table.source.context()
        )));
    }
    validate_columns(table)?;

    let existed = database.exists();
    ensure_database_compatible(database)?;

    let mut connection = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| database_error(database, error))?;

    // A PRAGMA touches the schema and gives a clear error for non-database
    // files before the transaction begins.
    connection
        .query_row("PRAGMA schema_version", [], |_| Ok(()))
        .map_err(|error| database_error(database, error))?;

    let result = (|| {
        let prepared = prepare_table(table)?;
        import_inner(
            &mut connection,
            database,
            table_name,
            &prepared,
            force,
            verify,
        )
    })();

    if result.is_err() && !existed {
        drop(connection);
        let _ = fs::remove_file(database);
    }

    result
}

fn import_inner(
    connection: &mut Connection,
    database: &Path,
    table_name: &str,
    table: &Table,
    force: bool,
    verify: bool,
) -> Result<ImportReport> {
    let existing = existing_relation(connection, table_name)
        .map_err(|error| database_error(database, error))?;

    if existing.is_some() && !force {
        return Err(Error::data(format!(
            "table '{table_name}' already exists in {} (use --force to replace it)",
            database.display()
        )));
    }

    let transaction = connection
        .transaction()
        .map_err(|error| database_error(database, error))?;

    if let Some(kind) = &existing {
        let statement = match kind.as_str() {
            "view" => format!("DROP VIEW {}", quote_identifier(table_name)),
            _ => format!("DROP TABLE {}", quote_identifier(table_name)),
        };
        transaction
            .execute(&statement, [])
            .map_err(|error| database_error(database, error))?;
    }

    let create = create_table_statement(table_name, table);
    transaction
        .execute(&create, [])
        .map_err(|error| database_error(database, error))?;

    let insert = insert_statement(table_name, table);
    {
        let mut statement = transaction
            .prepare(&insert)
            .map_err(|error| database_error(database, error))?;
        for (index, row) in table.rows.iter().enumerate() {
            let values = row
                .iter()
                .enumerate()
                .map(|(column, cell)| sql_value(cell, table, index, column))
                .collect::<Result<Vec<_>>>()?;
            statement
                .execute(params_from_iter(values))
                .map_err(|error| database_error(database, error))?;
        }
    }

    let imported: i64 = transaction
        .query_row(
            &format!("SELECT COUNT(*) FROM {}", quote_identifier(table_name)),
            [],
            |row| row.get(0),
        )
        .map_err(|error| database_error(database, error))?;

    if imported as usize != table.row_count() {
        return Err(Error::data(format!(
            "import verification failed for table '{table_name}': expected {} row(s), found {imported}",
            table.row_count()
        )));
    }

    if verify {
        verify_imported(&transaction, database, table_name, table)?;
    }

    transaction
        .commit()
        .map_err(|error| database_error(database, error))?;

    Ok(ImportReport {
        table: table_name.to_owned(),
        rows: table.row_count(),
        columns: table.width(),
        replaced: existing.is_some(),
        verified: verify,
        checksum: data_checksum(table),
    })
}

/// Run one read-only SQL statement.
pub fn query(database: &Path, sql: &str) -> Result<QueryResult> {
    if database == Path::new("-") {
        return Err(Error::usage(
            "database input must be a file path, not stdin",
        ));
    }
    if !database.is_file() {
        return Err(Error::data(format!(
            "database not found: {}",
            database.display()
        )));
    }
    ensure_database_compatible(database)?;

    let connection = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| database_error(database, error))?;

    let statement_text = single_statement(sql)?;
    let mut statement = connection
        .prepare(statement_text)
        .map_err(|error| query_error(database, error))?;

    if !statement.readonly() {
        return Err(Error::usage(
            "query is read-only; only SELECT, WITH, EXPLAIN, VALUES, and read-only PRAGMA statements are allowed",
        ));
    }

    let columns = statement
        .column_names()
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            if name.trim().is_empty() {
                format!("column_{}", index + 1)
            } else {
                name.to_owned()
            }
        })
        .collect::<Vec<_>>();

    let mut rows = statement
        .query([])
        .map_err(|error| query_error(database, error))?;
    let mut output = Vec::new();
    while let Some(row) = rows.next().map_err(|error| query_error(database, error))? {
        let mut values = Vec::with_capacity(columns.len());
        for index in 0..columns.len() {
            let value = row
                .get_ref(index)
                .map_err(|error| query_error(database, error))?;
            values.push(from_value_ref(value));
        }
        output.push(values);
    }

    Ok(QueryResult {
        columns,
        rows: output,
    })
}

/// The actual storage backend used by this build.
pub const fn backend() -> &'static str {
    BACKEND
}

fn prepare_table(table: &Table) -> Result<Table> {
    let mut prepared = table.clone();
    if matches!(
        prepared.source.format,
        crate::formats::FormatId::Csv
            | crate::formats::FormatId::Tsv
            | crate::formats::FormatId::Html
    ) {
        schema::infer_text_types(&mut prepared);
    }
    normalize_to_columns(&mut prepared)?;
    Ok(prepared)
}

/// Make every cell match its destination column type.
///
/// Workbook readers can produce a text column that still contains a few typed
/// cells (for example a merged "Total" row under a numeric id header). SQLite
/// applies the declared column affinity on insert, so relying on the raw cell
/// type would make verification compare different representations. Normalizing
/// once keeps the imported table, its declared schema, and verification in
/// agreement.
fn normalize_to_columns(table: &mut Table) -> Result<()> {
    for (row_index, row) in table.rows.iter_mut().enumerate() {
        for (column_index, cell) in row.iter_mut().enumerate() {
            let Some(column) = table.columns.get(column_index) else {
                continue;
            };
            let ty = column.ty;
            let name = column.name.clone();
            *cell = coerce_cell(cell, ty).ok_or_else(|| {
                Error::data(format!(
                    "cannot import {} row {} column '{}': {} value '{}' is not valid for a {} column",
                    table.source.context(),
                    row_index + 1,
                    name,
                    cell.type_of().as_str(),
                    cell.to_text(""),
                    ty.as_str()
                ))
            })?;
        }
    }
    Ok(())
}

fn coerce_cell(cell: &CellValue, ty: ColumnType) -> Option<CellValue> {
    if cell.is_null() {
        return Some(CellValue::Null);
    }
    match ty {
        ColumnType::Null => None,
        ColumnType::Text => Some(CellValue::Text(cell.to_text(""))),
        ColumnType::Bool => match cell {
            CellValue::Bool(_) => Some(cell.clone()),
            CellValue::Int(0) => Some(CellValue::Bool(false)),
            CellValue::Int(1) => Some(CellValue::Bool(true)),
            CellValue::Text(text) if text == "true" => Some(CellValue::Bool(true)),
            CellValue::Text(text) if text == "false" => Some(CellValue::Bool(false)),
            _ => None,
        },
        ColumnType::Int => match cell {
            CellValue::Int(_) => Some(cell.clone()),
            CellValue::Float(value)
                if value.is_finite()
                    && value.fract() == 0.0
                    && *value >= i64::MIN as f64
                    && *value <= i64::MAX as f64 =>
            {
                Some(CellValue::Int(*value as i64))
            }
            CellValue::Text(text) => schema::parse_safe_int(text).map(CellValue::Int),
            _ => None,
        },
        ColumnType::Float => match cell {
            CellValue::Float(_) => Some(cell.clone()),
            CellValue::Int(value) => Some(CellValue::Float(*value as f64)),
            CellValue::Text(text) => schema::parse_safe_float(text).map(CellValue::Float),
            _ => None,
        },
        ColumnType::Decimal => match cell {
            CellValue::Decimal(_) => Some(cell.clone()),
            CellValue::Text(text) => Some(CellValue::Decimal(text.clone())),
            CellValue::Int(_) | CellValue::Float(_) => Some(CellValue::Decimal(cell.to_text(""))),
            _ => None,
        },
        ColumnType::Date => match cell {
            CellValue::Date(_) => Some(cell.clone()),
            CellValue::Text(text) => schema::parse_date(text).map(CellValue::Date),
            _ => None,
        },
        ColumnType::DateTime => match cell {
            CellValue::DateTime(_) => Some(cell.clone()),
            CellValue::Text(text) => schema::parse_datetime(text).map(CellValue::DateTime),
            _ => None,
        },
        ColumnType::Time => match cell {
            CellValue::Time(_) => Some(cell.clone()),
            CellValue::Text(text) => schema::parse_time(text).map(CellValue::Time),
            _ => None,
        },
        ColumnType::Error => match cell {
            CellValue::Error(_) => Some(cell.clone()),
            CellValue::Text(text) => Some(CellValue::Error(text.clone())),
            _ => Some(CellValue::Error(cell.to_text(""))),
        },
    }
}

fn create_table_statement(table_name: &str, table: &Table) -> String {
    let columns = table
        .columns
        .iter()
        .map(|column| format!("{} {}", quote_identifier(&column.name), sql_type(column.ty)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("CREATE TABLE {} ({columns})", quote_identifier(table_name))
}

fn insert_statement(table_name: &str, table: &Table) -> String {
    let names = table
        .columns
        .iter()
        .map(|column| quote_identifier(&column.name))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = (1..=table.width())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "INSERT INTO {} ({names}) VALUES ({placeholders})",
        quote_identifier(table_name)
    )
}

const fn sql_type(ty: ColumnType) -> &'static str {
    match ty {
        ColumnType::Null => "TEXT",
        ColumnType::Bool => "BOOLEAN",
        ColumnType::Int => "BIGINT",
        ColumnType::Float => "DOUBLE",
        // Keep exact decimal text; SQLite's NUMERIC affinity would turn
        // "1.20" into a binary floating-point value.
        ColumnType::Decimal => "TEXT",
        ColumnType::Text => "TEXT",
        ColumnType::Date => "DATE",
        ColumnType::DateTime => "TIMESTAMP",
        ColumnType::Time => "TIME",
        ColumnType::Error => "TEXT",
    }
}

fn sql_value(
    cell: &CellValue,
    table: &Table,
    row: usize,
    column: usize,
) -> Result<rusqlite::types::Value> {
    use rusqlite::types::Value;

    Ok(match cell {
        CellValue::Null => Value::Null,
        CellValue::Bool(value) => Value::Integer(i64::from(*value)),
        CellValue::Int(value) => Value::Integer(*value),
        CellValue::Float(value) => {
            if !value.is_finite() {
                let name = table
                    .columns
                    .get(column)
                    .map(|column| column.name.as_str())
                    .unwrap_or("?");
                return Err(Error::data(format!(
                    "cannot import non-finite float '{value}' from {} row {} column '{name}'",
                    table.source.context(),
                    row + 1
                )));
            }
            Value::Real(*value)
        }
        CellValue::Decimal(text) | CellValue::Text(text) | CellValue::Error(text) => {
            Value::Text(text.clone())
        }
        CellValue::Date(_) | CellValue::DateTime(_) | CellValue::Time(_) => {
            Value::Text(cell.to_text(""))
        }
    })
}

fn from_value_ref(value: ValueRef<'_>) -> CellValue {
    match value {
        ValueRef::Null => CellValue::Null,
        ValueRef::Integer(value) => CellValue::Int(value),
        ValueRef::Real(value) => CellValue::Float(value),
        ValueRef::Text(bytes) => CellValue::Text(String::from_utf8_lossy(bytes).into_owned()),
        ValueRef::Blob(bytes) => CellValue::Text(blob_literal(bytes)),
    }
}

fn blob_literal(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2 + 3);
    out.push_str("x'");
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out.push('\'');
    out
}

fn verify_imported(
    transaction: &rusqlite::Transaction<'_>,
    database: &Path,
    table_name: &str,
    expected: &Table,
) -> Result<()> {
    let statement = format!(
        "SELECT * FROM {} ORDER BY rowid",
        quote_identifier(table_name)
    );
    let mut statement = transaction
        .prepare(&statement)
        .map_err(|error| database_error(database, error))?;
    let column_count = statement.column_count();
    if column_count != expected.width() {
        return Err(Error::data(format!(
            "import verification failed for table '{table_name}': expected {} column(s), found {column_count}",
            expected.width()
        )));
    }

    let mut rows = statement
        .query([])
        .map_err(|error| database_error(database, error))?;
    let mut index = 0;
    while let Some(row) = rows
        .next()
        .map_err(|error| database_error(database, error))?
    {
        if index >= expected.row_count() {
            return Err(Error::data(format!(
                "import verification failed for table '{table_name}': more rows than expected"
            )));
        }
        for column in 0..column_count {
            let actual = row
                .get_ref(column)
                .map_err(|error| database_error(database, error))?;
            let actual = from_value_ref(actual);
            let expected_cell = expected.rows[index].get(column).unwrap_or(&CellValue::Null);
            if !cell_matches(expected_cell, &actual) {
                return Err(Error::data(format!(
                    "import verification failed for table '{table_name}' row {} column '{}': expected {} '{}', found {} '{}'",
                    index + 1,
                    expected.columns[column].name,
                    expected_cell.type_of().as_str(),
                    expected_cell.to_text(""),
                    actual.type_of().as_str(),
                    actual.to_text("")
                )));
            }
        }
        index += 1;
    }

    if index != expected.row_count() {
        return Err(Error::data(format!(
            "import verification failed for table '{table_name}': expected {} row(s), found {index}",
            expected.row_count()
        )));
    }

    Ok(())
}

fn cell_matches(expected: &CellValue, actual: &CellValue) -> bool {
    match (expected, actual) {
        (CellValue::Null, CellValue::Null) => true,
        // SQLite stores booleans as integers.
        (CellValue::Bool(expected), CellValue::Int(actual)) => i64::from(*expected) == *actual,
        // A REAL column may return an integer when the value is whole.
        (CellValue::Float(expected), CellValue::Int(actual)) => *expected == *actual as f64,
        (CellValue::Float(expected), CellValue::Float(actual)) => expected == actual,
        (CellValue::Decimal(expected), CellValue::Text(actual))
        | (CellValue::Text(expected), CellValue::Text(actual))
        | (CellValue::Error(expected), CellValue::Text(actual)) => expected == actual,
        (CellValue::Date(_), CellValue::Text(actual))
        | (CellValue::DateTime(_), CellValue::Text(actual))
        | (CellValue::Time(_), CellValue::Text(actual)) => expected.to_text("") == *actual,
        (expected, actual) => expected == actual,
    }
}

fn data_checksum(table: &Table) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"tabmux-sql-v1\0");
    for row in &table.rows {
        for cell in row {
            let rendered = cell.to_text("");
            hasher.update(cell.type_of().as_str().as_bytes());
            hasher.update(b"\0");
            hasher.update((rendered.len() as u64).to_le_bytes());
            hasher.update(rendered.as_bytes());
        }
    }
    format!("{:x}", hasher.finalize())
}

fn validate_table_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        return Err(Error::usage("--table requires a non-empty name"));
    }
    if name.contains('\0') {
        return Err(Error::usage("--table may not contain a NUL byte"));
    }
    if name.to_ascii_lowercase().starts_with("sqlite_") {
        return Err(Error::usage(
            "table names beginning with 'sqlite_' are reserved by the SQL engine",
        ));
    }
    Ok(())
}

fn validate_columns(table: &Table) -> Result<()> {
    let mut seen = HashSet::with_capacity(table.width());
    for column in &table.columns {
        let name = &column.name;
        if name.trim().is_empty() {
            return Err(Error::data(format!(
                "cannot import {}: a column has an empty name",
                table.source.context()
            )));
        }
        if name.contains('\0') {
            return Err(Error::data(format!(
                "cannot import {}: column '{name}' contains a NUL byte",
                table.source.context()
            )));
        }
        if matches!(
            name.to_ascii_lowercase().as_str(),
            "rowid" | "_rowid_" | "oid"
        ) {
            return Err(Error::data(format!(
                "cannot import {}: column name '{name}' is reserved by the SQL engine",
                table.source.context()
            )));
        }
        let key = name.to_ascii_lowercase();
        if !seen.insert(key) {
            return Err(Error::data(format!(
                "cannot import {}: duplicate column name '{name}'",
                table.source.context()
            )));
        }
    }
    Ok(())
}

fn existing_relation(connection: &Connection, name: &str) -> rusqlite::Result<Option<String>> {
    let mut statement = connection.prepare(
        "SELECT type FROM sqlite_master WHERE name = ?1 COLLATE NOCASE AND type IN ('table','view') LIMIT 1",
    )?;
    let mut rows = statement.query([name])?;
    match rows.next()? {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}

fn ensure_database_compatible(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut file = fs::File::open(path)?;
    let mut header = [0u8; 16];
    let read = file.read(&mut header)?;
    let header = &header[..read];
    if header.starts_with(b"DUCK") {
        return Err(Error::unsupported(format!(
            "real DuckDB files are not supported by this build; {} was created by DuckDB (use a build with the native DuckDB backend)",
            path.display()
        )));
    }
    if read == 0 || header.starts_with(b"SQLite format 3\0") {
        return Ok(());
    }
    Err(Error::data(format!(
        "{} is not a SQL database supported by the embedded SQL backend",
        path.display()
    )))
}

/// Reject empty or multi-statement input. The returned slice is the single
/// statement with surrounding whitespace trimmed.
fn single_statement(sql: &str) -> Result<&str> {
    let statements = split_statements(sql);
    match statements.as_slice() {
        [] => Err(Error::usage("query is empty")),
        [statement] => Ok(statement),
        _ => Err(Error::usage(
            "query must contain exactly one read-only statement",
        )),
    }
}

fn split_statements(sql: &str) -> Vec<&str> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum State {
        Normal,
        Single,
        Double,
        Backtick,
        Bracket,
        LineComment,
        BlockComment,
    }

    let bytes = sql.as_bytes();
    let mut state = State::Normal;
    let mut statements = Vec::new();
    let mut start = 0;
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];
        let next = bytes.get(index + 1).copied();
        match state {
            State::Normal => match byte {
                b'\'' => state = State::Single,
                b'"' => state = State::Double,
                b'`' => state = State::Backtick,
                b'[' => state = State::Bracket,
                b'-' if next == Some(b'-') => {
                    state = State::LineComment;
                    index += 1;
                }
                b'/' if next == Some(b'*') => {
                    state = State::BlockComment;
                    index += 1;
                }
                b';' => {
                    let statement = sql[start..index].trim();
                    if !statement.is_empty() {
                        statements.push(statement);
                    }
                    start = index + 1;
                }
                _ => {}
            },
            State::Single => {
                if byte == b'\'' {
                    if next == Some(b'\'') {
                        index += 1;
                    } else {
                        state = State::Normal;
                    }
                }
            }
            State::Double => {
                if byte == b'"' {
                    if next == Some(b'"') {
                        index += 1;
                    } else {
                        state = State::Normal;
                    }
                }
            }
            State::Backtick => {
                if byte == b'`' {
                    if next == Some(b'`') {
                        index += 1;
                    } else {
                        state = State::Normal;
                    }
                }
            }
            State::Bracket => {
                if byte == b']' {
                    state = State::Normal;
                }
            }
            State::LineComment => {
                if byte == b'\n' {
                    state = State::Normal;
                }
            }
            State::BlockComment => {
                if byte == b'*' && next == Some(b'/') {
                    index += 1;
                    state = State::Normal;
                }
            }
        }
        index += 1;
    }

    let statement = sql[start..].trim();
    if !statement.is_empty() {
        statements.push(statement);
    }
    statements
}

fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn unique_column_keys(columns: &[String]) -> Vec<String> {
    let mut used = HashSet::with_capacity(columns.len());
    let mut keys = Vec::with_capacity(columns.len());
    for column in columns {
        let base = if column.trim().is_empty() {
            "column".to_owned()
        } else {
            column.clone()
        };
        let mut candidate = base.clone();
        let mut suffix = 2;
        while !used.insert(candidate.to_ascii_lowercase()) {
            candidate = format!("{base}_{suffix}");
            suffix += 1;
        }
        keys.push(candidate);
    }
    keys
}

fn cell_to_json(cell: &CellValue) -> JsonValue {
    match cell {
        CellValue::Null => JsonValue::Null,
        CellValue::Bool(value) => json!(value),
        CellValue::Int(value) => json!(value),
        CellValue::Float(value) => serde_json::Number::from_f64(*value)
            .map(JsonValue::Number)
            .unwrap_or_else(|| json!(value.to_string())),
        CellValue::Decimal(value) | CellValue::Text(value) | CellValue::Error(value) => {
            json!(value)
        }
        CellValue::Date(_) | CellValue::DateTime(_) | CellValue::Time(_) => {
            json!(cell.to_text(""))
        }
    }
}

fn database_error(path: &Path, error: rusqlite::Error) -> Error {
    Error::data(format!(
        "SQL backend error for '{}': {error}",
        path.display()
    ))
}

fn query_error(path: &Path, error: rusqlite::Error) -> Error {
    Error::data(format!("query failed for '{}': {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statement_splitter_ignores_semicolons_in_strings_and_comments() {
        assert_eq!(split_statements("select ';' as x"), ["select ';' as x"]);
        assert_eq!(
            split_statements("select 1; select 2"),
            ["select 1", "select 2"]
        );
        assert_eq!(
            split_statements("select 'a''; b' -- ; comment\n;"),
            ["select 'a''; b' -- ; comment"]
        );
        assert_eq!(
            split_statements("select 1 /* ; */ ; select 2;"),
            ["select 1 /* ; */", "select 2"]
        );
    }

    #[test]
    fn single_statement_rejects_empty_and_multiple_statements() {
        assert!(single_statement(" ; ").is_err());
        assert!(single_statement("select 1; select 2").is_err());
        assert_eq!(single_statement("select 1;").unwrap(), "select 1");
    }

    #[test]
    fn identifiers_are_quoted_and_escaped() {
        assert_eq!(quote_identifier("sales"), "\"sales\"");
        assert_eq!(quote_identifier("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn type_mapping_preserves_exact_decimal_text() {
        assert_eq!(sql_type(ColumnType::Int), "BIGINT");
        assert_eq!(sql_type(ColumnType::Float), "DOUBLE");
        assert_eq!(sql_type(ColumnType::Decimal), "TEXT");
        assert_eq!(sql_type(ColumnType::Date), "DATE");
        assert_eq!(sql_type(ColumnType::DateTime), "TIMESTAMP");
        assert_eq!(sql_type(ColumnType::Time), "TIME");
    }
}
