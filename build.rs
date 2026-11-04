//! Embed a short git hash in the provenance manifest when the build environment
//! has a git checkout. The build never fails when git is unavailable.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");

    let Ok(output) = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
    else {
        return;
    };

    if !output.status.success() {
        return;
    }

    let hash = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !hash.is_empty() {
        println!("cargo:rustc-env=TABMUX_GIT={hash}");
    }
}
