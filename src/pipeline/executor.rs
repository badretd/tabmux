//! Resolve, validate, and execute a linear pipeline manifest.
//!
//! The executor is deliberately split into a pure `execute` pass and a write
//! pass. `execute` loads every input and runs every step before any output
//! writer is opened, so an invalid column name or schema mismatch can never
//! leave a partial result on disk.

use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Instant, SystemTime};

use serde::Serialize;

use crate::cli::args::ReportArgs;
use crate::cli::reporting::ReportContext;
use crate::diagnostics::{DiagnosticsPolicy, DiagnosticsScope};
use crate::error::{Error, Result};
use crate::formats::{self, FormatId};
use crate::io::SafeWriter;
use crate::model::{LossEvent, SourceRef, Table};
use crate::ops::{dedupe, filter, merge, rename, select, sort};
use crate::report::{self, Artifact, LossReport, strict_violation, validate_auxiliary_paths};

use super::manifest::{
    InputSpec, Manifest, MergeModeSpec, Op, OutputSpec, SheetSpec, Step, output_format_from_path,
    parse_input_format, parse_output_format,
};

/// Per-run overrides supplied by the CLI. `None` means "use the manifest".
#[derive(Debug, Clone, Default)]
pub struct RunSettings {
    pub dry_run: bool,
    pub force: Option<bool>,
    pub strict: Option<bool>,
    pub backup: Option<bool>,
    pub save_plan: Option<String>,
    pub report: Option<String>,
    pub report_markdown: Option<String>,
    pub manifest: Option<String>,
    pub max_warnings: u64,
    pub verify: bool,
}

/// Resolved global options for one execution.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct ResolvedOptions {
    pub strict: bool,
    pub force: bool,
    pub backup: bool,
}

/// A fully resolved output ready to be staged.
#[derive(Debug, Clone)]
pub struct ResolvedOutput {
    pub spec: OutputSpec,
    /// Result name that produced this output.
    pub input: String,
    pub format: FormatId,
    pub table: Table,
}

