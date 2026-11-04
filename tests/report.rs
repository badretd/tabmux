//! Phase 9 integration tests: loss reports, provenance manifests, strict mode,
//! and output verification.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use sha2::{Digest as _, Sha256};

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

fn sha256(path: &Path) -> String {
    let digest = Sha256::digest(fs::read(path).unwrap());
    format!("{digest:x}")
}

fn toml_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).unwrap()).expect("report is valid JSON")
}

fn event_codes(report: &serde_json::Value) -> Vec<String> {
    report["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["code"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn lossless_conversion_writes_an_empty_report() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("out.csv");
    let report_path = dir.path().join("loss.json");

    let result = tabmux()
        .arg("convert")
        .arg(fixture("basic.csv"))
        .arg(&output)
        .arg("--report")
        .arg(&report_path)
        .output()
        .unwrap();
    assert_success(&result);

    let report = read_json(&report_path);
    assert_eq!(report["version"], 1);
    assert_eq!(report["events"].as_array().unwrap().len(), 0);
    assert_eq!(report["summary"]["info"], 0);
    assert_eq!(report["summary"]["warning"], 0);
    assert_eq!(report["summary"]["error"], 0);
    assert_eq!(
        report["inputs"][0]["path"],
        fixture("basic.csv").display().to_string()
    );
    assert_eq!(report["outputs"][0]["path"], output.display().to_string());
}

#[test]
fn rich_workbook_report_and_manifest_record_every_loss_and_checksum() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("rich.xlsx");
    let report_path = dir.path().join("loss.json");
    let markdown_path = dir.path().join("loss.md");
    let manifest_path = dir.path().join("manifest.json");

    let result = tabmux()
        .arg("convert")
        .arg(fixture("rich.xlsx"))
        .arg(&output)
        .arg("--all-sheets")
        .arg("--report")
        .arg(&report_path)
        .arg("--report-markdown")
        .arg(&markdown_path)
        .arg("--manifest")
        .arg(&manifest_path)
        .arg("--verify")
        .output()
        .unwrap();
    assert_success(&result);

    let report = read_json(&report_path);
    let codes = event_codes(&report);
    for code in [
        "excel.chart_dropped",
        "excel.style_dropped",
        "excel.formula_cached_value_only",
        "excel.merged_cell_expanded",
        "excel.macro_dropped",
        "excel.pivot_cache_dropped",
        "excel.pivot_values_materialized",
        "excel.comment_dropped",
        "excel.conditional_formatting_dropped",
        "excel.data_validation_dropped",
    ] {
        assert!(codes.iter().any(|found| found == code), "missing {code}");
    }
    let formula = report["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["code"] == "excel.formula_cached_value_only")
        .expect("formula event");
    assert_eq!(formula["count"], 2);
    assert!(
        formula["message"]
            .as_str()
            .unwrap()
            .contains("cached value may be stale")
    );
    assert!(report["summary"]["warning"].as_u64().unwrap() >= 7);
    assert!(report["summary"]["info"].as_u64().unwrap() >= 3);

    let pivot = tabmux()
        .arg("convert")
        .arg(&output)
        .arg("-")
        .arg("--sheet")
        .arg("Pivot")
        .output()
        .unwrap();
    assert_success(&pivot);
    assert_eq!(
        String::from_utf8(pivot.stdout).unwrap(),
        "region,amount\nNorth,200\nSouth,80.5\n"
    );

    let error = stderr(&result);
    assert!(error.contains("workbook features dropped:"));
    assert!(error.contains("charts(1)"));
    assert!(error.contains("styles(dropped)"));
    assert!(error.contains("macros(dropped)"));
    assert!(error.contains("pivot_caches(dropped)"));
    assert!(error.contains("data_validation(2)"));
    assert!(error.contains("2 formula(s) exported as cached values (may be stale)"));
    assert!(error.contains("merged range(s) expanded into covered cells"));
    assert!(error.contains("pivot values materialized as plain worksheet cells"));
    assert!(error.contains(&format!("loss report: {}", report_path.display())));
    assert!(error.contains(&format!("manifest: {}", manifest_path.display())));
    assert!(error.contains("(7 categories)"));
    assert!(error.contains("--strict would exit 5"));

    let markdown = fs::read_to_string(&markdown_path).unwrap();
    assert!(markdown.contains("excel.chart_dropped"));
    assert!(markdown.contains("## Events"));

    let manifest = read_json(&manifest_path);
    assert_eq!(manifest["version"], 1);
    assert_eq!(manifest["deterministic"], true);
    assert!(manifest["command"].as_array().unwrap().len() >= 2);
    assert_eq!(
        manifest["inputs"][0]["sha256"].as_str().unwrap(),
        sha256(&fixture("rich.xlsx"))
    );
    assert_eq!(
        manifest["outputs"][0]["sha256"].as_str().unwrap(),
        sha256(&output)
    );
    assert_eq!(
        manifest["inputs"][0]["bytes"].as_u64().unwrap(),
        fs::metadata(fixture("rich.xlsx")).unwrap().len()
    );
    assert_eq!(
        manifest["loss_summary"]["warning"].as_u64().unwrap(),
        report["summary"]["warning"].as_u64().unwrap()
    );
}

#[test]
fn csv_reports_hide_workbook_only_events_in_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("rich.csv");
    let report_path = dir.path().join("loss.json");

    let result = tabmux()
        .arg("convert")
        .arg(fixture("rich.xlsx"))
        .arg(&output)
        .arg("--sheet")
        .arg("Rich")
        .arg("--report")
        .arg(&report_path)
        .output()
        .unwrap();
    assert_success(&result);

    let error = stderr(&result);
    assert!(error.contains("formula(s) exported as cached values"));
    assert!(
        !error.contains("workbook features dropped"),
        "CSV stderr should not list workbook-only fidelity events:\n{error}"
    );
    assert!(!error.contains("charts("));
    assert!(!error.contains("styles("));

    // The canonical report still contains the workbook-only events.
    let report = read_json(&report_path);
    let codes = event_codes(&report);
    assert!(codes.iter().any(|code| code == "excel.chart_dropped"));
    assert!(codes.iter().any(|code| code == "excel.style_dropped"));
}

