# tabmux development plan

> **Status:** active plan, 2026-11-04  
> **Scope:** v1 conversion core only. The product is now a safe, reproducible, honest **table/workbook conversion engine**, not a general analytics platform.  
> **Companion documents:** [`features.md`](./features.md) (feature space and scope decisions), [`analogues.md`](./analogues.md) (research), [`dependencies.md`](./dependencies.md) (crate plan).

---

## 1. What we are building

**Starting point:** the repository currently contains `Cargo.toml` (edition 2024, no dependencies), an empty `docs/` set, and a hello-world `src/main.rs`. Phase 0 starts from that state.

### 1.1 One-sentence goal

> **`tabmux` converts messy tabular sources into clean tables and workbooks, with a replayable pipeline, safe writes, and a report that tells the user exactly what was lost.**

The five surviving differentiators are:

| ID | Differentiator | What it means in v1 |
|---|---|---|
| **N1** | Reproducible pipeline | A declarative conversion manifest that can be re-run byte-for-byte, with dry-run and validation before writing |
| **N2** | Messy-source ingestion | CSV/TSV, XLSX, ODS, HTML tables, JSON/JSONL, directories, ZIP archives → one clean table set |
| **N3** | Workbook structure/value fidelity | Sheets, values, types, dates, merged-cell expansion, basic formatting, honest limits |
| **N17** | Loss report | Every conversion emits a machine-readable report of dropped/approximated/unsupported content |
| **N21** | Safe-by-default execution | No silent overwrite, atomic writes, dry-run, checksums, provenance manifest, strict mode |

### 1.2 Explicit non-goals for v1

Do **not** build these. They are either discontinued or deferred:

- Geo analytics, graph analytics, statistics/ML/forecasting.
- NL copilot, PII/synthetic data, document rendering/mail merge.
- Terminal dashboards, TUI viewer.
- Serve/MCP/watch/chat modes, WASM plugins, cloud/federated I/O.
- Embedded scripting, jq, Lua, Python.
- Advanced SQL (federation, upsert, query cache, multi-database joins).
- Advanced Excel fidelity (charts, pivot caches, macros, full formula recalculation, conditional formatting, style cloning).
- Fuzzy/as-of/non-equi joins, pivot tables, crosstab/correlation.
- DBF, LTSV/logfmt, fixed-width, RST/LaTeX, Avro/ORC, 7z write, YAML/TOML table write, XML XPath.
- Exotic compression (bzip2, xz, lz4, snappy) and locale-aware formatting.

Deferred (not discontinued, but **not v1 blockers**):

- N4 table diff/patch/merge.
- N5 data contracts/drift.
- Advanced N9 external-memory spill/sort/join.
- N11 time-series/event operations.
- N13 text/log mining.
- N18 semantic data dictionary/lineage beyond the provenance manifest.
- N20 full test/fuzz/benchmark harness beyond the v1 quality gate.

### 1.3 Definition of done for v1

A user can:

1. Convert CSV/TSV, XLSX, ODS, HTML tables, JSON/JSONL, directories, and ZIP archives.
2. Select one sheet/table or all sheets/tables.
3. Write CSV/TSV/XLSX/ODS, including multi-sheet workbooks.
4. Merge tables with strict or loose schema matching.
5. Print a specific row or column to the terminal.
6. Re-run a conversion from a pipeline manifest.
7. Get a loss report and a provenance manifest with checksums.
8. Use `--dry-run`, `--force`, `--strict`, and atomic writes.
9. Export a table to DuckDB and run a simple query.
10. Trust that unsupported content is reported, not silently dropped.

---

## 2. Scope guardrails

These rules are part of the phase contract. A phase that violates them is rejected, even if the demo works.

1. **One format at a time.** Do not add a new format until its reader/writer, loss policy, and fixtures are all in scope.
2. **Every conversion reports loss.** Even a lossless CSV → CSV conversion builds an empty loss report; `--report PATH` writes it, otherwise warnings are printed to stderr.
3. **Every write is safe.** All outputs go through the same atomic-write layer. No `fs::write` in command code.
4. **No silent skips.** Unsupported sheets, cells, styles, or files become `LossEvent`s or hard errors.
5. **No network.** v1 is local-only.
6. **No query engine in the core path.** DuckDB is an optional export/query feature, not the execution engine for conversion.
7. **No presentation fidelity promises.** Basic formatting is best-effort; charts, pivots, macros, and full formula evaluation are out.
8. **No new CLI surface without a test and a manual demo.**
9. **No phase without a fixture.** If it cannot be tested with a checked-in fixture, it is not ready.
10. **Keep `main` runnable.** Each phase ends with a buildable binary and a passing smoke test.

---

## 3. Architecture for v1

### 3.1 Crate layout

```
src/
  main.rs                 # thin CLI entry point
  lib.rs                  # public modules, testable API
  cli/
    mod.rs                # clap definitions, command dispatch
    args.rs               # input/output specs, shared flags
  model/
    mod.rs                # Table, TableSet, Column, Cell, SourceRef
    types.rs              # ColumnType, CellValue, null semantics
  formats/
    mod.rs                # format registry, detection, capability matrix
    csv.rs                # CSV/TSV reader/writer, dialect sniffing
    json.rs               # JSON/JSONL reader
    xlsx.rs               # XLSX/XLS/XLSB read (calamine), XLSX write (rust_xlsxwriter)
    ods.rs                # ODS read/write
    html.rs               # HTML table extraction
    archive.rs            # ZIP/tar/directory traversal
  ops/
    mod.rs                # operation registry
    select.rs
    filter.rs
    sort.rs
    rename.rs
    concat.rs
    merge.rs
    dedupe.rs
  pipeline/
    mod.rs                # manifest model, validation, execution
    manifest.rs          # TOML/JSON parse/serialize
  report/
    mod.rs                # LossReport, Provenance, renderers
  io/
    mod.rs                # safe writes, temp files, checksums
  error.rs                # error types, exit codes
```

### 3.2 Core model

```rust
pub struct Table {
    pub name: String,
    pub source: SourceRef,
    pub columns: Vec<Column>,
    pub rows: RowSet,          // materialized for v1; streaming for CSV only
    pub warnings: Vec<LossEvent>,
}

pub struct TableSet {
    pub source: SourceRef,
    pub tables: Vec<Table>,    // sheets, HTML tables, archive members
}

pub struct Column {
    pub name: String,
    pub ty: ColumnType,        // Null, Bool, Int, Float, Decimal, Text, Date, DateTime, Time, Error
    pub source: Option<SourceRef>,
}

pub enum CellValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Decimal(String),           // preserve exact text
    Text(String),
    Date(NaiveDate),
    DateTime(NaiveDateTime),
    Time(NaiveTime),
    Error(String),             // #REF!, #N/A, parse errors, etc.
}
```

