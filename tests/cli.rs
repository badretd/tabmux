//! CLI integration tests for the Phase 0 and Phase 1 surface.

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

#[test]
fn version_prints_name_and_cargo_version() {
    let output = tabmux().arg("--version").output().expect("run tabmux");
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("version is UTF-8");
    assert_eq!(
        stdout.trim(),
        format!("tabmux {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn help_lists_the_formats_command() {
    let output = tabmux().arg("--help").output().expect("run tabmux");
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("help is UTF-8");
    assert!(stdout.contains("Usage: tabmux"));
    assert!(stdout.contains("formats"));
}

#[test]
fn formats_prints_the_scope_matrix() {
    let output = tabmux().arg("formats").output().expect("run tabmux");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());

    let stdout = String::from_utf8(output.stdout).expect("matrix is UTF-8");
    let lines: Vec<_> = stdout.lines().collect();

    assert_eq!(lines[0], "FORMAT   READ  WRITE  MULTI-TABLE  NOTES");
    assert_eq!(
        &lines[1..],
        &[
            "csv      yes   yes    no           RFC 4180 + dialects",
            "tsv      yes   yes    no",
            "xlsx     yes   yes    yes          basic fidelity",
            "ods      yes   yes    yes          basic fidelity",
            "html     yes   no     yes          table extraction",
            "json     yes   no     no           JSON/JSONL input",
        ]
    );
}

#[test]
fn unknown_command_is_a_usage_error() {
    let output = tabmux()
        .arg("definitely-not-a-command")
        .output()
        .expect("run tabmux");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(stderr.contains("unrecognized subcommand"));
}

#[test]
fn quoted_csv_round_trips_through_tsv() {
    let dir = tempfile::tempdir().unwrap();
    let tsv = dir.path().join("quoted.tsv");
    let back = dir.path().join("quoted.back.csv");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("quoted.csv"))
        .arg(&tsv)
        .output()
        .unwrap();
    assert_success(&output);

    let output = tabmux()
        .arg("convert")
        .arg(&tsv)
        .arg(&back)
        .output()
        .unwrap();
    assert_success(&output);

    let direct = tabmux()
        .arg("convert")
        .arg(fixture("quoted.csv"))
        .arg("-")
        .output()
        .unwrap();
    assert_success(&direct);
    assert_eq!(fs::read(&back).unwrap(), direct.stdout);
}

#[test]
fn semicolon_csv_is_sniffed_without_a_flag() {
    let output = tabmux()
        .arg("convert")
        .arg(fixture("semicolon.csv"))
        .arg("-")
        .output()
        .unwrap();
    assert_success(&output);

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().next(), Some("name,email,amount"));
    assert_eq!(stdout.lines().count(), 4);
}

#[test]
fn latin1_input_becomes_utf8_output() {
    let dir = tempfile::tempdir().unwrap();
    let utf8 = dir.path().join("utf8.csv");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("latin1.csv"))
        .arg(&utf8)
        .output()
        .unwrap();
    assert_success(&output);

    let text = fs::read_to_string(utf8).unwrap();
    assert!(text.contains("café"), "output was:\n{text}");
    assert!(text.contains("Zürich"), "output was:\n{text}");
}

#[test]
fn bom_and_crlf_fixtures_parse_cleanly() {
    for name in ["bom.csv", "crlf.csv"] {
        let output = tabmux().arg("count").arg(fixture(name)).output().unwrap();
        assert_success(&output);
        assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "2");
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn count_head_and_select_work_on_csv() {
    let output = tabmux()
        .arg("count")
        .arg(fixture("basic.csv"))
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "3\n");

    let output = tabmux()
        .arg("head")
        .arg(fixture("basic.csv"))
        .arg("--rows")
        .arg("2")
        .output()
        .unwrap();
    assert_success(&output);
    let lines = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = lines.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "name,email,amount");
    assert!(lines[1].starts_with("Alice,"));
    assert!(lines[2].starts_with("Bob,"));

    let output = tabmux()
        .arg("select")
        .arg(fixture("basic.csv"))
        .arg("--columns")
        .arg("name,email")
        .output()
        .unwrap();
    assert_success(&output);
    let lines = String::from_utf8(output.stdout).unwrap();
    assert_eq!(lines.lines().next(), Some("name,email"));
    assert_eq!(lines.lines().count(), 4);
}

#[test]
fn select_reports_unknown_columns() {
    let output = tabmux()
        .arg("select")
        .arg(fixture("basic.csv"))
        .arg("--columns")
        .arg("missing")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let error = stderr(&output);
    assert!(error.contains("unknown column 'missing'"));
    assert!(error.contains("name"));
}