#[test]
fn strict_mode_writes_the_report_but_not_the_output() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("rich-strict.xlsx");
    let report_path = dir.path().join("loss-strict.json");

    let result = tabmux()
        .arg("convert")
        .arg(fixture("rich.xlsx"))
        .arg(&output)
        .arg("--all-sheets")
        .arg("--report")
        .arg(&report_path)
        .arg("--strict")
        .output()
        .unwrap();

    assert_eq!(result.status.code(), Some(5));
    assert!(!output.exists(), "strict mode wrote a data output");
    assert!(report_path.exists(), "strict mode did not write the report");
    let report = read_json(&report_path);
    assert!(
        event_codes(&report)
            .iter()
            .any(|code| code == "excel.chart_dropped")
    );
    assert!(stderr(&result).contains("no output was written"));
}

#[test]
fn strict_max_warnings_is_a_threshold() {
    let dir = tempfile::tempdir().unwrap();

    let allowed = dir.path().join("allowed.csv");
    let result = tabmux()
        .arg("convert")
        .arg(fixture("messy.csv"))
        .arg(&allowed)
        .arg("--ragged")
        .arg("pad")
        .arg("--strict")
        .arg("--max-warnings")
        .arg("2")
        .output()
        .unwrap();
    assert_success(&result);
    assert!(allowed.exists());

    let blocked = dir.path().join("blocked.csv");
    let result = tabmux()
        .arg("convert")
        .arg(fixture("messy.csv"))
        .arg(&blocked)
        .arg("--ragged")
        .arg("pad")
        .arg("--strict")
        .arg("--max-warnings")
        .arg("1")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(5));
    assert!(!blocked.exists());
    assert!(stderr(&result).contains("--max-warnings 1"));
}

#[test]
fn lossy_overrides_a_strict_pipeline_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("rich.csv");
    let manifest_path = dir.path().join("pipeline.toml");
    let body = format!(
        r#"
version = 1

[options]
strict = true

[[inputs]]
id = "rich"
path = {}
sheet = "Rich"

[[outputs]]
input = "rich"
path = {}
format = "csv"
"#,
        toml_path(&fixture("rich.xlsx")),
        toml_path(&output)
    );
    fs::write(&manifest_path, body).unwrap();

    let blocked = tabmux().arg("run").arg(&manifest_path).output().unwrap();
    assert_eq!(blocked.status.code(), Some(5));
    assert!(!output.exists());

    let allowed = tabmux()
        .arg("run")
        .arg(&manifest_path)
        .arg("--lossy")
        .output()
        .unwrap();
    assert_success(&allowed);
    assert!(output.exists());
}

#[test]
fn report_paths_cannot_collide_with_data_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("out.csv");

    let result = tabmux()
        .arg("convert")
        .arg(fixture("basic.csv"))
        .arg(&output)
        .arg("--report")
        .arg(&output)
        .output()
        .unwrap();

    assert_eq!(result.status.code(), Some(2));
    assert!(!output.exists());
    assert!(stderr(&result).contains("same path"));
}

#[test]
fn verify_understands_a_custom_null_rendering() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("data.csv");

    let result = tabmux()
        .arg("convert")
        .arg(fixture("data.jsonl"))
        .arg(&output)
        .arg("--null")
        .arg("NULL")
        .arg("--verify")
        .output()
        .unwrap();
    assert_success(&result);
    assert!(fs::read_to_string(&output).unwrap().contains("NULL"));
}

#[test]
fn verify_reports_a_successful_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let csv = dir.path().join("out.csv");
    let xlsx = dir.path().join("out.xlsx");

    let result = tabmux()
        .arg("convert")
        .arg(fixture("a.csv"))
        .arg(&csv)
        .arg("--verify")
        .output()
        .unwrap();
    assert_success(&result);

    let result = tabmux()
        .arg("convert")
        .arg(fixture("a.csv"))
        .arg(&xlsx)
        .arg("--verify")
        .output()
        .unwrap();
    assert_success(&result);
}