`SourceRef` is the anchor for errors and loss reports:

```rust
pub struct SourceRef {
    pub path: PathBuf,
    pub format: FormatId,
    pub sheet: Option<String>,
    pub table: Option<String>,     // HTML table index/caption
    pub row: Option<u64>,          // 1-based data row
    pub column: Option<String>,
    pub cell: Option<String>,      // A1 notation where available
}
```

### 3.3 Loss report model

```rust
pub struct LossReport {
    pub version: u32,
    pub tool: ToolInfo,
    pub inputs: Vec<Artifact>,
    pub outputs: Vec<Artifact>,
    pub events: Vec<LossEvent>,
    pub summary: LossSummary,
}

pub struct LossEvent {
    pub severity: Severity,        // Info, Warning, Error
    pub code: &'static str,        // stable machine-readable code
    pub message: String,
    pub source: Option<SourceRef>,
    pub count: u64,                // aggregated events
}
```

Initial event codes:

```
excel.sheet_skipped
excel.formula_cached_value_only
excel.formula_dropped
excel.style_dropped
excel.chart_dropped
excel.pivot_cache_dropped
excel.macro_dropped
excel.merged_cell_expanded
excel.merged_cell_unsupported
excel.conditional_formatting_dropped
excel.data_validation_dropped
excel.comment_dropped
html.table_skipped
html.merged_cell_expanded
html.header_inferred
html.sheet_name_collision
csv.ragged_row
csv.bad_quote
csv.null_token
csv.encoding_replaced
json.flattened
json.array_truncated
archive.member_skipped
format.unsupported
format.truncated
output.sheet_name_sanitized
output.null_rendered_as_empty
```

### 3.4 Provenance manifest

```json
{
  "version": 1,
  "tool": { "name": "tabmux", "version": "0.1.0", "git": "..." },
  "started_at": "2026-11-04T12:00:00Z",
  "duration_ms": 1234,
  "command": ["convert", "book.xlsx", "out.xlsx"],
  "options": { "strict": false, "sheets": "all" },
  "inputs": [
    { "path": "book.xlsx", "format": "xlsx", "bytes": 12345, "sha256": "..." }
  ],
  "outputs": [
    { "path": "out.xlsx", "format": "xlsx", "bytes": 23456, "sha256": "..." }
  ],
  "loss_summary": { "info": 2, "warning": 3, "error": 0 },
  "deterministic": true
}
```

### 3.5 Safe write layer

```rust
pub struct SafeWriter { /* temp path + destination */ }

impl SafeWriter {
    pub fn create(dest: &Path, force: bool) -> Result<Self>;
    pub fn commit(self) -> Result<()>;  // fsync + atomic rename
}
```

Rules:

- Temp file is created in the destination directory (same filesystem).
- `commit()` flushes, fsyncs, and renames.
- `Drop` removes the temp file if not committed.
- Existing output requires `--force` (or `-y`); `--no-clobber` is the default.
- `--backup` copies the existing file to a timestamped `.bak` before replacement.
- Multi-output commands stage all outputs, then commit; if any commit fails, report which outputs were written and which were not.

### 3.6 Manual smoke test

Create `scripts/smoke.sh` in Phase 0. Every phase adds one block to it. It must be runnable with one command:

```sh
./scripts/smoke.sh
```

The script uses only checked-in fixtures and compares outputs with `cmp`, `diff`, or a small JSON assertion helper.

---

## 4. How phases work

A **phase** is one focused implementation pass, roughly **0.5–2 days** for one developer. It is not a sprint and not a feature epic.

Every phase must produce:

1. A runnable binary.
2. Tests for the new behavior.
3. A manual demo with exact commands and expected results (documented in this file).
4. A smoke-test block.
5. An updated `docs/` note if the behavior differs from the plan.

If a phase cannot fit in one pass, **split it before starting**. Do not carry half-finished subsystems across phases.

---

## 5. Phase overview

| Phase | Outcome | Manual demo | Est. |
|---|---|---|---|
| 0 | CLI skeleton, format registry, fixtures, smoke script | `tabmux formats` | 0.5 d |
| 1 | CSV/TSV/JSON core, safe writes, select/head/count | round-trip CSV, dry-run | 1 d |
| 2 | XLSX read, sheet selection, CSV conversion | workbook → CSV | 1 d |
| 3 | ODS read, all-sheets/per-sheet, row/column output | ODS → many CSVs | 1 d |
| 4 | XLSX/ODS write, multi-sheet assembly, basic formatting | CSVs → one workbook | 1.5 d |
| 5 | Merge/concat, schema diff, basic transforms | strict/loose merge | 1 d |
| 6 | HTML table ingestion, HTML → workbook | 3 tables → 3 sheets | 1.5 d |
| 7 | Directory/ZIP batch ingestion | folder/zip → converted set | 1 d |
| 8 | Pipeline manifest, dry-run, replay | same checksums twice | 1.5 d |
| 9 | Loss report, provenance, strict mode | rich workbook report | 1.5 d |
| 10 | DuckDB export and simple query | table → DuckDB → query | 1 d |
| 11 | Hardening, fixtures, conformance | full smoke + fuzz/security checks | 1–2 d |
| 12 | Release packaging and docs | release binary + smoke | 1 d |

**Total:** roughly 14–16 focused days for one developer, assuming no new features are added.

---

## 6. Detailed phases

### Phase 0 — Skeleton and scope guardrails

**Goal:** a buildable CLI with an explicit format matrix and no accidental scope.

**Single-pass scope**

- Create `lib.rs` + thin `main.rs`; wire `clap`.
- Implement `tabmux --version`, `--help`, `tabmux formats`.
- Define `FormatId`, `Capability` (read/write/stream/multi-table), and the initial registry.
- Define error types and exit codes.
- Add `fixtures/` with a README and the first tiny CSV/TSV files.
- Add `scripts/smoke.sh` that runs `--version` and `formats`.
- Add CI skeleton: `fmt`, `clippy`, `test` on Linux.

**Out of scope**

- Real parsing.
- TUI, SQL, DuckDB, HTML, XLSX.

**Deliverables**

```
src/main.rs
src/lib.rs
src/cli/mod.rs
src/formats/mod.rs
src/error.rs
fixtures/README.md
fixtures/basic.csv
fixtures/quoted.csv
scripts/smoke.sh
```

**Manual test**

```sh
cargo run -- --version
cargo run -- --help
cargo run -- formats
```

Expected:

- `--version` prints `tabmux x.y.z`.
- `formats` prints a table like:

```
FORMAT   READ  WRITE  MULTI-TABLE  NOTES
csv      yes   yes    no           RFC 4180 + dialects
tsv      yes   yes    no
xlsx     yes   yes    yes          basic fidelity
ods      yes   yes    yes          basic fidelity
html     yes   no     yes          table extraction
json     yes   no     no           JSON/JSONL input
```

