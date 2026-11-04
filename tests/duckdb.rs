//! Integration tests for the `sql-duckdb` export and read-only query surface.

#![cfg(feature = "sql-duckdb")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tabmux() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tabmux"))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name)
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed with {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8")
}

#[cfg(feature = "excel")]
fn import_sales(db: &Path) {
    let output = tabmux()
        .args(["to", "duckdb"])
        .arg(fixture("sales.xlsx"))
        .args(["--sheet", "Sales", "--table", "sales", "-o"])
        .arg(db)
        .arg("--verify")
        .output()
        .unwrap();
    assert_success(&output);
    assert!(
        stderr(&output).contains("imported 4 row(s) x 6 column(s)"),
        "unexpected import summary: {}",
        stderr(&output)
    );
}

fn query(db: &Path, sql: &str, format: &str) -> Output {
    tabmux()
        .arg("query")
        .arg(db)
        .arg(sql)
        .args(["-f", format])
        .output()
        .unwrap()
}

#[test]
fn csv_import_and_query_work_without_the_excel_feature() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("warehouse.duckdb");

    let output = tabmux()
        .args(["to", "duckdb"])
        .arg(fixture("a.csv"))
        .args(["--table", "a", "-o"])
        .arg(&db)
        .arg("--verify")
        .output()
        .unwrap();
    assert_success(&output);

    let output = query(
        &db,
        "select count(*) as n, sum(amount) as total from a",
        "csv",
    );
    assert_success(&output);
    assert_eq!(stdout(&output), "n,total\n3,400.5\n");
}

#[cfg(feature = "excel")]
#[test]
fn import_count_query_and_join() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("warehouse.duckdb");
    import_sales(&db);

    let output = query(&db, "select count(*) as n from sales", "csv");
    assert_success(&output);
    assert_eq!(stdout(&output), "n\n4\n");

    let output = query(
        &db,
        "select region, sum(amount) as total from sales \
         where region is not null group by region order by total desc",
        "csv",
    );
    assert_success(&output);
    assert_eq!(stdout(&output), "region,total\nNorth,320\nSouth,80.5\n");

    let output = tabmux()
        .args(["to", "duckdb"])
        .arg(fixture("regions.csv"))
        .args(["--table", "regions", "-o"])
        .arg(&db)
        .output()
        .unwrap();
    assert_success(&output);

    let output = query(
        &db,
        "select s.name, r.manager from sales s join regions r \
         on s.region = r.region where s.name is not null order by s.name",
        "csv",
    );
    assert_success(&output);
    assert_eq!(
        stdout(&output),
        "name,manager\nAlice,Alice\nBob,Bob\nCarol,Alice\n"
    );
}

#[cfg(feature = "excel")]
#[test]
fn replace_requires_force_and_keeps_the_old_table() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("warehouse.duckdb");
    import_sales(&db);

    let output = tabmux()
        .args(["to", "duckdb"])
        .arg(fixture("regions.csv"))
        .args(["--table", "sales", "-o"])
        .arg(&db)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("already exists"));
    assert!(stderr(&output).contains("--force"));

    let count = query(&db, "select count(*) as n from sales", "csv");
    assert_success(&count);
    assert_eq!(stdout(&count), "n\n4\n");

    let output = tabmux()
        .args(["to", "duckdb"])
        .arg(fixture("regions.csv"))
        .args(["--table", "sales", "-o"])
        .arg(&db)
        .arg("--force")
        .output()
        .unwrap();
    assert_success(&output);
    assert!(stderr(&output).contains("replaced existing table"));

    let count = query(&db, "select count(*) as n from sales", "csv");
    assert_success(&count);
    assert_eq!(stdout(&count), "n\n4\n");
}

#[cfg(feature = "excel")]
#[test]
fn query_is_read_only_and_rejects_multiple_statements() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("warehouse.duckdb");
    import_sales(&db);

    for sql in [
        "drop table sales",
        "delete from sales",
        "create table evil (id integer)",
        "select 1; drop table sales",
    ] {
        let output = query(&db, sql, "csv");
        assert!(
            !output.status.success(),
            "query unexpectedly succeeded: {sql}"
        );
        assert!(
            stderr(&output).contains("read-only")
                || stderr(&output).contains("exactly one read-only statement"),
            "unexpected guard error for {sql}: {}",
            stderr(&output)
        );
    }

    let count = query(&db, "select count(*) as n from sales", "csv");
    assert_success(&count);
    assert_eq!(stdout(&count), "n\n4\n");
}

