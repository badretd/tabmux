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
  or `.ods` destination with `--split-sheets` is a usage error. Phase 9 added
  multi-file stage-then-commit atomicity; workbook output itself is a single
  atomic `SafeWriter` commit.

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

## Phase 7 — Directory and ZIP batch ingestion

- **Discovery:** a single directory or `.zip` path is treated as a batch input.
  Directory traversal is opt-in with `--recursive`; ZIP members are always
  traversed. Discovery is lexicographic and deterministic.
- **Outputs:** batch output is a directory of delimited files. Relative member
  paths are preserved unless `--flatten` is used; collisions get stable numeric
  suffixes. The default name template is `{stem}.{format}`, or
  `{stem}.{sheet}.{format}` when every sheet/table is selected.
- **Unsupported members:** they emit `format.unsupported` and are skipped.
  `--strict` fails with exit code 5 before creating the output directory.
- **ZIP safety:** path traversal, absolute member paths, entry-count limits,
  total uncompressed-size limits, and compression-ratio limits are enforced
  before any output is written. Defaults are 10,000 entries, 512 MiB total, and
  a ratio of 200 for members at least 1 MiB; all three are configurable.
- **Atomicity:** every output is staged with `SafeWriter` before any commit. A
  commit failure reports which outputs were already written.

## Phase 8 — Pipeline manifest, dry-run, replay

- **Manifest parser:** `toml 0.8` plus `serde` derive. The manifest is parsed
  and structurally validated (version, unique ids, op-specific fields, output
  paths, alias collisions) before any I/O. Unknown fields and unknown ops are
  rejected as usage errors (exit 2).
- **Executor split:** `pipeline::executor::execute` loads every input and runs
  every step in memory, producing a `ResolvedPlan`; only then does `run` open
  `SafeWriter`s. This is what guarantees "wrong column name fails before
  writing any output" and "no partial outputs on validation failure".
- **Step results:** each step has a primary result name (`id` or `step-N`) and a
  convenience alias (`selected`, `filtered`, `sorted`, `renamed`, `deduped`,
  `merged`, `concatenated`). `input` defaults to the previous step or the only
  input; `merge`/`concat` name their secondary input with `with`. Multiple
  outputs can reference any named result. There is no DAG in v1.
- **Op mapping:** pipeline steps call the same `ops` functions as verb mode:
  `select`, `filter`, `sort`, `rename`, `dedupe`, and `merge` (with `concat` as
  the row-append spelling). Strict merge reuses the Phase 5 schema diff and
  fails before writing.
- **Inputs:** `sheet` accepts a name or a 1-based integer; `header = false`
  generates `column_1`, `column_2`, ... for delimited/workbook/HTML inputs.
  `sheet` is rejected for single-table formats and `header` is rejected for
  JSON. Explicit `format` overrides extension detection. Manifest paths are
  resolved against the process working directory, matching verb mode, so either
  run from the project root or use absolute paths.
- **Outputs:** `format` overrides the extension; only csv/tsv/xlsx/ods are
  writable. `sheet` is only valid for workbook outputs and is sanitized with an
  `output.sheet_name_sanitized` info event when it changes. One output may be
  `-` for delimited stdout.
- **Dry-run and plan:** `--dry-run` still reads and executes the chain so naming,
  schema, and reference errors are caught, but it opens no output writer. The
  current readers materialize tables, so dry-run reads full inputs to compute
  row estimates rather than reading headers only; this is a performance
  limitation, not a write. The plan summary lists inputs, steps, outputs, and
  row/column estimates. `--save-plan PATH` writes the resolved plan as
  deterministic JSON (no timestamps; `-` writes to stdout). A dry-run with
  `--save-plan` writes only that explicitly requested plan file.
- **Determinism:** results are materialized in input/step order, sheet names are
  stable, no randomness or timestamps enter data output, and the plan JSON is
  byte-identical across runs. The integration test compares SHA-256 hashes of
  two replays.