- No panics; unknown commands return exit code 2.

**Exit criteria**

- `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` pass.
- `scripts/smoke.sh` passes.
- The format matrix contains only in-scope formats.

---

### Phase 1 — Core table model, CSV/TSV/JSON, safe writes

**Goal:** the first real conversion path, with the safety layer from day one.

**Single-pass scope**

- Implement `Table`, `Column`, `CellValue`, `SourceRef`, `TableSet`.
- CSV/TSV reader:
  - RFC 4180 quoting, embedded newlines, escaped quotes.
  - delimiter sniffing (`,` `;` `\t` `|`), `--delimiter` override.
  - BOM handling, UTF-8/UTF-16/Latin-1 via `encoding_rs` + `chardetng`.
  - `--no-header` and generated column names.
  - ragged-row policy: `--ragged error|pad|skip` (default error).
- CSV/TSV writer: quoting policy, CRLF/LF, `--no-header`, null rendering.
- JSON/JSONL reader: arrays of objects, nested objects flattened to `a.b`, arrays kept as JSON text; `--json-path` optional (maybe defer).
- Safe writer: temp + fsync + rename; `--force`, `--dry-run`, `--backup`.
- Commands:
  - `tabmux convert <input> -o <output>`
  - `tabmux count <input>`
  - `tabmux head <input> --rows N`
  - `tabmux select <input> --columns a,b,c`
- Exit codes: 0, 1, 2, 3, 4.

**Out of scope**

- XLSX/ODS/HTML.
- Joins, merge, pipeline files.
- Loss report file (emit warnings to stderr only).

**Deliverables**

- Core model and CSV/JSON readers/writers.
- `io::SafeWriter`.
- Tests for quoted fields, CRLF, BOM, encoding, ragged rows.
- Fixtures: `basic.csv`, `quoted.csv`, `bom.csv`, `crlf.csv`, `semicolon.csv`, `ragged.csv`, `latin1.csv`, `data.jsonl`.

**Manual test**

```sh
# Round-trip with quotes and newlines
cargo run -- convert fixtures/quoted.csv /tmp/quoted.tsv
cargo run -- convert /tmp/quoted.tsv /tmp/quoted.back.csv
diff <(cargo run -- convert fixtures/quoted.csv -) /tmp/quoted.back.csv

# Delimiter sniffing
cargo run -- convert fixtures/semicolon.csv - | head -3

# Encoding
cargo run -- convert fixtures/latin1.csv /tmp/utf8.csv

# Selection and count
cargo run -- select fixtures/basic.csv --columns name,email
cargo run -- count fixtures/basic.csv

# Safety
cargo run -- convert fixtures/basic.csv /tmp/existing.csv   # must fail
cargo run -- convert fixtures/basic.csv /tmp/existing.csv --force
cargo run -- convert fixtures/basic.csv /tmp/nothing.csv --dry-run
test ! -e /tmp/nothing.csv
```

Expected:

- Quoted commas/newlines survive round-trip.
- Semicolon file parses without `--delimiter`.
- Latin-1 text becomes valid UTF-8.
- Existing output is not overwritten without `--force`.
- `--dry-run` writes nothing.

**Exit criteria**

- No data corruption in the round-trip fixtures.
- Every read error includes file, row, and column context.
- `scripts/smoke.sh` covers all commands.
- `SafeWriter` is the only write path (check with `rg 'fs::write|File::create' src`).

---

### Phase 2 — XLSX read, sheet selection, CSV conversion

**Goal:** the first spreadsheet workflow: `xlsx → csv`, with honest handling of workbook features.

**Single-pass scope**

- Add `calamine` behind the `excel` feature.
- Read `.xlsx`, `.xls`, `.xlsb` (if calamine supports the file; otherwise report unsupported).
- List sheets: `tabmux sheets book.xlsx`.
- Select sheets: `--sheet NAME`, `--sheet-index N`, `--all-sheets`.
- Convert one sheet to CSV/TSV.
- Type handling:
  - strings, booleans, integers, floats, dates, datetimes, times, errors.
  - Excel serial dates → ISO-8601 text.
  - formula cells: use cached values; emit `excel.formula_cached_value_only`.
- Merged cells: default expand value to all covered cells; emit `excel.merged_cell_expanded`.
- Empty trailing rows/columns: trim; emit info only if dimensions look suspicious.
- Emit warnings to stderr; no report file yet.

**Out of scope**

- ODS.
- Writing XLSX.
- Styles, charts, pivots, macros, formula evaluation.

**Deliverables**

- `formats/xlsx.rs` reader.
- `sheets` command.
- `convert --sheet` support.
- Fixtures: `sales.xlsx` with 3 sheets, dates, merged cells, a formula, a chart, a styled cell.
- Tests comparing converted CSV to expected values.

**Manual test**

```sh
cargo run -- sheets fixtures/sales.xlsx
# Sales
# Regions
# Notes

cargo run -- convert fixtures/sales.xlsx /tmp/sales.csv --sheet Sales
head -5 /tmp/sales.csv
cargo run -- convert fixtures/sales.xlsx /tmp/all/ --all-sheets --split-sheets
ls /tmp/all
```

Expected:

- Sheet list matches the workbook.
- Dates are ISO-8601, not Excel serial numbers.
- Merged cells are expanded and a warning is printed once per range.
- Formula cells show the cached value and a `formula_cached_value_only` warning.
- Unsupported chart/style is mentioned in warnings.

**Exit criteria**

- `xlsx → csv` works for all fixture sheets.
- Missing sheet gives a clear error listing available sheets.
- No panic on malformed/empty workbooks.

---

### Phase 3 — ODS read, all-sheets/per-sheet, row/column output

**Goal:** complete the first two user scenarios (ODS/XLSX → CSV, row/column to terminal).

**Single-pass scope**

- Add ODS read via `calamine`.
- `convert book.ods out.csv --sheet Sheet1`.
- `convert book.ods out/ --all-sheets --split-sheets --name-template '{stem}.{sheet}.csv'`.
- `convert book.xlsx out.csv --all-sheets --stack --add-source-column sheet` (concatenate sheets).
- Add `row` command:
  - `tabmux row <input> --number N`
  - `tabmux row <input> --where 'id == 1234'` (simple equality only)
  - `tabmux cell <input> --row N --column NAME`
- Add `--raw` (no pretty formatting) and `--pretty` (terminal table).
- Ensure exact handling of quoted/multiline cells.

**Out of scope**

- ODS write.
- Complex expressions in `--where` (only `col == value`, `col != value`).
- TUI.

**Deliverables**

