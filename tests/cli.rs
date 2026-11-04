//! CLI integration tests for the Phase 0 and Phase 1 surface.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

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
fn sheets_lists_workbook_sheets_in_order() {
    let output = tabmux()
        .arg("sheets")
        .arg(fixture("sales.xlsx"))
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Sales\nRegions\nNotes\n"
    );
}

#[test]
fn sheets_rejects_delimited_input() {
    let output = tabmux()
        .arg("sheets")
        .arg(fixture("basic.csv"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    assert!(stderr(&output).contains("not a multi-table workbook"));
}

#[test]
fn xlsx_sheet_converts_to_csv_with_dates_and_cached_values() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("sales.csv");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("sales.xlsx"))
        .arg(&destination)
        .arg("--sheet")
        .arg("Sales")
        .output()
        .unwrap();
    assert_success(&output);

    let text = fs::read_to_string(&destination).unwrap();
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines[0], "id,name,region,amount,sold_on,note");
    assert!(lines[1].starts_with("1,Alice,North,120,2026-01-15,"));
    assert!(lines[2].contains("2026-02-01"));
    assert!(
        text.contains("400.5"),
        "cached formula value missing:\n{text}"
    );

    let error = stderr(&output);
    assert!(
        !error.contains("excel.merged_cell_expanded"),
        "default convert should not print fidelity events:\n{error}"
    );
    assert!(!error.contains("excel.chart_dropped"));
    assert!(!error.contains("excel.style_dropped"));

    let fidelity_destination = dir.path().join("sales-fidelity.csv");
    let output = tabmux()
        .arg("convert")
        .arg(fixture("sales.xlsx"))
        .arg(&fidelity_destination)
        .arg("--sheet")
        .arg("Sales")
        .arg("--fidelity-report")
        .output()
        .unwrap();
    assert_success(&output);

    let error = stderr(&output);
    assert!(error.contains("excel.merged_cell_expanded"));
    assert!(error.contains("excel.formula_cached_value_only"));
    assert!(error.contains("excel.chart_dropped"));
    assert!(error.contains("excel.style_dropped"));
}

#[test]
fn xlsx_sheet_index_is_one_based() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("regions.csv");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("sales.xlsx"))
        .arg(&destination)
        .arg("--sheet-index")
        .arg("2")
        .output()
        .unwrap();
    assert_success(&output);

    let text = fs::read_to_string(&destination).unwrap();
    assert!(text.starts_with("region,manager\nNorth,Ada\n"));
}

#[test]
fn all_sheets_split_writes_one_file_per_sheet() {
    let dir = tempfile::tempdir().unwrap();
    let output_dir = dir.path().join("sheets");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("sales.xlsx"))
        .arg(&output_dir)
        .arg("--all-sheets")
        .arg("--split-sheets")
        .output()
        .unwrap();
    assert_success(&output);

    let mut names = fs::read_dir(&output_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        ["sales.Notes.csv", "sales.Regions.csv", "sales.Sales.csv"]
    );
}

#[test]
fn all_sheets_without_split_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("out.csv");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("sales.xlsx"))
        .arg(&destination)
        .arg("--all-sheets")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("--split-sheets"));
    assert!(!destination.exists());
}

#[test]
fn missing_sheet_lists_available_sheets() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("out.csv");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("sales.xlsx"))
        .arg(&destination)
        .arg("--sheet")
        .arg("Missing")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error = stderr(&output);
    assert!(error.contains("Missing"));
    assert!(error.contains("Sales"));
    assert!(error.contains("Regions"));
    assert!(!destination.exists());
}

#[test]
fn malformed_workbooks_fail_without_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("broken.xlsx");
    let destination = dir.path().join("out.csv");
    fs::write(&input, b"this is not a zip container").unwrap();

    let output = tabmux()
        .arg("convert")
        .arg(&input)
        .arg(&destination)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("cannot open workbook"));
    assert!(!destination.exists());
}

#[test]
fn xlsx_head_and_count_use_the_selected_sheet() {
    let count = tabmux()
        .arg("count")
        .arg(fixture("sales.xlsx"))
        .arg("--sheet")
        .arg("Regions")
        .output()
        .unwrap();
    assert_success(&count);
    assert_eq!(String::from_utf8(count.stdout).unwrap(), "2\n");

    let head = tabmux()
        .arg("head")
        .arg(fixture("sales.xlsx"))
        .arg("--sheet")
        .arg("Regions")
        .arg("--rows")
        .arg("1")
        .output()
        .unwrap();
    assert_success(&head);
    assert_eq!(
        String::from_utf8(head.stdout).unwrap(),
        "region,manager\nNorth,Ada\n"
    );
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

// ---------------------------------------------------------------------------
// Phase 3: ODS read, sheet splitting/stacking, row and cell output
// ---------------------------------------------------------------------------

#[test]
fn sheets_lists_ods_workbook_sheets_in_order() {
    let output = tabmux()
        .arg("sheets")
        .arg(fixture("sales.ods"))
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Sales\nRegions\n"
    );
}