- **Strict mode (basic):** any warning or error event fails with exit 5 before
  writing. This includes workbook fidelity warnings such as
  `excel.chart_dropped`. `--no-strict` overrides `strict = true`. Phase 9 adds
  the loss-report file and the `--max-warnings` threshold on top of this basic
  policy.
- **No seed:** no operation is randomized, so `seed` is intentionally absent
  rather than accepted and ignored.
- **Fixtures:** `pipeline.toml` is the replayable example; `pipeline-merge.toml`
  covers concat, `pipeline-invalid.toml` covers an unknown column, and
  `pipeline-strict.toml` covers a lossy workbook blocked by strict mode.

## Phase 9 — Loss report, provenance, strict mode

- **Report module:** `src/report/mod.rs` owns the versioned `LossReport` and
  `ProvenanceManifest` documents, event aggregation, Markdown rendering, strict
  policy, verification, and auxiliary-path validation. The JSON schema lives in
  `docs/loss-report.schema.json`; `docs/loss-report.md` is the human reference.
- **Aggregation:** events are grouped by `(severity, code, message)`, the first
  source reference is retained, and `count` is the number of occurrences.
  Records are sorted deterministically (error, warning, info; then code,
  message, source). The summary counts occurrences, not distinct records.
- **When reports are written:** every conversion builds a report in memory, but
  the file is only written when `--report` or `--report-markdown` is passed.
  A lossless CSV → CSV conversion writes an empty `events` array and a zeroed
  summary.
- **Strict policy:** `--strict` fails with exit code 5 when any `error` event
  exists or when warning occurrences exceed `--max-warnings N` (default 0).
  `--lossy` conflicts with `--strict` and is the explicit "allow warnings"
  switch; for `run`, `--lossy` is an alias for `--no-strict` and overrides
  `strict = true` in the manifest. Info events never fail strict mode.
- **Report on strict failure:** a blocked conversion still writes the loss
  report when `--report`/`--report-markdown` was requested, then exits 5 before
  opening any data writer. This is deliberate: the report is the explanation.
  The provenance manifest is only written after a successful data write because
  it records output checksums.
- **Provenance:** `--manifest PATH` writes tool name/version/git hash (embedded
  by `build.rs` when a git checkout is available), RFC 3339 start time,
  duration, the actual argv, resolved options, input/output byte counts and
  SHA-256 checksums, the loss summary, and `deterministic: true`. Stdin has no
  stable checksum and is omitted from `inputs`. The manifest itself is not
  byte-identical across runs (timestamp/duration); data outputs are.
- **Verification:** `--verify` re-reads every file output and every in-memory
  stdout output, then compares column names (when a header is present), sheet
  names for workbooks, row counts, and rendered cell values. Verification runs
  after a file commit, so a mismatch reports the output as written but fails
  with exit code 1.
- **Auxiliary paths:** report, Markdown report, and manifest paths are validated
  before writing. They may not equal each other, an input, a data output, or
  the resolved-plan path. They respect the normal no-clobber/`--force` policy.
- **Multi-output atomicity:** `--split-sheets` now stages every output before
  committing any of them, matching batch mode. A commit failure names the
  outputs already written.
- **Workbook determinism:** the rust_xlsxwriter path now writes a fixed
  document creation timestamp (`2000-01-01`). XLSX outputs are therefore
  byte-identical across runs; the manifest still records the real run time
  separately. ODS output was already deterministic.
- **Commands:** `convert` (single, split, workbook, and batch), `merge`, and
  `run` accept the report flags. Read-only inspection commands do not write
  reports. `--strict` is no longer limited to batch conversion.
- **Fixtures:** `fixtures/rich.xlsx` exercises chart, style, cached formula,
  merged cells, conditional formatting, data validation, comment, pivot cache,
  and macro events. `fixtures/messy.csv` covers ragged-row events and the
  `--max-warnings` threshold. `fixtures/reports/sample-report.json` is the
  golden report document.
