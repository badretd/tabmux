//! Phase 11 hardening tests: messy fixtures, security edges, and the expanded
//! conformance corpus.

use std::fs;
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
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8")
}

fn read_report(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).expect("read report")).expect("report is JSON")
}

fn event(report: &serde_json::Value, code: &str) -> Option<serde_json::Value> {
    report["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["code"] == code)
        .cloned()
}

#[test]
fn empty_csv_writes_a_truly_empty_file() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("empty-out.csv");

    let result = tabmux()
        .arg("convert")
        .arg(fixture("empty.csv"))
        .arg(&output)
        .output()
        .unwrap();
    assert_success(&result);
    assert!(
        fs::read(&output).unwrap().is_empty(),
        "empty input produced stray bytes"
    );

    let counted = tabmux().arg("count").arg(&output).output().unwrap();
    assert_success(&counted);
    assert_eq!(stdout(&counted).trim(), "0");
}

#[test]
fn formula_injection_is_reported_and_strict_mode_blocks_it() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("formula.csv");
    let report_path = dir.path().join("formula-report.json");

    let result = tabmux()
        .arg("convert")
        .arg(fixture("formula.csv"))
        .arg(&output)
        .arg("--report")
        .arg(&report_path)
        .output()
        .unwrap();
    assert_success(&result);

    let report = read_report(&report_path);
    let formula = event(&report, "output.formula_injection").expect("formula-injection event");
    assert_eq!(formula["severity"], "warning");
    assert_eq!(formula["count"], 4, "only the formula-like prefixes count");
    assert_eq!(formula["source"]["column"], "note");
    assert_eq!(report["summary"]["warning"], 4);

    // The negative number `-120` must survive as ordinary text.
    let text = fs::read_to_string(&output).unwrap();
    assert!(
        text.contains("-120"),
        "negative number was treated as a formula"
    );
    assert!(text.contains("=SUM(A1:A2)"));

    let strict_output = dir.path().join("formula-strict.csv");
    let strict = tabmux()
        .arg("convert")
        .arg(fixture("formula.csv"))
        .arg(&strict_output)
        .arg("--strict")
        .output()
        .unwrap();
    assert_eq!(strict.status.code(), Some(5));
    assert!(stderr(&strict).contains("output.formula_injection"));
    assert!(
        !strict_output.exists(),
        "strict formula conversion wrote an output"
    );
}

#[test]
fn formula_like_text_is_safe_in_workbook_output() {
    let dir = tempfile::tempdir().unwrap();
    let workbook = dir.path().join("formula.xlsx");
    let report_path = dir.path().join("formula-xlsx.json");

    let result = tabmux()
        .arg("convert")
        .arg(fixture("formula.csv"))
        .arg(&workbook)
        .arg("--report")
        .arg(&report_path)
        .output()
        .unwrap();
    assert_success(&result);

    let report = read_report(&report_path);
    assert!(
        event(&report, "output.formula_injection").is_none(),
        "workbook output writes text cells, not formulas"
    );

    let round_trip = tabmux()
        .arg("convert")
        .arg(&workbook)
        .arg("-")
        .output()
        .unwrap();
    assert_success(&round_trip);
    let text = stdout(&round_trip);
    assert!(
        text.contains("=SUM(A1:A2)"),
        "formula text was not preserved"
    );
    assert!(text.contains("-1+2"), "formula-like text was not preserved");
}

#[test]
fn hidden_sheets_are_exported_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    let output_dir = dir.path().join("hidden");
    let report_path = dir.path().join("hidden.json");

    let sheets = tabmux()
        .arg("sheets")
        .arg(fixture("hidden.xlsx"))
        .output()
        .unwrap();
    assert_success(&sheets);
    assert_eq!(
        stdout(&sheets),
        "Visible\nHidden\nVeryHidden\nData\nDATA\nEmpty\n"
    );

    let result = tabmux()
        .arg("convert")
        .arg(fixture("hidden.xlsx"))
        .arg(&output_dir)
        .arg("--all-sheets")
        .arg("--split-sheets")
        .arg("--report")
        .arg(&report_path)
        .output()
        .unwrap();
    assert_success(&result);

    for name in [
        "hidden.Visible.csv",
        "hidden.Hidden.csv",
        "hidden.VeryHidden.csv",
        "hidden.Data.csv",
        "hidden.DATA-2.csv",
        "hidden.Empty.csv",
    ] {
        assert!(
            output_dir.join(name).is_file(),
            "missing split output {name}"
        );
    }

    assert_eq!(
        fs::read_to_string(output_dir.join("hidden.Hidden.csv")).unwrap(),
        "secret\nhidden-value\n"
    );
    assert_eq!(
        fs::read(output_dir.join("hidden.Empty.csv")).unwrap().len(),
        0,
        "the empty sheet wrote stray bytes"
    );

    let report = read_report(&report_path);
    let hidden = event(&report, "excel.hidden_sheet_read").expect("hidden-sheet event");
    assert_eq!(
        hidden["count"], 2,
        "hidden and very-hidden sheets are reported"
    );
    assert_eq!(report["summary"]["error"], 0);
}

