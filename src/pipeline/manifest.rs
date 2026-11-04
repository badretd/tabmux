//! Declarative pipeline manifest (`pipeline.toml`).
//!
//! The manifest is configuration, not a table output. It describes named
//! inputs, a linear chain of table operations, and named outputs. Validation
//! happens entirely before any output writer is opened.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::formats::FormatId;
use crate::ops::rename;

/// Only manifest schema version understood by this build.
pub const MANIFEST_VERSION: u32 = 1;

/// A parsed and structurally validated pipeline manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Manifest schema version. Must be [`MANIFEST_VERSION`].
    pub version: u32,

    /// Global execution options.
    #[serde(default)]
    pub options: Options,

    /// Named inputs available to the pipeline.
    #[serde(default)]
    pub inputs: Vec<InputSpec>,

    /// Linear chain of table operations.
    #[serde(default)]
    pub steps: Vec<Step>,

    /// Named outputs written by the pipeline.
    #[serde(default)]
    pub outputs: Vec<OutputSpec>,
}

/// Global options that apply to the whole run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    /// Fail before writing when any warning or error event is produced.
    pub strict: bool,
    /// Overwrite existing outputs. The CLI can override this value.
    pub force: bool,
    /// Copy an existing output to a timestamped `.bak` before replacing it.
    pub backup: bool,
}

/// One named input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputSpec {
    /// Stable name used by steps and outputs.
    pub id: String,
    /// Source path. `-` reads stdin, in which case `format` is required.
    pub path: String,

    /// Workbook sheet or HTML table selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<SheetSpec>,

    /// Explicit input format. When absent, the path extension is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,

    /// Whether the first row is a header. Defaults to `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<bool>,
}

/// Sheet/table selector in a manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SheetSpec {
    /// 1-based sheet/table index.
    Index(usize),
    /// Exact sheet/table name.
    Name(String),
}

impl SheetSpec {
    /// Stable label used in the resolved plan.
    pub fn label(&self) -> String {
        match self {
            Self::Index(index) => format!("#{index}"),
            Self::Name(name) => name.clone(),
        }
    }
}

/// Operations available in a linear pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    Select,
    Filter,
    Sort,
    Rename,
    Dedupe,
    Merge,
    Concat,
}

impl Op {
    /// Stable operation name used in reports and errors.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Select => "select",
            Self::Filter => "filter",
            Self::Sort => "sort",
            Self::Rename => "rename",
            Self::Dedupe => "dedupe",
            Self::Merge => "merge",
            Self::Concat => "concat",
        }
    }

    /// Convenience result alias published by this operation.
    ///
    /// For example, a `select` step publishes `selected`, so a later step can
    /// refer to it without an explicit `id`.
    pub const fn alias(self) -> &'static str {
        match self {
            Self::Select => "selected",
            Self::Filter => "filtered",
            Self::Sort => "sorted",
            Self::Rename => "renamed",
            Self::Dedupe => "deduped",
            Self::Merge => "merged",
            Self::Concat => "concatenated",
        }
    }
}

/// Schema policy for a merge/concat step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeModeSpec {
    /// Column names, order, and compatible types must match.
    #[default]
    Strict,
    /// Union columns by name and fill missing cells with null.
    Loose,
}

impl MergeModeSpec {
    const fn is_default(&self) -> bool {
        matches!(self, Self::Strict)
    }
}

/// One operation in the linear chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// Operation to run.
    pub op: Op,

    /// Optional explicit result name. Defaults to `step-N`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// Primary input result name. Defaults to the previous step or the only input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,

    /// `select` columns (a comma string or an array of names/indices).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<StringList>,

    /// `filter` expression, for example `amount > 100`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expr: Option<String>,

    /// `sort` keys (a comma string or an array; each key may end in `:desc`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<StringList>,

    /// `rename` mappings (`old=new` string or a TOML table).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map: Option<RenameMap>,

    /// Secondary input result name for `merge`/`concat`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub with: Option<String>,

    /// Schema policy for `merge`/`concat`.
    #[serde(default, skip_serializing_if = "MergeModeSpec::is_default")]
    pub mode: MergeModeSpec,

    /// Append a provenance column with this name for `merge`/`concat`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add_source_column: Option<String>,

    /// Drop exact duplicate rows in `merge`/`concat`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub dedup: bool,

    /// Match column names without regard to case in `merge`/`concat`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub case_insensitive_headers: bool,
}

