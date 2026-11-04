//! Embed build provenance for `--version` and the provenance manifest.
//!
//! `TABMUX_GIT` is the short git hash when a checkout is available and
//! `unknown` otherwise. `TABMUX_VERSION` is the human-readable version string
//! shown by `tabmux --version`, including the commit and enabled features. The
//! build never fails when git is unavailable.

use std::process::Command;

/// Feature flags in the order they should appear in `--version`.
const FEATURES: &[(&str, &str)] = &[
    ("CORE", "core"),
    ("EXCEL", "excel"),
    ("HTML", "html"),
    ("RUST_XLSXWRITER", "rust_xlsxwriter"),
    ("SQL_DUCKDB", "sql-duckdb"),
];

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");
    for (variable, _) in FEATURES {
        println!("cargo:rerun-if-env-changed=CARGO_FEATURE_{variable}");
    }

    let git = git_hash();
    let features = enabled_features();

    println!("cargo:rustc-env=TABMUX_GIT={git}");
    println!("cargo:rustc-env=TABMUX_FEATURES={features}");
    println!(
        "cargo:rustc-env=TABMUX_VERSION={} (commit {git}; features {features})",
        env!("CARGO_PKG_VERSION")
    );
}

fn git_hash() -> String {
    let Ok(output) = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
    else {
        return "unknown".to_owned();
    };

    if !output.status.success() {
        return "unknown".to_owned();
    }

    let hash = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if hash.is_empty() {
        "unknown".to_owned()
    } else {
        hash
    }
}

fn enabled_features() -> String {
    let enabled = FEATURES
        .iter()
        .filter(|(variable, _)| std::env::var_os(format!("CARGO_FEATURE_{variable}")).is_some())
        .map(|(_, name)| *name)
        .collect::<Vec<_>>();

    if enabled.is_empty() {
        "none".to_owned()
    } else {
        enabled.join(",")
    }
}
