//! Phase 8 integration tests: pipeline manifests, dry-run, and replay.

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

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8")
}

fn toml_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

fn write_manifest(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).expect("write manifest");
    path
}

fn sha256(path: &Path) -> String {
    let bytes = fs::read(path).unwrap();
    let digest = Sha256::digest(&bytes);
    format!("{digest:x}")
}

/// A small pipeline that selects and filters `sales.xlsx`, with outputs in a
/// caller-provided temp directory.
fn sales_pipeline(dir: &Path, force: bool) -> PathBuf {
    let csv = dir.join("sales-filtered.csv");
    let xlsx = dir.join("sales-filtered.xlsx");
    let body = format!(
        r#"
version = 1

[options]
force = {force}

[[inputs]]
id = "sales"
path = {}
sheet = "Sales"

[[steps]]
op = "select"
input = "sales"
columns = ["id", "name", "region", "amount", "sold_on"]

[[steps]]
op = "filter"
input = "selected"
expr = "amount > 100"

[[outputs]]
id = "csv"
input = "filtered"
path = {}
format = "csv"

[[outputs]]
id = "xlsx"
input = "filtered"
path = {}
format = "xlsx"
sheet = "Filtered"
"#,
        toml_path(&fixture("sales.xlsx")),
        toml_path(&csv),
        toml_path(&xlsx)
    );
    write_manifest(dir, "sales-pipeline.toml", &body)
}

#[test]
fn dry_run_prints_inputs_steps_outputs_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = sales_pipeline(dir.path(), true);
    let csv = dir.path().join("sales-filtered.csv");
    let xlsx = dir.path().join("sales-filtered.xlsx");

    let output = tabmux()
        .arg("run")
        .arg(&manifest)
        .arg("--dry-run")
        .output()
        .unwrap();
    assert_success(&output);
    assert!(!csv.exists(), "--dry-run wrote the CSV output");
    assert!(!xlsx.exists(), "--dry-run wrote the XLSX output");

    let error = stderr(&output);
    assert!(error.contains("dry-run: pipeline plan"));
    assert!(error.contains("input 'sales'"));
    assert!(error.contains("step 1 select"));
    assert!(error.contains("step 2 filter"));
    assert!(error.contains("output 'csv'"));
    assert!(error.contains("output 'xlsx'"));
    assert!(error.contains("3 rows x 5 columns"));
    assert!(error.contains("wrote nothing"));
}

#[test]
fn run_writes_csv_and_xlsx_with_stable_content() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = sales_pipeline(dir.path(), true);
    let csv = dir.path().join("sales-filtered.csv");
    let xlsx = dir.path().join("sales-filtered.xlsx");

    let output = tabmux().arg("run").arg(&manifest).output().unwrap();
    assert_success(&output);
    assert!(stderr(&output).contains("wrote 2 output(s)"));
    assert!(csv.exists());
    assert!(xlsx.exists());

    let text = fs::read_to_string(&csv).unwrap();
    let lines = text.lines().collect::<Vec<_>>();
    assert_eq!(
        lines[0], "id,name,region,amount,sold_on",
        "unexpected CSV header:\n{text}"
    );
    assert_eq!(lines.len(), 4, "header plus three filtered rows:\n{text}");
    assert!(lines[1].starts_with("1,Alice,North,120,"));
    assert!(lines[2].starts_with("3,Carol,North,200,"));
    assert!(lines[3].starts_with("Total,Total,,400.5,"));

    let sheets = tabmux().arg("sheets").arg(&xlsx).output().unwrap();
    assert_success(&sheets);
    assert_eq!(stdout(&sheets), "Filtered\n");

    let round_trip = tabmux()
        .arg("convert")
        .arg(&xlsx)
        .arg("-")
        .arg("--sheet")
        .arg("Filtered")
        .output()
        .unwrap();
    assert_success(&round_trip);
    assert_eq!(stdout(&round_trip), text);
}