const fn is_false(value: &bool) -> bool {
    !*value
}

impl Step {
    /// Primary result name (`id` or `step-N`).
    pub fn result_name(&self, index: usize) -> String {
        self.id
            .clone()
            .unwrap_or_else(|| format!("step-{}", index + 1))
    }

    /// Validate that the fields present are meaningful for this operation.
    fn validate_shape(&self) -> Result<()> {
        if let Some(id) = &self.id
            && id.trim().is_empty()
        {
            return Err(Error::usage(
                "pipeline step id must not be empty (or omit id to use step-N)",
            ));
        }
        if let Some(input) = &self.input
            && input.trim().is_empty()
        {
            return Err(Error::usage("pipeline step input must not be empty"));
        }

        let has = |value: &Option<StringList>| value.is_some();
        let only = |allowed: &[&str]| -> Result<()> {
            let mut present = Vec::new();
            if has(&self.columns) {
                present.push("columns");
            }
            if self.expr.is_some() {
                present.push("expr");
            }
            if has(&self.by) {
                present.push("by");
            }
            if self.map.is_some() {
                present.push("map");
            }
            if self.with.is_some() {
                present.push("with");
            }
            if self.add_source_column.is_some() {
                present.push("add_source_column");
            }
            if self.dedup {
                present.push("dedup");
            }
            if self.case_insensitive_headers {
                present.push("case_insensitive_headers");
            }
            if !self.mode.is_default() {
                present.push("mode");
            }

            for field in present {
                if !allowed.contains(&field) {
                    return Err(Error::usage(format!(
                        "pipeline op '{}' does not accept '{field}'",
                        self.op.label()
                    )));
                }
            }
            Ok(())
        };

        match self.op {
            Op::Select => {
                only(&["columns"])?;
                let columns = self
                    .columns
                    .as_ref()
                    .ok_or_else(|| Error::usage("pipeline select step requires 'columns'"))?;
                if columns.to_vec().is_empty() {
                    return Err(Error::usage(
                        "pipeline select step requires at least one column",
                    ));
                }
            }
            Op::Filter => {
                only(&["expr"])?;
                let expr = self
                    .expr
                    .as_deref()
                    .ok_or_else(|| Error::usage("pipeline filter step requires 'expr'"))?;
                if expr.trim().is_empty() {
                    return Err(Error::usage(
                        "pipeline filter step requires a non-empty 'expr'",
                    ));
                }
            }
            Op::Sort => {
                only(&["by"])?;
                let by = self
                    .by
                    .as_ref()
                    .ok_or_else(|| Error::usage("pipeline sort step requires 'by'"))?;
                if by.to_vec().is_empty() {
                    return Err(Error::usage("pipeline sort step requires at least one key"));
                }
            }
            Op::Rename => {
                only(&["map"])?;
                let map = self
                    .map
                    .as_ref()
                    .ok_or_else(|| Error::usage("pipeline rename step requires 'map'"))?;
                if map.to_pairs()?.is_empty() {
                    return Err(Error::usage(
                        "pipeline rename step requires at least one mapping",
                    ));
                }
            }
            Op::Dedupe => {
                only(&[])?;
            }
            Op::Merge | Op::Concat => {
                only(&[
                    "with",
                    "mode",
                    "add_source_column",
                    "dedup",
                    "case_insensitive_headers",
                ])?;
                let with = self.with.as_deref().ok_or_else(|| {
                    Error::usage(format!(
                        "pipeline {} step requires 'with' (the secondary input)",
                        self.op.label()
                    ))
                })?;
                if with.trim().is_empty() {
                    return Err(Error::usage(format!(
                        "pipeline {} step requires a non-empty 'with'",
                        self.op.label()
                    )));
                }
                if let Some(column) = &self.add_source_column
                    && column.trim().is_empty()
                {
                    return Err(Error::usage("pipeline add_source_column must not be empty"));
                }
            }
        }

        Ok(())
    }
}

/// A comma-separated string or an array of strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StringList {
    Text(String),
    List(Vec<String>),
}

impl StringList {
    /// Split the text form on commas and trim all entries.
    pub fn to_vec(&self) -> Vec<String> {
        match self {
            Self::Text(text) => text
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect(),
            Self::List(values) => values
                .iter()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect(),
        }
    }
}