#[test]
fn ods_sheet_converts_to_csv_with_dates_merges_and_cached_values() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("sales.csv");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("sales.ods"))
        .arg(&destination)
        .arg("--sheet")
        .arg("Sales")
        .output()
        .unwrap();
    assert_success(&output);

    let text = fs::read_to_string(&destination).unwrap();
    assert_eq!(
        text.lines().next(),
        Some("id,name,region,amount,sold_on,note")
    );
    assert!(text.contains("1,Alice,North,120,2026-01-15,first order"));
    assert!(text.contains("400.5"));
    // The merged A5:B5 range is expanded into both covered cells.
    assert!(text.contains("Total,Total,,400.5,,"));

    let error = stderr(&output);
    assert!(
        !error.contains("excel.merged_cell_expanded"),
        "default convert should not print fidelity events:\n{error}"
    );
    assert!(!error.contains("excel.style_dropped"));

    let fidelity_destination = dir.path().join("sales-fidelity.csv");
    let output = tabmux()
        .arg("convert")
        .arg(fixture("sales.ods"))
        .arg(&fidelity_destination)
        .arg("--sheet")
        .arg("Sales")
        .arg("--fidelity-report")
        .output()
        .unwrap();
    assert_success(&output);

    let error = stderr(&output);
    assert!(error.contains("excel.merged_cell_expanded"));
    assert!(error.contains("excel.formula_cached_value_only"));
    assert!(error.contains("excel.style_dropped"));
}

#[test]
fn ods_split_uses_the_name_template_placeholders() {
    let dir = tempfile::tempdir().unwrap();
    let output_dir = dir.path().join("ods");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("sales.ods"))
        .arg(&output_dir)
        .arg("--all-sheets")
        .arg("--split-sheets")
        .arg("--name-template")
        .arg("{stem}.{index}.{sheet}.{ext}")
        .output()
        .unwrap();
    assert_success(&output);

    let mut names = fs::read_dir(&output_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        ["sales.1.Sales.csv", "sales.2.Regions.csv"],
        "unexpected split filenames"
    );
}

#[test]
fn split_name_collisions_get_stable_suffixes() {
    let dir = tempfile::tempdir().unwrap();
    let output_dir = dir.path().join("collide");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("sales.ods"))
        .arg(&output_dir)
        .arg("--all-sheets")
        .arg("--split-sheets")
        .arg("--name-template")
        .arg("{stem}.{format}")
        .output()
        .unwrap();
    assert_success(&output);
    assert!(stderr(&output).contains("output.sheet_name_sanitized"));

    let mut names = fs::read_dir(&output_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, ["sales-2.csv", "sales.csv"]);
}

#[test]
fn split_name_template_cannot_escape_the_output_directory() {
    let dir = tempfile::tempdir().unwrap();
    let output_dir = dir.path().join("safe");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("sales.ods"))
        .arg(&output_dir)
        .arg("--all-sheets")
        .arg("--split-sheets")
        .arg("--name-template")
        .arg("../{sheet}.csv")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("relative path"));
    assert!(!output_dir.exists());
}

#[test]
fn all_sheets_stack_concatenates_columns_and_adds_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("stacked.csv");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("sales.ods"))
        .arg(&destination)
        .arg("--all-sheets")
        .arg("--stack")
        .arg("--add-source-column")
        .arg("sheet")
        .output()
        .unwrap();
    assert_success(&output);

    let text = fs::read_to_string(&destination).unwrap();
    let lines = text.lines().collect::<Vec<_>>();
    assert_eq!(lines[0], "id,name,region,amount,sold_on,note,manager,sheet");
    assert_eq!(lines.len(), 7, "expected 6 data rows:\n{text}");
    assert!(lines.iter().any(|line| line.ends_with(",Sales")));
    assert!(lines.iter().any(|line| line.ends_with(",Regions")));
    assert!(lines[1].contains("Alice"));
    assert!(lines[5].contains("North,,,"));
}

#[test]
fn stack_requires_all_sheets() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("stacked.csv");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("sales.ods"))
        .arg(&destination)
        .arg("--stack")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(!destination.exists());
}

#[test]
fn row_by_number_prints_only_the_data_record() {
    let output = tabmux()
        .arg("row")
        .arg(fixture("sales.xlsx"))
        .arg("--sheet")
        .arg("Sales")
        .arg("--number")
        .arg("2")
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "2,Bob,South,80.5,2026-02-01,\"repeat, with comma\"\n"
    );
}

#[test]
fn row_where_matches_equality_and_inequality() {
    let output = tabmux()
        .arg("row")
        .arg(fixture("sales.xlsx"))
        .arg("--sheet")
        .arg("Sales")
        .arg("--where")
        .arg("id == 3")
        .output()
        .unwrap();
    assert_success(&output);
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .starts_with("3,Carol,North,200,2026-03-10,")
    );

    let output = tabmux()
        .arg("row")
        .arg(fixture("sales.xlsx"))
        .arg("--sheet")
        .arg("Regions")
        .arg("--where")
        .arg("manager != Ada")
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "South,Grace\n");
}

#[test]
fn row_rejects_bad_selectors_and_unknown_where_columns() {
    let output = tabmux()
        .arg("row")
        .arg(fixture("basic.csv"))
        .arg("--number")
        .arg("0")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));

    let output = tabmux()
        .arg("row")
        .arg(fixture("basic.csv"))
        .arg("--number")
        .arg("99")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("out of range"));

    let output = tabmux()
        .arg("row")
        .arg(fixture("basic.csv"))
        .arg("--where")
        .arg("missing == 1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("unknown column 'missing'"));
}

#[test]
fn cell_output_is_exact_for_multiline_quoted_data() {
    let output = tabmux()
        .arg("cell")
        .arg(fixture("wide.csv"))
        .arg("--row")
        .arg("2")
        .arg("--column")
        .arg("note")
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "contains, comma and \"quotes\"\n"
    );

    let output = tabmux()
        .arg("cell")
        .arg(fixture("wide.csv"))
        .arg("--row")
        .arg("3")
        .arg("--column")
        .arg("note")
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "trailing newline\n\n"
    );
}