#[test]
fn deterministic_replay_produces_identical_sha256() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = sales_pipeline(dir.path(), true);
    let csv = dir.path().join("sales-filtered.csv");

    assert_success(&tabmux().arg("run").arg(&manifest).output().unwrap());
    let first = sha256(&csv);
    let first_bytes = fs::read(&csv).unwrap();

    assert_success(&tabmux().arg("run").arg(&manifest).output().unwrap());
    let second = sha256(&csv);
    let second_bytes = fs::read(&csv).unwrap();

    assert_eq!(first, second, "replay changed the CSV SHA-256");
    assert_eq!(first_bytes, second_bytes, "replay changed the CSV bytes");
}

#[test]
fn save_plan_writes_a_resolved_deterministic_json_plan() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = sales_pipeline(dir.path(), true);
    let plan_a = dir.path().join("plan-a.json");
    let plan_b = dir.path().join("plan-b.json");

    let output = tabmux()
        .arg("run")
        .arg(&manifest)
        .arg("--dry-run")
        .arg("--save-plan")
        .arg(&plan_a)
        .output()
        .unwrap();
    assert_success(&output);
    assert!(plan_a.exists());
    assert!(!dir.path().join("sales-filtered.csv").exists());

    let output = tabmux()
        .arg("run")
        .arg(&manifest)
        .arg("--dry-run")
        .arg("--save-plan")
        .arg(&plan_b)
        .output()
        .unwrap();
    assert_success(&output);

    let plan: serde_json::Value = serde_json::from_slice(&fs::read(&plan_a).unwrap()).unwrap();
    assert_eq!(plan["version"], 1);
    assert_eq!(plan["dry_run"], true);
    assert_eq!(plan["manifest"], manifest.display().to_string());
    assert_eq!(plan["inputs"][0]["id"], "sales");
    assert_eq!(plan["inputs"][0]["format"], "xlsx");
    assert_eq!(plan["inputs"][0]["rows"], 4);
    assert_eq!(plan["steps"][0]["op"], "select");
    assert_eq!(plan["steps"][0]["alias"], "selected");
    assert_eq!(plan["steps"][1]["op"], "filter");
    assert_eq!(plan["steps"][1]["alias"], "filtered");
    assert_eq!(plan["outputs"][1]["format"], "xlsx");
    assert_eq!(plan["outputs"][1]["sheet"], "Filtered");
    assert!(
        plan.get("started_at").is_none(),
        "plan must not contain a timestamp"
    );

    assert_eq!(
        fs::read(&plan_a).unwrap(),
        fs::read(&plan_b).unwrap(),
        "two dry-runs produced different plans"
    );
}

#[test]
fn invalid_column_fails_before_writing_any_output() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("invalid.csv");
    let body = format!(
        r#"
version = 1

[[inputs]]
id = "sales"
path = {}
sheet = "Sales"

[[steps]]
op = "select"
input = "sales"
columns = ["id", "missing"]

[[outputs]]
id = "csv"
input = "selected"
path = {}
format = "csv"
"#,
        toml_path(&fixture("sales.xlsx")),
        toml_path(&destination)
    );
    let manifest = write_manifest(dir.path(), "invalid.toml", &body);

    let output = tabmux().arg("run").arg(&manifest).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("unknown column 'missing'"));
    assert!(stderr(&output).contains("available columns"));
    assert!(!destination.exists(), "invalid pipeline wrote an output");
}

#[test]
fn unknown_op_is_a_manifest_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("out.csv");
    let body = format!(
        r#"
version = 1

[[inputs]]
id = "a"
path = {}

[[steps]]
op = "explode"

[[outputs]]
path = {}
"#,
        toml_path(&fixture("a.csv")),
        toml_path(&destination)
    );
    let manifest = write_manifest(dir.path(), "unknown-op.toml", &body);

    let output = tabmux().arg("run").arg(&manifest).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("invalid pipeline manifest"));
    assert!(!destination.exists());
}

#[test]
fn missing_inputs_report_the_input_id_and_path() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does-not-exist.csv");
    let destination = dir.path().join("out.csv");
    let body = format!(
        r#"
version = 1

[[inputs]]
id = "missing"
path = {}

[[outputs]]
path = {}
"#,
        toml_path(&missing),
        toml_path(&destination)
    );
    let manifest = write_manifest(dir.path(), "missing.toml", &body);

    let output = tabmux().arg("run").arg(&manifest).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error = stderr(&output);
    assert!(error.contains("pipeline input 'missing'"));
    assert!(error.contains(&missing.display().to_string()));
    assert!(!destination.exists());
}

