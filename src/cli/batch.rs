//! Batch conversion for directory and ZIP inputs.
//!
//! Batch mode is deliberately a thin orchestration layer over the normal
//! readers and writers: it discovers files deterministically, reads every
//! selected table before creating an output, and stages all writes through
//! [`SafeWriter`] so a validation failure leaves no partial results.

use std::path::{Component, Path, PathBuf};
use std::time::{Instant, SystemTime};

use crate::diagnostics::{DiagnosticsPolicy, DiagnosticsScope};
use crate::error::{Error, Result};
use crate::formats::archive;
use crate::formats::{self, FormatId, SheetSelection};
use crate::io::SafeWriter;
use crate::model::{LossEvent, SourceRef, Table};
use crate::report::{self, Artifact};

use super::args::ConvertArgs;
use super::reporting::{ReportContext, output_artifact};
use super::{
    is_workbook_output, print_dry_run, print_event, print_warnings, render_name_template,
    sanitize_sheet_name, split_output_format, stack_tables, unique_relative_path,
};

/// Convert one directory or ZIP archive into a directory of output files.
pub(super) fn convert_batch(
    input: &str,
    output: &str,
    args: &ConvertArgs,
    diagnostics: &DiagnosticsPolicy,
) -> Result<()> {
    validate_batch_args(input, output, args)?;
    let started = Instant::now();
    let started_at = SystemTime::now();

    let selection = args.read.sheets.selection()?;
    let default_template = if selection == SheetSelection::All {
        "{stem}.{sheet}.{format}"
    } else {
        "{stem}.{format}"
    };
    let template = args.name_template.as_deref().unwrap_or(default_template);
    let output_format = split_output_format(template)?;
    if is_workbook_output(output_format) {
        return Err(Error::usage(
            "batch output writes one table file per input; use .csv or .tsv in --name-template",
        ));
    }

    let scan = archive::scan(
        Path::new(input),
        &archive::BatchOptions {
            recursive: args.recursive,
            include: args.include.clone(),
            exclude: args.exclude.clone(),
            limits: archive::ArchiveLimits {
                max_entries: args.max_archive_entries,
                max_uncompressed_size: args.max_uncompressed_size,
                max_compression_ratio: args.max_compression_ratio,
            },
        },
    )?;

    let mut skipped = scan.unsupported.len();
    let mut events = Vec::<LossEvent>::new();
    for unsupported in &scan.unsupported {
        events.push(report_unsupported(
            &unsupported.source,
            &unsupported.reason,
            diagnostics,
        ));
    }

    let input_artifacts = scan
        .sources
        .iter()
        .map(|source| {
            Artifact::from_bytes(
                source.source.path.display().to_string(),
                source.format,
                &source.bytes,
            )
        })
        .collect::<Vec<_>>();
    let context = ReportContext::with_started(
        &args.reporting,
        input_artifacts,
        args.write.force,
        started,
        started_at,
    );

    let read_options = args.read.options()?;
    let mut plans = Vec::<PlannedTable>::new();
    let mut converted_inputs = 0usize;
    let mut used = std::collections::HashSet::<String>::new();

    for source in scan.sources {
        let stem = source
            .relative
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .unwrap_or("table")
            .to_owned();
        let base_directory = if args.flatten {
            PathBuf::new()
        } else {
            source
                .relative
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default()
        };

        let mut set =
            match formats::read_tables(&source.source, &source.bytes, source.format, &read_options)
            {
                Ok(set) => set,
                Err(Error::Unsupported(message)) => {
                    events.push(report_unsupported(&source.source, &message, diagnostics));
                    skipped += 1;
                    continue;
                }
                Err(error) => return Err(error),
            };

        // Delimited and JSON readers derive a table name from the source path.
        // For ZIP members that path is synthetic, so use the safe member stem
        // instead. Workbook sheet names and HTML table names stay untouched.
        if matches!(
            source.format,
            FormatId::Csv | FormatId::Tsv | FormatId::Json
        ) {
            for table in &mut set.tables {
                table.name.clone_from(&stem);
            }
        }

        if set.tables.is_empty() {
            return Err(Error::data(format!(
                "{}: no tables were selected",
                source.source.path.display()
            )));
        }

        converted_inputs += 1;
        let tables = if args.stack {
            vec![stack_tables(set.tables, args.add_source_column.as_deref())?]
        } else {
            set.tables
        };

        for mut table in tables {
            let uses_table_name = template.contains("{sheet}") || template.contains("{name}");
            let sanitized = if uses_table_name {
                sanitize_sheet_name(&table.name)
            } else {
                table.name.clone()
            };
            if sanitized != table.name {
                table.push_warning(
                    LossEvent::info(
                        "output.sheet_name_sanitized",
                        format!(
                            "table name '{}' was sanitized to '{}' for the output filename",
                            table.name, sanitized
                        ),
                    )
                    .with_source(table.source.clone()),
                );
            }

            let rendered =
                render_name_template(template, &stem, &sanitized, output_format, plans.len() + 1)?;
            let relative = base_directory.join(Path::new(&rendered));
            let relative = unique_relative_path(&relative, &mut used);
            if relative != base_directory.join(Path::new(&rendered)) {
                table.push_warning(
                    LossEvent::info(
                        "output.sheet_name_sanitized",
                        format!(
                            "output name '{}' collided with an earlier table; wrote '{}'",
                            rendered,
                            relative.display()
                        ),
                    )
                    .with_source(table.source.clone()),
                );
            }

            plans.push(PlannedTable {
                table,
                relative,
                format: output_format,
            });
        }
    }

    if plans.is_empty() {
        return Err(Error::data(format!(
            "no supported tables were found in '{input}'"
        )));
    }

    for plan in &mut plans {
        report::push_delimited_output_warnings(&mut plan.table, plan.format, &args.write.null);
    }
    events.extend(
        plans
            .iter()
            .flat_map(|plan| plan.table.warnings.iter().cloned()),
    );

    let destinations = plans
        .iter()
        .map(|plan| batch_destination(Path::new(output), &plan.relative))
        .collect::<Result<Vec<_>>>()?;
    let protected = destinations
        .iter()
        .map(|destination| ("output", destination.as_path()))
        .collect::<Vec<_>>();
    context.validate_paths(&protected)?;

    // Print every source diagnostic before any output appears. This includes
    // workbook fidelity events when the user asked for them with
    // --fidelity-report.
    let include_fidelity = diagnostics.fidelity_report || args.reporting.strict;
    if context.aggregates_diagnostics(diagnostics) {
        context.print_event_summary(
            &events,
            diagnostics,
            DiagnosticsScope::Write,
            include_fidelity,
        );
    } else {
        for plan in &plans {
            print_warnings(
                &plan.table,
                diagnostics,
                DiagnosticsScope::Write,
                include_fidelity,
            );
        }
    }
    context.check_strict(&events)?;

    if args.write.dry_run {
        for (plan, destination) in plans.iter().zip(&destinations) {
            print_dry_run(&destination.display().to_string(), &plan.table, plan.format);
        }
        if diagnostics.enabled() {
            eprintln!(
                "tabmux: dry-run: {}",
                batch_summary(plans.len(), converted_inputs, skipped, true)
            );
        }
        context.write_loss(&events, &[])?;
        context.print_dry_run_reports(&events, diagnostics);
        return Ok(());
    }

    std::fs::create_dir_all(output)?;
    let mut options = formats::WriteOptions::for_format(output_format);
    options.has_header = !args.read.no_header;
    options.line_ending = args.write.line_ending;
    options.null = args.write.null.clone();

    // Stage every output before committing any of them. A read or write error
    // in the middle of the batch drops the temporary files instead of leaving
    // a half-written result set behind.
    let mut staged = Vec::with_capacity(plans.len());
    for (plan, destination) in plans.iter().zip(&destinations) {
        let mut writer = SafeWriter::create(destination, args.write.force, args.write.backup)?;
        formats::write_table(&mut writer, &plan.table, plan.format, &options)?;
        staged.push((destination.clone(), writer));
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
                    "batch commit failed for '{}': {error}; committed outputs: {completed}",
                    destination.display()
                )));
            }
        }
    }

    let mut outputs = Vec::with_capacity(plans.len());
    for (plan, destination) in plans.iter().zip(&destinations) {
        context.verify_file(
            destination,
            plan.format,
            std::slice::from_ref(&plan.table),
            &read_options,
            &args.write.null,
        )?;
        outputs.push(output_artifact(destination, plan.format)?);
    }
    context.write_loss(&events, &outputs)?;
    context.write_manifest(
        &events,
        &outputs,
        batch_manifest_options(args, output_format),
    )?;

    if diagnostics.enabled() {
        eprintln!(
            "tabmux: {}{}",
            batch_summary(written.len(), converted_inputs, skipped, false),
            context.summary_suffix(&events)
        );
    }
    Ok(())
}

