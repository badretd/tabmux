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
  only (1-based, header excluded). `row --where` shares the Phase 5 predicate
  parser with `filter`: equality is textual and numeric ordering comparisons
  are available; values may be single- or double-quoted.
- **Raw vs pretty:** `--raw` is the default for row/cell output so it can be
  piped. `row` raw output writes data records without a header; `cell` raw
  output writes only the cell value. `--pretty` prints an aligned terminal
  table, and multiline cells become extra aligned physical lines.
- **Wide fixture:** `fixtures/wide.csv` carries commas, quotes, embedded
  newlines, and a trailing newline inside a cell for exact-addressing tests.

## Phase 4 — XLSX/ODS write, multi-sheet assembly, basic formatting

- **Writer crates:** `rust_xlsxwriter 0.102` is part of the default build via
  the `rust_xlsxwriter` feature. `formats/xlsx_writer.rs` is the normal XLSX
  writer and uses the library's `Workbook`/`Worksheet` API. A build with only
  `excel` enabled (`--no-default-features --features core,excel`) keeps the
  built-in `formats/xlsx_manual.rs` writer as a fallback for constrained
  environments. `spreadsheet-ods` is not used; ODS writing remains the built-in
  ODF writer. Both XLSX paths expose the same `formats::write_tables` contract.
- **Multi-input conversion:** `convert a.csv b.csv -o out.xlsx` writes one
  worksheet per input. Without `-o`, the final positional argument remains the
  output path, so the earlier `convert INPUT OUTPUT` form still works.
  `--sheet-per-input` is the explicit spelling of this default; it does not
  merge multiple sheets from a single workbook. Multi-input output is limited to
  workbooks until merge/concat lands in Phase 5.
- **Worksheet naming:** the default name is the source stem (CSV) or the source
  sheet name (workbook). `--sheet-name-template` supports `{stem}`, `{sheet}`,
  `{name}`, and `{index}`. `--sheet-name` (alias `--output-sheet`) renames a
  single output sheet. `--sheet` itself remains the *input* sheet selector.
