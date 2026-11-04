//! Shared loss-report, provenance, strict-mode, and verification plumbing.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Instant, SystemTime};

use crate::diagnostics::{DiagnosticsLevel, DiagnosticsPolicy, DiagnosticsScope};
use crate::error::{Error, Result};
use crate::formats::{FormatId, ReadOptions};
use crate::model::{LossEvent, Table};
use crate::report::{
    self, Artifact, LossReport, loss_severity_label, lossy_event_counts, strict_violation,
    validate_auxiliary_paths, verify_bytes, verify_output,
};

use super::args::ReportArgs;

/// Per-command report state. Created before any output is opened so the same
/// input artifacts, start time, and command line are used by both the loss
/// report and the provenance manifest.
pub(crate) struct ReportContext {
    pub args: ReportArgs,
    pub inputs: Vec<Artifact>,
    pub command: Vec<String>,
    pub started: Instant,
    pub started_at: SystemTime,
    pub force: bool,
}

impl ReportContext {
    pub fn new(args: &ReportArgs, inputs: Vec<Artifact>, force: bool) -> Self {
        Self::with_started(args, inputs, force, Instant::now(), SystemTime::now())
    }

    pub fn with_started(
        args: &ReportArgs,
        inputs: Vec<Artifact>,
        force: bool,
        started: Instant,
        started_at: SystemTime,
    ) -> Self {
        Self {
            args: args.clone(),
            inputs,
            command: std::env::args_os()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
            started,
            started_at,
            force,
        }
    }

    /// Auxiliary output paths that must not collide with data outputs.
    pub fn auxiliary(&self) -> Vec<(&'static str, &Path)> {
        let mut paths = Vec::new();
        if let Some(path) = &self.args.report {
            paths.push(("loss report", Path::new(path)));
        }
        if let Some(path) = &self.args.report_markdown {
            paths.push(("markdown loss report", Path::new(path)));
        }
        if let Some(path) = &self.args.manifest {
            paths.push(("provenance manifest", Path::new(path)));
        }
        paths
    }

    /// Validate auxiliary paths against protected data paths and the no-clobber
    /// policy.
    pub fn validate_paths(&self, protected: &[(&str, &Path)]) -> Result<()> {
        validate_auxiliary_paths(&self.auxiliary(), protected, self.force)
    }

    /// Whether a loss report file was requested.
    pub fn report_requested(&self) -> bool {
        self.args.report.is_some() || self.args.report_markdown.is_some()
    }

    /// When a report is requested, summary diagnostics are aggregated into a
    /// few lines instead of one line per event. `--verbose` still shows every
    /// event.
    pub fn aggregates_diagnostics(&self, diagnostics: &DiagnosticsPolicy) -> bool {
        self.report_requested() && diagnostics.level == DiagnosticsLevel::Summary
    }

    /// Print the aggregated stderr summary for a report-backed run.
    pub fn print_event_summary(
        &self,
        events: &[LossEvent],
        diagnostics: &DiagnosticsPolicy,
        scope: DiagnosticsScope,
        include_fidelity: bool,
    ) {
        if !diagnostics.enabled()
            || diagnostics.level != DiagnosticsLevel::Summary
            || !self.report_requested()
        {
            return;
        }

        let visible = events
            .iter()
            .filter(|event| diagnostics.shows(event, scope, include_fidelity))
            .cloned()
            .collect::<Vec<_>>();
        if visible.is_empty() {
            return;
        }

        let pointer = self.report_pointer();
        let mut errors = BTreeMap::<String, u64>::new();
        let mut workbook_warnings = BTreeMap::<String, u64>::new();
        let mut other_warnings = BTreeMap::<String, u64>::new();
        let mut formula_count = 0u64;
        let mut merged_count = 0u64;
        let mut pivot_values_count = 0u64;
        let mut other_infos = BTreeMap::<String, u64>::new();

        for record in report::aggregate_events(&visible) {
            let label = event_label(&record.code);
            match record.severity.as_str() {
                "error" => *errors.entry(label).or_insert(0) += record.count,
                "warning" => {
                    if crate::diagnostics::is_workbook_fidelity_event(&record.code) {
                        *workbook_warnings.entry(label).or_insert(0) += record.count;
                    } else {
                        *other_warnings.entry(label).or_insert(0) += record.count;
                    }
                }
                _ => {
                    if record.code == "excel.formula_cached_value_only" {
                        formula_count = formula_count.saturating_add(record.count);
                    } else if matches!(
                        record.code.as_str(),
                        "excel.merged_cell_expanded" | "html.merged_cell_expanded"
                    ) {
                        merged_count = merged_count.saturating_add(record.count);
                    } else if record.code == "excel.pivot_values_materialized" {
                        pivot_values_count = pivot_values_count.saturating_add(record.count);
                    } else {
                        *other_infos.entry(label).or_insert(0) += record.count;
                    }
                }
            }
        }

        if !errors.is_empty() {
            eprintln!(
                "tabmux: error: {}{}",
                format_counts(&errors, false),
                pointer_suffix(&pointer)
            );
        }
        if !workbook_warnings.is_empty() {
            eprintln!(
                "tabmux: warning: workbook features dropped: {}{}",
                format_counts(&workbook_warnings, true),
                pointer_suffix(&pointer)
            );
        }
        if !other_warnings.is_empty() {
            eprintln!(
                "tabmux: warning: data issues: {}{}",
                format_counts(&other_warnings, false),
                pointer_suffix(&pointer)
            );
        }
        if formula_count > 0 {
            eprintln!(
                "tabmux: info: {formula_count} formula(s) exported as cached values (may be stale){}",
                pointer_suffix(&pointer)
            );
        }
        if merged_count > 0 {
            eprintln!(
                "tabmux: info: {merged_count} merged range(s) expanded into covered cells{}",
                pointer_suffix(&pointer)
            );
        }
        if pivot_values_count > 0 {
            eprintln!(
                "tabmux: info: pivot values materialized as plain worksheet cells{}",
                pointer_suffix(&pointer)
            );
        }
        if !other_infos.is_empty() {
            eprintln!(
                "tabmux: info: {}{}",
                format_counts(&other_infos, false),
                pointer_suffix(&pointer)
            );
        }
    }