- **Event coverage:** the existing readers already emit the documented codes.
  The report path adds no silent drops; batch `format.unsupported` events are
  now collected into the report as well as printed to stderr. The CSV reader
  also detects an unterminated quoted field and emits `csv.bad_quote`.
  Delimited output emits `output.null_rendered_as_empty` (info) when native
  null cells are written as empty text, with the occurrence count.
- **Formula events:** every formula cell emits
  `excel.formula_cached_value_only` with the stable message “cached value may
  be stale; formula text not preserved”. The report aggregates them into one
  entry with `count = N` and retains the first cell as its source.
- **Pivot semantics:** `excel.pivot_cache_dropped` reports that pivot
  definitions/caches were not copied. `excel.pivot_values_materialized` (info)
  records that worksheet values are exported as plain cells. The fixture test
  asserts the `Pivot` sheet’s values survive the round-trip.
- **Post-write summary:** successful writes print one summary line naming the
  requested loss report and manifest, the lossy category/item counts, and the
  strict outcome, for example
  `loss severity: lossy (7 categories, 8 items; --strict would exit 5)`.
  Global workbook properties are printed as `styles(dropped)`,
  `macros(dropped)`, and `pivot_caches(dropped)` rather than `(1)`. Dry-runs
  confirm requested report files separately and never claim a manifest.
- **Strict whitelist:** `excel.pivot_values_materialized`, merged-cell
  expansion, and `excel.formula_cached_value_only` are normal transformations,
  not strict failures. They are still reported, but they are excluded from the
  lossy item count and from `--max-warnings`.
- **Report-backed stderr:** when a report is requested, summary mode prints
  aggregated category lines and points at the JSON/Markdown report instead of
  one line per event. Merged-cell expansion and pivot-value materialization get
  separate info lines because one is a reader transformation and the other is
  a writer materialization. `--verbose` prints the per-event details.

## Phase 10 — DuckDB export and read-only query

- **Backend deviation:** the offline registry used by this build has no
  `duckdb` crate and there is no DuckDB binary on the machine. The
  `sql-duckdb` feature is therefore backed by the embedded SQLite engine
  (`rusqlite`, bundled) instead of bundled DuckDB. The command surface,
  transaction semantics, read-only guard, import types, and verification are
  the Phase 10 contract; the on-disk file is a SQLite database. A file whose
  header begins with `DUCK` is rejected with a clear unsupported-feature error
  rather than being misread. Replacing the backend with the real `duckdb` crate
  later does not change the CLI.
- **Feature gating:** `sql-duckdb` is not part of the default feature set and
  is not enabled by `--all-features` in the minimal build unless requested.
  Without the feature, `to duckdb` and `query` still parse and return exit code
  4 with “enable the sql-duckdb feature”.
- **Commands:** `tabmux to duckdb <input> --table NAME -o DB [--sheet NAME]
  [--force] [--dry-run] [--verify]` imports exactly one selected table.
  `--all-sheets` selects several and is rejected with the available table
  names. `tabmux query <DB> "<SQL>" [-f table|csv|tsv|json]` prints one result
  set to stdout; the default is an aligned terminal table.
- **Import semantics:** the destination table is created inside a transaction.
  A failure rolls back. A new database file is removed when its first import
  fails, so a failed command leaves no partial database. Importing a second,
  differently named table into an existing database is allowed (the S1
  workflow); replacing an existing table or view requires `--force`. The
  replacement and the insert run in one transaction, so the old table survives
  a failed replacement.
- **Type mapping:** `int` → `BIGINT`, `float` → `DOUBLE`, `bool` → `BOOLEAN`
  (stored as 0/1), `decimal` → `TEXT` to preserve exact digits, dates/times →
  `DATE`/`TIMESTAMP`/`TIME` ISO text, text/error → `TEXT`, null → SQL null.
  Columns are inferred with the same conservative rules as workbook output
  (`007` stays text, mixed columns become text, integral floats become ints).
  Every cell is normalized to its final column type before insert so SQLite's
  declared affinity cannot change the representation behind the verifier's
  back. Non-finite floats and duplicate/empty column names are hard errors,
  not silent drops.
