//! Loss reports and provenance manifests.
//!
//! Every conversion builds a [`LossReport`], even when it is lossless. The
//! report is a versioned, machine-readable document with stable event codes,
//! aggregated counts, and the first source reference for each repeated event.
//! A [`ProvenanceManifest`] records the tool version, command, options, input
//! and output checksums, duration, and the loss summary.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use sha2::{Digest as _, Sha256};

use crate::error::{Error, Result};
use crate::formats::{self, FormatId, ReadOptions, SheetSelection};
use crate::io::SafeWriter;
use crate::model::{LossEvent, Severity, SourceRef, Table};

/// Version of the loss report document.
pub const REPORT_VERSION: u32 = 1;
/// Version of the provenance manifest document.
pub const MANIFEST_VERSION: u32 = 1;

/// Tool identity embedded in reports and manifests.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ToolInfo {
    pub name: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git: Option<String>,
}

impl ToolInfo {
    pub fn current() -> Self {
        Self {
            name: "tabmux".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            git: option_env!("TABMUX_GIT")
                .filter(|git| !git.is_empty())
                .map(str::to_owned),
        }
    }
}

/// One file or stream that participated in a conversion.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Artifact {
    pub path: String,
    pub format: String,
    pub bytes: u64,
    pub sha256: String,
}

impl Artifact {
    /// Build an artifact from bytes that are already in memory.
    pub fn from_bytes(path: impl Into<String>, format: FormatId, bytes: &[u8]) -> Self {
        Self {
            path: path.into(),
            format: format.label().to_owned(),
            bytes: bytes.len() as u64,
            sha256: sha256_hex(bytes),
        }
    }

    /// Read a file and build its artifact. The format is detected from the
    /// extension, or reported as `unknown`.
    pub fn from_file(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        let format = formats::detect_path(path).unwrap_or(FormatId::Unknown);
        Ok(Self::from_bytes(path.display().to_string(), format, &bytes))
    }

    /// Read a file and build an artifact with an explicit format.
    pub fn from_file_as(path: &Path, format: FormatId) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        Ok(Self::from_bytes(path.display().to_string(), format, &bytes))
    }
}

/// A source location in a report. Optional fields are omitted when absent.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SourceRecord {
    pub path: String,
    pub format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sheet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nested_in: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell: Option<String>,
}

impl From<&SourceRef> for SourceRecord {
    fn from(source: &SourceRef) -> Self {
        Self {
            path: source.path.display().to_string(),
            format: source.format.label().to_owned(),
            sheet: source.sheet.clone(),
            table: source.table.clone(),
            caption: source.caption.clone(),
            nested_in: source.nested_in.clone(),
            row: source.row,
            column: source.column.clone(),
            cell: source.cell.clone(),
        }
    }
}

/// One aggregated loss event in a report.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EventRecord {
    pub severity: String,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceRecord>,
    pub count: u64,
}

/// Event counts by severity. Counts are occurrences (the sum of aggregated
/// event counts), not the number of distinct event records.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
pub struct LossSummary {
    pub info: u64,
    pub warning: u64,
    pub error: u64,
}

/// The versioned loss report written by `--report`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LossReport {
    pub version: u32,
    pub tool: ToolInfo,
    pub inputs: Vec<Artifact>,
    pub outputs: Vec<Artifact>,
    pub events: Vec<EventRecord>,
    pub summary: LossSummary,
}

impl LossReport {
    /// Build a report, aggregating repeated events by severity, code, and
    /// message while keeping the first source reference.
    pub fn build(inputs: Vec<Artifact>, outputs: Vec<Artifact>, events: &[LossEvent]) -> Self {
        let records = aggregate_events(events);
        let summary = summarize(&records);
        Self {
            version: REPORT_VERSION,
            tool: ToolInfo::current(),
            inputs,
            outputs,
            events: records,
            summary,
        }
    }

    /// Whether the report contains no events at all.
    pub fn is_lossless(&self) -> bool {
        self.events.is_empty()
    }
}