/// Rename mappings in either `old=new,other->name` text or table form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RenameMap {
    Text(String),
    Table(BTreeMap<String, String>),
}

impl RenameMap {
    /// Convert to ordered `(from, to)` pairs.
    pub fn to_pairs(&self) -> Result<Vec<(String, String)>> {
        match self {
            Self::Text(text) => Ok(rename::parse_mappings(text)?
                .into_iter()
                .map(|mapping| (mapping.from, mapping.to))
                .collect()),
            Self::Table(map) => Ok(map
                .iter()
                .map(|(from, to)| (from.clone(), to.clone()))
                .collect()),
        }
    }
}

/// One named output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputSpec {
    /// Optional output label used only in the resolved plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// Result name to write. Defaults to the final step result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,

    /// Destination path, or `-` for stdout.
    pub path: String,

    /// Explicit output format. When absent, the path extension is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,

    /// Worksheet name for a single-sheet workbook output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<String>,

    /// Whether to write a header for delimited output. Defaults to `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<bool>,
}

impl Manifest {
    /// Parse and validate a manifest from TOML text.
    pub fn parse(text: &str) -> Result<Self> {
        let manifest: Self = toml::from_str(text)
            .map_err(|error| Error::usage(format!("invalid pipeline manifest: {error}")))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Read and validate a manifest from disk.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|error| {
            Error::data(format!(
                "cannot read pipeline manifest '{}': {error}",
                path.display()
            ))
        })?;
        Self::parse(&text)
    }

    /// Serialize back to TOML. Used by round-trip tests and future manifests.
    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self)
            .map_err(|error| Error::data(format!("cannot serialize pipeline manifest: {error}")))
    }

    /// Validate structure and references that can be checked without I/O.
    pub fn validate(&self) -> Result<()> {
        if self.version != MANIFEST_VERSION {
            return Err(Error::usage(format!(
                "unsupported pipeline manifest version {} (this build supports version {MANIFEST_VERSION})",
                self.version
            )));
        }
        if self.inputs.is_empty() {
            return Err(Error::usage(
                "pipeline manifest must declare at least one [[inputs]] entry",
            ));
        }
        if self.outputs.is_empty() {
            return Err(Error::usage(
                "pipeline manifest must declare at least one [[outputs]] entry",
            ));
        }

        let mut input_ids = HashSet::<String>::new();
        for input in &self.inputs {
            if input.id.trim().is_empty() {
                return Err(Error::usage("pipeline input id must not be empty"));
            }
            if input.path.trim().is_empty() {
                return Err(Error::usage(format!(
                    "pipeline input '{}' must not have an empty path",
                    input.id
                )));
            }
            if !input_ids.insert(input.id.clone()) {
                return Err(Error::usage(format!(
                    "duplicate pipeline input id '{}'",
                    input.id
                )));
            }
            if let Some(format) = &input.format {
                parse_input_format(format)?;
            }
            match &input.sheet {
                Some(SheetSpec::Index(index)) if *index == 0 => {
                    return Err(Error::usage(
                        "pipeline sheet indices are 1-based; 0 is not valid",
                    ));
                }
                Some(SheetSpec::Name(name)) if name.trim().is_empty() => {
                    return Err(Error::usage(format!(
                        "pipeline input '{}' has an empty sheet name",
                        input.id
                    )));
                }
                _ => {}
            }
        }

        let mut output_ids = HashSet::<String>::new();
        let mut output_paths = HashSet::<String>::new();
        let mut stdout_outputs = 0usize;
        for output in &self.outputs {
            if output.path.trim().is_empty() {
                return Err(Error::usage("pipeline output path must not be empty"));
            }
            if let Some(id) = &output.id {
                if id.trim().is_empty() {
                    return Err(Error::usage("pipeline output id must not be empty"));
                }
                if !output_ids.insert(id.clone()) {
                    return Err(Error::usage(format!("duplicate pipeline output id '{id}'")));
                }
            }
            if !output_paths.insert(output.path.clone()) {
                return Err(Error::usage(format!(
                    "duplicate pipeline output path '{}'",
                    output.path
                )));
            }
            if output.path == "-" {
                stdout_outputs += 1;
            }
            if let Some(format) = &output.format {
                parse_output_format(format)?;
            }
            if let Some(sheet) = &output.sheet
                && sheet.trim().is_empty()
            {
                return Err(Error::usage(format!(
                    "pipeline output '{}' has an empty sheet name",
                    output.path
                )));
            }
        }
        if stdout_outputs > 1 {
            return Err(Error::usage(
                "a pipeline may write at most one output to stdout",
            ));
        }

        let mut primary_names = input_ids.clone();
        for (index, step) in self.steps.iter().enumerate() {
            step.validate_shape()?;
            let primary = step.result_name(index);
            if !primary_names.insert(primary.clone()) {
                return Err(Error::usage(format!(
                    "pipeline result name '{primary}' is declared more than once; give the step an explicit id"
                )));
            }
        }

        // Derived aliases are a convenience. They may be reused by later steps
        // of the same kind, but must not shadow an input or a primary result
        // name (which would make references ambiguous).
        for (index, step) in self.steps.iter().enumerate() {
            let alias = step.op.alias();
            let primary = step.result_name(index);
            if alias != primary && primary_names.contains(alias) {
                return Err(Error::usage(format!(
                    "pipeline step alias '{alias}' collides with an input or step result name; rename the input or give the step an explicit id"
                )));
            }
        }

        Ok(())
    }
}

