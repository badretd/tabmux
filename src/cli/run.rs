//! `tabmux run` — execute a reproducible pipeline manifest.

use std::path::Path;

use crate::diagnostics::DiagnosticsPolicy;
use crate::error::Result;
use crate::pipeline::{self, Manifest};

use super::args::RunArgs;

/// Load a manifest and execute it with the CLI overrides.
pub(super) fn run_pipeline(args: RunArgs, diagnostics: DiagnosticsPolicy) -> Result<()> {
    let path = Path::new(&args.manifest);
    let manifest = Manifest::load(path)?;
    pipeline::run(&manifest, path, &args.settings(), &diagnostics)
}