/// Aggregate repeated events and sort them deterministically.
///
/// The key is `(severity, code, message)`. The first source reference wins, and
/// subsequent occurrences only increase `count`.
pub fn aggregate_events(events: &[LossEvent]) -> Vec<EventRecord> {
    let mut records: Vec<EventRecord> = Vec::new();

    for event in events {
        let severity = event.severity.as_str().to_owned();
        if let Some(existing) = records.iter_mut().find(|record| {
            record.severity == severity
                && record.code == event.code
                && record.message == event.message
        }) {
            existing.count = existing.count.saturating_add(event.count);
            continue;
        }

        records.push(EventRecord {
            severity,
            code: event.code.to_owned(),
            message: event.message.clone(),
            source: event.source.as_ref().map(SourceRecord::from),
            count: event.count,
        });
    }

    records.sort_by(|left, right| {
        severity_rank(&left.severity)
            .cmp(&severity_rank(&right.severity))
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.message.cmp(&right.message))
            .then_with(|| source_key(&left.source).cmp(&source_key(&right.source)))
    });

    records
}

fn summarize(records: &[EventRecord]) -> LossSummary {
    let mut summary = LossSummary::default();
    for record in records {
        match record.severity.as_str() {
            "error" => summary.error = summary.error.saturating_add(record.count),
            "warning" => summary.warning = summary.warning.saturating_add(record.count),
            _ => summary.info = summary.info.saturating_add(record.count),
        }
    }
    summary
}

fn severity_rank(severity: &str) -> u8 {
    match severity {
        "error" => 0,
        "warning" => 1,
        _ => 2,
    }
}

fn source_key(source: &Option<SourceRecord>) -> String {
    source
        .as_ref()
        .map(|source| {
            format!(
                "{}\u{1e}{}\u{1e}{}\u{1e}{}\u{1e}{}",
                source.path,
                source.sheet.as_deref().unwrap_or_default(),
                source.table.as_deref().unwrap_or_default(),
                source.row.map(|row| row.to_string()).unwrap_or_default(),
                source.column.as_deref().unwrap_or_default()
            )
        })
        .unwrap_or_default()
}

/// Report null cells that are rendered as empty text in delimited output.
///
/// This is an informational event: the value is intentionally empty in the
/// target format, but the report still records that it happened.
pub fn null_rendering_events(
    tables: &[&Table],
    format: FormatId,
    null_text: &str,
) -> Vec<LossEvent> {
    if !matches!(format, FormatId::Csv | FormatId::Tsv) || !null_text.is_empty() {
        return Vec::new();
    }

    let mut count = 0u64;
    let mut source = None;
    for table in tables {
        for row in &table.rows {
            for cell in row {
                if cell.is_null() {
                    count = count.saturating_add(1);
                    source.get_or_insert_with(|| table.source.clone());
                }
            }
        }
    }

    let Some(source) = source else {
        return Vec::new();
    };

    vec![
        LossEvent::new(
            Severity::Info,
            "output.null_rendered_as_empty",
            format!("{count} null cell(s) were rendered as empty text"),
        )
        .with_source(source)
        .with_count(count),
    ]
}

/// Append a null-rendering info event to a table when applicable.
pub fn push_null_rendering_warning(table: &mut Table, format: FormatId, null_text: &str) {
    let events = null_rendering_events(&[table], format, null_text);
    table.warnings.extend(events);
}

/// Categories that describe normal, non-lossy transformations and therefore do
/// not fail `--strict`. They are still reported, but they are not counted as
/// lossy items for the strict threshold.
pub fn is_strict_exempt(code: &str) -> bool {
    matches!(
        code,
        "excel.pivot_values_materialized"
            | "excel.merged_cell_expanded"
            | "html.merged_cell_expanded"
            | "excel.formula_cached_value_only"
    )
}

/// Count warning and error occurrences for the strict-mode policy, excluding
/// the strict whitelist.
pub fn strict_violation(events: &[LossEvent], max_warnings: u64) -> Option<String> {
    let mut errors = 0u64;
    let mut warnings = 0u64;
    for event in events {
        if is_strict_exempt(event.code) {
            continue;
        }
        match event.severity {
            Severity::Error => errors = errors.saturating_add(event.count),
            Severity::Warning => warnings = warnings.saturating_add(event.count),
            Severity::Info => {}
        }
    }

    if errors > 0 {
        Some(format!("conversion produced {errors} error event(s)"))
    } else if warnings > max_warnings {
        Some(format!(
            "conversion produced {warnings} warning event(s), exceeding --max-warnings {max_warnings}"
        ))
    } else {
        None
    }
}