fn validate_batch_args(input: &str, output: &str, args: &ConvertArgs) -> Result<()> {
    if output == "-" {
        return Err(Error::usage(
            "batch conversion requires an output directory, not stdout",
        ));
    }
    if Path::new(output).exists() && !Path::new(output).is_dir() {
        return Err(Error::usage(format!(
            "batch output must be a directory, but '{output}' is a file"
        )));
    }
    if Path::new(input).is_dir()
        && let (Ok(input_root), Ok(output_root)) =
            (std::fs::canonicalize(input), std::fs::canonicalize(output))
        && output_root.starts_with(&input_root)
    {
        return Err(Error::usage(
            "batch output directory must not be inside the batch input directory",
        ));
    }
    if args.split_sheets {
        return Err(Error::usage(
            "--split-sheets is not supported for a batch input; use --all-sheets to write one file per sheet",
        ));
    }
    if args.sheet_per_table {
        return Err(Error::usage(
            "--sheet-per-table requires a workbook output and cannot be used for batch conversion",
        ));
    }
    if args.index_sheet {
        return Err(Error::usage(
            "--index-sheet requires a workbook output and cannot be used for batch conversion",
        ));
    }
    if args.sheet_per_input || args.sheet_name.is_some() || args.sheet_name_template.is_some() {
        return Err(Error::usage(
            "--sheet-per-input, --sheet-name, and --sheet-name-template require a workbook output",
        ));
    }
    if args.add_source_column.is_some() && !args.stack {
        return Err(Error::usage("--add-source-column requires --stack"));
    }
    if input.is_empty() {
        return Err(Error::usage("missing batch input path"));
    }
    Ok(())
}