#[test]
fn twenty_html_tables_become_sheets_with_stable_collisions() {
    let dir = tempfile::tempdir().unwrap();
    let workbook = dir.path().join("twenty.xlsx");

    let result = tabmux()
        .arg("convert")
        .arg(fixture("twenty-tables.html"))
        .arg(&workbook)
        .arg("--tables")
        .arg("all")
        .arg("--sheet-per-table")
        .arg("--use-captions")
        .arg("--index-sheet")
        .arg("--verify")
        .output()
        .unwrap();
    assert_success(&result);

    let sheets = tabmux().arg("sheets").arg(&workbook).output().unwrap();
    assert_success(&sheets);
    let names = stdout(&sheets);
    let lines = names.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 21, "20 data tables plus the index sheet");
    assert_eq!(lines[0], "_index");
    assert!(lines.contains(&"Sales"));
    assert!(
        lines.contains(&"Sales-2"),
        "duplicate caption was not suffixed"
    );
    assert!(lines.contains(&"Region_2026 20"));

    let error = stderr(&result);
    assert!(error.contains("html.header_colspan_expanded"));
    assert!(error.contains("html.table_skipped"));
    assert!(error.contains("html.sheet_name_collision"));
}

#[test]
fn malformed_html_is_tolerated_and_skipped_tables_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let workbook = dir.path().join("malformed.xlsx");

    let result = tabmux()
        .arg("convert")
        .arg(fixture("malformed.html"))
        .arg(&workbook)
        .arg("--tables")
        .arg("all")
        .arg("--sheet-per-table")
        .arg("--use-captions")
        .output()
        .unwrap();
    assert_success(&result);
    assert!(stderr(&result).contains("html.table_skipped"));

    let sheets = tabmux().arg("sheets").arg(&workbook).output().unwrap();
    assert_success(&sheets);
    assert_eq!(stdout(&sheets), "Broken & Free\n");

    let table = tabmux()
        .arg("convert")
        .arg(&workbook)
        .arg("-")
        .arg("--sheet")
        .arg("Broken & Free")
        .output()
        .unwrap();
    assert_success(&table);
    assert_eq!(stdout(&table), "id,name\n1,Alice\n2,Bob\n");
}

#[test]
fn malformed_zip_fails_cleanly_without_an_output_directory() {
    let dir = tempfile::tempdir().unwrap();
    let output_dir = dir.path().join("malformed-out");

    let result = tabmux()
        .arg("convert")
        .arg(fixture("malformed.zip"))
        .arg(&output_dir)
        .output()
        .unwrap();

    assert_eq!(
        result.status.code(),
        Some(1),
        "malformed ZIP is a data error"
    );
    let error = stderr(&result);
    assert!(
        error.contains("archive"),
        "error lacked archive context: {error}"
    );
    assert!(error.contains("invalid Zip archive") || error.contains("ZIP"));
    assert!(
        !output_dir.exists(),
        "malformed archive created an output directory"
    );
}

#[test]
fn every_supported_format_and_security_case_has_a_fixture() {
    for name in [
        "basic.csv",
        "quoted.csv",
        "bom.csv",
        "crlf.csv",
        "semicolon.csv",
        "ragged.csv",
        "latin1.csv",
        "empty.csv",
        "badquote.csv",
        "messy.csv",
        "formula.csv",
        "data.json",
        "data.jsonl",
        "sales.xlsx",
        "sales.ods",
        "rich.xlsx",
        "hidden.xlsx",
        "tables.html",
        "twenty-tables.html",
        "header-row.html",
        "malformed.html",
        "archive.zip",
        "traversal.zip",
        "bomb.zip",
        "malformed.zip",
    ] {
        assert!(fixture(name).is_file(), "fixture catalog is missing {name}");
    }
}