    /// Suffix for the post-write summary line.
    pub fn summary_suffix(&self, events: &[LossEvent]) -> String {
        let (categories, items) = lossy_event_counts(events);
        let category_word = if categories == 1 {
            "category"
        } else {
            "categories"
        };
        let item_word = if items == 1 { "item" } else { "items" };
        let mut parts = Vec::new();
        if let Some(path) = &self.args.report {
            parts.push(format!(
                "loss report: {path} ({categories} {category_word})"
            ));
        }
        if let Some(path) = &self.args.report_markdown {
            parts.push(format!("markdown report: {path}"));
        }
        if let Some(path) = &self.args.manifest {
            parts.push(format!("manifest: {path}"));
        }
        let strict_outcome = if items == 0 {
            "--strict would pass".to_owned()
        } else if self.args.strict {
            format!(
                "--strict did not block (max warnings {})",
                self.args.max_warnings
            )
        } else {
            "--strict would exit 5".to_owned()
        };
        parts.push(format!(
            "loss severity: {} ({categories} {category_word}, {items} {item_word}; {strict_outcome})",
            loss_severity_label(events)
        ));
        format!("; {}", parts.join("; "))
    }

    /// Confirm explicitly requested report files in a dry-run.
    pub fn print_dry_run_reports(&self, events: &[LossEvent], diagnostics: &DiagnosticsPolicy) {
        if !diagnostics.enabled() {
            return;
        }
        let suffix = self.dry_run_suffix(events);
        if !suffix.is_empty() {
            eprintln!("tabmux: dry-run: {suffix}");
        }
    }

    /// Suffix for a dry-run summary. Manifests are not written by a dry-run.
    pub fn dry_run_suffix(&self, events: &[LossEvent]) -> String {
        let (categories, _) = lossy_event_counts(events);
        let category_word = if categories == 1 {
            "category"
        } else {
            "categories"
        };
        let mut parts = Vec::new();
        if let Some(path) = &self.args.report {
            parts.push(format!(
                "loss report: {path} ({categories} {category_word})"
            ));
        }
        if let Some(path) = &self.args.report_markdown {
            parts.push(format!("markdown report: {path}"));
        }
        parts.join("; ")
    }

    fn report_pointer(&self) -> Option<String> {
        self.args
            .report
            .clone()
            .or_else(|| self.args.report_markdown.clone())
    }

    /// Write the JSON/Markdown loss report, if either path was requested.
    pub fn write_loss(&self, events: &[LossEvent], outputs: &[Artifact]) -> Result<()> {
        if self.args.report.is_none() && self.args.report_markdown.is_none() {
            return Ok(());
        }
        let report = LossReport::build(self.inputs.clone(), outputs.to_vec(), events);
        report::write_loss_reports(
            &report,
            self.args.report.as_deref(),
            self.args.report_markdown.as_deref(),
            self.force,
        )
    }

    /// Apply the strict-mode policy. A blocked conversion still writes the loss
    /// report (when requested) so the user can see why it was blocked, but no
    /// data output is created.
    pub fn check_strict(&self, events: &[LossEvent]) -> Result<()> {
        if !self.args.strict {
            return Ok(());
        }

        if let Some(reason) = strict_violation(events, self.args.max_warnings) {
            self.write_loss(events, &[])?;
            let mut message = format!(
                "{reason}; no output was written (use --lossy or raise --max-warnings to allow them)"
            );
            if let Some(path) = &self.args.report {
                message.push_str(&format!("; loss report: {path}"));
            } else if let Some(path) = &self.args.report_markdown {
                message.push_str(&format!("; loss report: {path}"));
            }
            return Err(Error::LossyBlocked(message));
        }

        Ok(())
    }