/// The resolved plan is a deterministic, machine-readable description of what
/// the run will do. It contains no timestamps, so two replays produce the same
/// document.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedPlan {
    pub version: u32,
    pub manifest: String,
    pub dry_run: bool,
    pub options: ResolvedOptions,
    pub inputs: Vec<PlanInput>,
    pub steps: Vec<PlanStep>,
    pub outputs: Vec<PlanOutput>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanInput {
    pub id: String,
    pub path: String,
    pub format: String,
    pub sheet: Option<String>,
    pub rows: usize,
    pub columns: usize,
    pub warnings: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanStep {
    pub index: usize,
    pub op: String,
    pub id: String,
    pub alias: String,
    pub input: String,
    pub rows: usize,
    pub columns: usize,
    pub warnings: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanOutput {
    pub id: Option<String>,
    pub input: String,
    pub path: String,
    pub format: String,
    pub sheet: Option<String>,
    pub rows: usize,
    pub columns: usize,
    pub warnings: usize,
}

/// Everything needed to print a plan or write the outputs.
#[derive(Debug, Clone)]
pub struct Execution {
    pub plan: ResolvedPlan,
    pub outputs: Vec<ResolvedOutput>,
    pub warnings: Vec<LossEvent>,
    pub options: ResolvedOptions,
}

/// Resolve inputs, execute the step chain, and prepare outputs without opening
/// a writer.
pub fn execute(
    manifest: &Manifest,
    manifest_path: &Path,
    settings: &RunSettings,
) -> Result<Execution> {
    let options = ResolvedOptions {
        strict: settings.strict.unwrap_or(manifest.options.strict),
        force: settings.force.unwrap_or(manifest.options.force),
        backup: settings.backup.unwrap_or(manifest.options.backup),
    };

    let mut results = HashMap::<String, Rc<Table>>::new();
    let mut plan_inputs = Vec::with_capacity(manifest.inputs.len());

    for input in &manifest.inputs {
        let (table, format) = load_input(input)?;
        let table = Rc::new(table);
        results.insert(input.id.clone(), Rc::clone(&table));
        plan_inputs.push(PlanInput {
            id: input.id.clone(),
            path: input.path.clone(),
            format: format.label().to_owned(),
            sheet: input.sheet.as_ref().map(SheetSpec::label),
            rows: table.row_count(),
            columns: table.width(),
            warnings: table.warnings.len(),
        });
    }

    let mut plan_steps = Vec::with_capacity(manifest.steps.len());
    let mut last_step: Option<(String, Rc<Table>)> = None;

    for (index, step) in manifest.steps.iter().enumerate() {
        let primary = step.result_name(index);
        let (input_name, input_table) =
            resolve_step_input(step, index, &results, last_step.as_ref(), manifest)?;
        let output_table = apply_step(step, index, &input_table, &results)?;
        let output_table = Rc::new(output_table);

        results.insert(primary.clone(), Rc::clone(&output_table));
        let alias = step.op.alias().to_owned();
        if alias != primary {
            results.insert(alias.clone(), Rc::clone(&output_table));
        }

        plan_steps.push(PlanStep {
            index: index + 1,
            op: step.op.label().to_owned(),
            id: primary.clone(),
            alias,
            input: input_name,
            rows: output_table.row_count(),
            columns: output_table.width(),
            warnings: output_table.warnings.len(),
        });
        last_step = Some((primary, output_table));
    }

    let mut outputs = Vec::with_capacity(manifest.outputs.len());
    let mut plan_outputs = Vec::with_capacity(manifest.outputs.len());

    for output in &manifest.outputs {
        let (input_name, table) =
            resolve_output_input(output, &results, last_step.as_ref(), manifest)?;
        let format = match &output.format {
            Some(name) => parse_output_format(name)?,
            None => output_format_from_path(&output.path)?,
        };
        validate_output_format(format)?;

        if output.path == "-" && !matches!(format, FormatId::Csv | FormatId::Tsv) {
            return Err(Error::usage(format!(
                "pipeline output '{}' writes {} to stdout; only csv and tsv can be streamed",
                output.path,
                format.label()
            )));
        }
        if output.sheet.is_some() && !is_workbook_output(format) {
            return Err(Error::usage(format!(
                "pipeline output '{}' sets sheet but the format is {}; sheet requires xlsx or ods",
                output.path,
                format.label()
            )));
        }

        let mut table = (*table).clone();
        if is_workbook_output(format) {
            let sheet = output.sheet.clone().unwrap_or_else(|| table.name.clone());
            apply_output_sheet(&mut table, &sheet);
        }

        plan_outputs.push(PlanOutput {
            id: output.id.clone(),
            input: input_name.clone(),
            path: output.path.clone(),
            format: format.label().to_owned(),
            sheet: if is_workbook_output(format) {
                Some(table.name.clone())
            } else {
                None
            },
            rows: table.row_count(),
            columns: table.width(),
            warnings: table.warnings.len(),
        });
        outputs.push(ResolvedOutput {
            spec: output.clone(),
            input: input_name,
            format,
            table,
        });
    }

    let warnings = collect_warnings(&outputs);
    let plan = ResolvedPlan {
        version: manifest.version,
        manifest: manifest_path.display().to_string(),
        dry_run: settings.dry_run,
        options,
        inputs: plan_inputs,
        steps: plan_steps,
        outputs: plan_outputs,
    };

    Ok(Execution {
        plan,
        outputs,
        warnings,
        options,
    })
}

/// Execute a manifest and write all outputs unless `--dry-run` is active.
pub fn run(
    manifest: &Manifest,
    manifest_path: &Path,
    settings: &RunSettings,
    diagnostics: &DiagnosticsPolicy,
) -> Result<()> {
    let started = Instant::now();
    let started_at = SystemTime::now();
    let mut execution = execute(manifest, manifest_path, settings)?;
    let inputs = input_artifacts(manifest)?;
    for output in &mut execution.outputs {
        report::push_null_rendering_warning(&mut output.table, output.format, "");
    }
    execution.warnings = collect_warnings(&execution.outputs);
    let events = execution.warnings.clone();
    let include_fidelity = execution
        .outputs
        .iter()
        .any(|output| is_workbook_output(output.format))
        || diagnostics.fidelity_report
        || execution.options.strict;
    let report_args = ReportArgs {
        report: settings.report.clone(),
        report_markdown: settings.report_markdown.clone(),
        manifest: settings.manifest.clone(),
        strict: execution.options.strict,
        lossy: false,
        max_warnings: settings.max_warnings,
        verify: settings.verify,
    };
    let context = ReportContext::with_started(
        &report_args,
        inputs.clone(),
        execution.options.force,
        started,
        started_at,
    );

    if diagnostics.enabled() {
        if context.aggregates_diagnostics(diagnostics) {
            context.print_event_summary(
                &events,
                diagnostics,
                DiagnosticsScope::Write,
                include_fidelity,
            );
        } else {
            for event in &execution.warnings {
                if diagnostics.shows(event, DiagnosticsScope::Write, include_fidelity) {
                    crate::cli::print_event(event);
                }
            }
        }
    }

    validate_report_paths(
        manifest,
        &execution.outputs,
        settings,
        execution.options.force,
    )?;

    if execution.options.strict
        && let Some(reason) = strict_violation(&events, settings.max_warnings)
    {
        write_loss_report(settings, &inputs, &[], &events, execution.options.force)?;
        let mut message = format!(
            "pipeline produced {reason}; no output was written (use --no-strict, --lossy, or set strict = false to allow them)"
        );
        if let Some(path) = &settings.report {
            message.push_str(&format!("; loss report: {path}"));
        } else if let Some(path) = &settings.report_markdown {
            message.push_str(&format!("; loss report: {path}"));
        }
        return Err(Error::LossyBlocked(message));
    }

    if let Some(path) = &settings.save_plan {
        validate_plan_path(manifest, &execution.outputs, path, execution.options.force)?;
    }
    validate_output_paths(manifest, &execution.outputs, execution.options.force)?;

    if settings.dry_run {
        print_dry_run(&execution, diagnostics);
        write_loss_report(settings, &inputs, &[], &events, execution.options.force)?;
        context.print_dry_run_reports(&events, diagnostics);
        if let Some(path) = &settings.save_plan {
            write_plan(&execution.plan, path, execution.options.force)?;
        }
        return Ok(());
    }

    if let Some(path) = &settings.save_plan {
        write_plan(&execution.plan, path, execution.options.force)?;
    }

    let mut staged = Vec::with_capacity(execution.outputs.len());
    let mut stdout = None;

    for output in &execution.outputs {
        let options = write_options(output);
        if output.spec.path == "-" {
            let mut buffer = Vec::new();
            formats::write_table(&mut buffer, &output.table, output.format, &options)?;
            stdout = Some(buffer);
            continue;
        }

        let destination = Path::new(&output.spec.path);
        let mut writer = SafeWriter::create(
            destination,
            execution.options.force,
            execution.options.backup,
        )?;
        if is_workbook_output(output.format) {
            formats::write_tables(
                writer.file_mut(),
                std::slice::from_ref(&output.table),
                output.format,
                &options,
            )?;
        } else {
            formats::write_table(&mut writer, &output.table, output.format, &options)?;
        }
        staged.push((destination.to_path_buf(), writer));
    }

    let mut written = Vec::with_capacity(staged.len());
    for (destination, writer) in staged {
        match writer.commit() {
            Ok(()) => written.push(destination),
            Err(error) => {
                let completed = written
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                let completed = if completed.is_empty() {
                    "none".to_owned()
                } else {
                    completed
                };
                return Err(Error::Partial(format!(
                    "pipeline commit failed for '{}': {error}; committed outputs: {completed}",
                    destination.display()
                )));
            }
        }
    }

    let outputs = output_artifacts(&execution.outputs, stdout.as_deref())?;
    if settings.verify {
        verify_outputs(&execution.outputs, stdout.as_deref())?;
    }

    if let Some(buffer) = stdout {
        crate::io::write_stdout(&buffer)?;
    }

    write_loss_report(
        settings,
        &inputs,
        &outputs,
        &events,
        execution.options.force,
    )?;
    write_manifest(
        settings, &execution, started, started_at, &inputs, &outputs, &events,
    )?;

    if diagnostics.enabled() {
        let paths = execution
            .outputs
            .iter()
            .map(|output| output.spec.path.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!(
            "tabmux: wrote {} output(s): {paths}{}",
            execution.outputs.len(),
            context.summary_suffix(&events)
        );
    }

    Ok(())
}

fn input_artifacts(manifest: &Manifest) -> Result<Vec<Artifact>> {
    let mut artifacts = Vec::new();
    for input in &manifest.inputs {
        if input.path == "-" {
            continue;
        }
        let format = match &input.format {
            Some(name) => parse_input_format(name)?,
            None => formats::detect_path(Path::new(&input.path)).unwrap_or(FormatId::Unknown),
        };
        artifacts.push(Artifact::from_file_as(Path::new(&input.path), format)?);
    }
    Ok(artifacts)
}

fn output_artifacts(outputs: &[ResolvedOutput], stdout: Option<&[u8]>) -> Result<Vec<Artifact>> {
    let mut artifacts = Vec::new();
    for output in outputs {
        if output.spec.path == "-" {
            let buffer = stdout.ok_or_else(|| {
                Error::data("pipeline stdout output was not buffered before reporting")
            })?;
            artifacts.push(Artifact::from_bytes("-", output.format, buffer));
        } else {
            artifacts.push(Artifact::from_file_as(
                Path::new(&output.spec.path),
                output.format,
            )?);
        }
    }
    Ok(artifacts)
}

fn verify_outputs(outputs: &[ResolvedOutput], stdout: Option<&[u8]>) -> Result<()> {
    for output in outputs {
        let read_options = formats::ReadOptions {
            has_header: output.spec.header.unwrap_or(true),
            ..formats::ReadOptions::default()
        };
        if output.spec.path == "-" {
            let buffer = stdout.ok_or_else(|| {
                Error::data("pipeline stdout output was not buffered before verification")
            })?;
            report::verify_bytes(
                "stdout",
                output.format,
                std::slice::from_ref(&output.table),
                &read_options,
                buffer,
                "",
            )?;
        } else {
            report::verify_output(
                Path::new(&output.spec.path),
                output.format,
                std::slice::from_ref(&output.table),
                &read_options,
                "",
            )?;
        }
    }
    Ok(())
}

fn auxiliary_paths(settings: &RunSettings) -> Vec<(&'static str, &Path)> {
    let mut paths = Vec::new();
    if let Some(path) = &settings.report {
        paths.push(("loss report", Path::new(path)));
    }
    if let Some(path) = &settings.report_markdown {
        paths.push(("markdown loss report", Path::new(path)));
    }
    if let Some(path) = &settings.manifest {
        paths.push(("provenance manifest", Path::new(path)));
    }
    paths
}

fn validate_report_paths(
    manifest: &Manifest,
    outputs: &[ResolvedOutput],
    settings: &RunSettings,
    force: bool,
) -> Result<()> {
    let auxiliary = auxiliary_paths(settings);
    if auxiliary.is_empty() {
        return Ok(());
    }

    let mut protected = Vec::new();
    for input in &manifest.inputs {
        if input.path != "-" {
            protected.push(("input", Path::new(&input.path)));
        }
    }
    for output in outputs {
        if output.spec.path != "-" {
            protected.push(("output", Path::new(&output.spec.path)));
        }
    }
    if let Some(path) = &settings.save_plan
        && path != "-"
    {
        protected.push(("resolved plan", Path::new(path)));
    }

    validate_auxiliary_paths(&auxiliary, &protected, force)
}

fn write_loss_report(
    settings: &RunSettings,
    inputs: &[Artifact],
    outputs: &[Artifact],
    events: &[LossEvent],
    force: bool,
) -> Result<()> {
    if settings.report.is_none() && settings.report_markdown.is_none() {
        return Ok(());
    }
    let report = LossReport::build(inputs.to_vec(), outputs.to_vec(), events);
    report::write_loss_reports(
        &report,
        settings.report.as_deref(),
        settings.report_markdown.as_deref(),
        force,
    )
}

fn write_manifest(
    settings: &RunSettings,
    execution: &Execution,
    started: Instant,
    started_at: SystemTime,
    inputs: &[Artifact],
    outputs: &[Artifact],
    events: &[LossEvent],
) -> Result<()> {
    let Some(path) = &settings.manifest else {
        return Ok(());
    };
    let options = serde_json::json!({
        "command": "run",
        "manifest": execution.plan.manifest,
        "dry_run": execution.plan.dry_run,
        "strict": execution.options.strict,
        "force": execution.options.force,
        "backup": execution.options.backup,
        "max_warnings": settings.max_warnings,
        "verify": settings.verify,
    });
    report::write_manifest(
        path,
        started_at,
        started.elapsed(),
        std::env::args_os()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect(),
        options,
        inputs,
        outputs,
        events,
        execution.options.force,
    )?;
    Ok(())
}

fn load_input(input: &InputSpec) -> Result<(Table, FormatId)> {
    let format = match &input.format {
        Some(name) => parse_input_format(name)?,
        None => formats::detect_path(Path::new(&input.path)).ok_or_else(|| {
            Error::unsupported(format!(
                "cannot detect input format for '{}'; set format = \"...\"",
                input.path
            ))
        })?,
    };

    if input.sheet.is_some() && !matches!(format, FormatId::Xlsx | FormatId::Ods | FormatId::Html) {
        return Err(Error::usage(format!(
            "pipeline input '{}' sets sheet but format {} is not a multi-table input",
            input.id,
            format.label()
        )));
    }
    if input.header.is_some() && matches!(format, FormatId::Json) {
        return Err(Error::usage(format!(
            "pipeline input '{}': header is not supported for JSON input",
            input.id
        )));
    }

    let bytes = crate::io::read_input(&input.path).map_err(|error| {
        Error::data(format!(
            "pipeline input '{}' ({}): {error}",
            input.id, input.path
        ))
    })?;

    let mut source = SourceRef::new(&input.path, format);
    let selection = match &input.sheet {
        Some(SheetSpec::Name(name)) => {
            source = source.with_sheet(name.clone());
            formats::SheetSelection::Named(name.clone())
        }
        Some(SheetSpec::Index(index)) => formats::SheetSelection::Index(*index),
        None => formats::SheetSelection::Default,
    };

    let options = formats::ReadOptions {
        has_header: input.header.unwrap_or(true),
        sheets: selection,
        ..formats::ReadOptions::default()
    };

    let set = formats::read_tables(&source, &bytes, format, &options)?;
    let table = set
        .into_single()
        .map_err(|error| Error::data(format!("pipeline input '{}': {error}", input.id)))?;
    Ok((table, format))
}

fn resolve_step_input(
    step: &Step,
    index: usize,
    results: &HashMap<String, Rc<Table>>,
    last_step: Option<&(String, Rc<Table>)>,
    manifest: &Manifest,
) -> Result<(String, Rc<Table>)> {
    if let Some(name) = &step.input {
        let table = results.get(name).cloned().ok_or_else(|| {
            Error::data(format!(
                "pipeline step {} ({}) references unknown input '{}'; available results: {}",
                index + 1,
                step.op.label(),
                name,
                available_results(results)
            ))
        })?;
        return Ok((name.clone(), table));
    }

    if let Some((name, table)) = last_step {
        return Ok((name.clone(), Rc::clone(table)));
    }

    if manifest.inputs.len() == 1 {
        let name = manifest.inputs[0].id.clone();
        let table = results
            .get(&name)
            .cloned()
            .ok_or_else(|| Error::data(format!("pipeline input '{name}' was not loaded")))?;
        return Ok((name, table));
    }

    Err(Error::usage(format!(
        "pipeline step {} ({}) has no input; set input = \"...\" because the manifest declares {} inputs",
        index + 1,
        step.op.label(),
        manifest.inputs.len()
    )))
}

fn apply_step(
    step: &Step,
    index: usize,
    input: &Table,
    results: &HashMap<String, Rc<Table>>,
) -> Result<Table> {
    match step.op {
        Op::Select => {
            let specification = step
                .columns
                .as_ref()
                .expect("manifest validation requires columns")
                .to_vec();
            select::select_columns_named(input, &specification)
        }
        Op::Filter => {
            let expression = step
                .expr
                .as_deref()
                .expect("manifest validation requires expr");
            let predicate = filter::parse(expression)?;
            filter::filter(input, &predicate)
        }
        Op::Sort => {
            let specification = step
                .by
                .as_ref()
                .expect("manifest validation requires by")
                .to_vec();
            let mut keys = Vec::new();
            for key in specification {
                keys.extend(sort::parse_keys(&key)?);
            }
            sort::sort(input, &keys)
        }
        Op::Rename => {
            let pairs = step
                .map
                .as_ref()
                .expect("manifest validation requires map")
                .to_pairs()?;
            let mappings = pairs
                .into_iter()
                .map(|(from, to)| rename::Mapping { from, to })
                .collect::<Vec<_>>();
            rename::rename(input, &mappings)
        }
        Op::Dedupe => Ok(dedupe::dedupe(input)),
        Op::Merge | Op::Concat => {
            let with_name = step
                .with
                .as_deref()
                .expect("manifest validation requires with");
            let secondary = results.get(with_name).cloned().ok_or_else(|| {
                Error::data(format!(
                    "pipeline step {} ({}) references unknown secondary input '{}'; available results: {}",
                    index + 1,
                    step.op.label(),
                    with_name,
                    available_results(results)
                ))
            })?;

            let mode = match step.mode {
                MergeModeSpec::Strict => merge::MergeMode::Strict,
                MergeModeSpec::Loose => merge::MergeMode::Loose,
            };

            if mode == merge::MergeMode::Strict {
                let tables = [input.clone(), (*secondary).clone()];
                let diffs = merge::strict_diffs(&tables, step.case_insensitive_headers);
                if !diffs.is_empty() {
                    for diff in &diffs {
                        eprint!("{}", diff.render_text());
                    }
                    return Err(Error::data(format!(
                        "pipeline step {} ({}): strict merge found schema differences; no output was written",
                        index + 1,
                        step.op.label()
                    )));
                }
            }

            merge::merge(
                vec![input.clone(), (*secondary).clone()],
                &merge::MergeOptions {
                    source_column: step.add_source_column.clone(),
                    dedup: step.dedup,
                    case_insensitive_headers: step.case_insensitive_headers,
                },
            )
        }
    }
}

fn resolve_output_input(
    output: &OutputSpec,
    results: &HashMap<String, Rc<Table>>,
    last_step: Option<&(String, Rc<Table>)>,
    manifest: &Manifest,
) -> Result<(String, Rc<Table>)> {
    if let Some(name) = &output.input {
        let table = results.get(name).cloned().ok_or_else(|| {
            Error::data(format!(
                "pipeline output '{}' references unknown input '{}'; available results: {}",
                output.path,
                name,
                available_results(results)
            ))
        })?;
        return Ok((name.clone(), table));
    }

    if let Some((name, table)) = last_step {
        return Ok((name.clone(), Rc::clone(table)));
    }

    if manifest.inputs.len() == 1 {
        let name = manifest.inputs[0].id.clone();
        let table = results
            .get(&name)
            .cloned()
            .ok_or_else(|| Error::data(format!("pipeline input '{name}' was not loaded")))?;
        return Ok((name, table));
    }

    Err(Error::usage(format!(
        "pipeline output '{}' has no input; set input = \"...\" because the manifest declares {} inputs",
        output.path,
        manifest.inputs.len()
    )))
}

fn available_results(results: &HashMap<String, Rc<Table>>) -> String {
    let mut names = results.keys().cloned().collect::<Vec<_>>();
    names.sort();
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(", ")
    }
}

fn collect_warnings(outputs: &[ResolvedOutput]) -> Vec<LossEvent> {
    let mut seen = HashSet::new();
    let mut warnings = Vec::new();

    let mut push = |event: &LossEvent| {
        let key = format!(
            "{}\u{1e}{}\u{1e}{}\u{1e}{}",
            event.severity.as_str(),
            event.code,
            event.message,
            event
                .source
                .as_ref()
                .map(SourceRef::context)
                .unwrap_or_default()
        );
        if seen.insert(key) {
            warnings.push(event.clone());
        }
    };

    for output in outputs {
        for event in &output.table.warnings {
            push(event);
        }
    }

    warnings.sort_by(|left, right| {
        let left_source = left
            .source
            .as_ref()
            .map(SourceRef::context)
            .unwrap_or_default();
        let right_source = right
            .source
            .as_ref()
            .map(SourceRef::context)
            .unwrap_or_default();
        left_source
            .cmp(&right_source)
            .then_with(|| left.code.cmp(right.code))
            .then_with(|| left.message.cmp(&right.message))
    });
    warnings
}

fn validate_output_format(format: FormatId) -> Result<()> {
    match format {
        FormatId::Csv | FormatId::Tsv => Ok(()),
        #[cfg(feature = "excel")]
        FormatId::Xlsx | FormatId::Ods => Ok(()),
        #[cfg(not(feature = "excel"))]
        FormatId::Xlsx | FormatId::Ods => Err(Error::unsupported(
            "workbook writing is not compiled in (enable the excel feature)",
        )),
        FormatId::Html | FormatId::Json => Err(Error::unsupported(format!(
            "writing {} is not implemented yet",
            format.label()
        ))),
        FormatId::Unknown => Err(Error::unsupported("unknown output format")),
    }
}

fn write_options(output: &ResolvedOutput) -> formats::WriteOptions {
    let mut options = formats::WriteOptions::for_format(output.format);
    options.has_header = output.spec.header.unwrap_or(true);
    options
}

fn is_workbook_output(format: FormatId) -> bool {
    matches!(format, FormatId::Xlsx | FormatId::Ods)
}

#[cfg(feature = "excel")]
fn apply_output_sheet(table: &mut Table, sheet: &str) {
    let sanitized = formats::workbook_write::sanitize_sheet_name(sheet);
    if sanitized != sheet {
        table.push_warning(
            LossEvent::info(
                "output.sheet_name_sanitized",
                format!("sheet name '{sheet}' was sanitized to '{sanitized}' for workbook output"),
            )
            .with_source(table.source.clone()),
        );
    }
    table.name = sanitized;
}

#[cfg(not(feature = "excel"))]
fn apply_output_sheet(table: &mut Table, sheet: &str) {
    table.name = sheet.to_owned();
}

fn validate_output_paths(
    manifest: &Manifest,
    outputs: &[ResolvedOutput],
    force: bool,
) -> Result<()> {
    let input_paths = manifest
        .inputs
        .iter()
        .filter(|input| input.path != "-")
        .map(|input| PathBuf::from(&input.path))
        .collect::<Vec<_>>();

    for output in outputs {
        if output.spec.path == "-" {
            continue;
        }
        let destination = Path::new(&output.spec.path);
        for input in &input_paths {
            if same_path(destination, input) {
                return Err(Error::usage(format!(
                    "pipeline output '{}' would overwrite input '{}'",
                    destination.display(),
                    input.display()
                )));
            }
        }
        SafeWriter::ensure_writable(destination, force)?;
    }

    for (index, output) in outputs.iter().enumerate() {
        if output.spec.path == "-" {
            continue;
        }
        for other in outputs.iter().skip(index + 1) {
            if other.spec.path == "-" {
                continue;
            }
            if same_path(Path::new(&output.spec.path), Path::new(&other.spec.path)) {
                return Err(Error::usage(format!(
                    "pipeline outputs '{}' and '{}' resolve to the same path",
                    output.spec.path, other.spec.path
                )));
            }
        }
    }

    Ok(())
}

fn validate_plan_path(
    manifest: &Manifest,
    outputs: &[ResolvedOutput],
    plan_path: &str,
    force: bool,
) -> Result<()> {
    if plan_path == "-" {
        if outputs.iter().any(|output| output.spec.path == "-") {
            return Err(Error::usage(
                "cannot write both the resolved plan and a data output to stdout",
            ));
        }
        return Ok(());
    }

    let plan_path = Path::new(plan_path);
    for input in &manifest.inputs {
        if input.path != "-" && same_path(plan_path, Path::new(&input.path)) {
            return Err(Error::usage(format!(
                "resolved plan path '{}' would overwrite input '{}'",
                plan_path.display(),
                input.path
            )));
        }
    }
    for output in outputs {
        if output.spec.path != "-" && same_path(plan_path, Path::new(&output.spec.path)) {
            return Err(Error::usage(format!(
                "resolved plan path '{}' is also a pipeline output",
                plan_path.display()
            )));
        }
    }

    SafeWriter::ensure_writable(plan_path, force)
}

fn same_path(left: &Path, right: &Path) -> bool {
    absolute_path(left) == absolute_path(right)
}

fn absolute_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path)
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

fn print_dry_run(execution: &Execution, diagnostics: &DiagnosticsPolicy) {
    if !diagnostics.enabled() {
        return;
    }

    eprintln!(
        "tabmux: dry-run: pipeline plan for {}",
        execution.plan.manifest
    );
    for input in &execution.plan.inputs {
        eprintln!(
            "tabmux: dry-run: input '{}' {}{}: {} rows x {} columns",
            input.id,
            input.path,
            input
                .sheet
                .as_ref()
                .map(|sheet| format!("#{sheet}"))
                .unwrap_or_default(),
            input.rows,
            input.columns
        );
    }
    for step in &execution.plan.steps {
        eprintln!(
            "tabmux: dry-run: step {} {} '{}' -> '{}' (alias '{}'): {} rows x {} columns",
            step.index, step.op, step.input, step.id, step.alias, step.rows, step.columns
        );
    }
    for output in &execution.plan.outputs {
        eprintln!(
            "tabmux: dry-run: output '{}' {} ({}): {} rows x {} columns from '{}'",
            output.id.as_deref().unwrap_or("-"),
            output.path,
            output.format,
            output.rows,
            output.columns,
            output.input
        );
    }
    eprintln!("tabmux: dry-run: wrote nothing");
}

fn write_plan(plan: &ResolvedPlan, path: &str, force: bool) -> Result<()> {
    let mut rendered = serde_json::to_string_pretty(plan)
        .map_err(|error| Error::data(format!("cannot serialize pipeline plan: {error}")))?;
    rendered.push('\n');

    if path == "-" {
        return crate::io::write_stdout(rendered.as_bytes());
    }

    let mut writer = SafeWriter::create(Path::new(path), force, false)?;
    writer.write_all(rendered.as_bytes())?;
    writer.commit()
}