/// Parse an input format name using the same aliases as the CLI.
pub fn parse_input_format(name: &str) -> Result<FormatId> {
    FormatId::from_extension(name).ok_or_else(|| {
        Error::unsupported(format!(
            "unsupported input format '{name}' (supported: csv, tsv, xlsx, xls, xlsb, ods, html, json, jsonl)"
        ))
    })
}

/// Parse an output format name. Only formats with a writer are accepted.
pub fn parse_output_format(name: &str) -> Result<FormatId> {
    let normalized = name.trim().trim_start_matches('.').to_ascii_lowercase();
    match normalized.as_str() {
        "csv" => Ok(FormatId::Csv),
        "tsv" | "tab" => Ok(FormatId::Tsv),
        "xlsx" => Ok(FormatId::Xlsx),
        "ods" => Ok(FormatId::Ods),
        "json" | "jsonl" | "ndjson" | "html" | "htm" => Err(Error::unsupported(format!(
            "writing {normalized} is not implemented yet"
        ))),
        "xls" | "xlsb" | "xlsm" => Err(Error::unsupported(
            "writing legacy or macro-enabled Excel containers is not supported; use .xlsx",
        )),
        _ => Err(Error::unsupported(format!(
            "unsupported output format '{name}' (supported: csv, tsv, xlsx, ods)"
        ))),
    }
}

