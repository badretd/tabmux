# Loss report and provenance manifest

> **Status:** shipped in Phase 9. Both documents are versioned and
> deterministic apart from the manifest's timestamp and duration.

## When reports are written

Every conversion builds a loss report in memory. The report is written to disk
only when `--report PATH` or `--report-markdown PATH` is passed. A lossless
conversion still produces a report with an empty `events` array.

`--strict` applies the loss policy before any data output is opened:

- any `error` event fails with exit code 5;
- more than `--max-warnings N` (default `0`) warning occurrences fail with exit
  code 5.

When strict mode blocks a conversion, the loss report is still written if
`--report` was requested, so the user can see why. No data output is created.
`--lossy` (or `--no-strict` for `run`) explicitly allows warnings.

`--manifest PATH` writes the provenance manifest after a successful write.
`--verify` re-reads each output and compares columns, row counts, and rendered
cell values with the expected table.

## Post-write summary

Successful write commands print one summary line. It names requested report and
manifest files, the lossy category/item counts, and the strict outcome:

```text
tabmux: wrote 2 sheet(s): Rich, Pivot; loss report: /tmp/loss.json (7 categories); manifest: /tmp/manifest.json; loss severity: lossy (7 categories, 8 items; --strict would exit 5)
```

- **Categories** are distinct lossy event codes.
- **Items** are lossy warning/error occurrences. Global workbook properties
  such as styles, macros, and pivot caches are printed as `styles(dropped)`,
  `macros(dropped)`, and `pivot_caches(dropped)` rather than with a numeric
  count.
- `--strict` has an explicit whitelist of normal transformations that do not
  fail strict mode: `excel.pivot_values_materialized`, merged-cell expansion
  (`excel.merged_cell_expanded`, `html.merged_cell_expanded`), and cached
  formula values (`excel.formula_cached_value_only`). They are still reported,
  but they are not lossy items and do not count toward `--max-warnings`.
- Severity labels are `lossless`, `approximate` (only non-lossy/info events),
  and `lossy`. The strict outcome is one of `--strict would pass`,
  `--strict would exit 5`, or `--strict did not block (max warnings N)`.

Dry-runs confirm requested report files separately and never claim a manifest.

## Diagnostics display matrix

When a report is requested, summary-mode stderr is aggregated and points at the
report; `--verbose` prints every event. Without a report, the normal summary
matrix applies:

| Event class | CSV/TSV write | XLSX/ODS write | Read-only |
|---|---|---|---|
| Workbook-only fidelity (`chart_dropped`, `style_dropped`, `macro_dropped`, pivot cache/values, comments, conditional formatting, data validation, merged-cell expansion) | hidden | shown | hidden unless `--fidelity-report` |
| Value-affecting fidelity (`formula_cached_value_only`, future precision loss) | shown | shown | hidden unless `--fidelity-report` |
| Encoding replacement | shown | shown | shown |
| Other warnings/errors | shown | shown | shown |
| Informational output events | shown | shown | hidden |

`--verbose`/`--debug` overrides the matrix and prints every event. This keeps
CSV conversions honest about formula/encoding loss without flooding stderr with
workbook features that do not apply to the target format.

## Loss report (`--report`)

```json
{
  "version": 1,
  "tool": { "name": "tabmux", "version": "0.1.0", "git": "af7e209" },
  "inputs": [
    { "path": "fixtures/rich.xlsx", "format": "xlsx", "bytes": 4171, "sha256": "..." }
  ],
  "outputs": [
    { "path": "out.csv", "format": "csv", "bytes": 512, "sha256": "..." }
  ],
  "events": [
    {
      "severity": "warning",
      "code": "excel.chart_dropped",
      "message": "the workbook contains 1 chart(s); charts are not copied",
      "source": { "path": "fixtures/rich.xlsx", "format": "xlsx" },
      "count": 1
    }
  ],
  "summary": { "info": 3, "warning": 7, "error": 0 }
}
```

- `version` is the report schema version (`1`).
- `events` are aggregated by `(severity, code, message)`. `count` is the number
  of occurrences; the first source reference is retained.
- `summary` counts occurrences by severity, not distinct event records.
- `source` is omitted when an event is not tied to a file.
- Event order is deterministic: error, warning, info; then code, message, and
  source.

The machine-readable schema is in [`loss-report.schema.json`](./loss-report.schema.json).
A checked-in golden report lives in `fixtures/reports/sample-report.json`.

## Provenance manifest (`--manifest`)

```json
{
  "version": 1,
  "tool": { "name": "tabmux", "version": "0.1.0", "git": "af7e209" },
  "started_at": "2026-11-04T12:00:00Z",
  "duration_ms": 12,
  "command": ["tabmux", "convert", "book.xlsx", "out.xlsx"],
  "options": { "strict": false, "force": true },
  "inputs": [
    { "path": "book.xlsx", "format": "xlsx", "bytes": 12345, "sha256": "..." }
  ],
  "outputs": [
    { "path": "out.xlsx", "format": "xlsx", "bytes": 23456, "sha256": "..." }
  ],
  "loss_summary": { "info": 0, "warning": 2, "error": 0 },
  "deterministic": true
}
```

- `command` is the actual argv used for the run.
- `started_at` is RFC 3339 UTC with second precision; `duration_ms` is wall
  time for the conversion.
- Inputs and outputs carry byte counts and SHA-256 checksums. Stdin has no
  stable checksum and is omitted.
- `deterministic` is `true`: no timestamp or random value enters the data
  output.
- The manifest itself is not byte-identical across runs because of
  `started_at` and `duration_ms`; the data outputs are.

## Markdown report (`--report-markdown`)

The Markdown renderer prints a summary table and one row per aggregated event.
It is intended for humans; use the JSON report for automation.

## Event codes

The stable code list is maintained in the development plan. Common codes:

| Code | Meaning |
|---|---|
| `excel.chart_dropped` | Charts were not copied |
| `excel.style_dropped` | Cell styles/number formats were not preserved |
| `excel.formula_cached_value_only` | A formula was not recalculated |
| `excel.merged_cell_expanded` | A merged range was expanded |
| `excel.macro_dropped` | A VBA project was not copied |
| `excel.pivot_cache_dropped` | Pivot caches were not copied |
| `excel.comment_dropped` | Comments were not copied |
| `excel.conditional_formatting_dropped` | Conditional formatting was not copied |
| `excel.data_validation_dropped` | Data validation was not copied |
| `html.table_skipped` | A layout table was skipped |
| `html.merged_cell_expanded` | A colspan/rowspan cell was expanded |
| `html.header_inferred` | The header row was inferred |
| `html.sheet_name_collision` | Two HTML table names collided |
| `csv.ragged_row` | A row had the wrong field count and was padded/skipped |
| `csv.bad_quote` | The input ends inside an unterminated quoted field |
| `csv.encoding_replaced` | Invalid bytes were replaced during decoding |
| `json.flattened` | A nested object was flattened to `a.b` |
| `archive.member_skipped` | A batch member was skipped |
| `format.unsupported` | A file or format is unsupported |
| `output.sheet_name_sanitized` | An output sheet/file name changed |
| `output.null_rendered_as_empty` | A null was rendered as empty text |
