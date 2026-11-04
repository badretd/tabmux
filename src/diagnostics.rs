//! Diagnostic visibility policy.
//!
//! Read-only commands should not drown the user in source-fidelity chatter.
//! By default they show only diagnostics that affect the data being displayed;
//! workbook-fidelity events require `--fidelity-report` (or a workbook output).

use clap::ValueEnum;

use crate::model::{LossEvent, Severity};

/// How much diagnostic detail to print.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DiagnosticsLevel {
    /// Only warnings/errors that affect the displayed or written data.
    Summary,
    /// Every event, including information and source-fidelity events.
    Full,
    /// Suppress diagnostics entirely.
    None,
}

/// Whether the command is reading for display or writing an output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticsScope {
    ReadOnly,
    Write,
}

/// Resolved diagnostic visibility for one command invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticsPolicy {
    pub level: DiagnosticsLevel,
    /// Include per-row events for rows that were filtered out of the output.
    pub debug: bool,
    /// Show workbook-fidelity events even outside a workbook write.
    pub fidelity_report: bool,
}

impl DiagnosticsPolicy {
    /// Resolve `--diagnostics`, `--quiet`, `--verbose`, and `--debug`.
    pub fn resolve(
        level: Option<DiagnosticsLevel>,
        quiet: bool,
        verbose: bool,
        debug: bool,
        fidelity_report: bool,
    ) -> Self {
        let level = if quiet {
            DiagnosticsLevel::None
        } else if verbose {
            DiagnosticsLevel::Full
        } else {
            level.unwrap_or(if debug {
                DiagnosticsLevel::Full
            } else {
                DiagnosticsLevel::Summary
            })
        };

        Self {
            level,
            debug,
            fidelity_report,
        }
    }

    /// Decide whether an event should be printed in this context.
    ///
    /// `include_fidelity` is true when the command writes a workbook or the
    /// user explicitly asked for a fidelity report.
    pub fn shows(
        &self,
        event: &LossEvent,
        scope: DiagnosticsScope,
        include_fidelity: bool,
    ) -> bool {
        match self.level {
            DiagnosticsLevel::None => false,
            DiagnosticsLevel::Full => true,
            DiagnosticsLevel::Summary => {
                if is_fidelity_event(event.code) {
                    return include_fidelity;
                }
                match event.severity {
                    Severity::Error | Severity::Warning => true,
                    // Information about output naming/formatting is useful when
                    // writing, but is noise for a read-only inspection.
                    Severity::Info => scope == DiagnosticsScope::Write,
                }
            }
        }
    }
}

/// Source-fidelity events describe workbook features that cannot survive a
/// conversion. They are reported when writing a workbook or on request.
pub fn is_fidelity_event(code: &str) -> bool {
    matches!(
        code,
        "excel.chart_dropped"
            | "excel.style_dropped"
            | "excel.formula_cached_value_only"
            | "excel.merged_cell_expanded"
            | "excel.merged_cell_unsupported"
            | "excel.macro_dropped"
            | "excel.pivot_cache_dropped"
            | "excel.comment_dropped"
            | "excel.conditional_formatting_dropped"
            | "excel.data_validation_dropped"
            | "html.merged_cell_expanded"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(code: &'static str, severity: Severity) -> LossEvent {
        LossEvent::new(severity, code, "test event")
    }

    #[test]
    fn summary_hides_fidelity_and_info_for_read_only_commands() {
        let policy = DiagnosticsPolicy::resolve(None, false, false, false, false);
        assert!(!policy.shows(
            &event("excel.chart_dropped", Severity::Warning),
            DiagnosticsScope::ReadOnly,
            false
        ));
        assert!(!policy.shows(
            &event("excel.merged_cell_expanded", Severity::Info),
            DiagnosticsScope::ReadOnly,
            false
        ));
        assert!(policy.shows(
            &event("csv.ragged_row", Severity::Warning),
            DiagnosticsScope::ReadOnly,
            false
        ));
        assert!(policy.shows(
            &event("output.sheet_name_sanitized", Severity::Info),
            DiagnosticsScope::Write,
            false
        ));
    }

    #[test]
    fn fidelity_report_restores_fidelity_events() {
        let policy = DiagnosticsPolicy::resolve(None, false, false, false, true);
        assert!(policy.shows(
            &event("excel.chart_dropped", Severity::Warning),
            DiagnosticsScope::ReadOnly,
            true
        ));
    }

    #[test]
    fn quiet_suppresses_everything_and_verbose_shows_everything() {
        let quiet = DiagnosticsPolicy::resolve(None, true, false, false, false);
        assert!(!quiet.shows(
            &event("csv.ragged_row", Severity::Warning),
            DiagnosticsScope::ReadOnly,
            false
        ));

        let debug = DiagnosticsPolicy::resolve(None, false, false, true, false);
        assert_eq!(debug.level, DiagnosticsLevel::Full);
        assert!(debug.debug);
        assert!(debug.shows(
            &event("excel.chart_dropped", Severity::Warning),
            DiagnosticsScope::ReadOnly,
            false
        ));
    }
}