- `formats/ods.rs` reader.
- `row`/`cell` commands.
- Sheet-name template and collision handling.
- Fixtures: `sales.ods` with 2 sheets; `wide.csv` with multiline cells.
- Tests for row/column addressing.

**Manual test**

```sh
cargo run -- convert fixtures/sales.ods /tmp/ods/ --all-sheets --split-sheets
ls /tmp/ods

cargo run -- row fixtures/sales.xlsx --sheet Sales --number 2
cargo run -- row fixtures/sales.xlsx --sheet Sales --where 'id == 3'
cargo run -- cell fixtures/sales.xlsx --sheet Sales --row 2 --column email

cargo run -- select fixtures/wide.csv --columns note
```

Expected:

- One CSV per ODS sheet, correctly named.
- Row 2 and cell output match the source exactly, including multiline text.
- `--raw` output can be piped; `--pretty` is human-readable.
- S2 and S3 are complete.

**Exit criteria**

- Sheet names with spaces/slashes are sanitized and reported.
- `--all-sheets` on a 2-sheet workbook produces exactly 2 files.
- Row/column output is byte-exact for quoted data.

---

### Phase 4 — XLSX/ODS write, multi-sheet assembly, basic formatting

**Goal:** produce clean workbooks from clean tables.

**Single-pass scope**

- Add `rust_xlsxwriter` (default build) and `spreadsheet-ods` writers. The ODS
  writer is currently the built-in ODF implementation until `spreadsheet-ods`
  is adopted.
- `convert a.csv b.csv -o out.xlsx` → one sheet per input (default).
- `--sheet-per-input` explicit; `--sheet-name-template '{stem}'`.
- `--sheet-name NAME` (alias `--output-sheet`) to name a single output sheet.
  `--sheet` remains the input-sheet selector used by the earlier phases.
- Basic formatting only:
  - header bold + freeze top row;
  - autofilter on the used range;
  - sensible column widths;
  - number/date formats for typed columns;
  - no style cloning, charts, conditional formatting, pivots, macros.
  - ODS output carries bold headers, native types, and column widths; freeze
    panes and autofilter are XLSX-only in this phase.
- Write values as native types where safe (int, float, bool, date, datetime); otherwise text.
- Re-read the generated workbook with calamine in tests and compare values to the source.

**Out of scope**

- Preserving styles from an input workbook (`excel-preserve`).
- Charts, pivot caches, macros, conditional formatting, merged-cell styling.
- Formula writing except simple `=SUM(...)` if explicitly requested (optional; not required).

**Deliverables**

- `formats/xlsx.rs` writer.
- `formats/ods.rs` writer.
- Multi-sheet assembly.
- Fixtures: two CSVs with compatible/different types.
- Round-trip tests.

**Manual test**

```sh
cargo run -- convert fixtures/a.csv fixtures/b.csv -o /tmp/report.xlsx
cargo run -- sheets /tmp/report.xlsx
# a
# b

cargo run -- convert fixtures/sales.ods -o /tmp/sales.roundtrip.xlsx --all-sheets
cargo run -- sheets /tmp/sales.roundtrip.xlsx
```

Expected:

- Workbook opens in LibreOffice/Excel without repair prompts.
- Each input becomes a sheet.
- Header is bold, top row frozen, autofilter present.
- Values/dates are native types, not all text.
- Re-reading with `tabmux convert` produces the same values.

**Exit criteria**

- Generated XLSX/ODS are valid ZIP/ODF packages.
- No duplicate sheet names; collisions get suffixes and a warning.
- Round-trip tests pass for strings, ints, floats, booleans, dates, nulls.

---

### Phase 5 — Merge/concat, schema diff, basic transforms

**Goal:** complete S4 and the minimal table-shaping surface.

**Single-pass scope**

- `merge`/`concat`:
  - `--mode strict`: column names and order must match exactly.
  - `--mode loose`: union columns by name, fill missing with null.
  - `--add-source-column NAME` (default `source`).
  - `--dedup` exact-row dedupe.
  - `--case-insensitive-headers` optional.
- Schema diff output for strict failure:
  - missing columns, extra columns, order differences, type conflicts.
  - machine-readable `--json` option.
- Basic transforms:
  - `select` (already), `rename`, `filter` (`col == value`, `col != value`, numeric comparisons), `sort` (one or more columns, asc/desc), `dedupe`.
- No joins yet; no pivot/crosstab/correlation.

**Out of scope**

- Fuzzy/as-of/non-equi joins.
- Aggregation/pivot.
- Multi-key expressions beyond simple comparisons.

**Deliverables**

- `ops/merge.rs`, `ops/filter.rs`, `ops/sort.rs`, `ops/rename.rs`, `ops/dedupe.rs`.
- Schema diff renderer.
- Fixtures: `a.csv`, `b.csv` identical; `c.csv` reordered/missing column; `d.csv` type conflict.

**Manual test**

```sh
cargo run -- merge fixtures/a.csv fixtures/b.csv -o /tmp/merged.csv --mode strict
cargo run -- count /tmp/merged.csv

cargo run -- merge fixtures/a.csv fixtures/c.csv -o /tmp/merged.csv --mode strict
# expected: schema diff, exit 1, no output file

cargo run -- merge fixtures/a.csv fixtures/c.csv -o /tmp/merged.csv --mode loose --add-source-column origin
cargo run -- select fixtures/a.csv --columns name,amount | cargo run -- filter - --where 'amount > 100'
cargo run -- sort fixtures/a.csv --by amount:desc
```

Expected:

- Strict merge succeeds only for identical schemas.
- Mismatch fails before writing and prints a column-level diff.
- Loose merge fills missing fields with empty/null and adds provenance.
- S4 is complete.

**Exit criteria**

- Strict failure leaves no partial output.
- Schema diff is actionable (column names, expected vs actual).
- Dedupe is stable and preserves first occurrence.

---

### Phase 6 — HTML table ingestion, HTML → workbook

**Goal:** complete S5 and the first "messy source" story.

**Single-pass scope**

- Add `scraper` behind the `html` feature.
- Parse all `<table>` elements:
  - `<caption>`, `<thead>`, `<tbody>`, `<tfoot>`.
  - `colspan`/`rowspan` expansion (default `expand`; `--merge-policy first|error` optional).
  - nested tables: inner table becomes a separate table; outer cell gets a text marker.
  - header-row inference: `<th>` row, first row, or `--header-row N`.
- `convert page.html out.xlsx --tables all --sheet-per-table`.
- Sheet naming:
  - caption if present, else `table-1`, `table-2`.
  - sanitize, deduplicate, report collisions.
- `--use-captions`: prefer `<caption>` for sheet names; otherwise use `table-N`.
- `--index-sheet`: adds a `_index` sheet with source file, table number, caption, rows, columns.
- `--min-rows`, `--min-cols` to skip layout tables.
- Emit `html.*` loss events.