#[test]
fn select_and_head_preserve_multiline_values_and_support_pretty() {
    let output = tabmux()
        .arg("select")
        .arg(fixture("wide.csv"))
        .arg("--columns")
        .arg("note")
        .output()
        .unwrap();
    assert_success(&output);
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("\"first line\nsecond line\""));
    assert!(text.contains("\"trailing newline\n\""));

    let output = tabmux()
        .arg("head")
        .arg(fixture("basic.csv"))
        .arg("--rows")
        .arg("1")
        .arg("--pretty")
        .output()
        .unwrap();
    assert_success(&output);
    let text = String::from_utf8(output.stdout).unwrap();
    let lines = text.lines().collect::<Vec<_>>();
    assert!(lines[0].contains("name"));
    assert!(lines[0].contains("|"));
    assert!(lines[1].contains("---+"));
    assert!(lines[2].starts_with("Alice"));
}

#[test]
fn cell_pretty_prints_a_header_and_value() {
    let output = tabmux()
        .arg("cell")
        .arg(fixture("basic.csv"))
        .arg("--row")
        .arg("1")
        .arg("--column")
        .arg("email")
        .arg("--pretty")
        .output()
        .unwrap();
    assert_success(&output);
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.starts_with("email"));
    assert!(text.contains("alice@example.com"));
}

#[test]
fn ods_count_head_and_sheet_index_work() {
    let count = tabmux()
        .arg("count")
        .arg(fixture("sales.ods"))
        .arg("--sheet")
        .arg("Regions")
        .output()
        .unwrap();
    assert_success(&count);
    assert_eq!(String::from_utf8(count.stdout).unwrap(), "2\n");

    let head = tabmux()
        .arg("head")
        .arg(fixture("sales.ods"))
        .arg("--sheet-index")
        .arg("2")
        .arg("--rows")
        .arg("1")
        .output()
        .unwrap();
    assert_success(&head);
    assert_eq!(
        String::from_utf8(head.stdout).unwrap(),
        "region,manager\nNorth,Ada\n"
    );
}

#[test]
fn read_only_commands_hide_workbook_fidelity_by_default() {
    let output = tabmux()
        .arg("row")
        .arg(fixture("sales.xlsx"))
        .arg("--sheet")
        .arg("Sales")
        .arg("--number")
        .arg("2")
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(stderr(&output), "");

    let output = tabmux()
        .arg("row")
        .arg(fixture("sales.xlsx"))
        .arg("--sheet")
        .arg("Sales")
        .arg("--number")
        .arg("2")
        .arg("--fidelity-report")
        .output()
        .unwrap();
    assert_success(&output);
    let error = stderr(&output);
    assert!(error.contains("excel.chart_dropped"));
    assert!(error.contains("excel.style_dropped"));
    // Row 5 is outside the selected output row, so its merge/formula events
    // stay hidden even with a fidelity report.
    assert!(!error.contains("excel.merged_cell_expanded"));
}

#[test]
fn row_where_collects_diagnostics_only_for_matched_rows() {
    let output = tabmux()
        .arg("row")
        .arg(fixture("ragged.csv"))
        .arg("--ragged")
        .arg("pad")
        .arg("--where")
        .arg("name == Bob")
        .output()
        .unwrap();
    assert_success(&output);
    let error = stderr(&output);
    assert!(error.contains("row 2"), "stderr was:\n{error}");
    assert!(!error.contains("row 3"), "stderr was:\n{error}");

    let output = tabmux()
        .arg("row")
        .arg(fixture("ragged.csv"))
        .arg("--ragged")
        .arg("pad")
        .arg("--where")
        .arg("name == Bob")
        .arg("--debug")
        .output()
        .unwrap();
    assert_success(&output);
    let error = stderr(&output);
    assert!(error.contains("row 2"), "stderr was:\n{error}");
    assert!(
        error.contains("row 3"),
        "--debug should include filtered-out rows:\n{error}"
    );
}

#[test]
fn quiet_and_none_suppress_diagnostics() {
    for flag in ["--quiet", "--diagnostics"] {
        let mut command = tabmux();
        command
            .arg("select")
            .arg(fixture("ragged.csv"))
            .arg("--ragged")
            .arg("pad")
            .arg("--columns")
            .arg("name")
            .arg(flag);
        if flag == "--diagnostics" {
            command.arg("none");
        }
        let output = command.output().unwrap();
        assert_success(&output);
        assert_eq!(stderr(&output), "", "{flag} did not suppress diagnostics");
    }
}

// ---------------------------------------------------------------------------
// Phase 4: XLSX/ODS write, multi-sheet assembly, basic formatting
// ---------------------------------------------------------------------------

use tabmux::formats::{self, FormatId, ReadOptions, SheetSelection};
use tabmux::model::{CellValue, SourceRef};

fn read_workbook(path: &Path, format: FormatId) -> Vec<tabmux::model::Table> {
    let bytes = fs::read(path).unwrap();
    let source = SourceRef::new(path, format);
    let options = ReadOptions {
        has_header: true,
        sheets: SheetSelection::All,
        ..ReadOptions::default()
    };
    formats::read_tables(&source, &bytes, format, &options)
        .unwrap()
        .tables
}

#[test]
fn multi_input_csv_builds_a_workbook_with_native_types() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("report.xlsx");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("a.csv"))
        .arg(fixture("b.csv"))
        .arg("-o")
        .arg(&destination)
        .output()
        .unwrap();
    assert_success(&output);

    let sheets = tabmux().arg("sheets").arg(&destination).output().unwrap();
    assert_success(&sheets);
    assert_eq!(String::from_utf8(sheets.stdout).unwrap(), "a\nb\n");

    let tables = read_workbook(&destination, FormatId::Xlsx);
    assert_eq!(tables.len(), 2);
    assert_eq!(tables[0].name, "a");
    assert_eq!(tables[1].name, "b");
    assert_eq!(tables[0].rows[0][0], CellValue::Int(1));
    assert_eq!(tables[0].rows[0][1], CellValue::Text("Alice".into()));
    assert_eq!(tables[0].rows[0][2], CellValue::Float(120.5));
    assert_eq!(tables[0].rows[0][3], CellValue::Bool(true));
    assert!(matches!(tables[0].rows[0][4], CellValue::Date(_)));
    assert_eq!(tables[1].rows[0][0], CellValue::Int(4));
}

