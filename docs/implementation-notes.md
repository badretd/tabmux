# Implementation notes

> **Status:** updated as phases land. This file records deliberate deviations
> from `devplan.md` so the plan and the shipped behavior stay honest.

## Phase 0 — CLI skeleton

- `clap` is pinned to `4.5.60` because the build environment resolved crates
  offline. The CLI surface is the same as the planned 4.6 line; the pin can be
  relaxed when the workspace has network access.
- CI lives in `.github/workflows/ci.yml` and runs fmt, clippy, tests, and the
  smoke script on Linux.

## Phase 1 — Core model, delimited text, JSON, safe writes

- **Encoding detection:** `chardetng` was not available offline, so the reader
  uses `encoding_rs` plus a conservative built-in detector (UTF-8 BOM, UTF-16
  BOM/heuristic, otherwise Windows-1252). `--encoding LABEL` overrides
  detection. This still satisfies the round-trip fixtures, but it is not a
  statistical charset detector; add `chardetng` when dependency updates are
  possible.
- **CSV value typing:** delimited cells are kept as `CellValue::Text` so
  conversion is byte-faithful and never reinterprets a ZIP code as a number.
  JSON values are typed (`Null`, `Bool`, `Int`, `Float`, `Decimal`, `Text`).
  CSV type inference can be added when filtering/sorting needs it (Phase 5).
- **JSON output:** only reading is in scope in Phase 1. `convert data.jsonl
  out.csv` works; writing JSON exits with code 4 (`unsupported`).
- **`--json-path`:** deferred, as the devplan allows.
- **Materialization:** v1 reads into memory. The CSV capability is marked as
  streaming in the format registry as a target, but the current implementation
  is materialized; streaming conversion is a later optimization.
- **Quoting policy:** the writer uses RFC 4180 “quote when necessary” output.
  A user-selectable quoting policy is not exposed yet.
- **Loss reporting:** warnings are printed to stderr (`csv.ragged_row`,
  `csv.encoding_replaced`, `json.flattened`). The JSON loss report file arrives
  in Phase 9.

## Phase 2 — XLSX read, sheet selection, CSV conversion

- **Calamine version:** the offline registry has `calamine 0.37.0`, not the
  planned `0.38`. The APIs used here (`workbook_auto_from_rs`,
  `worksheet_range`, `worksheet_formula`, `merge_cells_by_sheet_name`) are
  stable across both lines.
- **Legacy containers:** `.xls`, `.xlsb`, and `.xlsm` extensions are routed
  through the `xlsx` capability row because the registry describes conversion
  formats, not container generations. `calamine` auto-detects the actual
  container. Only `sales.xlsx` is covered by a checked-in fixture today.
- **Number typing:** XLSX stores all numbers as doubles. Integral floats are
  normalized to `CellValue::Int` (so ids print as `1`, not `1.0`); fractional
  values stay `Float`. Real spreadsheets still cannot distinguish `1` from
  `1.0` in the file format.
- **Dates:** cells with a date/time number format are converted to ISO-8601
  `Date`/`DateTime`/`Time` values. Duration-formatted cells use `Time` when
  they fit in one day and text otherwise.
- **Merged cells:** the value is copied to every covered empty cell and one
  `excel.merged_cell_expanded` info event is emitted per range. Non-empty
  covered cells are left untouched.
- **Formulas:** cached values are exported; each formula cell emits
  `excel.formula_cached_value_only`. Formulas are never evaluated.
- **Workbook-level loss events:** the reader scans the OOXML package for charts,
  non-default styles, VBA projects, pivot caches, comments, conditional
  formatting, and data validation. These produce workbook-level warnings on the
  first selected table. This is intentionally a shallow package scan; Phase 9
  turns the same events into the JSON loss report.
- **Sheet listing:** `tabmux sheets <workbook>` prints one sheet name per line.
- **Sheet selection:** `--sheet NAME` (exact, case-sensitive) and
  `--sheet-index N` (1-based) select one sheet. `--all-sheets` requires
  `--split-sheets` in Phase 2; stacking sheets into one output arrives with the
  Phase 3 transforms.