#[test]
fn no_header_affects_input_and_output() {
    let output = tabmux()
        .arg("head")
        .arg(fixture("basic.csv"))
        .arg("--rows")
        .arg("1")
        .arg("--no-header")
        .output()
        .unwrap();
    assert_success(&output);

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout, "name,email,amount\n");
}

#[test]
fn jsonl_is_converted_with_nested_objects_flattened() {
    let dir = tempfile::tempdir().unwrap();
    let csv = dir.path().join("data.csv");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("data.jsonl"))
        .arg(&csv)
        .output()
        .unwrap();
    assert_success(&output);

    let text = fs::read_to_string(&csv).unwrap();
    assert_eq!(
        text.lines().next(),
        Some("id,user.name,user.tags,amount,active")
    );
    assert_eq!(text.lines().count(), 3);
    assert!(stderr(&output).contains("json.flattened"));
}

#[test]
fn json_array_is_counted_as_rows() {
    let output = tabmux()
        .arg("count")
        .arg(fixture("data.json"))
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "2\n");
}

#[test]
fn existing_output_is_not_overwritten_without_force() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("existing.csv");
    fs::write(&destination, "old\n").unwrap();

    let output = tabmux()
        .arg("convert")
        .arg(fixture("basic.csv"))
        .arg(&destination)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("already exists"));
    assert_eq!(fs::read_to_string(&destination).unwrap(), "old\n");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("basic.csv"))
        .arg(&destination)
        .arg("--force")
        .output()
        .unwrap();
    assert_success(&output);
    assert!(
        fs::read_to_string(&destination)
            .unwrap()
            .starts_with("name,email,amount\n")
    );
}

#[test]
fn dry_run_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("nothing.csv");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("basic.csv"))
        .arg(&destination)
        .arg("--dry-run")
        .output()
        .unwrap();
    assert_success(&output);
    assert!(!destination.exists());
    assert!(stderr(&output).contains("dry-run"));
}

#[test]
fn backup_keeps_the_previous_output() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("out.csv");
    fs::write(&destination, "old\n").unwrap();

    let output = tabmux()
        .arg("convert")
        .arg(fixture("basic.csv"))
        .arg(&destination)
        .arg("--force")
        .arg("--backup")
        .output()
        .unwrap();
    assert_success(&output);

    assert!(
        fs::read_to_string(&destination)
            .unwrap()
            .starts_with("name,email,amount\n")
    );
    let backups = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "bak"))
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    assert_eq!(fs::read_to_string(backups[0].path()).unwrap(), "old\n");
}

#[test]
fn ragged_rows_follow_the_selected_policy() {
    let output = tabmux()
        .arg("convert")
        .arg(fixture("ragged.csv"))
        .arg("-")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("expected 3"));

    let output = tabmux()
        .arg("convert")
        .arg(fixture("ragged.csv"))
        .arg("-")
        .arg("--ragged")
        .arg("pad")
        .output()
        .unwrap();
    assert_success(&output);
    assert!(stderr(&output).contains("csv.ragged_row"));
    assert_eq!(String::from_utf8(output.stdout).unwrap().lines().count(), 4);

    let output = tabmux()
        .arg("convert")
        .arg(fixture("ragged.csv"))
        .arg("-")
        .arg("--ragged")
        .arg("skip")
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(String::from_utf8(output.stdout).unwrap().lines().count(), 2);
}

#[test]
fn output_line_endings_can_be_crlf() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("crlf-out.csv");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("basic.csv"))
        .arg(&destination)
        .arg("--line-ending")
        .arg("crlf")
        .output()
        .unwrap();
    assert_success(&output);

    let bytes = fs::read(destination).unwrap();
    assert!(bytes.starts_with(b"name,email,amount\r\n"));
}

#[test]
fn unsupported_output_format_exits_with_code_4() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("out.json");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("basic.csv"))
        .arg(&destination)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    assert!(stderr(&output).contains("writing json"));
}

#[test]
fn input_and_output_must_not_be_the_same_file() {
    let output = tabmux()
        .arg("convert")
        .arg(fixture("basic.csv"))
        .arg(fixture("basic.csv"))
        .arg("--force")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("same file"));
}

#[test]
fn output_can_be_written_with_the_short_flag_or_to_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("out.csv");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("basic.csv"))
        .arg("-o")
        .arg(&destination)
        .output()
        .unwrap();
    assert_success(&output);
    assert!(destination.exists());

    let output = tabmux()
        .arg("convert")
        .arg(fixture("basic.csv"))
        .arg("-o")
        .arg("-")
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().lines().next(),
        Some("name,email,amount")
    );
}