#[test]
fn multi_input_csv_builds_an_ods_workbook() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("report.ods");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("a.csv"))
        .arg(fixture("b.csv"))
        .arg("-o")
        .arg(&destination)
        .output()
        .unwrap();
    assert_success(&output);

    let tables = read_workbook(&destination, FormatId::Ods);
    assert_eq!(tables.len(), 2);
    assert_eq!(tables[0].name, "a");
    assert_eq!(tables[0].rows[0][0], CellValue::Int(1));
    assert_eq!(tables[0].rows[0][2], CellValue::Float(120.5));
    assert_eq!(tables[0].rows[0][3], CellValue::Bool(true));
    assert!(matches!(tables[0].rows[0][4], CellValue::Date(_)));
}

#[test]
fn typed_csv_keeps_native_types_in_both_workbook_formats() {
    for (extension, format) in [("xlsx", FormatId::Xlsx), ("ods", FormatId::Ods)] {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join(format!("typed.{extension}"));

        let output = tabmux()
            .arg("convert")
            .arg(fixture("typed.csv"))
            .arg("-o")
            .arg(&destination)
            .arg("--sheet-name")
            .arg("Types")
            .output()
            .unwrap();
        assert_success(&output);

        let tables = read_workbook(&destination, format);
        assert_eq!(tables.len(), 1);
        let table = &tables[0];
        assert_eq!(table.name, "Types");
        assert_eq!(table.rows[0][0], CellValue::Int(1));
        assert_eq!(table.rows[0][1], CellValue::Float(1.5));
        assert_eq!(table.rows[0][2], CellValue::Bool(true));
        assert!(matches!(table.rows[0][3], CellValue::Date(_)));
        assert!(matches!(table.rows[0][4], CellValue::DateTime(_)));
        assert!(matches!(table.rows[0][5], CellValue::Time(_)));
        // A leading zero must keep the column textual.
        assert_eq!(table.rows[0][6], CellValue::Text("007".into()));
        assert_eq!(table.columns[6].ty, tabmux::model::ColumnType::Text);
    }
}

#[test]
fn null_values_round_trip_in_workbook_formats() {
    for (extension, format) in [("xlsx", FormatId::Xlsx), ("ods", FormatId::Ods)] {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join(format!("nulls.{extension}"));

        let output = tabmux()
            .arg("convert")
            .arg(fixture("nulls.csv"))
            .arg("-o")
            .arg(&destination)
            .output()
            .unwrap();
        assert_success(&output);

        let tables = read_workbook(&destination, format);
        assert_eq!(tables[0].rows[0][1], CellValue::Text("Alice".into()));
        assert_eq!(tables[0].rows[1][1], CellValue::Null);
        assert_eq!(tables[0].rows[1][2], CellValue::Null);
        assert_eq!(tables[0].rows[1][3], CellValue::Null);
        assert_eq!(tables[0].rows[1][4], CellValue::Null);
    }
}

#[test]
fn multiline_cells_round_trip_through_ods() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("wide.ods");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("wide.csv"))
        .arg("-o")
        .arg(&destination)
        .output()
        .unwrap();
    assert_success(&output);

    let original = tabmux()
        .arg("convert")
        .arg(fixture("wide.csv"))
        .arg("-")
        .output()
        .unwrap();
    let round_trip = tabmux()
        .arg("convert")
        .arg(&destination)
        .arg("-")
        .output()
        .unwrap();
    assert_success(&round_trip);
    assert_eq!(round_trip.stdout, original.stdout);
}

#[test]
fn sheet_per_input_and_name_templates_are_stable() {
    let dir = tempfile::tempdir().unwrap();

    let explicit = dir.path().join("explicit.xlsx");
    let output = tabmux()
        .arg("convert")
        .arg(fixture("a.csv"))
        .arg(fixture("b.csv"))
        .arg("-o")
        .arg(&explicit)
        .arg("--sheet-per-input")
        .output()
        .unwrap();
    assert_success(&output);
    let sheets = tabmux().arg("sheets").arg(&explicit).output().unwrap();
    assert_eq!(String::from_utf8(sheets.stdout).unwrap(), "a\nb\n");

    let templated = dir.path().join("templated.xlsx");
    let output = tabmux()
        .arg("convert")
        .arg(fixture("a.csv"))
        .arg(fixture("b.csv"))
        .arg("-o")
        .arg(&templated)
        .arg("--sheet-name-template")
        .arg("sheet-{index}")
        .output()
        .unwrap();
    assert_success(&output);
    let sheets = tabmux().arg("sheets").arg(&templated).output().unwrap();
    assert_eq!(
        String::from_utf8(sheets.stdout).unwrap(),
        "sheet-1\nsheet-2\n"
    );
}

#[test]
fn sheet_name_renames_and_sanitizes_the_single_sheet() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("renamed.xlsx");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("a.csv"))
        .arg("-o")
        .arg(&destination)
        .arg("--sheet-name")
        .arg("Quarterly/Report")
        .output()
        .unwrap();
    assert_success(&output);
    assert!(stderr(&output).contains("output.sheet_name_sanitized"));

    let sheets = tabmux().arg("sheets").arg(&destination).output().unwrap();
    assert_eq!(
        String::from_utf8(sheets.stdout).unwrap(),
        "Quarterly_Report\n"
    );
}

