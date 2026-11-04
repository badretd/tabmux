//! Phase 7 integration tests: directory and ZIP batch ingestion.

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

fn paths_under(root: &Path) -> Vec<String> {
    let mut paths = Vec::new();
    fn walk(root: &Path, current: &Path, paths: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(current) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, paths);
            } else {
                paths.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    walk(root, root, &mut paths);
    paths.sort();
    paths
}

#[cfg(all(feature = "excel", feature = "html"))]
#[test]
fn directory_batch_converts_supported_files_and_reports_skips() {
    let dir = tempfile::tempdir().unwrap();
    let output_dir = dir.path().join("out");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("batch"))
        .arg(&output_dir)
        .arg("--recursive")
        .arg("--include")
        .arg("*.csv,*.xlsx,*.html")
        .arg("--name-template")
        .arg("{stem}.csv")
        .output()
        .unwrap();
    assert_success(&output);

    assert_eq!(
        paths_under(&output_dir),
        ["book.csv", "nested/regions.csv", "page.csv", "sales.csv"]
    );
    assert!(
        fs::read_to_string(output_dir.join("sales.csv"))
            .unwrap()
            .starts_with("name,email,amount\n")
    );

    let error = stderr(&output);
    assert!(error.contains("notes.txt"), "stderr was:\n{error}");
    assert!(error.contains("format.unsupported"), "stderr was:\n{error}");
    assert!(
        error.contains("wrote 4 output(s) from 4 supported input(s); skipped 1"),
        "stderr was:\n{error}"
    );
}

#[test]
fn directory_batch_is_deterministic_and_preserves_relative_paths() {
    let input = tempfile::tempdir().unwrap();
    fs::write(input.path().join("b.csv"), "id,value\n2,b\n").unwrap();
    fs::write(input.path().join("a.csv"), "id,value\n1,a\n").unwrap();
    fs::create_dir_all(input.path().join("nested")).unwrap();
    fs::write(input.path().join("nested").join("c.csv"), "id,value\n3,c\n").unwrap();

    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();

    let output = tabmux()
        .arg("convert")
        .arg(input.path())
        .arg(first.path())
        .arg("--recursive")
        .output()
        .unwrap();
    assert_success(&output);
    let output = tabmux()
        .arg("convert")
        .arg(input.path())
        .arg(second.path())
        .arg("--recursive")
        .output()
        .unwrap();
    assert_success(&output);

    assert_eq!(
        paths_under(first.path()),
        ["a.csv", "b.csv", "nested/c.csv"]
    );
    assert_eq!(
        paths_under(first.path()),
        paths_under(second.path()),
        "two runs must produce the same file set"
    );
    for relative in paths_under(first.path()) {
        assert_eq!(
            fs::read(first.path().join(&relative)).unwrap(),
            fs::read(second.path().join(&relative)).unwrap(),
            "output differed for {relative}"
        );
    }
}

#[test]
fn non_recursive_batch_only_visits_the_top_level() {
    let input = tempfile::tempdir().unwrap();
    fs::write(input.path().join("top.csv"), "id\n1\n").unwrap();
    fs::create_dir_all(input.path().join("nested")).unwrap();
    fs::write(input.path().join("nested").join("inner.csv"), "id\n2\n").unwrap();

    let flat = tempfile::tempdir().unwrap();
    let output = tabmux()
        .arg("convert")
        .arg(input.path())
        .arg(flat.path())
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(paths_under(flat.path()), ["top.csv"]);

    let recursive = tempfile::tempdir().unwrap();
    let output = tabmux()
        .arg("convert")
        .arg(input.path())
        .arg(recursive.path())
        .arg("--recursive")
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(
        paths_under(recursive.path()),
        ["nested/inner.csv", "top.csv"]
    );
}

#[test]
fn include_exclude_filter_supported_batch_files() {
    let input = tempfile::tempdir().unwrap();
    fs::write(input.path().join("a.csv"), "id\n1\n").unwrap();
    fs::write(input.path().join("b.csv"), "id\n2\n").unwrap();
    fs::write(input.path().join("notes.txt"), "ignored\n").unwrap();

    let output_dir = tempfile::tempdir().unwrap();
    let output = tabmux()
        .arg("convert")
        .arg(input.path())
        .arg(output_dir.path())
        .arg("--include")
        .arg("*.csv")
        .arg("--exclude")
        .arg("b.csv")
        .output()
        .unwrap();
    assert_success(&output);

    assert_eq!(paths_under(output_dir.path()), ["a.csv"]);
    let error = stderr(&output);
    assert!(error.contains("format.unsupported"));
    assert!(error.contains("notes.txt"));
}

#[test]
fn flatten_drops_subdirectories_and_resolves_collisions() {
    let input = tempfile::tempdir().unwrap();
    fs::write(input.path().join("a.csv"), "id,value\n1,root\n").unwrap();
    fs::create_dir_all(input.path().join("nested")).unwrap();
    fs::write(
        input.path().join("nested").join("a.csv"),
        "id,value\n2,nested\n",
    )
    .unwrap();

    let output_dir = tempfile::tempdir().unwrap();
    let output = tabmux()
        .arg("convert")
        .arg(input.path())
        .arg(output_dir.path())
        .arg("--recursive")
        .arg("--flatten")
        .output()
        .unwrap();
    assert_success(&output);

    assert_eq!(paths_under(output_dir.path()), ["a-2.csv", "a.csv"]);
    assert!(stderr(&output).contains("output.sheet_name_sanitized"));
}

