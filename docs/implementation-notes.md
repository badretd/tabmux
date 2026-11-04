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

## Scope guard

Everything else follows the devplan scope: the XLSX reader is implemented, but
ODS/HTML readers, workbook writers, merge/pipeline commands, and the loss-report
file are still ahead. There is no network, and no `fs::write` in command code.
All file outputs go through `io::SafeWriter`.