#[test]
fn duplicate_sheet_names_get_stable_suffixes() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("collision.xlsx");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("a.csv"))
        .arg(fixture("a.csv"))
        .arg("-o")
        .arg(&destination)
        .output()
        .unwrap();
    assert_success(&output);
    assert!(stderr(&output).contains("output.sheet_name_sanitized"));

    let sheets = tabmux().arg("sheets").arg(&destination).output().unwrap();
    assert_eq!(String::from_utf8(sheets.stdout).unwrap(), "a\na-2\n");
}

#[test]
fn all_sheets_round_trip_between_xlsx_and_ods() {
    let dir = tempfile::tempdir().unwrap();

    let xlsx = dir.path().join("sales.xlsx");
    let output = tabmux()
        .arg("convert")
        .arg(fixture("sales.ods"))
        .arg("-o")
        .arg(&xlsx)
        .arg("--all-sheets")
        .output()
        .unwrap();
    assert_success(&output);
    let generated = read_workbook(&xlsx, FormatId::Xlsx);
    let original = read_workbook(&fixture("sales.ods"), FormatId::Ods);
    assert_eq!(generated.len(), original.len());
    for (generated, original) in generated.iter().zip(original.iter()) {
        assert_eq!(generated.name, original.name);
        assert_eq!(generated.column_names(), original.column_names());
        assert_eq!(generated.rows, original.rows);
    }

    let ods = dir.path().join("sales.ods");
    let output = tabmux()
        .arg("convert")
        .arg(fixture("sales.xlsx"))
        .arg("-o")
        .arg(&ods)
        .arg("--all-sheets")
        .output()
        .unwrap();
    assert_success(&output);
    let generated = read_workbook(&ods, FormatId::Ods);
    let original = read_workbook(&fixture("sales.xlsx"), FormatId::Xlsx);
    assert_eq!(generated.len(), original.len());
    for (generated, original) in generated.iter().zip(original.iter()) {
        assert_eq!(generated.name, original.name);
        assert_eq!(generated.column_names(), original.column_names());
        assert_eq!(generated.rows, original.rows);
    }
}

#[test]
fn workbook_dry_run_writes_nothing_and_lists_sheets() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("nothing.xlsx");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("a.csv"))
        .arg(fixture("b.csv"))
        .arg("-o")
        .arg(&destination)
        .arg("--dry-run")
        .output()
        .unwrap();
    assert_success(&output);
    assert!(!destination.exists());
    let error = stderr(&output);
    assert!(error.contains("dry-run"));
    assert!(error.contains("sheet 'a'"));
    assert!(error.contains("sheet 'b'"));
    assert!(error.contains("2 sheet(s)"));
}

#[test]
fn workbook_output_validation_happens_before_writing() {
    let dir = tempfile::tempdir().unwrap();

    let csv = dir.path().join("multi.csv");
    let output = tabmux()
        .arg("convert")
        .arg(fixture("a.csv"))
        .arg(fixture("b.csv"))
        .arg("-o")
        .arg(&csv)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("multiple inputs require a workbook output"));
    assert!(!csv.exists());

    let split = dir.path().join("split.xlsx");
    let output = tabmux()
        .arg("convert")
        .arg(fixture("a.csv"))
        .arg("-o")
        .arg(&split)
        .arg("--split-sheets")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(!split.exists());

    let templated = dir.path().join("bad-template.xlsx");
    let output = tabmux()
        .arg("convert")
        .arg(fixture("a.csv"))
        .arg("-o")
        .arg(&templated)
        .arg("--sheet-name-template")
        .arg("bad-{unknown}")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("unknown placeholder"));
    assert!(!templated.exists());

    let legacy = dir.path().join("legacy.xls");
    let output = tabmux()
        .arg("convert")
        .arg(fixture("a.csv"))
        .arg("-o")
        .arg(&legacy)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    assert!(stderr(&output).contains("legacy"));
    assert!(!legacy.exists());

    fs::write(&templated, "sentinel").unwrap();
    let output = tabmux()
        .arg("convert")
        .arg(fixture("a.csv"))
        .arg("-o")
        .arg(&templated)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("already exists"));
    assert_eq!(fs::read_to_string(&templated).unwrap(), "sentinel");
}

#[test]
fn stacking_multiple_inputs_into_one_workbook_sheet() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("stacked.xlsx");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("a.csv"))
        .arg(fixture("b.csv"))
        .arg("-o")
        .arg(&destination)
        .arg("--all-sheets")
        .arg("--stack")
        .arg("--add-source-column")
        .arg("origin")
        .arg("--sheet-name")
        .arg("Merged")
        .output()
        .unwrap();
    assert_success(&output);

    let tables = read_workbook(&destination, FormatId::Xlsx);
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].name, "Merged");
    assert_eq!(tables[0].rows.len(), 5);
    assert_eq!(tables[0].column_names().last().copied(), Some("origin"));
    assert_eq!(tables[0].rows[0].last(), Some(&CellValue::Text("a".into())));
    assert_eq!(tables[0].rows[4].last(), Some(&CellValue::Text("b".into())));
}

// ---------------------------------------------------------------------------
// Phase 5: merge/concat, schema diff, and basic transforms
// ---------------------------------------------------------------------------

fn run_with_stdin(args: &[&str], input: &[u8]) -> Output {
    let mut child = tabmux()
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tabmux");
    child
        .stdin
        .as_mut()
        .expect("stdin is piped")
        .write_all(input)
        .expect("write stdin");
    child.wait_with_output().expect("wait for tabmux")
}