/// Infer an output format from a destination path.
pub fn output_format_from_path(path: &str) -> Result<FormatId> {
    if path == "-" {
        return Err(Error::usage(
            "stdout output requires an explicit format = \"csv\" or \"tsv\"",
        ));
    }

    let extension = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .ok_or_else(|| {
            Error::unsupported(format!(
                "cannot detect output format for '{path}' (use format = \"csv\", \"tsv\", \"xlsx\", or \"ods\")"
            ))
        })?;
    parse_output_format(extension)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
version = 1

[options]
force = true

[[inputs]]
id = "sales"
path = "fixtures/sales.xlsx"
sheet = "Sales"

[[inputs]]
id = "extra"
path = "fixtures/a.csv"
header = false

[[steps]]
op = "select"
input = "sales"
columns = ["id", "email", "amount"]

[[steps]]
op = "filter"
input = "selected"
expr = "amount > 100"

[[outputs]]
id = "csv"
input = "filtered"
path = "/tmp/pipeline.csv"
format = "csv"

[[outputs]]
id = "xlsx"
input = "filtered"
path = "/tmp/pipeline.xlsx"
format = "xlsx"
sheet = "Filtered"
"#;

    #[test]
    fn parses_and_round_trips_the_documented_manifest() {
        let manifest = Manifest::parse(EXAMPLE).unwrap();
        assert_eq!(manifest.version, 1);
        assert!(manifest.options.force);
        assert_eq!(manifest.inputs.len(), 2);
        assert_eq!(
            manifest.inputs[0].sheet,
            Some(SheetSpec::Name("Sales".into()))
        );
        assert_eq!(manifest.inputs[1].header, Some(false));
        assert_eq!(manifest.steps.len(), 2);
        assert_eq!(manifest.outputs.len(), 2);

        let serialized = manifest.to_toml().unwrap();
        let reparsed = Manifest::parse(&serialized).unwrap();
        assert_eq!(manifest, reparsed);
    }

    #[test]
    fn rejects_unknown_ops_and_unknown_fields() {
        let unknown_op = r#"
version = 1
[[inputs]]
id = "a"
path = "a.csv"
[[steps]]
op = "explode"
[[outputs]]
path = "out.csv"
"#;
        let error = Manifest::parse(unknown_op).unwrap_err();
        assert_eq!(error.exit_code(), crate::error::ExitCode::Usage);

        let unknown_field = r#"
version = 1
[[inputs]]
id = "a"
path = "a.csv"
[[steps]]
op = "dedupe"
columns = ["id"]
[[outputs]]
path = "out.csv"
"#;
        let error = Manifest::parse(unknown_field).unwrap_err();
        assert!(error.to_string().contains("does not accept 'columns'"));
    }

    #[test]
    fn rejects_duplicate_inputs_results_and_outputs() {
        let duplicate_input = r#"
version = 1
[[inputs]]
id = "a"
path = "a.csv"
[[inputs]]
id = "a"
path = "b.csv"
[[outputs]]
path = "out.csv"
"#;
        assert!(Manifest::parse(duplicate_input).is_err());

        let duplicate_result = r#"
version = 1
[[inputs]]
id = "a"
path = "a.csv"
[[steps]]
op = "dedupe"
id = "same"
[[steps]]
op = "dedupe"
id = "same"
[[outputs]]
path = "out.csv"
"#;
        assert!(Manifest::parse(duplicate_result).is_err());

        let duplicate_output = r#"
version = 1
[[inputs]]
id = "a"
path = "a.csv"
[[outputs]]
path = "out.csv"
[[outputs]]
path = "out.csv"
"#;
        assert!(Manifest::parse(duplicate_output).is_err());
    }

    #[test]
    fn rejects_missing_required_op_fields() {
        let cases = [
            ("select", ""),
            ("filter", ""),
            ("sort", ""),
            ("rename", ""),
            ("merge", ""),
        ];
        for (op, argument) in cases {
            let text = format!(
                "version = 1\n[[inputs]]\nid = \"a\"\npath = \"a.csv\"\n[[steps]]\nop = \"{op}\"\n{argument}\n[[outputs]]\npath = \"out.csv\"\n"
            );
            assert!(
                Manifest::parse(&text).is_err(),
                "expected {op} without required fields to fail"
            );
        }
    }

    #[test]
    fn rejects_alias_collisions_with_input_ids() {
        let text = r#"
version = 1
[[inputs]]
id = "selected"
path = "a.csv"
[[steps]]
op = "select"
columns = ["id"]
[[outputs]]
path = "out.csv"
"#;
        let error = Manifest::parse(text).unwrap_err();
        assert!(error.to_string().contains("alias 'selected'"));
    }

    #[test]
    fn accepts_merge_and_concat_steps() {
        for op in ["merge", "concat"] {
            let text = format!(
                r#"
version = 1
[[inputs]]
id = "a"
path = "a.csv"
[[inputs]]
id = "b"
path = "b.csv"
[[steps]]
op = "{op}"
input = "a"
with = "b"
mode = "loose"
add_source_column = "origin"
dedup = true
case_insensitive_headers = true
[[outputs]]
input = "concatenated"
path = "out.csv"
"#
            );
            let manifest = Manifest::parse(&text).unwrap();
            assert_eq!(
                manifest.steps[0].op,
                if op == "merge" { Op::Merge } else { Op::Concat }
            );
            assert_eq!(manifest.steps[0].with.as_deref(), Some("b"));
            assert_eq!(manifest.steps[0].mode, MergeModeSpec::Loose);
            assert_eq!(
                manifest.steps[0].add_source_column.as_deref(),
                Some("origin")
            );
            assert!(manifest.steps[0].dedup);
            assert!(manifest.steps[0].case_insensitive_headers);
        }
    }

    #[test]
    fn accepts_no_step_pipelines() {
        let text = r#"
version = 1
[[inputs]]
id = "a"
path = "a.csv"
[[outputs]]
path = "out.csv"
"#;
        let manifest = Manifest::parse(text).unwrap();
        assert!(manifest.steps.is_empty());
    }
}