    /// Write the provenance manifest when `--manifest` was requested.
    pub fn write_manifest(
        &self,
        events: &[LossEvent],
        outputs: &[Artifact],
        options: serde_json::Value,
    ) -> Result<()> {
        let Some(path) = &self.args.manifest else {
            return Ok(());
        };
        report::write_manifest(
            path,
            self.started_at,
            self.started.elapsed(),
            self.command.clone(),
            options,
            &self.inputs,
            outputs,
            events,
            self.force,
        )?;
        Ok(())
    }

    /// Verify a file output when `--verify` was requested.
    pub fn verify_file(
        &self,
        path: &Path,
        format: FormatId,
        expected: &[Table],
        read_options: &ReadOptions,
        expected_null: &str,
    ) -> Result<()> {
        if !self.args.verify {
            return Ok(());
        }
        verify_output(path, format, expected, read_options, expected_null)
    }

    /// Verify an in-memory output when `--verify` was requested.
    pub fn verify_bytes(
        &self,
        label: &str,
        format: FormatId,
        expected: &[Table],
        read_options: &ReadOptions,
        bytes: &[u8],
        expected_null: &str,
    ) -> Result<()> {
        if !self.args.verify {
            return Ok(());
        }
        verify_bytes(label, format, expected, read_options, bytes, expected_null)
    }
}

fn event_label(code: &str) -> String {
    match code {
        "excel.chart_dropped" => "charts",
        "excel.style_dropped" => "styles",
        "excel.macro_dropped" => "macros",
        "excel.pivot_cache_dropped" => "pivot_caches",
        "excel.pivot_values_materialized" => "pivot_values",
        "excel.comment_dropped" => "comments",
        "excel.conditional_formatting_dropped" => "conditional_formatting",
        "excel.data_validation_dropped" => "data_validation",
        "excel.merged_cell_expanded" => "merged_cells",
        "excel.formula_cached_value_only" => "formula_cached_values",
        "csv.ragged_row" => "ragged_rows",
        "csv.bad_quote" => "bad_quotes",
        "csv.encoding_replaced" => "encoding_replacements",
        "json.flattened" => "flattened_objects",
        "html.table_skipped" => "skipped_tables",
        "html.merged_cell_expanded" => "merged_cells",
        "html.header_inferred" => "inferred_headers",
        "html.sheet_name_collision" => "sheet_name_collisions",
        "format.unsupported" => "unsupported_files",
        "output.sheet_name_sanitized" => "sanitized_names",
        "output.null_rendered_as_empty" => "null_cells",
        "output.formula_injection" => "formula_injection",
        "excel.hidden_sheet_read" => "hidden_sheets",
        other => return other.to_owned(),
    }
    .to_owned()
}

fn workbook_label_order(label: &str) -> u8 {
    match label {
        "charts" => 0,
        "styles" => 1,
        "macros" => 2,
        "pivot_caches" => 3,
        "pivot_values" => 4,
        "comments" => 5,
        "conditional_formatting" => 6,
        "data_validation" => 7,
        "merged_cells" => 8,
        _ => 100,
    }
}

fn format_counts(counts: &BTreeMap<String, u64>, workbook_order: bool) -> String {
    let mut entries = counts.iter().collect::<Vec<_>>();
    if workbook_order {
        entries.sort_by_key(|(label, _)| (workbook_label_order(label), label.as_str()));
    }
    entries
        .iter()
        .map(|(label, count)| {
            if workbook_order && is_global_workbook_label(label) {
                format!("{label}(dropped)")
            } else {
                format!("{label}({count})")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn is_global_workbook_label(label: &str) -> bool {
    matches!(label, "styles" | "macros" | "pivot_caches")
}

fn pointer_suffix(pointer: &Option<String>) -> String {
    pointer
        .as_ref()
        .map(|path| format!("; see {path}"))
        .unwrap_or_default()
}

/// Collect all warnings from a set of tables in deterministic table order.
pub(super) fn table_events(tables: &[Table]) -> Vec<LossEvent> {
    tables
        .iter()
        .flat_map(|table| table.warnings.iter().cloned())
        .collect()
}

/// Build input artifacts for ordinary file inputs. Stdin has no stable
/// checksum, so it is omitted from the manifest inputs.
pub(super) fn input_artifacts(inputs: &[String]) -> Result<Vec<Artifact>> {
    let mut artifacts = Vec::new();
    for input in inputs {
        if input == "-" {
            continue;
        }
        artifacts.push(Artifact::from_file(Path::new(input))?);
    }
    Ok(artifacts)
}

/// Build an output artifact after a successful write.
pub(super) fn output_artifact(path: &Path, format: FormatId) -> Result<Artifact> {
    Artifact::from_file_as(path, format)
}