#[test]
fn strict_merge_concatenates_compatible_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("merged.csv");

    let output = tabmux()
        .arg("merge")
        .arg(fixture("a.csv"))
        .arg(fixture("b.csv"))
        .arg("-o")
        .arg(&destination)
        .arg("--mode")
        .arg("strict")
        .output()
        .unwrap();
    assert_success(&output);

    let text = fs::read_to_string(&destination).unwrap();
    let lines = text.lines().collect::<Vec<_>>();
    assert_eq!(lines[0], "id,name,amount,active,joined");
    assert_eq!(lines.len(), 6);
    assert!(lines[4].starts_with("4,Dave,"));
    assert!(lines[5].starts_with("5,Eve,"));
}

#[test]
fn concat_is_an_alias_for_merge() {
    let output = tabmux()
        .arg("concat")
        .arg(fixture("a.csv"))
        .arg(fixture("b.csv"))
        .arg("-o")
        .arg("-")
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(String::from_utf8(output.stdout).unwrap().lines().count(), 6);
}

#[test]
fn strict_merge_failure_reports_a_schema_diff_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("merged.csv");

    let output = tabmux()
        .arg("merge")
        .arg(fixture("a.csv"))
        .arg(fixture("c.csv"))
        .arg("-o")
        .arg(&destination)
        .arg("--mode")
        .arg("strict")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error = stderr(&output);
    assert!(error.contains("schema diff"));
    assert!(error.contains("missing column 'joined'"));
    assert!(error.contains("column order differs"));
    assert!(!destination.exists());

    let output = tabmux()
        .arg("merge")
        .arg(fixture("a.csv"))
        .arg(fixture("c.csv"))
        .arg("-o")
        .arg(&destination)
        .arg("--mode")
        .arg("strict")
        .arg("--json")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let json = String::from_utf8(output.stdout).unwrap();
    assert!(json.contains("\"missing\""));
    assert!(json.contains("\"joined\""));
    assert!(json.contains("\"order\""));
    assert!(!destination.exists());
}

#[test]
fn strict_merge_reports_type_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("merged.csv");

    let output = tabmux()
        .arg("merge")
        .arg(fixture("a.csv"))
        .arg(fixture("d.csv"))
        .arg("-o")
        .arg(&destination)
        .arg("--mode")
        .arg("strict")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error = stderr(&output);
    assert!(error.contains("type conflict on 'amount': float vs text"));
    assert!(!destination.exists());
}

#[test]
fn loose_merge_unions_columns_and_keeps_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("merged.csv");

    let output = tabmux()
        .arg("merge")
        .arg(fixture("a.csv"))
        .arg(fixture("c.csv"))
        .arg("-o")
        .arg(&destination)
        .arg("--mode")
        .arg("loose")
        .arg("--add-source-column")
        .arg("origin")
        .arg("--dedup")
        .output()
        .unwrap();
    assert_success(&output);

    let text = fs::read_to_string(&destination).unwrap();
    let lines = text.lines().collect::<Vec<_>>();
    assert_eq!(lines[0], "id,name,amount,active,joined,origin");
    assert_eq!(lines.len(), 5);
    assert!(lines[1].ends_with(",a.csv"));
    assert!(lines[4].ends_with(",c.csv"));
    assert!(lines[4].contains("6,Zoe,55,true,,c.csv"));
}

#[test]
fn merge_dedup_collapses_identical_rows_before_provenance() {
    let output = tabmux()
        .arg("merge")
        .arg(fixture("a.csv"))
        .arg(fixture("a.csv"))
        .arg("-o")
        .arg("-")
        .arg("--mode")
        .arg("loose")
        .arg("--add-source-column")
        .arg("origin")
        .arg("--dedup")
        .output()
        .unwrap();
    assert_success(&output);

    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        text.lines().count(),
        4,
        "header + three unique rows:\n{text}"
    );
    assert!(text.lines().all(|line| !line.contains("a-2")));
}

#[test]
fn case_insensitive_headers_can_make_strict_merge_succeed() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("one.csv");
    let second = dir.path().join("two.csv");
    fs::write(&first, "ID,Name\n1,Alice\n").unwrap();
    fs::write(&second, "id,name\n2,Bob\n").unwrap();
    let destination = dir.path().join("merged.csv");

    let output = tabmux()
        .arg("merge")
        .arg(&first)
        .arg(&second)
        .arg("-o")
        .arg(&destination)
        .arg("--mode")
        .arg("strict")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));

    let output = tabmux()
        .arg("merge")
        .arg(&first)
        .arg(&second)
        .arg("-o")
        .arg(&destination)
        .arg("--mode")
        .arg("strict")
        .arg("--case-insensitive-headers")
        .arg("--force")
        .output()
        .unwrap();
    assert_success(&output);
    let text = fs::read_to_string(&destination).unwrap();
    assert!(text.starts_with("ID,Name\n"));
    assert_eq!(text.lines().count(), 3);
}

#[test]
fn filter_supports_numeric_and_equality_comparisons() {
    let output = tabmux()
        .arg("filter")
        .arg(fixture("a.csv"))
        .arg("--where")
        .arg("amount > 100")
        .output()
        .unwrap();
    assert_success(&output);
    let text = String::from_utf8(output.stdout).unwrap();
    let lines = text.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 3);
    assert!(lines[1].starts_with("1,Alice,"));
    assert!(lines[2].starts_with("3,Carol,"));

    let output = tabmux()
        .arg("filter")
        .arg(fixture("a.csv"))
        .arg("--where")
        .arg("name == Bob")
        .output()
        .unwrap();
    assert_success(&output);
    assert!(String::from_utf8(output.stdout).unwrap().contains(",Bob,"));
}