#[test]
fn strict_mode_fails_before_writing_a_lossy_workbook() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("strict.csv");
    let body = format!(
        r#"
version = 1

[options]
strict = true
force = true

[[inputs]]
id = "sales"
path = {}
sheet = "Sales"

[[outputs]]
id = "csv"
input = "sales"
path = {}
format = "csv"
"#,
        toml_path(&fixture("sales.xlsx")),
        toml_path(&destination)
    );
    let manifest = write_manifest(dir.path(), "strict.toml", &body);

    let output = tabmux().arg("run").arg(&manifest).output().unwrap();
    assert_eq!(output.status.code(), Some(5));
    let error = stderr(&output);
    assert!(error.contains("strict mode"));
    assert!(error.contains("excel.chart_dropped"));
    assert!(!destination.exists(), "strict mode wrote an output");
}

#[test]
fn no_strict_overrides_manifest_strict() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("allowed.csv");
    let body = format!(
        r#"
version = 1

[options]
strict = true

[[inputs]]
id = "sales"
path = {}
sheet = "Sales"

[[outputs]]
input = "sales"
path = {}
"#,
        toml_path(&fixture("sales.xlsx")),
        toml_path(&destination)
    );
    let manifest = write_manifest(dir.path(), "allow-strict.toml", &body);

    let output = tabmux()
        .arg("run")
        .arg(&manifest)
        .arg("--no-strict")
        .arg("--fidelity-report")
        .output()
        .unwrap();
    assert_success(&output);
    assert!(destination.exists());
    assert!(stderr(&output).contains("excel.chart_dropped"));
}

#[test]
fn no_partial_outputs_when_one_destination_already_exists() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.csv");
    let second = dir.path().join("second.csv");
    fs::write(&first, "sentinel\n").unwrap();

    let body = format!(
        r#"
version = 1

[[inputs]]
id = "a"
path = {}

[[outputs]]
path = {}
format = "csv"

[[outputs]]
path = {}
format = "csv"
"#,
        toml_path(&fixture("a.csv")),
        toml_path(&first),
        toml_path(&second)
    );
    let manifest = write_manifest(dir.path(), "no-clobber.toml", &body);

    let output = tabmux().arg("run").arg(&manifest).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("already exists"));
    assert_eq!(fs::read_to_string(&first).unwrap(), "sentinel\n");
    assert!(
        !second.exists(),
        "validation failure wrote a partial output"
    );
}

#[test]
fn output_may_not_overwrite_an_input() {
    let input = tempfile::tempdir().unwrap();
    let source = input.path().join("source.csv");
    fs::write(&source, "id\n1\n").unwrap();
    let body = format!(
        r#"
version = 1

[[inputs]]
id = "source"
path = {}

[[outputs]]
path = {}
format = "csv"
"#,
        toml_path(&source),
        toml_path(&source)
    );
    let manifest = write_manifest(input.path(), "self.toml", &body);

    let output = tabmux().arg("run").arg(&manifest).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("would overwrite input"));
    assert_eq!(fs::read_to_string(&source).unwrap(), "id\n1\n");
}

#[test]
fn concat_step_merges_inputs_and_adds_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("merged.csv");
    let body = format!(
        r#"
version = 1

[options]
force = true

[[inputs]]
id = "a"
path = {}

[[inputs]]
id = "b"
path = {}

[[steps]]
op = "concat"
input = "a"
with = "b"
mode = "loose"
add_source_column = "origin"

[[outputs]]
id = "csv"
input = "concatenated"
path = {}
format = "csv"
"#,
        toml_path(&fixture("a.csv")),
        toml_path(&fixture("b.csv")),
        toml_path(&destination)
    );
    let manifest = write_manifest(dir.path(), "merge.toml", &body);

    let output = tabmux().arg("run").arg(&manifest).output().unwrap();
    assert_success(&output);

    let text = fs::read_to_string(&destination).unwrap();
    let lines = text.lines().collect::<Vec<_>>();
    assert_eq!(lines[0], "id,name,amount,active,joined,origin");
    assert_eq!(lines.len(), 6);
    assert!(lines[1].ends_with(",a.csv"));
    assert!(lines[5].ends_with(",b.csv"));
}