**Out of scope**

- PDF/DOCX/EML ingestion (deferred; N2 later).
- JavaScript rendering.
- CSS layout reconstruction.

**Deliverables**

- `formats/html.rs`.
- `--tables`/`--sheet-per-table`/`--index-sheet`.
- Fixtures: `tables.html` with 3 tables, captions, colspan/rowspan, nested table, layout table.
- Tests for header inference and merged cells.

**Manual test**

```sh
cargo run -- convert fixtures/tables.html /tmp/tables.xlsx \
  --tables all --sheet-per-table --use-captions --index-sheet

cargo run -- sheets /tmp/tables.xlsx
# _index
# Sales
# Regions
# Notes

cargo run -- convert fixtures/tables.html /tmp/tables.csv --tables all --stack --add-source-column table
```

Expected:

- 3 data sheets + index sheet.
- Captions become sheet names; duplicates get suffixes.
- Colspan/rowspan cells are expanded and reported.
- Layout table below `--min-rows`/`--min-cols` is skipped with an info event.
- S5 is complete.

**Exit criteria**

- No table is silently dropped; skipped tables are reported.
- Sheet names are valid for Excel (31 chars, no `[]:*?/\`).
- Index sheet row count matches the number of data tables.

---

### Phase 7 — Directory and ZIP batch ingestion

**Goal:** convert a folder or archive of mixed tabular sources in one pass.

**Single-pass scope**

- Recursive directory traversal via `walkdir`:
  - `--recursive`, `--include '*.csv,*.xlsx,*.html'`, `--exclude`.
  - deterministic ordering (lexicographic by path).
- ZIP archive read via `zip`:
  - `convert archive.zip out/ --recursive`.
  - path traversal protection (reject `..`, absolute paths).
  - zip-bomb limits (max entries, max uncompressed size, max ratio).
- TAR read optional; 7z read/write are not in v1 (7z write is cut; 7z read is deferred).
- Batch output:
  - preserve relative directory structure or flatten with `--flatten`.
  - `--name-template`.
  - per-input report summary.
- Skip unsupported files with a `format.unsupported` loss event, not a hard failure unless `--strict`.

**Out of scope**

- 7z write.
- Cloud/remote archives.
- Nested archive recursion (maybe one level only).

**Deliverables**

- `formats/archive.rs`.
- Batch output naming and collision handling.
- Fixtures: `batch/` with 5 files (csv, xlsx, html, txt, nested dir), `archive.zip`.
- Security tests for path traversal and zip bombs.

**Manual test**

```sh
cargo run -- convert fixtures/batch/ /tmp/batch/ --recursive \
  --include '*.csv,*.xlsx,*.html' --name-template '{stem}.csv'

find /tmp/batch -type f | sort

cargo run -- convert fixtures/archive.zip /tmp/archive-out/ --recursive
find /tmp/archive-out -type f | sort
```

Expected:

- 4 supported files converted; `.txt` skipped with a report event.
- Nested directory structure preserved.
- ZIP traversal attempts are rejected with a clear error.
- `--strict` fails on unsupported files; non-strict continues.

**Exit criteria**

- Deterministic output order across runs.
- No file escapes the output directory.
- Archive limits are enforced and tested.

**Implementation note (2026-11-04)**

- A batch input is a single directory or `.zip` path. `--recursive` controls
  directory descent; ZIP members are always traversed. Output must be a
  directory, not stdout.
- Batch outputs are delimited tables (`.csv`/`.tsv`). The default name template
  is `{stem}.{format}`, or `{stem}.{sheet}.{format}` when every sheet/table is
  selected. Relative directories are preserved unless `--flatten` is used;
  collisions receive stable numeric suffixes.
- Unsupported members emit `format.unsupported` and are skipped. `--strict`
  exits with code 5 before creating the output directory. The full loss-report
  file and the general strict-mode policy arrive in Phase 9.
- ZIP defaults: 10,000 entries, 512 MiB uncompressed total, and a compression
  ratio of 200 for members of at least 1 MiB. The limits are configurable with
  `--max-archive-entries`, `--max-uncompressed-size`, and
  `--max-compression-ratio`.

---

### Phase 8 — Pipeline manifest, dry-run, replay

**Goal:** make conversions reproducible (N1) before adding more formats.

**Single-pass scope**

- Define `pipeline.toml` (TOML is config, not a table output; the YAML/TOML table-write cut does not apply).
- Manifest supports:
  - multiple named inputs (`path`, `sheet`, `format`, `header`);
  - a linear chain of steps (`select`, `filter`, `sort`, `rename`, `dedupe`, `merge`, `concat`);
  - multiple named outputs (`path`, `format`, `sheet`);
  - global options (`strict`, `seed` if needed, `force`).
- `tabmux run pipeline.toml`.
- `tabmux run pipeline.toml --dry-run`:
  - resolve inputs, read schemas only, validate step graph, print output plan, write nothing.
- `--save-plan plan.json` optional: print the resolved plan for debugging.
- Deterministic execution:
  - stable sheet names, stable row order, no timestamps in data output;
  - provenance timestamp only in the manifest (Phase 9).
- Basic validation errors before opening any writer.

**Out of scope**

- Arbitrary DAG/fan-out/fan-in (v1 is a linear chain with multiple inputs/outputs).
- Expressions/scripting.
- Basic equi-join (optional stretch; not a release blocker).

**Deliverables**

- `pipeline/manifest.rs`, `pipeline/executor.rs`.
- Example `fixtures/pipeline.toml`.
- Tests for invalid manifests, missing inputs, unknown ops, cycles (trivial because linear).
- Determinism test: run twice, compare SHA-256.

**Manual test**

```toml
# fixtures/pipeline.toml
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

[[steps]]
op = "select"
input = "sales"
columns = ["id", "name", "region", "amount", "sold_on"]

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

[[outputs]]
id = "extra-csv"
input = "extra"
path = "/tmp/pipeline-extra.csv"
format = "csv"
```

```sh
cargo run -- run fixtures/pipeline.toml --dry-run
# prints inputs, steps, outputs, row estimates; writes nothing
cargo run -- run fixtures/pipeline.toml --dry-run --save-plan /tmp/plan.json
cargo run -- run fixtures/pipeline.toml
sha256sum /tmp/pipeline.csv
cargo run -- run fixtures/pipeline.toml
sha256sum /tmp/pipeline.csv   # identical
```

Expected:

- Dry-run lists the resolved plan and writes nothing.
- Re-running produces identical CSV checksums.
- A wrong column name fails before writing any output.
- Outputs are written atomically.

**Exit criteria**

- Manifest round-trips (parse → serialize → parse).
- No partial outputs on validation failure.
- Pipeline is the recommended reproducible interface; verb mode still works.

**Implementation note (2026-11-04)**

- **Manifest schema:** `version`, `[options]` (`strict`, `force`, `backup`),
  `[[inputs]]` (`id`, `path`, `sheet`, `format`, `header`), `[[steps]]`
  (`op` plus op-specific fields), and `[[outputs]]` (`id`, `input`, `path`,
  `format`, `sheet`, `header`). Unknown fields and unknown ops are rejected at
  parse time, before any file is opened.
- **Step naming:** each step has a primary result name (`id`, or `step-N` when
  omitted). Ops also publish a past-tense alias (`selected`, `filtered`,
  `sorted`, `renamed`, `deduped`, `merged`, `concatenated`) so the documented
  `input = "selected"` / `input = "filtered"` references work. Later steps
  overwrite an earlier alias; primary names must be unique and may not collide
  with input ids.
- **Linear chain:** a step's `input` defaults to the previous step's result, or
  to the only input when it is the first step. `merge`/`concat` use `with` for
  their secondary input. There is no fan-out/fan-in graph in v1; multiple
  outputs may read any named result.
- **Formats:** inputs use the same detection aliases as verb mode (`xls`,
  `xlsb`, `xlsm`, `jsonl`, `ndjson`). Outputs are limited to delimited and
  workbook writers. `format` overrides the path extension; stdout output
  requires an explicit `format`.
- **Dry-run and plan:** dry-run executes the chain in memory so unknown columns,
  schema mismatches, and output naming errors are caught, but it opens no output
  writer. `--save-plan PATH` writes the deterministic resolved plan JSON (use
  `-` for stdout). It contains no timestamps, so repeated dry-runs are
  byte-identical. A dry-run with `--save-plan` writes only the explicitly
  requested plan file.
- **Safe writes:** all output destinations are validated before staging. Inputs
  and outputs may not resolve to the same path, outputs may not share a path,
  and the plan path may not shadow an input/output. File outputs are staged with
  `SafeWriter`; if a commit fails, the error lists the already-committed
  outputs. A stdout output is emitted only after all file outputs commit.
- **Strict mode:** Phase 8 implements the basic policy: any `Warning` or
  `Error` event fails with exit code 5 before writing. The loss-report file and
  the warning-threshold policy arrive in Phase 9. `--no-strict` (or
  `strict = false`) allows the same run to write.
- **No seed:** no v1 step is randomized, so a `seed` option is not part of the
  manifest yet instead of being recorded and ignored.
- **Fixtures:** `pipeline.toml` is the replayable example; `pipeline-merge.toml`
  covers concat, `pipeline-invalid.toml` covers an unknown column, and
  `pipeline-strict.toml` covers a lossy workbook blocked by strict mode.

---

### Phase 9 — Loss report, provenance, strict mode

**Goal:** make the core promise honest: every conversion reports what it lost.

**Single-pass scope**

- Implement `LossReport` JSON schema (versioned).
- Aggregate repeated events with counts; keep first source reference.
- `--report PATH` writes JSON; `--report-markdown PATH` optional.
- `--strict`:
  - fail if any `Error` severity event exists;
  - fail if `Warning` exceeds `--max-warnings N` (default: any warning? decide: strict = any warning/error fails).
- `--lossy` explicitly allows warnings; default non-strict writes and reports.
- Provenance manifest:
  - `--manifest PATH` (default: `<output>.manifest.json`? optional).
  - tool version/git, command, options, input/output bytes + SHA-256, duration, loss summary.
- `--verify` optional: re-read the output and compare values/row counts.
- Event coverage:
  - Excel: dropped chart/style/pivot/macro/conditional formatting/comment; merged-cell expansion; cached formula only.
  - HTML: skipped table, merged-cell expansion, inferred header, sheet-name collision.
  - CSV: ragged rows, bad quotes, encoding replacement.
  - Archive: skipped member.
  - Output: sheet-name sanitization, null rendering, truncation.

**Out of scope**

- Full fidelity preservation.
- Machine-readable diff/patch.
- Contracts/validation beyond loss reporting.

**Deliverables**

- `report/mod.rs`, JSON schema/fixture, report renderer.
- `--report`, `--strict`, `--lossy`, `--manifest`.
- Fixtures: `rich.xlsx` with chart, formula, style, merged cells, pivot-like table (if possible), comment; `messy.csv` with ragged rows.
- Golden JSON reports in tests (with normalized timestamps/paths).

**Manual test**

```sh
# Non-strict: writes output + report
cargo run -- convert fixtures/rich.xlsx /tmp/rich.xlsx \
  --all-sheets --report /tmp/loss.json --manifest /tmp/manifest.json
cat /tmp/loss.json
cat /tmp/manifest.json

# Strict: must fail before writing
cargo run -- convert fixtures/rich.xlsx /tmp/rich-strict.xlsx \
  --report /tmp/loss-strict.json --strict
echo $?   # 5

# Verify checksums
sha256sum fixtures/rich.xlsx /tmp/rich.xlsx
```

Expected:

- Report lists each dropped feature with code, severity, count, and source.
- `--strict` exits 5 and does not write the output.
- Manifest has input/output checksums and tool version.
- A lossless CSV → CSV conversion still writes a report with zero events.

**Exit criteria**

- Every conversion path builds a report (even empty); `--report` writes it, otherwise a warning summary goes to stderr.
- No silent drop without an event.
- Strict mode is tested for both output and report behavior.
- Report schema is documented and versioned.

**Implementation note (2026-11-04)**

- **Report flags:** `--report`, `--report-markdown`, `--manifest`, `--strict`,
  `--lossy`, `--max-warnings`, and `--verify` are available on `convert`
  (single, split, workbook, and batch), `merge`, and `run`. Read-only
  inspection commands do not write reports.
- **Schema:** the JSON report is version 1 and is documented in
  `docs/loss-report.md` plus `docs/loss-report.schema.json`. Events aggregate
  by `(severity, code, message)`, keep the first source reference, and record
  occurrence counts. The summary counts occurrences by severity.
- **Strict policy:** any `error` event fails; warning occurrences above
  `--max-warnings` (default 0) fail. `--lossy` explicitly allows warnings and
  conflicts with `--strict`; for `run`, `--lossy` is an alias for `--no-strict`
  and overrides `strict = true`.
- **Report on failure:** a strict-blocked conversion still writes the requested
  loss report, then exits 5 before opening any data writer. The provenance
  manifest is written only after a successful data write.
- **Provenance:** the manifest records tool name/version/git, RFC 3339 start
  time, duration, argv, options, input/output bytes and SHA-256, loss summary,
  and `deterministic: true`. Stdin is omitted because it has no stable
  checksum. The manifest itself contains a timestamp; data outputs remain
  deterministic.
- **Verification:** `--verify` re-reads each output and compares columns, sheet
  names, row counts, and rendered values. File outputs are verified after
  commit; stdout outputs are verified from the in-memory buffer.
- **Atomicity:** `--split-sheets` now stages all outputs before committing, and
  batch mode already did. A commit failure names the outputs already written.
- **Post-write summary:** successful writes name the requested report and
  manifest files, the lossy category/item counts, and the strict outcome, for
  example `loss severity: lossy (7 categories, 8 items; --strict would exit
  5)`. Global workbook properties are shown as `styles(dropped)`,
  `macros(dropped)`, and `pivot_caches(dropped)` rather than `(1)`. Dry-runs
  confirm requested report files separately and never claim a manifest.
- **Strict whitelist:** `excel.pivot_values_materialized`, merged-cell
  expansion, and `excel.formula_cached_value_only` are normal transformations
  and do not fail `--strict` or count toward `--max-warnings`. They remain in
  the report and the lossy-item counts exclude them.
- **Report-backed stderr:** when a report is requested, summary mode prints a
  few aggregated category lines and points at the report. `--verbose` prints
  per-event details. Merged-cell expansion and pivot-value materialization are
  separate info lines. The display matrix is explicit: workbook-only features
  (chart/style/macro/pivot/comment/conditional formatting/data validation/
  merged expansion) are hidden for CSV/TSV output; value-affecting events
  (`formula_cached_value_only`, encoding replacement) are always shown.
- **Formula and pivot semantics:** formula events aggregate into one entry
  with `count = N` and the note “cached value may be stale; formula text not
  preserved”. `excel.pivot_cache_dropped` covers lost pivot definitions;
  `excel.pivot_values_materialized` records that worksheet values are exported
  as plain cells. A fixture test asserts the `Pivot` sheet’s values survive.
- **Fixtures:** `fixtures/rich.xlsx` (chart, style, formula, merged range,
  conditional formatting, data validation, comment, pivot cache, macro),
  `fixtures/messy.csv` (ragged rows), and
  `fixtures/reports/sample-report.json` (golden report).

---

### Phase 10 — DuckDB export and simple query

**Goal:** complete S1 without building a SQL platform.

**Single-pass scope**

- Add `duckdb` behind `sql-duckdb`.
- `tabmux to duckdb <input> --sheet NAME --table NAME -o DB`:
  - create or replace table;
  - infer column types;
  - no upsert, no federation, no query cache;
  - transaction per import; rollback on failure.
- `tabmux query <db.duckdb> "<SQL>"`:
  - output CSV/TSV/JSON/pretty table;
  - read-only by default;
  - no writes/DDL.
- `tabmux query --sql` over input files: optional basic support via `read_csv_auto`/`read_xlsx`? Keep optional; not required for S1.
- Verification:
  - row count matches input;
  - optional `--verify` checks a value checksum.
- Clear error when the DuckDB feature is not compiled in.

**Out of scope**

- Federation, cloud, Postgres/MySQL, upsert, query cache, `EXPLAIN` UI.
- Advanced SQL helpers.

**Deliverables**

- `formats/duckdb.rs` (feature-gated).
- `to duckdb` and `query` commands.
- Fixtures: `sales.xlsx`, `regions.csv`.
- Tests: import, count, query output, rollback on bad data.

**Manual test**

```sh
cargo run --features sql-duckdb -- to duckdb fixtures/sales.xlsx \
  --sheet Sales --table sales -o /tmp/warehouse.duckdb

cargo run --features sql-duckdb -- query /tmp/warehouse.duckdb \
  "select count(*) as n from sales"

cargo run --features sql-duckdb -- query /tmp/warehouse.duckdb \
  "select region, sum(amount) as total from sales group by region order by total desc" \
  -f csv
```

Expected:

- Import creates one table with inferred types.
- Count matches `tabmux count`.
- Query output is valid CSV; diagnostics go to stderr.
- Import failure leaves the database unchanged (transaction rollback).
- S1 is complete.

**Exit criteria**

- Feature-gated build works; default build does not require DuckDB.
- No write path other than `to duckdb`.
- Query has a hard read-only guard.

---

### Phase 11 — Hardening, fixtures, conformance

**Goal:** make v1 credible before packaging it.

**Single-pass scope**

- Build a messy-fixture corpus:
  - CSV: quotes, newlines, BOM, CRLF, semicolon, ragged, Latin-1, empty.
  - XLSX: multi-sheet, dates, merged cells, formula, chart, style, hidden sheet, duplicate-ish names.
  - ODS: multi-sheet, dates, merged cells.
  - HTML: 20 tables, captions, colspan/rowspan, nested, layout tables.
  - Archives: ZIP with traversal attempt, large-entry bomb, unsupported file.
- Tests:
  - unit + integration + golden files;
  - property tests for CSV round-trip and type inference;
  - fuzz targets for CSV/XLSX/HTML (nightly, non-blocking);
  - security tests: path traversal, zip bomb, formula injection warning, malformed HTML/ZIP.
- Performance smoke:
  - 100 MB CSV filter/select under a documented time/memory budget;
  - 100k-row XLSX read/write under a documented budget.
- CI:
  - Linux/macOS/Windows;
  - `fmt`, `clippy -D warnings`, `test`, `deny`, `audit`;
  - feature matrix: `default`, `excel`, `html`, `sql-duckdb`.

**Out of scope**

- Full external-memory engine.
- Continuous fuzzing infrastructure.
- Release packaging and installers (Phase 12).

**Manual test**

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --features excel,html
cargo test --features sql-duckdb
cargo deny check
cargo audit
./scripts/smoke.sh
```

Expected:

- All checks pass.
- Messy fixtures produce reports, not panics.
- Every supported format has a round-trip or conversion test.
- Security fixtures fail safely.

**Exit criteria**

- No known silent-data-loss bug.
- Every v1 format has a fixture and a conversion test.
- Loss reports are golden-tested.
- Docs match the shipped scope.

---

### Phase 12 — Release packaging and documentation

**Goal:** produce the first shippable artifact.

**Single-pass scope**

- `cargo build --release`; binary size check.
- Run `scripts/smoke.sh` against the release binary.
- Update README, `docs/features.md`, `docs/devplan.md`, and `docs/dependencies.md` to match the shipped behavior.
- Produce checksums and an SBOM.
- Tag `v0.1.0`.
- Optional: `cargo-dist` config for Linux/macOS/Windows archives.
- Document optional LibreOffice bridge (not bundled).

**Out of scope**

- Homebrew/Scoop/Winget submissions (post-release).
- Code signing/notarization unless already available.
- Continuous release automation beyond a tag.

**Manual test**

```sh
cargo build --release
./target/release/tabmux --version
./target/release/tabmux formats
./scripts/smoke.sh ./target/release/tabmux
sha256sum target/release/tabmux
```

Expected:

- Release binary runs the full smoke suite.
- Startup and binary size are within documented budgets.
- `--version` includes git hash and feature flags.
- Docs match the release scope.

**Exit criteria**

- `v0.1.0` tag exists with checksums/SBOM.
- Release notes list scope, non-goals, and known fidelity limits.
- No stale documentation references a discontinued feature as planned.

---

## 7. Fixture plan

Create fixtures incrementally; never add a feature without one.

### 7.1 Text fixtures

| Fixture | Purpose |
|---|---|
| `basic.csv` | Simple header + rows, selection, count |
| `quoted.csv` | Commas, quotes, CRLF, multiline cells |
| `bom.csv` | UTF-8 BOM handling |
| `latin1.csv` | Encoding detection/replacement |
| `semicolon.csv` | Delimiter sniffing |
| `ragged.csv` | Ragged-row policy |
| `wide.csv` | Wide table, multiline cells, column selection |
| `a.csv`, `b.csv` | Identical schemas for merge |
| `c.csv` | Reordered/missing columns for schema diff |
| `d.csv` | Type conflicts |
| `data.jsonl` | JSONL input, nested flattening |

### 7.2 Workbook fixtures

| Fixture | Purpose |
|---|---|
| `sales.xlsx` | 3 sheets, dates, merged cells, formula, chart, style |
| `sales.ods` | 2 sheets, dates, merged cells |
| `rich.xlsx` | Chart, style, formula, comment, conditional formatting (loss report) |
| `hidden.xlsx` | Hidden sheet, duplicate-ish names, empty sheet |
| `large.xlsx` | 100k rows × 20 cols (generated by script, not committed) |

### 7.3 Document/archive fixtures

| Fixture | Purpose |
|---|---|
| `tables.html` | 3 tables, captions, colspan/rowspan, nested table, layout table |
| `twenty-tables.html` | Sheet-per-table stress test |
| `batch/` | Mixed CSV/XLSX/HTML/TXT + nested directory |
| `archive.zip` | Mixed members + unsupported file |
| `traversal.zip` | `../escape.csv` attempt (must be rejected) |
| `bomb.zip` | High compression ratio / entry count (must be limited) |

### 7.4 Pipeline fixtures

| Fixture | Purpose |
|---|---|
| `pipeline.toml` | Select/filter, CSV + XLSX outputs |
| `pipeline-invalid.toml` | Unknown op/column, must fail before write |
| `pipeline-strict.toml` | Strict mode with a lossy workbook |

---

## 8. Release milestones

| Version | Contents | Manual demo |
|---|---|---|
| `0.1.0-alpha.1` | Phases 0–1 | CSV/TSV/JSON round-trip, safe writes |
| `0.1.0-alpha.2` | Phases 2–3 | XLSX/ODS → CSV, all sheets, row/column |
| `0.1.0-alpha.3` | Phases 4–5 | Multi-sheet workbook, merge |
| `0.1.0-beta.1` | Phases 6–7 | HTML → workbook, directory/ZIP batch |
| `0.1.0-beta.2` | Phases 8–9 | Pipeline replay, loss report, strict mode |
| `0.1.0` | Phases 10–12 | DuckDB export/query, hardening, release packaging |

No release after `0.1.0` should add a discontinued feature without a new scope decision.

---

## 9. Phase acceptance checklist

Copy this into every phase issue:

```markdown
## Phase N: <name>

### Scope
- [ ] Exactly one coherent slice
- [ ] No discontinued features touched
- [ ] No new format without fixture + loss policy

### Implementation
- [ ] Code merged behind the right feature flag (if optional)
- [ ] Errors include file/sheet/row/column context
- [ ] All writes use SafeWriter
- [ ] Loss events emitted for unsupported content
- [ ] Deterministic output (stable order/names)

### Tests
- [ ] Unit tests
- [ ] Integration test with fixture
- [ ] Golden output or schema assertion
- [ ] Failure-path test (bad input, no partial output)

### Manual demo
- [ ] Commands documented in devplan.md
- [ ] Smoke script block added
- [ ] Output verified by an independent reader where possible

### Docs
- [ ] `docs/features.md` scope still accurate
- [ ] `docs/dependencies.md` still accurate
- [ ] README/devplan updated if behavior changed

### Exit
- [ ] `cargo fmt --check`
- [ ] `cargo clippy -- -D warnings`
- [ ] `cargo test`
- [ ] `./scripts/smoke.sh`
```

---

## 10. Risk register for v1

| Risk | Impact | Mitigation |
|---|---|---|
| Scope creep from "just one more format" | Misses the conversion-core goal | Phase contract + explicit cut list |
| Excel fidelity promises | User disappointment / support burden | Publish a fidelity matrix; report loss; `--strict` |
| HTML table heuristics | Wrong headers/sheets | Confidence warnings; `--min-rows/cols`; preview/dry-run |
| Merged cells/date edge cases | Silent corruption | Fixtures + round-trip tests + explicit policies |
| ZIP/path traversal | Security | Canonicalize, reject `..`, limit entries/size/ratio |
| CSV formula injection | Security | Warn/sanitize on export (`=`, `+`, `-`, `@`) |
| DuckDB build time | CI slowdown | Feature-gate; cache; separate job |
| Loss report noise | Users ignore warnings | Stable codes, severity, counts, concise Markdown summary |
| Atomic multi-output failure | Partial results | Stage then commit; report committed outputs |
| Non-deterministic sheet names | Pipeline replay fails | Sanitize + stable collision suffixes; tests |

---

## 11. What comes after v1

Only after v1 ships and is stable:

1. **N4 diff/patch/merge** — table diff, portable patch, three-way merge.
2. **N5 contracts/drift** — schema/contract validation and drift reports.
3. **Advanced N9** — external-memory sort/join/group with spill.
4. **N13 text/log mining** — logfmt/LTSV/Grok/fuzzy joins (reintroduce only with a clear user need).
5. **N11 time-series** — only if it does not become a separate analytics product.
6. **N18 data dictionary/lineage** — extend the provenance manifest into a data dictionary.
7. **N20 full harness** — golden/property/fuzz/benchmark suite beyond the v1 quality gate.

Discontinued features (geo, graph, ML, NL, PII, plugins, serve/MCP, cloud/federation, dashboards, mail merge, TUI) require a new product decision and a separate roadmap. They are not "someday" items in this plan.

---

## 12. First three actions

1. Create the Phase 0 issue from the checklist above.
2. Add the fixture catalog and `scripts/smoke.sh` skeleton.
3. Implement `tabmux formats` with only the in-scope matrix; use it as the living scope guard.