/// Distinct lossy categories and lossy item occurrences. Strict-exempt
/// transformations are excluded from both counts.
pub fn lossy_event_counts(events: &[LossEvent]) -> (usize, u64) {
    let mut codes = std::collections::HashSet::new();
    let mut items = 0u64;
    for event in events {
        if is_strict_exempt(event.code) {
            continue;
        }
        if matches!(event.severity, Severity::Warning | Severity::Error) {
            codes.insert(event.code);
            items = items.saturating_add(event.count);
        }
    }
    (codes.len(), items)
}

/// Short loss-severity label for the post-write summary.
pub fn loss_severity_label(events: &[LossEvent]) -> &'static str {
    let (_, items) = lossy_event_counts(events);
    if items > 0 {
        "lossy"
    } else if events.iter().any(|event| event.count > 0) {
        "approximate"
    } else {
        "lossless"
    }
}

/// The versioned provenance manifest written by `--manifest`.
#[derive(Debug, Clone, Serialize)]
pub struct ProvenanceManifest {
    pub version: u32,
    pub tool: ToolInfo,
    pub started_at: String,
    pub duration_ms: u64,
    pub command: Vec<String>,
    pub options: serde_json::Value,
    pub inputs: Vec<Artifact>,
    pub outputs: Vec<Artifact>,
    pub loss_summary: LossSummary,
    pub deterministic: bool,
}

/// Build and write a provenance manifest.
#[allow(clippy::too_many_arguments)]
pub fn write_manifest(
    path: &str,
    started: SystemTime,
    duration: Duration,
    command: Vec<String>,
    options: serde_json::Value,
    inputs: &[Artifact],
    outputs: &[Artifact],
    events: &[LossEvent],
    force: bool,
) -> Result<ProvenanceManifest> {
    let summary = summarize(&aggregate_events(events));
    let manifest = ProvenanceManifest {
        version: MANIFEST_VERSION,
        tool: ToolInfo::current(),
        started_at: DateTime::<Utc>::from(started).to_rfc3339_opts(SecondsFormat::Secs, true),
        duration_ms: duration.as_millis().min(u128::from(u64::MAX)) as u64,
        command,
        options,
        inputs: inputs.to_vec(),
        outputs: outputs.to_vec(),
        loss_summary: summary,
        deterministic: true,
    };

    write_json(path, &manifest, force)?;
    Ok(manifest)
}

/// Write the loss report as JSON and/or Markdown.
pub fn write_loss_reports(
    report: &LossReport,
    json_path: Option<&str>,
    markdown_path: Option<&str>,
    force: bool,
) -> Result<()> {
    if let Some(path) = json_path {
        write_json(path, report, force)?;
    }
    if let Some(path) = markdown_path {
        write_text(path, &render_markdown(report), force)?;
    }
    Ok(())
}

/// Render a concise Markdown loss report.
pub fn render_markdown(report: &LossReport) -> String {
    let mut out = String::new();
    out.push_str("# tabmux loss report\n\n");
    out.push_str(&format!(
        "Tool: {} {}\n\n",
        report.tool.name, report.tool.version
    ));

    out.push_str("## Summary\n\n");
    out.push_str("| Severity | Count |\n|---|---:|\n");
    out.push_str(&format!("| info | {} |\n", report.summary.info));
    out.push_str(&format!("| warning | {} |\n", report.summary.warning));
    out.push_str(&format!("| error | {} |\n\n", report.summary.error));

    if report.events.is_empty() {
        out.push_str("No loss events: this conversion was lossless.\n");
        return out;
    }

    out.push_str("## Events\n\n");
    out.push_str("| Severity | Code | Count | Message | Source |\n");
    out.push_str("|---|---|---:|---|---|\n");
    for event in &report.events {
        let message = event.message.replace('|', "\\|").replace('\n', " ");
        let source = event
            .source
            .as_ref()
            .map(render_source)
            .unwrap_or_default()
            .replace('|', "\\|");
        out.push_str(&format!(
            "| {} | `{}` | {} | {} | {} |\n",
            event.severity, event.code, event.count, message, source
        ));
    }
    out
}