#[test]
fn filter_reads_stdin_for_shell_pipelines() {
    let output = run_with_stdin(
        &["filter", "-", "--where", "amount > 100"],
        b"name,amount\nAlice,120\nBob,80\n",
    );
    assert_success(&output);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "name,amount\nAlice,120\n"
    );
}

#[test]
fn filter_reports_non_numeric_values_in_numeric_comparisons() {
    let output = tabmux()
        .arg("filter")
        .arg(fixture("d.csv"))
        .arg("--where")
        .arg("amount > 100")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error = stderr(&output);
    assert!(error.contains("not numeric"));
    assert!(error.contains("amount"));
}

#[test]
fn sort_supports_multiple_keys_and_directions() {
    let output = tabmux()
        .arg("sort")
        .arg(fixture("a.csv"))
        .arg("--by")
        .arg("amount:desc")
        .output()
        .unwrap();
    assert_success(&output);
    let text = String::from_utf8(output.stdout).unwrap();
    let names = text
        .lines()
        .skip(1)
        .map(|line| line.split(',').nth(1).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, ["Carol", "Alice", "Bob"]);

    let output = tabmux()
        .arg("sort")
        .arg(fixture("a.csv"))
        .arg("--by")
        .arg("active,name")
        .output()
        .unwrap();
    assert_success(&output);
    let names = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .skip(1)
        .map(|line| line.split(',').nth(1).unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, ["Bob", "Alice", "Carol"]);
}

#[test]
fn rename_changes_headers_and_rejects_duplicates() {
    let output = tabmux()
        .arg("rename")
        .arg(fixture("a.csv"))
        .arg("--map")
        .arg("amount=total,active=enabled")
        .output()
        .unwrap();
    assert_success(&output);
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.starts_with("id,name,total,enabled,joined\n"));
    assert!(text.contains("1,Alice,120.5,true,2026-01-15"));

    let output = tabmux()
        .arg("rename")
        .arg(fixture("a.csv"))
        .arg("--map")
        .arg("amount=id")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("duplicate column 'id'"));
}

#[test]
fn dedupe_keeps_the_first_occurrence() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("duplicates.csv");
    fs::write(&input, "id,name\n1,Alice\n2,Bob\n1,Alice\n").unwrap();

    let output = tabmux().arg("dedupe").arg(&input).output().unwrap();
    assert_success(&output);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "id,name\n1,Alice\n2,Bob\n"
    );

    let output = tabmux()
        .arg("dedupe")
        .arg(fixture("a.csv"))
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(String::from_utf8(output.stdout).unwrap().lines().count(), 4);
}

#[test]
fn merge_uses_safe_writes_and_dry_run() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("merged.csv");
    fs::write(&destination, "sentinel\n").unwrap();

    let output = tabmux()
        .arg("merge")
        .arg(fixture("a.csv"))
        .arg(fixture("b.csv"))
        .arg("-o")
        .arg(&destination)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("already exists"));
    assert_eq!(fs::read_to_string(&destination).unwrap(), "sentinel\n");

    let output = tabmux()
        .arg("merge")
        .arg(fixture("a.csv"))
        .arg(fixture("b.csv"))
        .arg("-o")
        .arg(&destination)
        .arg("--force")
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(fs::read_to_string(&destination).unwrap().lines().count(), 6);

    let dry = dir.path().join("dry.csv");
    let output = tabmux()
        .arg("merge")
        .arg(fixture("a.csv"))
        .arg(fixture("b.csv"))
        .arg("-o")
        .arg(&dry)
        .arg("--dry-run")
        .output()
        .unwrap();
    assert_success(&output);
    assert!(!dry.exists());
    assert!(stderr(&output).contains("dry-run"));
}

#[test]
fn merge_can_write_a_workbook_output() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("merged.xlsx");

    let output = tabmux()
        .arg("merge")
        .arg(fixture("a.csv"))
        .arg(fixture("b.csv"))
        .arg("-o")
        .arg(&destination)
        .arg("--mode")
        .arg("strict")
        .output()
        .unwrap();
    assert_success(&output);

    let tables = read_workbook(&destination, FormatId::Xlsx);
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].name, "a");
    assert_eq!(tables[0].rows.len(), 5);
    assert_eq!(tables[0].rows[4][0], CellValue::Int(5));
}

// ---------------------------------------------------------------------------
// Phase 6: HTML table ingestion, HTML -> workbook
// ---------------------------------------------------------------------------