- **Split output:** `--split-sheets` writes one CSV per selected table using
  `{stem}.{sheet}.csv`. Names are sanitized and collisions get stable `-2`,
  `-3`, ... suffixes. Multi-output staging is sequential in this phase; full
  stage-then-commit atomicity is a Phase 4/9 hardening item.
- **Trailing empty cells:** leading/trailing empty rows and columns are trimmed.
  No warning is emitted for ordinary trimming because it does not change data.

## Phase 3 — ODS read, all-sheets/per-sheet, row/column output

- **Shared workbook reader:** XLSX and ODS now share `formats/workbook.rs`.
  `formats/xlsx.rs` and `formats/ods.rs` are thin format-specific entry points.
- **ODS merged cells:** `calamine 0.37` does not expose merged ranges for ODS.
  The reader inspects `content.xml` with `quick-xml` and expands
  `number-columns-spanned` / `number-rows-spanned` ranges itself. ODS formulas
  still use the cached value and emit `excel.formula_cached_value_only`. ODS
  `office:time-value` durations such as `PT12H30M00S` become `Time` cells;
  durations of a day or more stay text. ODS cell styles are reported as
  `excel.style_dropped`. If package inspection fails, the conversion emits
  `excel.merged_cell_unsupported` rather than silently claiming there were no
  merges.
- **Name templates:** `--split-sheets --name-template` supports `{stem}`,
  `{sheet}`, `{name}` (alias for `{sheet}`), `{format}`, `{ext}`, and
  `{index}`. Unknown placeholders, absolute paths, and `..` components are
  rejected. Collisions get stable `-2`, `-3`, ... suffixes before the
  extension, compared case-insensitively.
- **Stacking:** `--all-sheets --stack` unions columns by name in first-seen
  order and fills missing cells with null. `--add-source-column NAME` appends
  the sheet name as the last column. Strict/loose schema policy and full merge
  diagnostics arrive in Phase 5; stacking is intentionally permissive today.
- **Row/cell addressing:** `row --number` and `cell --row` count data rows
  only (1-based, header excluded). `row --where` accepts only
  `column == value` and `column != value`; the value is compared as rendered
  text and may be single- or double-quoted.
- **Raw vs pretty:** `--raw` is the default for row/cell output so it can be
  piped. `row` raw output writes data records without a header; `cell` raw
  output writes only the cell value. `--pretty` prints an aligned terminal
  table, and multiline cells become extra aligned physical lines.
- **Wide fixture:** `fixtures/wide.csv` carries commas, quotes, embedded
  newlines, and a trailing newline inside a cell for exact-addressing tests.

## Diagnostics policy

- **Default (`summary`):** read-only commands show only warnings/errors that
  affect the data being displayed. Informational events are reserved for write
  commands, and source-fidelity events are hidden.
- **Fidelity events:** `excel.chart_dropped`, `excel.style_dropped`,
  `excel.formula_cached_value_only` (formula recalculation),
  `excel.merged_cell_expanded` (merge policy), and the other dropped-workbook
  feature events are printed only when writing a workbook or when
  `--fidelity-report` is passed. Writing CSV/TSV does not count as a workbook
  save, so it stays quiet by default.
- **`--diagnostics summary|full|none`:** explicit detail control.
  `--quiet` maps to `none`; `--verbose` maps to `full`; `--debug` implies
  `full` and additionally restores per-row events for rows filtered out by
  `row --where`.
- **Row filtering:** `row --number`, `row --where`, `head`, and `cell` attach
  diagnostics only to rows that are part of the output. The `row --where`
  case filters first, then prints diagnostics for matched rows only; use
  `--debug` to see the rest.
- **Row numbering:** each `Table` carries `first_data_row`, the 1-based source
  row number of its first data row, so row-filtered diagnostics can be matched
  back to the source even after headers or leading blank rows are trimmed.

## Scope guard

Everything else follows the devplan scope: the XLSX reader is implemented, but
ODS/HTML readers, workbook writers, merge/pipeline commands, and the loss-report
file are still ahead. There is no network, and no `fs::write` in command code.
All file outputs go through `io::SafeWriter`.