- **Sheet-name rules:** names are sanitized for Excel (`[]:*?/\`, control
  characters, 31-character limit), trimmed, and deduplicated case-insensitively
  with stable `-2`, `-3`, ... suffixes. Every change emits
  `output.sheet_name_sanitized` when writing the workbook.
- **Native types:** CSV/TSV sources are all text so CSV/TSV output stays
  byte-faithful. Type inference runs only for workbook output. Integers, floats,
  booleans, ISO dates, ISO datetimes, and times become native values; leading
  zeroes (`007`) and mixed columns stay text; empty fields become null cells.
  Empty columns become `null`. `Decimal` values are written as text to preserve
  exact digits.
- **XLSX formatting:** bold header, frozen top row, autofilter on the used
  range, readable column widths, and date/datetime/time number formats. Time
  cells use the `[h]:mm:ss` format so readers classify them as times rather
  than datetimes.
- **ODS formatting:** bold header, readable column widths, and native cell
  types. Freeze panes and autofilter are XLSX-only in this phase; ODF view
  settings and database ranges are not generated. This is basic formatting, not
  style preservation.
- **Nulls and control characters:** null cells are written as empty cells.
  Text containing XML-invalid control characters is rejected with a hard error
  that includes the sheet and cell context rather than being silently dropped.
- **Split output:** `--split-sheets` remains delimited-only; passing a `.xlsx`
  or `.ods` destination with `--split-sheets` is a usage error. Multi-file
  stage-then-commit atomicity is still a Phase 9 hardening item; workbook output
  itself is a single atomic `SafeWriter` commit.

## Phase 5 — Merge/concat, schema diff, basic transforms

- **Operations module:** select/filter/sort/rename/dedupe/merge now live in
  `src/ops/` as pure functions over `Table`. The CLI keeps argument parsing,
  safe writes, and diagnostics. `row --where` uses the same predicate parser as
  `filter`, so equality semantics cannot drift between commands.
- **Merge modes:** `--mode strict` compares every input against the first. It
  requires the same column names in the same order and rejects incompatible
  types (any pair whose merged type would fall back to `text`). Compatible
  widenings such as `int` + `float` are allowed. `--mode loose` unions columns
  by name in first-seen order and fills missing cells with null.
- **Schema diff:** both modes share `ops::schema`. On a strict failure the
  command prints a column-level diff to stderr and exits 1 before opening an
  output writer. `--json` writes the same diff as one machine-readable JSON
  document on stdout. Delimited sources keep text cells, so their diff types
  are inferred conservatively from the values; workbook and JSON sources use
  their native types.
- **Case-insensitive headers:** `--case-insensitive-headers` lowercases header
  names for matching and keeps the first input's spelling in the output. It is
  opt-in; the default remains exact and order-sensitive.
- **Provenance and dedup:** `--add-source-column NAME` (bare flag uses
  `source`) appends the input file name, plus `#sheet` for workbook inputs.
  `--dedup` runs on the merged data rows before the provenance column is
  appended, so identical records from different inputs collapse and keep the
  first input's provenance.
- **Merge outputs:** `-o` defaults to stdout. CSV/TSV and workbook destinations
  are accepted; a workbook gets one sheet named after the first input. Strict
  validation and `--dry-run` happen before `SafeWriter::create`.
- **Transforms:** `filter`, `sort`, `rename`, and `dedupe` write delimited rows
  to stdout so they compose with pipes. `filter` compares `==`/`!=` as text and
  numeric ordering (`>`, `>=`, `<`, `<=`) when the right operand is a number;
  non-numeric values in a numeric comparison fail with file/row/column context.
  `sort` supports comma-separated keys and `:asc`/`:desc`, is stable, and keeps
  nulls last in both directions. `rename` accepts `old=new` and `old->new`.
  `dedupe` keeps the first exact row, with cell type and value length included
  in the row key.
- **Fixtures:** `c.csv` covers missing/reordered headers and `d.csv` covers a
  type conflict. Both are checked in and covered by integration tests and the
  smoke script.

## Phase 6 — HTML table ingestion, HTML → workbook

- **Parser:** `scraper` was not available in the offline registry, so
  `formats/html.rs` contains a small, dependency-free, tolerant HTML tokenizer
  and table parser. It handles comments, doctypes, quoted/unquoted attributes,
  character references, `<caption>`, `<thead>`/`<tbody>`/`<tfoot>`, `<tr>`,
  `<td>`/`<th>`, `colspan`, `rowspan`, `<br>`, and script/style suppression. It
  does not execute scripts or load external resources. The `html` feature is
  therefore a no-dependency feature and is part of the default build.
- **Nested tables:** nested `<table>` elements are extracted as separate tables
  in document order. The outer cell keeps a `[table N]` marker and the outer
  table emits a warning (`html.nested_table_extracted`) because its cell content
  was modified. The nested table records its parent (`table-N`), and the index
  sheet explains the marker in its `reason` column. Table numbering follows
  document order, including layout tables that are later skipped.
- **Merged cells:** `--merge-policy expand` (default) copies the value to every
  covered cell and reports `html.merged_cell_expanded`. `first` keeps the value
  in the top-left cell and reports `html.merged_cell_first_only`. `error` fails
  before any output is written. Overlapping spans produce
  `html.merged_cell_overlap` and keep the existing value. A merge that only
  spans header columns is reported as `html.header_colspan_expanded` instead of
  `html.merged_cell_expanded`, so the same expansion is not logged twice.
- **Header inference:** an explicit `--header-row N` wins (rows before it are
  discarded). Otherwise the first row inside `<thead>` or the first row with
  `<th>` cells becomes the header; when no `<th>` exists, the first row is used
  and `html.header_inferred` is reported. `--no-header` generates
  `column_1`, `column_2`, ... and treats every row as data. A header cell that
  spans several columns produces `territory`, `territory_2`, ... and reports
  `html.header_colspan_expanded` at info severity; that is normal expansion, not
  loss. Two independent header cells with the same text still produce a
  `html.duplicate_header` warning.
- **Layout tables:** `--min-rows` (default 2) and `--min-cols` (default 1)
  filter layout tables. The reason names only the failing threshold
  (for example `1 row(s) < --min-rows 2`). A skipped table emits
  `html.table_skipped` and is retained as inventory metadata rather than
  discarded.
- **Table selection:** `--tables all` is equivalent to `--all-sheets` for HTML.
  `--tables N` selects a 1-based table number (including skipped numbers) and
  `--tables NAME` accepts a caption, `table-N` name, or output table name.
  `--sheet`/`--sheet-index` remain aliases for the same selection.
- **Output naming:** `--use-captions` uses `<caption>` text for sheet names;
  otherwise names are `table-1`, `table-2`, ... . Duplicate names are
  deduplicated case-insensitively with stable suffixes and report
  `html.sheet_name_collision`. The existing workbook sanitizer still enforces
  Excel's 31-character and invalid-character rules.
- **`--sheet-per-table`:** explicitly requests one worksheet per selected HTML
  table. It requires a workbook output and cannot be combined with `--stack`.
  `--index-sheet` prepends an `_index` worksheet with `source`, `table`,
  `caption`, `rows`, `columns`, `status`, and `reason`. Every parsed table gets a
  row: included tables use `status=included`, skipped tables use
  `status=skipped` with the failing threshold, and nested tables explain the
  outer-cell marker in `reason`. The index itself is not counted. The index
  works for any multi-table input, not only HTML.
- **Workbook summary:** after a successful workbook write (unless `--quiet`), a
  one-line summary names every written sheet and lists skipped tables with their
  reasons, for example `wrote 4 sheet(s): _index, Sales, Regions, Notes; skipped
  table-4 (1 row(s) < --min-rows 2)`.
- **HTML writing:** still out of scope. HTML is a read-only, multi-table source.
- **Encoding:** HTML input uses the shared text decoder. Replacement characters
  are reported as `csv.encoding_replaced` because the event is about the text
  decoding path, not the HTML structure.
- **Fixture:** `fixtures/tables.html` has three data tables (`Sales`, `Regions`,
  nested `Notes`) plus a one-row layout table. `fixtures/header-row.html` covers
  explicit header rows and first-row inference.

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

Everything else follows the devplan scope: CSV/TSV/JSON and XLSX/ODS
read/write, HTML table ingestion, split/stack, row/cell inspection, and
merge/concat with basic transforms are implemented. Directory/ZIP batch mode,
the pipeline manifest, the loss-report file, and DuckDB export are still ahead.
There is no network, and no `fs::write` in command code. All file outputs go
through `io::SafeWriter`.