- **Verification:** `--verify` re-reads the imported rows inside the
  transaction and compares row counts and every value, including boolean 0/1
  and integral-float normalization. The import summary reports a SHA-256
  checksum of the imported values. Verification failures roll the import back.
- **Query safety:** the database is opened with `SQLITE_OPEN_READ_ONLY`, and
  the prepared statement must report `readonly()`. The input must contain
  exactly one statement; a small splitter ignores semicolons inside strings,
  quoted identifiers, and comments. `drop`, `delete`, `create`, `attach`, and
  multi-statement input fail with exit code 2 before touching data.
- **Query output:** `table` uses the shared terminal renderer, `csv`/`tsv` use
  the normal delimited writer (header plus rows), and `json` is a pretty-printed
  array of objects with typed numbers and booleans. SQL errors and missing
  databases are data errors with the database path in the message.
- **Fixtures:** `fixtures/sales.xlsx#Sales` and `fixtures/regions.csv` cover
  import, count, aggregation, join, replacement, and read-only checks.
- **Smoke test:** the Phase 10 block detects whether the binary was built with
  `sql-duckdb`. `./scripts/smoke.sh` skips with an explicit message when the
  feature is absent; `TABMUX_FEATURES=sql-duckdb ./scripts/smoke.sh` runs the
  full import/query/guard sequence.

## Diagnostics policy

- **Default (`summary`):** read-only commands show only warnings/errors that
  affect the data being displayed. Informational events are reserved for write
  commands, and source-fidelity events are hidden.
- **Workbook-only fidelity events:** `excel.chart_dropped`,
  `excel.style_dropped`, `excel.macro_dropped`, pivot cache/values, comments,
  conditional formatting, data validation, and merged-cell expansion are
  printed only when writing a workbook, when `--strict` is active, or when
  `--fidelity-report` is passed. Writing CSV/TSV does not count as a workbook
  save, so they stay quiet by default.
- **Value-affecting fidelity events:** `excel.formula_cached_value_only`
  (formula text loss) is printed for every write, including CSV/TSV. Future
  precision-loss events follow the same rule. Encoding replacement is already a
  normal warning and is always shown.
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
- **Display matrix (summary mode):**
  - Workbook-only fidelity events (`excel.chart_dropped`,
    `excel.style_dropped`, `excel.macro_dropped`, pivot cache/values, comments,
    conditional formatting, data validation, merged-cell expansion) are shown
    only for XLSX/ODS output, with `--fidelity-report`, or with `--verbose`.
  - Value-affecting events (`excel.formula_cached_value_only`, future precision
    loss, `csv.encoding_replaced`) are shown for every write, including
    CSV/TSV.
  - Read-only commands show only warnings/errors that affect the displayed
    rows; workbook-only events require `--fidelity-report`.
  - `--verbose`/`--debug` shows every event.
- **Report-backed diagnostics:** when `--report` or `--report-markdown` is
  requested, summary mode prints a few aggregated lines and points at the
  report instead of one line per event. `--verbose` still prints every event.
  The post-write summary always names requested report/manifest files and the
  loss severity (`lossless`, `approximate`, or `lossy`).

## Scope guard

Everything else follows the devplan scope: CSV/TSV/JSON and XLSX/ODS
read/write, HTML table ingestion, split/stack, row/cell inspection,
merge/concat with basic transforms, directory/ZIP batch mode, the reproducible
pipeline manifest, the loss report, provenance manifest, verification, the
warning-threshold strict policy, and the feature-gated SQL export/query surface
are implemented. Hardening and release packaging (Phases 11–12) are still
ahead. There is no network, and no `fs::write` in command code. Table file
outputs go through `io::SafeWriter`; the SQL database is written by its engine
inside a transaction (see the Phase 10 note).