#[test]
fn pipeline_report_and_manifest_cover_all_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let csv = dir.path().join("pipeline.csv");
    let xlsx = dir.path().join("pipeline.xlsx");
    let report_path = dir.path().join("loss.json");
    let manifest_path = dir.path().join("manifest.json");
    let pipeline = dir.path().join("pipeline.toml");
    let body = format!(
        r#"
version = 1

[options]
force = true

[[inputs]]
id = "sales"
path = {}
sheet = "Sales"

[[steps]]
op = "select"
input = "sales"
columns = ["id", "name", "amount"]

[[outputs]]
input = "selected"
path = {}
format = "csv"

[[outputs]]
input = "selected"
path = {}
format = "xlsx"
sheet = "Filtered"
"#,
        toml_path(&fixture("sales.xlsx")),
        toml_path(&csv),
        toml_path(&xlsx)
    );
    fs::write(&pipeline, body).unwrap();

    let result = tabmux()
        .arg("run")
        .arg(&pipeline)
        .arg("--report")
        .arg(&report_path)
        .arg("--manifest")
        .arg(&manifest_path)
        .arg("--verify")
        .output()
        .unwrap();
    assert_success(&result);

    let report = read_json(&report_path);
    assert!(
        event_codes(&report)
            .iter()
            .any(|code| code == "excel.chart_dropped")
    );
    assert_eq!(report["outputs"].as_array().unwrap().len(), 2);

    let manifest = read_json(&manifest_path);
    assert_eq!(manifest["outputs"].as_array().unwrap().len(), 2);
    assert_eq!(
        manifest["outputs"][0]["sha256"].as_str().unwrap(),
        sha256(&csv)
    );
    assert_eq!(
        manifest["outputs"][1]["sha256"].as_str().unwrap(),
        sha256(&xlsx)
    );
}

#[test]
fn unterminated_quotes_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("out.csv");
    let report_path = dir.path().join("loss.json");

    let result = tabmux()
        .arg("convert")
        .arg(fixture("badquote.csv"))
        .arg(&output)
        .arg("--ragged")
        .arg("pad")
        .arg("--report")
        .arg(&report_path)
        .output()
        .unwrap();
    assert_success(&result);

    let report = read_json(&report_path);
    assert!(
        event_codes(&report)
            .iter()
            .any(|code| code == "csv.bad_quote")
    );
}

#[test]
fn nulls_rendered_as_empty_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("nulls.csv");
    let report_path = dir.path().join("loss.json");

    let result = tabmux()
        .arg("convert")
        .arg(fixture("data.jsonl"))
        .arg(&output)
        .arg("--report")
        .arg(&report_path)
        .output()
        .unwrap();
    assert_success(&result);

    let report = read_json(&report_path);
    let codes = event_codes(&report);
    assert!(
        codes
            .iter()
            .any(|code| code == "output.null_rendered_as_empty")
    );
    assert!(report["summary"]["info"].as_u64().unwrap() >= 1);
}

#[test]
fn workbook_output_is_byte_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.xlsx");
    let second = dir.path().join("second.xlsx");

    for destination in [&first, &second] {
        let result = tabmux()
            .arg("convert")
            .arg(fixture("a.csv"))
            .arg(destination)
            .output()
            .unwrap();
        assert_success(&result);
    }

    assert_eq!(
        fs::read(&first).unwrap(),
        fs::read(&second).unwrap(),
        "two workbook conversions produced different bytes"
    );
}

#[test]
fn merge_writes_a_report_and_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("merged.csv");
    let report_path = dir.path().join("loss.json");
    let manifest_path = dir.path().join("manifest.json");

    let result = tabmux()
        .arg("merge")
        .arg(fixture("a.csv"))
        .arg(fixture("b.csv"))
        .arg("-o")
        .arg(&output)
        .arg("--mode")
        .arg("strict")
        .arg("--report")
        .arg(&report_path)
        .arg("--manifest")
        .arg(&manifest_path)
        .arg("--verify")
        .output()
        .unwrap();
    assert_success(&result);

    let report = read_json(&report_path);
    assert_eq!(report["events"].as_array().unwrap().len(), 0);
    assert_eq!(report["outputs"].as_array().unwrap().len(), 1);

    let manifest = read_json(&manifest_path);
    assert_eq!(manifest["inputs"].as_array().unwrap().len(), 2);
    assert_eq!(
        manifest["outputs"][0]["sha256"].as_str().unwrap(),
        sha256(&output)
    );
}

#[test]
fn batch_report_collects_unsupported_members() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("out");
    let report_path = dir.path().join("loss.json");

    let result = tabmux()
        .arg("convert")
        .arg(fixture("batch"))
        .arg(&output)
        .arg("--recursive")
        .arg("--include")
        .arg("*.csv,*.xlsx,*.html")
        .arg("--name-template")
        .arg("{stem}.csv")
        .arg("--report")
        .arg(&report_path)
        .output()
        .unwrap();
    assert_success(&result);

    let report = read_json(&report_path);
    let codes = event_codes(&report);
    assert!(codes.iter().any(|code| code == "format.unsupported"));
    assert_eq!(report["outputs"].as_array().unwrap().len(), 4);
}