#[cfg(feature = "excel")]
#[test]
fn import_dry_run_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("warehouse.duckdb");

    let output = tabmux()
        .args(["to", "duckdb"])
        .arg(fixture("sales.xlsx"))
        .args(["--sheet", "Sales", "--table", "sales", "-o"])
        .arg(&db)
        .arg("--dry-run")
        .output()
        .unwrap();
    assert_success(&output);
    assert!(stderr(&output).contains("dry-run"));
    assert!(!db.exists(), "dry-run created the database");

    let output = tabmux()
        .args(["to", "duckdb"])
        .arg(fixture("sales.xlsx"))
        .args(["--all-sheets", "--table", "sales", "-o"])
        .arg(&db)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("exactly one table"));
    assert!(!db.exists(), "validation failure created the database");
}

#[cfg(feature = "excel")]
#[test]
fn failed_import_leaves_an_existing_database_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("warehouse.duckdb");
    import_sales(&db);

    let duplicate = dir.path().join("duplicate.csv");
    std::fs::write(&duplicate, "id,id\n1,2\n").unwrap();

    let output = tabmux()
        .args(["to", "duckdb"])
        .arg(&duplicate)
        .args(["--table", "sales", "-o"])
        .arg(&db)
        .arg("--force")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("duplicate column name"));

    let count = query(&db, "select count(*) as n from sales", "csv");
    assert_success(&count);
    assert_eq!(stdout(&count), "n\n4\n");
}

#[cfg(feature = "excel")]
#[test]
fn a_failure_inside_the_transaction_rolls_back_the_replaced_table() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("warehouse.duckdb");
    import_sales(&db);

    // More columns than the SQLite build accepts: table creation fails after
    // the existing table has been dropped inside the transaction, so only a
    // rollback can keep the old table alive.
    let width = 3000;
    let mut csv = String::new();
    for index in 0..width {
        if index > 0 {
            csv.push(',');
        }
        csv.push_str(&format!("c{index}"));
    }
    csv.push('\n');
    for index in 0..width {
        if index > 0 {
            csv.push(',');
        }
        csv.push('1');
    }
    csv.push('\n');
    let wide = dir.path().join("wide.csv");
    std::fs::write(&wide, csv).unwrap();

    let output = tabmux()
        .args(["to", "duckdb"])
        .arg(&wide)
        .args(["--table", "sales", "-o"])
        .arg(&db)
        .arg("--force")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));

    let count = query(&db, "select count(*) as n from sales", "csv");
    assert_success(&count);
    assert_eq!(stdout(&count), "n\n4\n");

    let fresh = dir.path().join("fresh.duckdb");
    let output = tabmux()
        .args(["to", "duckdb"])
        .arg(&wide)
        .args(["--table", "wide", "-o"])
        .arg(&fresh)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        !fresh.exists(),
        "a failed first import left a partial database behind"
    );
}

#[cfg(feature = "excel")]
#[test]
fn query_supports_json_and_pretty_output() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("warehouse.duckdb");
    import_sales(&db);

    let output = query(
        &db,
        "select id, name from sales order by id limit 2",
        "json",
    );
    assert_success(&output);
    let value: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let rows = value.as_array().expect("JSON result is an array");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], "1");
    assert_eq!(rows[0]["name"], "Alice");

    let output = query(&db, "select count(*) as n from sales", "table");
    assert_success(&output);
    let rendered = stdout(&output);
    assert!(rendered.contains("n"), "pretty output was:\n{rendered}");
    assert!(
        rendered.lines().any(|line| line.trim() == "-"),
        "pretty output was:\n{rendered}"
    );
    assert!(
        rendered
            .lines()
            .last()
            .is_some_and(|line| line.contains('4'))
    );
}

#[test]
fn missing_database_is_a_clear_data_error() {
    let dir = tempfile::tempdir().unwrap();
    let output = tabmux()
        .arg("query")
        .arg(dir.path().join("missing.duckdb"))
        .arg("select 1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("database not found"));
}

#[test]
fn real_duckdb_magic_is_rejected_instead_of_misread() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("native.duckdb");
    std::fs::write(&db, b"DUCK\x00\x01\x00\x00not-a-sqlite-file").unwrap();

    let output = tabmux()
        .arg("query")
        .arg(&db)
        .arg("select 1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    assert!(stderr(&output).contains("real DuckDB files are not supported"));
}