fn render_source(source: &SourceRecord) -> String {
    let mut out = source.path.clone();
    if let Some(sheet) = &source.sheet {
        out.push_str(&format!("#{sheet}"));
    }
    if let Some(table) = &source.table {
        out.push_str(&format!("#{table}"));
    }
    if let Some(row) = source.row {
        out.push_str(&format!(" row {row}"));
    }
    if let Some(column) = &source.column {
        out.push_str(&format!(" column '{column}'"));
    }
    if let Some(cell) = &source.cell {
        out.push_str(&format!(" cell {cell}"));
    }
    out
}

/// Verify a written file by re-reading it and comparing columns, row counts,
/// and rendered cell values with the expected tables.
pub fn verify_output(
    path: &Path,
    format: FormatId,
    expected: &[Table],
    read_options: &ReadOptions,
    expected_null: &str,
) -> Result<()> {
    let bytes = std::fs::read(path)?;
    verify_bytes(
        &path.display().to_string(),
        format,
        expected,
        read_options,
        &bytes,
        expected_null,
    )
}

/// Verify bytes that were just written (stdout output or an in-memory buffer).
pub fn verify_bytes(
    label: &str,
    format: FormatId,
    expected: &[Table],
    read_options: &ReadOptions,
    bytes: &[u8],
    expected_null: &str,
) -> Result<()> {
    let expected_null = if matches!(format, FormatId::Csv | FormatId::Tsv) {
        expected_null
    } else {
        ""
    };
    let source = SourceRef::new(label, format);
    let mut options = read_options.clone();
    options.sheets = SheetSelection::All;
    let actual = formats::read_tables(&source, bytes, format, &options).map_err(|error| {
        Error::data(format!(
            "verification failed for {label}: could not re-read output: {error}"
        ))
    })?;

    if actual.tables.len() != expected.len() {
        return Err(Error::data(format!(
            "verification failed for {label}: expected {} table(s), found {}",
            expected.len(),
            actual.tables.len()
        )));
    }

    for (index, (expected, actual)) in expected.iter().zip(actual.tables.iter()).enumerate() {
        let context = format!("{label} table {}", index + 1);
        if expected.width() != actual.width() {
            return Err(Error::data(format!(
                "verification failed for {context}: expected {} column(s), found {}",
                expected.width(),
                actual.width()
            )));
        }
        if read_options.has_header && expected.column_names() != actual.column_names() {
            return Err(Error::data(format!(
                "verification failed for {context}: expected columns {:?}, found {:?}",
                expected.column_names(),
                actual.column_names()
            )));
        }
        if matches!(format, FormatId::Xlsx | FormatId::Ods) && expected.name != actual.name {
            return Err(Error::data(format!(
                "verification failed for {context}: expected sheet '{}', found '{}'",
                expected.name, actual.name
            )));
        }
        if expected.row_count() != actual.row_count() {
            return Err(Error::data(format!(
                "verification failed for {context}: expected {} data row(s), found {}",
                expected.row_count(),
                actual.row_count()
            )));
        }

        for (row_index, (expected_row, actual_row)) in
            expected.rows.iter().zip(actual.rows.iter()).enumerate()
        {
            for column_index in 0..expected.width() {
                let expected_value = expected_row
                    .get(column_index)
                    .unwrap_or(&crate::model::CellValue::Null)
                    .to_text(expected_null);
                let actual_value = actual_row
                    .get(column_index)
                    .unwrap_or(&crate::model::CellValue::Null)
                    .to_text("");
                if expected_value != actual_value {
                    return Err(Error::data(format!(
                        "verification failed for {context} row {} column '{}': expected '{}', found '{}'",
                        row_index + 1,
                        expected.columns[column_index].name,
                        expected_value,
                        actual_value
                    )));
                }
            }
        }
    }

    Ok(())
}