#[test]
fn strict_batch_failure_writes_nothing() {
    let input = tempfile::tempdir().unwrap();
    fs::write(input.path().join("a.csv"), "id\n1\n").unwrap();
    fs::write(input.path().join("notes.txt"), "ignored\n").unwrap();
    let output_dir = input.path().join("out");

    let output = tabmux()
        .arg("convert")
        .arg(input.path())
        .arg(&output_dir)
        .arg("--strict")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(5));
    assert!(!output_dir.exists(), "--strict created an output directory");
    let error = stderr(&output);
    assert!(error.contains("format.unsupported"));
    assert!(error.contains("no output was written"));
}

#[test]
fn dry_run_lists_the_plan_and_writes_nothing() {
    let input = tempfile::tempdir().unwrap();
    fs::write(input.path().join("b.csv"), "id\n2\n").unwrap();
    fs::write(input.path().join("a.csv"), "id\n1\n").unwrap();
    let output_dir = input.path().join("out");

    let output = tabmux()
        .arg("convert")
        .arg(input.path())
        .arg(&output_dir)
        .arg("--dry-run")
        .output()
        .unwrap();
    assert_success(&output);
    assert!(
        !output_dir.exists(),
        "--dry-run created the output directory"
    );

    let error = stderr(&output);
    assert!(error.contains("would write"));
    assert!(error.contains("would write 2 output(s) from 2 supported input(s)"));
    let first = error.find("a.csv").unwrap();
    let second = error.find("b.csv").unwrap();
    assert!(
        first < second,
        "dry-run plan was not deterministic:\n{error}"
    );
}

#[cfg(all(feature = "excel", feature = "html"))]
#[test]
fn zip_batch_converts_members_and_preserves_paths() {
    let dir = tempfile::tempdir().unwrap();
    let output_dir = dir.path().join("out");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("archive.zip"))
        .arg(&output_dir)
        .arg("--recursive")
        .output()
        .unwrap();
    assert_success(&output);

    assert_eq!(
        paths_under(&output_dir),
        ["book.csv", "nested/regions.csv", "page.csv", "sales.csv"]
    );
    let error = stderr(&output);
    assert!(error.contains("archive.zip::notes.txt"));
    assert!(error.contains("format.unsupported"));
}

#[test]
fn zip_path_traversal_is_rejected_before_writing() {
    let dir = tempfile::tempdir().unwrap();
    let output_dir = dir.path().join("out");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("traversal.zip"))
        .arg(&output_dir)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(!output_dir.exists(), "traversal archive created output");
    assert!(stderr(&output).contains("escapes the archive root"));
}

#[test]
fn zip_bomb_ratio_limit_is_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let output_dir = dir.path().join("out");

    let output = tabmux()
        .arg("convert")
        .arg(fixture("bomb.zip"))
        .arg(&output_dir)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(!output_dir.exists(), "bomb archive created output");
    let error = stderr(&output);
    assert!(error.contains("compression ratio"), "stderr was:\n{error}");
    assert!(error.contains("--max-compression-ratio"));
}

#[test]
fn archive_entry_and_size_limits_can_be_lowered() {
    let dir = tempfile::tempdir().unwrap();

    let output = tabmux()
        .arg("convert")
        .arg(fixture("archive.zip"))
        .arg(dir.path().join("entries"))
        .arg("--max-archive-entries")
        .arg("1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("--max-archive-entries"));

    let output = tabmux()
        .arg("convert")
        .arg(fixture("archive.zip"))
        .arg(dir.path().join("size"))
        .arg("--max-uncompressed-size")
        .arg("16")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("--max-uncompressed-size"));
}

#[test]
fn batch_name_template_cannot_escape_the_output_directory() {
    let input = tempfile::tempdir().unwrap();
    fs::write(input.path().join("a.csv"), "id\n1\n").unwrap();
    let output_dir = input.path().join("out");

    let output = tabmux()
        .arg("convert")
        .arg(input.path())
        .arg(&output_dir)
        .arg("--name-template")
        .arg("../escaped-{stem}.csv")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(!output_dir.exists());
    assert!(stderr(&output).contains("relative path inside the output directory"));
}

#[test]
fn batch_output_cannot_live_inside_an_existing_input_directory() {
    let input = tempfile::tempdir().unwrap();
    fs::write(input.path().join("a.csv"), "id\n1\n").unwrap();
    let output_dir = input.path().join("out");
    fs::create_dir_all(&output_dir).unwrap();

    let output = tabmux()
        .arg("convert")
        .arg(input.path())
        .arg(&output_dir)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("must not be inside the batch input directory"));
}

#[test]
fn batch_output_must_be_a_directory_or_stdout_is_rejected() {
    let input = tempfile::tempdir().unwrap();
    fs::write(input.path().join("a.csv"), "id\n1\n").unwrap();

    let stdout_output = tabmux()
        .arg("convert")
        .arg(input.path())
        .arg("-")
        .output()
        .unwrap();
    assert_eq!(stdout_output.status.code(), Some(2));
    assert!(stderr(&stdout_output).contains("requires an output directory"));

    let file_output = input.path().join("out.csv");
    fs::write(&file_output, "not a directory\n").unwrap();
    let output = tabmux()
        .arg("convert")
        .arg(input.path())
        .arg(&file_output)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("must be a directory"));
}