#[cfg(feature = "html")]
#[test]
fn html_tables_become_sheets_with_captions_and_an_index() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("tables.xlsx");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("tables.html"))
        .arg(&destination)
        .arg("--tables")
        .arg("all")
        .arg("--sheet-per-table")
        .arg("--use-captions")
        .arg("--index-sheet")
        .output()
        .unwrap();
    assert_success(&output);

    let diagnostics = stderr(&output);
    assert!(
        diagnostics.contains("html.merged_cell_expanded"),
        "merged cells were not reported: {diagnostics}"
    );
    assert!(
        diagnostics.contains("html.header_colspan_expanded"),
        "colspan header expansion was not reported: {diagnostics}"
    );
    assert!(
        !diagnostics.contains("merged cell at table row 1 column 3"),
        "header colspan expansion was also reported as a merged cell: {diagnostics}"
    );
    assert!(
        !diagnostics.contains("html.duplicate_header"),
        "colspan expansion must not be reported as a duplicate header: {diagnostics}"
    );
    assert!(
        diagnostics.contains("html.table_skipped"),
        "the layout table was not reported as skipped: {diagnostics}"
    );
    assert!(
        diagnostics.contains("html.nested_table_extracted"),
        "the nested table was not reported: {diagnostics}"
    );
    assert!(
        diagnostics.contains("wrote 4 sheet(s)"),
        "the workbook summary was not printed: {diagnostics}"
    );
    assert!(
        diagnostics.contains("skipped table-4 (1 row(s) < --min-rows 2)"),
        "the workbook summary did not describe the skipped table: {diagnostics}"
    );

    let tables = read_workbook(&destination, FormatId::Xlsx);
    let names = tables
        .iter()
        .map(|table| table.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["_index", "Sales", "Regions", "Notes"]);

    let index = &tables[0];
    assert_eq!(
        index.column_names(),
        [
            "source", "table", "caption", "rows", "columns", "status", "reason"
        ]
    );
    assert_eq!(
        index.rows.len(),
        4,
        "index rows must include every data table and skipped table"
    );
    assert_eq!(index.rows[0][2], CellValue::Text("Sales".into()));
    assert_eq!(index.rows[0][5], CellValue::Text("included".into()));
    assert_eq!(index.rows[2][2], CellValue::Text("Notes".into()));
    assert_eq!(index.rows[2][5], CellValue::Text("included".into()));
    assert!(
        index.rows[2][6]
            .to_text("")
            .contains("nested table extracted from table-2"),
        "the nested table should explain the outer-cell marker"
    );
    assert_eq!(index.rows[3][1], CellValue::Int(4));
    assert_eq!(index.rows[3][5], CellValue::Text("skipped".into()));
    assert_eq!(
        index.rows[3][6],
        CellValue::Text("1 row(s) < --min-rows 2".into())
    );

    let sales = &tables[1];
    assert_eq!(
        sales.column_names(),
        ["id", "name", "territory", "territory_2", "amount"]
    );
    assert_eq!(
        sales.rows[0],
        vec![
            CellValue::Int(1),
            CellValue::Text("Alice".into()),
            CellValue::Text("North".into()),
            CellValue::Text("EU".into()),
            CellValue::Int(120),
        ]
    );
    assert_eq!(
        sales.rows[1],
        vec![
            CellValue::Int(2),
            CellValue::Text("Bob".into()),
            CellValue::Text("North".into()),
            CellValue::Text("US".into()),
            CellValue::Float(80.5),
        ],
        "rowspan must duplicate North and colspan must keep US in the second subcolumn"
    );

    let regions = &tables[2];
    assert_eq!(
        regions.rows[0][2],
        CellValue::Text("[table 3]".into()),
        "the outer cell should keep a marker for the nested table"
    );
}

#[cfg(feature = "html")]
#[test]
fn html_sheets_are_listed_by_caption() {
    let output = tabmux()
        .arg("sheets")
        .arg(fixture("tables.html"))
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Sales\nRegions\nNotes\n"
    );
}

#[cfg(feature = "html")]
#[test]
fn html_all_tables_can_be_stacked_with_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("stacked.csv");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("tables.html"))
        .arg(&destination)
        .arg("--tables")
        .arg("all")
        .arg("--stack")
        .arg("--add-source-column")
        .arg("table")
        .output()
        .unwrap();
    assert_success(&output);

    let text = fs::read_to_string(&destination).unwrap();
    assert_eq!(text.lines().count(), 7, "header + six data rows");
    assert!(
        text.starts_with(
            "id,name,territory,territory_2,amount,region,manager,details,note,table\n"
        )
    );
    assert!(text.contains(",table-1"));
    assert!(text.contains(",table-3"));
    assert!(
        !text.contains("Logo"),
        "layout table leaked into the output"
    );
}

#[cfg(feature = "html")]
#[test]
fn html_merge_policy_and_header_row_are_honored() {
    let first = tabmux()
        .arg("convert")
        .arg(fixture("tables.html"))
        .arg("-")
        .arg("--tables")
        .arg("1")
        .arg("--merge-policy")
        .arg("first")
        .output()
        .unwrap();
    assert_success(&first);
    let text = String::from_utf8(first.stdout).unwrap();
    assert!(text.lines().next().unwrap().contains("column_4"));
    assert!(text.contains("2,Bob,,US,80.5"));

    let error = tabmux()
        .arg("convert")
        .arg(fixture("tables.html"))
        .arg("-")
        .arg("--tables")
        .arg("1")
        .arg("--merge-policy")
        .arg("error")
        .output()
        .unwrap();
    assert_eq!(error.status.code(), Some(1));
    assert!(stderr(&error).contains("--merge-policy"));

    let explicit = tabmux()
        .arg("convert")
        .arg(fixture("header-row.html"))
        .arg("-")
        .arg("--header-row")
        .arg("2")
        .output()
        .unwrap();
    assert_success(&explicit);
    assert_eq!(
        String::from_utf8(explicit.stdout).unwrap(),
        "id,amount\n1,10\n2,20\n"
    );

    let inferred = tabmux()
        .arg("convert")
        .arg(fixture("header-row.html"))
        .arg("-")
        .output()
        .unwrap();
    assert_success(&inferred);
    assert!(
        String::from_utf8(inferred.stdout)
            .unwrap()
            .starts_with("Quarterly report,FY26\n")
    );
}

#[cfg(feature = "html")]
#[test]
fn html_sheet_per_table_requires_a_workbook_output() {
    let output = tabmux()
        .arg("convert")
        .arg(fixture("tables.html"))
        .arg("-")
        .arg("--tables")
        .arg("all")
        .arg("--sheet-per-table")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("--sheet-per-table"));
}

#[cfg(feature = "html")]
#[test]
fn html_unknown_table_lists_available_tables() {
    let output = tabmux()
        .arg("convert")
        .arg(fixture("tables.html"))
        .arg("-")
        .arg("--tables")
        .arg("Missing")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("Missing"));
    assert!(message.contains("table-1"));
}