#[test]
fn sort_rename_and_dedupe_steps_are_wired() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("transformed.csv");
    let body = format!(
        r#"
version = 1

[[inputs]]
id = "a"
path = {}

[[steps]]
op = "sort"
by = ["amount:desc"]

[[steps]]
op = "rename"
map = {{ amount = "total" }}

[[steps]]
op = "dedupe"

[[outputs]]
input = "deduped"
path = {}
format = "csv"
"#,
        toml_path(&fixture("a.csv")),
        toml_path(&destination)
    );
    let manifest = write_manifest(dir.path(), "transforms.toml", &body);

    let output = tabmux().arg("run").arg(&manifest).output().unwrap();
    assert_success(&output);

    let text = fs::read_to_string(&destination).unwrap();
    let lines = text.lines().collect::<Vec<_>>();
    assert_eq!(lines[0], "id,name,total,active,joined");
    assert_eq!(lines.len(), 4);
    assert!(lines[1].starts_with("3,Carol,200,"));
    assert!(lines[2].starts_with("1,Alice,120.5,"));
    assert!(lines[3].starts_with("2,Bob,80,"));
}

#[test]
fn forward_step_references_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("forward.csv");
    let body = format!(
        r#"
version = 1

[[inputs]]
id = "a"
path = {}

[[steps]]
op = "select"
input = "step-2"
columns = ["id"]

[[steps]]
op = "dedupe"

[[outputs]]
input = "step-2"
path = {}
format = "csv"
"#,
        toml_path(&fixture("a.csv")),
        toml_path(&destination)
    );
    let manifest = write_manifest(dir.path(), "forward.toml", &body);

    let output = tabmux().arg("run").arg(&manifest).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("unknown input 'step-2'"));
    assert!(!destination.exists());
}

#[test]
fn workbook_sheet_names_are_sanitized_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("sanitized.xlsx");
    let body = format!(
        r#"
version = 1

[[inputs]]
id = "a"
path = {}

[[outputs]]
input = "a"
path = {}
format = "xlsx"
sheet = "Quarterly/Report"
"#,
        toml_path(&fixture("a.csv")),
        toml_path(&destination)
    );
    let manifest = write_manifest(dir.path(), "sheet-name.toml", &body);

    let output = tabmux().arg("run").arg(&manifest).output().unwrap();
    assert_success(&output);
    assert!(stderr(&output).contains("output.sheet_name_sanitized"));

    let sheets = tabmux().arg("sheets").arg(&destination).output().unwrap();
    assert_success(&sheets);
    assert_eq!(stdout(&sheets), "Quarterly_Report\n");
}

#[test]
fn output_can_be_written_to_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let body = format!(
        r#"
version = 1

[[inputs]]
id = "a"
path = {}

[[outputs]]
input = "a"
path = "-"
format = "csv"
"#,
        toml_path(&fixture("a.csv"))
    );
    let manifest = write_manifest(dir.path(), "stdout.toml", &body);

    let output = tabmux().arg("run").arg(&manifest).output().unwrap();
    assert_success(&output);
    assert!(stdout(&output).starts_with("id,name,amount,active,joined\n"));
    assert_eq!(stdout(&output).lines().count(), 4);
}

#[test]
fn manifest_round_trips_through_toml() {
    let manifest = tabmux::pipeline::Manifest::load(&fixture("pipeline-merge.toml")).unwrap();
    let serialized = manifest.to_toml().unwrap();
    let reparsed = tabmux::pipeline::Manifest::parse(&serialized).unwrap();
    assert_eq!(manifest, reparsed);
}

#[test]
fn checked_in_pipeline_fixtures_parse() {
    for name in [
        "pipeline.toml",
        "pipeline-invalid.toml",
        "pipeline-strict.toml",
        "pipeline-merge.toml",
    ] {
        let manifest = tabmux::pipeline::Manifest::load(&fixture(name));
        assert!(manifest.is_ok(), "could not parse {name}: {manifest:?}");
    }
}