struct PlannedTable {
    table: Table,
    relative: PathBuf,
    format: FormatId,
}

fn batch_destination(root: &Path, relative: &Path) -> Result<PathBuf> {
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(Error::usage(format!(
            "batch output path escapes the output directory: {}",
            relative.display()
        )));
    }
    Ok(root.join(relative))
}

fn report_unsupported(
    source: &SourceRef,
    reason: &str,
    diagnostics: &DiagnosticsPolicy,
) -> LossEvent {
    let event =
        LossEvent::warning("format.unsupported", reason.to_owned()).with_source(source.clone());
    if diagnostics.shows(&event, DiagnosticsScope::Write, false) {
        print_event(&event);
    }
    event
}

fn batch_manifest_options(args: &ConvertArgs, format: FormatId) -> serde_json::Value {
    serde_json::json!({
        "command": "convert",
        "mode": "batch",
        "output_format": format.label(),
        "strict": args.reporting.strict,
        "lossy": args.reporting.lossy,
        "max_warnings": args.reporting.max_warnings,
        "verify": args.reporting.verify,
        "force": args.write.force,
        "backup": args.write.backup,
        "dry_run": args.write.dry_run,
        "recursive": args.recursive,
        "include": &args.include,
        "exclude": &args.exclude,
        "flatten": args.flatten,
        "all_sheets": args.read.sheets.all_sheets,
    })
}

fn batch_summary(outputs: usize, inputs: usize, skipped: usize, dry_run: bool) -> String {
    let verb = if dry_run { "would write" } else { "wrote" };
    let mut summary = format!("{verb} {outputs} output(s) from {inputs} supported input(s)");
    if skipped > 0 {
        summary.push_str(&format!("; skipped {skipped} unsupported input(s)"));
    }
    summary
}