/// Reject auxiliary report paths that would overwrite a data input/output or
/// each other. Every path is also checked against the no-clobber policy.
pub fn validate_auxiliary_paths(
    auxiliary: &[(&str, &Path)],
    protected: &[(&str, &Path)],
    force: bool,
) -> Result<()> {
    for (label, path) in auxiliary {
        if *path == Path::new("-") {
            return Err(Error::usage(format!(
                "{label} must be a file path, not stdout"
            )));
        }
        for (other_label, other_path) in protected {
            if same_path(path, other_path) {
                return Err(Error::usage(format!(
                    "{label} '{}' is the same path as {other_label} '{}'",
                    path.display(),
                    other_path.display()
                )));
            }
        }
        SafeWriter::ensure_writable(path, force)?;
    }

    for (index, (label, path)) in auxiliary.iter().enumerate() {
        for (other_label, other_path) in auxiliary.iter().skip(index + 1) {
            if same_path(path, other_path) {
                return Err(Error::usage(format!(
                    "{label} '{}' and {other_label} '{}' resolve to the same path",
                    path.display(),
                    other_path.display()
                )));
            }
        }
    }

    Ok(())
}

fn same_path(left: &Path, right: &Path) -> bool {
    absolute_path(left) == absolute_path(right)
}

fn absolute_path(path: &Path) -> std::path::PathBuf {
    std::fs::canonicalize(path)
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

fn write_json<T: Serialize>(path: &str, value: &T, force: bool) -> Result<()> {
    let mut rendered = serde_json::to_string_pretty(value)
        .map_err(|error| Error::data(format!("cannot serialize report: {error}")))?;
    rendered.push('\n');
    write_text(path, &rendered, force)
}

fn write_text(path: &str, text: &str, force: bool) -> Result<()> {
    let mut writer = SafeWriter::create(Path::new(path), force, false)?;
    std::io::Write::write_all(&mut writer, text.as_bytes())?;
    writer.commit()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("{digest:x}")
}

/// Count events by severity, used by tests and callers that only need totals.
pub fn count_by_severity(events: &[LossEvent]) -> HashMap<&'static str, u64> {
    let mut counts = HashMap::new();
    for event in events {
        *counts.entry(event.severity.as_str()).or_insert(0) += event.count;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CellValue, Column, ColumnType};

    fn event(severity: Severity, code: &'static str, message: &str) -> LossEvent {
        LossEvent::new(severity, code, message)
    }

    #[test]
    fn repeated_events_aggregate_and_keep_the_first_source() {
        let first = SourceRef::new("a.csv", FormatId::Csv).with_row(2);
        let second = SourceRef::new("a.csv", FormatId::Csv).with_row(9);
        let events = vec![
            event(Severity::Warning, "csv.ragged_row", "short row").with_source(first.clone()),
            event(Severity::Warning, "csv.ragged_row", "short row").with_source(second),
        ];

        let records = aggregate_events(&events);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].count, 2);
        assert_eq!(
            records[0].source.as_ref().unwrap().row,
            first.row,
            "the first source reference must be retained"
        );
    }

    #[test]
    fn reports_are_deterministic_and_summarize_occurrences() {
        let events = vec![
            event(Severity::Info, "output.sheet_name_sanitized", "renamed"),
            event(Severity::Warning, "excel.chart_dropped", "chart"),
            event(Severity::Warning, "excel.chart_dropped", "chart"),
            event(Severity::Error, "format.unsupported", "bad"),
        ];

        let report = LossReport::build(Vec::new(), Vec::new(), &events);
        assert_eq!(report.version, REPORT_VERSION);
        assert_eq!(report.summary.info, 1);
        assert_eq!(report.summary.warning, 2);
        assert_eq!(report.summary.error, 1);
        assert_eq!(report.events[0].severity, "error");
        assert_eq!(report.events[1].count, 2);
    }

    #[test]
    fn strict_policy_counts_errors_and_warning_thresholds() {
        let warning = vec![event(Severity::Warning, "csv.ragged_row", "short row")];
        assert!(strict_violation(&warning, 0).is_some());
        assert!(strict_violation(&warning, 1).is_none());

        let error = vec![event(Severity::Error, "format.unsupported", "bad")];
        assert!(strict_violation(&error, 100).is_some());
    }

    #[test]
    fn strict_whitelist_exempts_normal_transformations() {
        let events = vec![
            event(
                Severity::Info,
                "excel.formula_cached_value_only",
                "cached value may be stale",
            ),
            event(
                Severity::Info,
                "excel.merged_cell_expanded",
                "merged range expanded",
            ),
            event(
                Severity::Info,
                "excel.pivot_values_materialized",
                "values materialized",
            ),
        ];
        assert!(strict_violation(&events, 0).is_none());
        assert_eq!(lossy_event_counts(&events), (0, 0));
        assert_eq!(loss_severity_label(&events), "approximate");

        let mut with_chart = events;
        with_chart.push(event(
            Severity::Warning,
            "excel.chart_dropped",
            "chart dropped",
        ));
        assert!(strict_violation(&with_chart, 0).is_some());
        assert_eq!(lossy_event_counts(&with_chart), (1, 1));
        assert_eq!(loss_severity_label(&with_chart), "lossy");
    }

    #[test]
    fn markdown_is_lossless_and_lists_events() {
        let report = LossReport::build(Vec::new(), Vec::new(), &[]);
        let markdown = render_markdown(&report);
        assert!(markdown.contains("No loss events"));
        assert!(markdown.contains("| warning | 0 |"));

        let events = vec![event(Severity::Warning, "excel.chart_dropped", "chart")];
        let report = LossReport::build(Vec::new(), Vec::new(), &events);
        let markdown = render_markdown(&report);
        assert!(markdown.contains("excel.chart_dropped"));
        assert!(markdown.contains("| warning | 1 |"));
    }

    #[test]
    fn null_rendering_is_reported_only_for_empty_delimited_nulls() {
        let source = SourceRef::new("data.csv", FormatId::Csv);
        let table = Table::with_columns(
            "data",
            source,
            vec![Column::new("a", ColumnType::Text)],
            vec![vec![CellValue::Null], vec![CellValue::Text("x".into())]],
        );

        let events = null_rendering_events(&[&table], FormatId::Csv, "");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].code, "output.null_rendered_as_empty");
        assert_eq!(events[0].severity, Severity::Info);
        assert_eq!(events[0].count, 1);

        assert!(null_rendering_events(&[&table], FormatId::Csv, "NULL").is_empty());
        assert!(null_rendering_events(&[&table], FormatId::Xlsx, "").is_empty());
    }

    #[test]
    fn verify_detects_value_and_row_count_changes() {
        let source = SourceRef::new("data.csv", FormatId::Csv);
        let table = Table::with_columns(
            "data",
            source,
            vec![
                Column::new("id", ColumnType::Text),
                Column::new("name", ColumnType::Text),
            ],
            vec![vec![
                CellValue::Text("1".into()),
                CellValue::Text("Alice".into()),
            ]],
        );
        let options = ReadOptions::default();

        verify_bytes(
            "buffer",
            FormatId::Csv,
            std::slice::from_ref(&table),
            &options,
            b"id,name\n1,Alice\n",
            "",
        )
        .unwrap();

        let error = verify_bytes(
            "buffer",
            FormatId::Csv,
            std::slice::from_ref(&table),
            &options,
            b"id,name\n1,Bob\n",
            "",
        )
        .unwrap_err();
        assert!(error.to_string().contains("verification failed"));

        let error = verify_bytes(
            "buffer",
            FormatId::Csv,
            std::slice::from_ref(&table),
            &options,
            b"id,name\n",
            "",
        )
        .unwrap_err();
        assert!(error.to_string().contains("data row"));
    }

    #[test]
    fn golden_report_matches_the_checked_in_fixture() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("reports")
            .join("sample-report.json");
        let expected: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixture).unwrap()).unwrap();

        let inputs = vec![Artifact::from_bytes(
            "fixtures/rich.xlsx",
            FormatId::Xlsx,
            b"input",
        )];
        let outputs = vec![Artifact::from_bytes("out.csv", FormatId::Csv, b"output")];
        let events = vec![
            event(
                Severity::Warning,
                "excel.chart_dropped",
                "the workbook contains 1 chart(s); charts are not copied",
            )
            .with_source(SourceRef::new("fixtures/rich.xlsx", FormatId::Xlsx)),
            event(
                Severity::Info,
                "excel.merged_cell_expanded",
                "expanded A4:B4",
            )
            .with_source(SourceRef::new("fixtures/rich.xlsx", FormatId::Xlsx).with_sheet("Rich")),
        ];

        let report = LossReport::build(inputs, outputs, &events);
        let actual = serde_json::to_value(&report).unwrap();
        for key in ["version", "inputs", "outputs", "events", "summary"] {
            assert_eq!(
                actual[key], expected[key],
                "golden report field '{key}' changed"
            );
        }
    }
}
