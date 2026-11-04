# tabmux feature specification

> **Status:** shipped v0.1.0 feature inventory (Phases 0–12), 2026-11-04  
> **Companion documents:** [`analogues.md`](./analogues.md) (competitive research), [`dependencies.md`](./dependencies.md) (crate/tooling plan), and [`devplan.md`](./devplan.md) (authoritative v1 phase plan).  
> **Goal:** define the feature surface of a single-binary, FFmpeg-style Swiss Army knife for tabular data.

> **Scope reset (2026-11-04).** The product is now focused on the conversion core:
>
> **N1 reproducible pipeline + N2 messy-source ingestion + N3 workbook structure/value fidelity + N17 loss report + N21 safe execution.**
>
> **Discontinued:** geo analytics (N12), graph analytics (N14), statistics/ML/forecasting (N15), document rendering/mail merge (N19), terminal dashboards (N22), NL copilot (N7), serve/MCP/watch/chat (N10), WASM plugins (N16), federated queries/cloud I/O (N8), PII/synthetic data (N6), TUI viewer, embedded scripting/jq, fuzzy/as-of/non-equi joins, pivot/crosstab/correlation, advanced SQL/federation/upsert/query cache, advanced Excel fidelity (charts, pivot caches, macros, full formula recalculation, conditional formatting, style cloning), DBF, LTSV/logfmt, fixed-width, RST/LaTeX, Avro/ORC, 7z write, YAML/TOML table write, XML XPath, and exotic compression/locale support.
>
> **Deferred (not v1 blockers):** N4 table diff/patch/merge, N5 contracts/drift, advanced N9 external-memory spill, N11 time-series, N13 text/log mining, N18 data dictionary/lineage, full N20 test harness.
>
> Sections 4 and 5 below keep the broader research and idea inventory for context. **The authoritative v1 scope and phase order are in [`devplan.md`](./devplan.md).**

---

## 1. Vision and design principles

### 1.1 One-sentence vision

> **`tabmux` reads any tabular source, applies a typed, composable pipeline, and writes any tabular target — with SQL, terminal exploration, and workbook-aware output when you need them.**

### 1.2 The FFmpeg analogy

FFmpeg is successful because it separates four concerns cleanly:

1. **Demuxers** turn containers into streams.
2. **Decoders** turn streams into frames.
3. **Filters** transform frames.
4. **Muxers/encoders** turn frames back into containers.

`tabmux` should do the same for tables:

| FFmpeg concept | tabmux concept |
|---|---|
| Container / demuxer | File format + container (CSV, XLSX workbook, HTML document, ZIP archive, database) |
| Stream | Table / sheet / query result |
| Frame | Row (or column batch) |
| Filter | Select, filter, rename, merge, concat, … |
| Filtergraph | Pipeline DAG with named streams |
| Muxer / encoder | Output format + workbook assembly |
| `-map` | Choose which input table/sheet goes where |
| `-f` | Explicit input/output format override |
| `-y` / `-n` | Overwrite / no-clobber |
| `-progress` | Machine-readable progress on stderr |
| `-loglevel` | Diagnostics verbosity |
| `-filter_complex` | Multi-input, multi-output table graph |
| Presets | Named pipeline templates (`clean`, `report`, `warehouse`) |

### 1.3 Principles

1. **Format-agnostic at the boundary, typed in the middle.** Every reader produces the same table model; every writer consumes it.
2. **Streaming first, materialization when necessary.** Common operations must work in bounded memory; sort/join/aggregate may spill to disk.
3. **Two interfaces, one engine.** Small composable verbs (`tabmux select`, `tabmux join`) and one pipeline mode (`tabmux -i … -o …`) share the same implementation.
4. **SQL is a power tool, not the only tool.** DuckDB is available for complex queries, but simple operations do not require SQL.
5. **Workbook-aware, not workbook-limited.** Sheets, styles, formulas, and multi-table containers are first-class, but plain CSV remains the simplest path.
6. **Safe by default.** No silent overwrites, no silent data loss, no hidden network access. Destructive operations support `--dry-run`, previews, and atomic writes.
7. **Deterministic and reproducible.** Seeds are explicit, pipeline files are replayable, and outputs carry provenance.
8. **Extensible without forking.** A stable plugin ABI and a declarative pipeline format let users add readers, writers, and transforms.
9. **Terminal-native.** Clear tables, progress, diagnostics, and shell completions are part of the product. There is no TUI in v1.
10. **Single static binary by default.** Optional features may pull in heavy engines, but the core install is one file with no runtime.

---

## 2. CLI model

### 2.1 Two modes

**Verb mode** — Unix-style, one operation per invocation:

```sh
tabmux select sales.csv --columns date,region,amount
tabmux filter sales.csv --where 'amount > 100'
tabmux join sales.csv regions.csv --on region_id=id --type left
tabmux convert book.ods book.csv --sheet all
```

**Pipeline mode** — FFmpeg-style, multiple inputs/outputs and a graph:

```sh
tabmux \
  -i sales.xlsx#Sheet1 \
  -i regions.csv \
  --filter 'amount > 100' \
  --join 'regions:left:region_id=id' \
  --select 'date,region,amount' \
  --sort 'date:desc' \
  -o report.xlsx#Summary
```

### 2.2 Global conventions

| Convention | Behavior |
|---|---|
| `-i, --input` | Input source; repeatable. Supports `path`, `path#sheet`, `path#table`, `-` for stdin, URLs, and database URLs. |
| `-o, --output` | Output target; repeatable. Supports `path`, `path#sheet`, `-` for stdout. |
| `-f, --format` | Explicit format override for the next input/output. |
| `--input-format` / `--output-format` | Format override without positional ambiguity. |
| `--sheet` / `--table` | Select a sheet/table inside a container. |
| `--all-sheets` | Read/write every sheet. |
| `--split-sheets` | Write one output file per sheet. |
| `-y, --yes` | Overwrite existing outputs. |
| `-n, --no-clobber` | Never overwrite; fail if output exists. |
| `--dry-run` | Show the plan, schema, sample, and loss warnings; do not write. |
| `--explain` | Show the execution plan and cost estimates. |
| `--progress` | Machine-readable progress on stderr (`json` or `plain`). |
| `--quiet` / `--verbose` / `--debug` | Diagnostic levels (`--quiet` = none, `--verbose` = full, `--debug` includes filtered-out row detail). |
| `--diagnostics summary\|full\|none` | Explicit diagnostic detail policy. Read-only commands default to summary. |
| `--fidelity-report` | Show workbook-fidelity events even when the command is read-only. |
| `--report PATH` / `--report-markdown PATH` | Write the machine-readable or Markdown loss report. |
| `--manifest PATH` | Write the provenance manifest with input/output checksums. |
| `--strict` / `--lossy` / `--max-warnings N` | Strict loss policy and explicit warning allowance. |
| `--verify` | Re-read outputs and compare values and row counts. |
| `--color auto/always/never` | Color policy; respects `NO_COLOR`. |
| `--memory-limit` | Soft limit before spilling to disk. |
| `--temp-dir` | Spill directory. |
| `--seed` | Deterministic seed for sampling/shuffling/synthesis. |
| `--config` | Config file path. |
| `--json` | Machine-readable output for inspection commands. |

### 2.3 Exit codes

| Code | Meaning |
|---|---|
| `0` | Success |
| `1` | User/data error (bad input, validation failure) |
| `2` | CLI usage error |
| `3` | I/O error |
| `4` | Unsupported format/feature |
| `5` | Lossy conversion blocked by `--strict` |
| `6` | Partial success (some rows/tables failed; see report) |
| `130` | Interrupted |

---

## 3. Starter scenarios (the five ideas)

These are the first end-to-end use cases and should be part of the MVP acceptance suite.

### S1. Convert a table to DuckDB for SQL queries

```sh
# Import one or more tables into a persistent DuckDB database
tabmux to duckdb sales.xlsx --sheet Sheet1 --table sales -o warehouse.duckdb
tabmux to duckdb regions.csv --table regions -o warehouse.duckdb

# Query it later (read-only)
tabmux query warehouse.duckdb \
  "select region, sum(amount) as total
   from sales join regions on sales.region_id = regions.id
   group by region order by total desc"

# Export a query result
tabmux query warehouse.duckdb "select * from sales limit 10" -f csv
```

**Requirements:** type inference, create/replace table, transaction/rollback, row-count verification, and read-only query. No federation, upsert, query cache, `EXPLAIN` UI, or network access in v1.

> **Build note (2026-11-04):** the `sql-duckdb` feature currently uses the
> embedded SQLite engine because the offline registry has no `duckdb` crate.
> The CLI and transaction/read-only guarantees are the same; the database file
> is SQLite rather than DuckDB's native format. See
> [`implementation-notes.md`](./implementation-notes.md).

### S2. Print a specific row or column to the terminal

```sh
# Column selection by name, index, range, or glob
tabmux select data.xlsx#Sheet1 --columns name,email
tabmux select data.csv --columns 1,3-5

# One row by number (1-based, data rows only by default)
tabmux row data.xlsx --sheet Sheet1 --number 42

# One row by key
tabmux row users.csv --where 'id == 1234'

# One cell
tabmux cell data.csv --row 42 --column email

# Pretty terminal output, or raw for piping
tabmux select data.csv --columns name,email --pretty
tabmux select data.csv --columns name,email --raw
```

**Requirements:** exact row/column addressing, header-aware indices, `--no-header`, `--raw`, `--pretty`, and no accidental parsing of the whole file for indexed formats where possible.

### S3. Convert ODS and XLSX to CSV

```sh
# One sheet
tabmux convert book.xlsx out.csv --sheet Sheet1

# Every sheet to separate files
tabmux convert book.ods out/ --all-sheets --split-sheets --name-template '{stem}.{sheet}.csv'

# Every sheet concatenated into one CSV with a source sheet column
tabmux convert book.xlsx out.csv --all-sheets --stack --add-source-column sheet
```

**Requirements:** XLSX/XLS/XLSB/ODS read, sheet listing, date/number/merged-cell policies, formula cached values, encoding/line-ending control, and clear warnings for unsupported features.

### S4. Merge two tables with identical columns

```sh
# Strict: schemas must match exactly
tabmux merge a.csv b.csv -o all.csv --mode strict

# Loose: union columns, fill missing with empty/null
tabmux merge a.csv b.csv -o all.csv --mode loose --fill-null

# Keep provenance and deduplicate
tabmux merge a.csv b.csv -o all.csv --add-source-column source --dedup

# Merge many files from a directory
tabmux merge 'data/*.csv' -o all.csv --mode strict --progress
```

**Requirements:** header comparison, case/whitespace normalization options, column-order reconciliation, type unification, provenance column, dedup, and a machine-readable schema-mismatch report.

### S5. Convert every HTML table to XLSX, one sheet per table

```sh
# All tables from a local HTML file
tabmux convert page.html report.xlsx --tables all --sheet-per-table

# From a local directory, with captions as sheet names and an index sheet
tabmux convert 'pages/**/*.html' report.xlsx \
  --tables all --sheet-per-table --use-captions --index-sheet

# Custom sheet naming
tabmux convert 'pages/**/*.html' report.xlsx \
  --tables all --sheet-per-table --sheet-name '{file}.{table}'
```

**Requirements:** robust HTML parsing, `colspan`/`rowspan` expansion, header-row detection, nested tables, caption/title extraction, duplicate sheet-name resolution, an index sheet with source metadata, and a confidence/warning report.

---

## 4. Basic features (parity research)

> **Historical parity research, not a v1 commitment.** The tables below record what the analogues do and what a broad product could cover. The v1 base is deliberately narrower; see the scope reset above and [`devplan.md`](./devplan.md).

These features already exist in one or more analogues and are necessary just to be competitive. IDs are stable references for issues and roadmap discussions.

### 4.0 Base-scope cuts

The following are explicitly **out of the base scope** because they bloat the format/support matrix or distract from the conversion core:

| Cut from base | Reason |
|---|---|
| DBF, LTSV/logfmt, fixed-width | Rare in core conversion scenarios; specialized importers exist |
| RST/LaTeX output | Documentation tooling, not table conversion |
| Avro/ORC | Columnar analytics formats; Parquet/DuckDB cover the supported path |
| 7z write | Rare; ZIP covers the common case. 7z read may be added later if needed |
| YAML/TOML **table write** | Read/config support remains; table output adds little for the core |
| XML XPath | Full XML querying is out of scope; basic XML table extraction may be deferred |
| Bzip2/xz/lz4/snappy, locale-aware formatting | Exotic compression/locale support is not core; gzip/zstd/ZIP remain |
| Advanced SQL (federation, upsert, query cache, Postgres/MySQL) | DuckDB import + simple query is enough for S1 |
| Advanced Excel fidelity (charts, pivot caches, macros, full formula recalc, conditional formatting, style cloning) | High complexity, low conversion value; report loss instead |
| TUI, embedded scripting/jq | Separate product surfaces; no place in the conversion core |
| Fuzzy/as-of/non-equi joins, pivot/crosstab/correlation | Analytics, not conversion |
| Geo, graph, ML/statistics, PII/synthetic, NL, plugins, serve/MCP, cloud, dashboards, mail merge | Discontinued; standalone products with domain/security/legal complexity |

### 4.1 I/O and format support

| ID | Feature | Details | Found in |
|---|---|---|---|
| B1.1 | Format autodetection | Extension + magic bytes + content sniffing; explicit overrides | qsv, DuckDB, xan, csvkit |
| B1.2 | stdin/stdout | `-` for both; data on stdout, diagnostics on stderr | All Unix tools |
| B1.3 | CSV/TSV/DSV dialects | RFC 4180, custom delimiter/quote/escape, comments, blank-line policy, BOM, CRLF/LF | csvkit, xsv/qsv, Miller, csvtk |
| B1.4 | Encoding support | UTF-8, UTF-16, Latin-1, Windows-125x, encoding detection, output encoding | csvkit, qsv, csvtk |
| B1.5 | Compression | gzip, bzip2, xz, zstd, lz4, snappy read/write; transparent by extension | csvtk, trdsql, xan |
| B1.6 | Archives | Read tables from ZIP/TAR/7z; write ZIP; recursive directory ingestion | VisiData, xan, qsv |
| B1.7 | URLs | HTTP(S) input with redirects, auth, caching, range requests | qsv, DuckDB, xan |
| B1.8 | XLSX/XLS/XLSB read | Sheet listing, dimension handling, dates, formulas (cached values), merged cells | calamine-based tools, qsv, in2csv, xlsx2csv |
| B1.9 | XLSX write | Multi-sheet, types, number formats, freeze panes, autofilter, tables, formulas, basic charts | qsv `to`, csvtk `csv2xlsx`, rust_xlsxwriter |
| B1.10 | ODS read/write | Read and write OpenDocument spreadsheets; multi-sheet | qsv `excel`/`to`, LibreOffice, ssconvert |
| B1.11 | HTML tables in | Extract one/all tables, header detection, colspan/rowspan | VisiData, Pandoc, xan `scrape` |
| B1.12 | HTML out | Standalone responsive table with CSS; one or many tables | csvtk, csvkit, trdsql |
| B1.13 | JSON/JSONL/NDJSON | Nested-object flattening policies, JSON Pointer/jq selection, JSONL streaming | csvkit, qsv, Miller, trdsql |
| B1.14 | YAML/TOML/XML | Read/write; XML table extraction; XPath selection | dasel, yq, trdsql |
| B1.15 | Markdown/RST/LaTeX/ASCII | Table output with alignment and escaping | csvtk, qsv, trdsql, Pandoc |
| B1.16 | Fixed-width | Schema-driven read/write; auto-detection helpers | in2csv, qsv, trdsql |
| B1.17 | DBF | Read/write dBase files | csvkit, qsv, ssconvert |
| B1.18 | LTSV/logfmt | Read/write labeled formats | trdsql, qsv |
| B1.19 | Parquet/Arrow/Avro/ORC | Read/write Parquet/Arrow/Avro; ORC read where feasible | qsv, DuckDB, Polars, xan |
| B1.20 | SQLite/DuckDB/Postgres/MySQL | Read/write tables; connection URLs; append/replace/upsert | qsv, DuckDB, trdsql, csvkit |
| B1.21 | Multi-table containers | Sheets, database tables, HTML documents, archives as table collections | VisiData, qsv, DuckDB |
| B1.22 | Type inference | Null/boolean/integer/float/decimal/date/datetime/time/text; per-column overrides | csvkit, qsv, DuckDB, xan |
| B1.23 | Null semantics | Configurable null tokens, empty-vs-null distinction, output null representation | csvkit, trdsql, DuckDB |
| B1.24 | Number/date formatting | Locale-aware parsing/formatting, Excel serial dates, timezones | csvkit, qsv, csvtk, Gnumeric |
| B1.25 | Encoding/BOM/newline control | Explicit output encoding, BOM on/off, CRLF/LF/CR | csvkit, csvtk, qsv |
| B1.26 | Recursive globs | `**/*.csv`, exclude patterns, symlink policy | qsv, DuckDB, xan |

### 4.2 Inspection and profiling

| ID | Feature | Details | Found in |
|---|---|---|---|
| B2.1 | `head`/`tail`/`slice` | First/last N rows, ranges, negative offsets | xsv/qsv, xan, csvtk, Miller |
| B2.2 | `count`/`dim` | Rows, columns, file size, estimated rows for streams | All |
| B2.3 | `headers` | Column names, indices, types, duplicates | xsv/qsv, xan, csvtk |
| B2.4 | `view`/`pretty`/`table` | Aligned, colored, width-aware terminal table | csvkit, xsv/qsv, xan, csvtk, trdsql |
| B2.5 | `flatten`/vertical view | One record per block; useful for wide tables | xsv/qsv, xan, VisiData |
| B2.6 | `search`/`grep` | Regex/literal, per-column, invert, count-only, include/exclude | csvkit, xsv/qsv, xan, csvtk |
| B2.7 | `filter` expressions | Typed expressions with null-safe semantics | xan, qsv, csvtk, Miller |
| B2.8 | `stats`/`summary` | Count, nulls, distinct, min/max, mean, stddev, median, quantiles | csvkit, xsv/qsv, xan, csvtk |
| B2.9 | `frequency`/`unique` | Value counts, cardinality, top-k | xsv/qsv, xan, csvtk |
| B2.10 | `sample`/`shuffle`/`top` | Reservoir, Bernoulli, stratified, weighted; deterministic seed | xsv/qsv, xan, csvtk |
| B2.11 | `schema`/`sniff` | Dialect, header row, column types, row width, encoding | qsv, DuckDB, csvkit |
| B2.12 | `describe` | Human-readable profile with type suggestions and warnings | qsv, VisiData, pandas-like tools |

### 4.3 Selection, shaping, and cleaning

| ID | Feature | Details | Found in |
|---|---|---|---|
| B3.1 | Column selection | By name, index, range, glob, regex, exclusion, reorder, duplicate | All |
| B3.2 | Column rename | Exact, regex, case conversion, safe names | qsv, csvtk, xan |
| B3.3 | Computed columns | Expressions, arithmetic, string/date functions, conditionals | xan, qsv, Miller, csvtk |
| B3.4 | Regex replace | Per-column, capture groups, replacement templates | qsv, csvtk, xan |
| B3.5 | Fill/blank/complete | Fill empty, fill down, blank repeated values, complete missing keys | qsv, xan, csvtk |
| B3.6 | Deduplicate | Exact, key-based, keep first/last, stable order | All |
| B3.7 | Sort | Lexical, numeric, natural, date, case-insensitive, multi-key, reverse | All |
| B3.8 | Transpose | Rows ↔ columns with header handling | qsv, csvtk, xan, Miller |
| B3.9 | Pivot/melt | Long ↔ wide, multi-value, aggregation | qsv, csvtk, xan, VisiData |
| B3.10 | Explode/implode | Split delimited cells into rows; join rows into cells | qsv, xan, csvtk |
| B3.11 | Separate columns | Split one column into N columns by delimiter/regex/width | csvtk, xan, qsv |
| B3.12 | Split/partition | By row count, chunk count, file size, column value | xsv/qsv, xan, csvtk |
| B3.13 | Date/time formatting | Parse many formats, format output, timezone conversion | qsv, csvtk, xan |
| B3.14 | Number/currency formatting | Thousands separators, decimals, rounding, currency parsing | csvtk, qsv, xan |
| B3.15 | Bins/window | Numeric binning, rolling/cumulative windows, lag/lead | xan, qsv, SQL |
| B3.16 | Header normalization | Lowercase, trim, replace spaces, deduplicate, database-safe names | qsv, csvtk |

### 4.4 Combining tables

| ID | Feature | Details | Found in |
|---|---|---|---|
| B4.1 | Concatenate by rows | Strict/loose schema matching, fill missing, source column | csvkit, csvtk, qsv, xan, Miller |
| B4.2 | Paste by columns | Align by row position, pad shorter inputs | csvtk, qsv `cat` |
| B4.3 | Join | Inner, left, right, full, cross, anti, semi | qsv, xan, csvtk, Miller, DuckDB |
| B4.4 | Advanced join | As-of, non-equi, fuzzy, case-insensitive, composite keys | qsv `joinp`, DuckDB, Polars |
| B4.5 | Set operations | Union, intersect, except, distinct | csvtk, SQL, xan |
| B4.6 | Merge sorted inputs | Streaming merge of already-sorted files | xan, Miller |
| B4.7 | Workbook merge | Combine sheets/workbooks into one file | ssconvert, LibreOffice, qsv |

### 4.5 Aggregation and analytics

| ID | Feature | Details | Found in |
|---|---|---|---|
| B5.1 | Group-by aggregation | Count, sum, mean, min, max, median, stddev, variance, first/last, list | qsv, xan, Miller, SQL |
| B5.2 | Frequency/crosstab | One- and two-way tables, top-k, percentages | xsv/qsv, xan, csvtk |
| B5.3 | Correlation/covariance | Pearson/Spearman, pairwise matrix | csvtk, xan, qsv `moarstats` |
| B5.4 | Histograms/box/line/bar/scatter | Terminal and file output | csvtk, xan, qsv `viz` |
| B5.5 | Pivot tables | Multi-dimension aggregation with subtotals | qsv `pivotp`, xan, Excel |
| B5.6 | Window functions | Running totals, rolling averages, rank, lag/lead | xan, qsv, SQL |
| B5.7 | SQL aggregates | Full SQL aggregate/window semantics | DuckDB, SQLite, Postgres |

### 4.6 SQL and databases

| ID | Feature | Details | Found in |
|---|---|---|---|
| B6.1 | Query files as tables | One or many files; each file/sheet is a table | DuckDB, trdsql, q, csvkit `csvsql` |
| B6.2 | Inline/file SQL | `--sql`, `--query-file`, parameter binding | trdsql, DuckDB |
| B6.3 | Persistent DuckDB | Create/replace/append tables; transactions; indexes | DuckDB |
| B6.4 | SQLite support | Read/write, attach, query, FTS | sqlite-utils, DuckDB, trdsql |
| B6.5 | PostgreSQL/MySQL | Read/write tables, upsert, connection URLs | qsv `to`, trdsql, csvkit |
| B6.6 | `EXPLAIN`/`DESCRIBE`/`SUMMARIZE` | Query plans, schema introspection, profile output | DuckDB |
| B6.7 | Query result output | CSV/TSV/JSON/JSONL/Parquet/XLSX/ODS/table | DuckDB, trdsql, qsv |
| B6.8 | Schema introspection | List tables/columns/types; autocomplete from live DB | DuckDB, qsv |

### 4.7 Conversion and workbook operations

| ID | Feature | Details | Found in |
|---|---|---|---|
| B7.1 | N×M conversion | Any supported input to any supported output | All converters |
| B7.2 | Sheet selection | By index/name; list sheets; all sheets | xlsx2csv, in2csv, qsv, ssconvert |
| B7.3 | Per-sheet output | One file per sheet with name templates | xlsx2csv, ssconvert |
| B7.4 | Multi-input workbook | One sheet per input file/table | csvtk `csv2xlsx`, ssconvert |
| B7.5 | HTML tables → workbook | All tables to sheets (see S5; extended in N2) | Partial: VisiData, Pandoc |
| B7.6 | Styles/number formats | Bold, colors, widths, number/date formats | LibreOffice, rust_xlsxwriter |
| B7.7 | Formulas | Write formulas; read cached values; optional recalc | LibreOffice, Gnumeric, umya |
| B7.8 | Merged cells/hyperlinks | Read/expand/write; link preservation | xlsx2csv, LibreOffice |
| B7.9 | CSV dialect output | Delimiter, quote, escape, line ending, quoting policy | All |
| B7.10 | Encoding output | Explicit output charset and transliteration policy | csvkit, qsv, Gnumeric |

### 4.8 Terminal UX

| ID | Feature | Details | Found in |
|---|---|---|---|
| B8.1 | Colored aligned tables | Width-aware Unicode, numeric alignment, truncation/wrapping | csvkit, qsv, xan, csvtk, trdsql |
| B8.2 | Pager support | Auto-pager when output is a TTY | xan, csvlens |
| B8.3 | Progress/ETA | Rows/sec, bytes/sec, ETA; machine-readable option | qsv, xan, csvtk |
| B8.4 | TUI viewer | Scroll, search, filter, sort, select, copy, export | VisiData, csvlens, tabiew, qsv `lens` |
| B8.5 | Interactive prompts | Overwrite confirmation, sheet selection, column picker | qsv, csvlens |
| B8.6 | Completions/man pages | Bash/Zsh/Fish/PowerShell; column-name completion | csvtk, qsv, xan |
| B8.7 | Config/env vars | Config file, environment overrides, profiles | trdsql, csvtk, qsv |
| B8.8 | Color policy | TTY detection, `NO_COLOR`, `CLICOLOR`, `--color` | xan, qsv |

### 4.9 Scripting and automation

| ID | Feature | Details | Found in |
|---|---|---|---|
| B9.1 | Expression language | Typed row/column expressions, functions, null semantics | xan, qsv, Miller |
| B9.2 | jq engine | JSON/JSONL/NDJSON selection and transformation | trdsql, qsv |
| B9.3 | Embedded scripting | Lua/Luau, Python, Rhai (optional) | qsv `luau`/`py`, VisiData |
| B9.4 | Templates | MiniJinja/Handlebars-style row rendering | qsv `template`, Pandoc |
| B9.5 | Pipeline files | Replayable declarative pipelines | xan `run`, qsv `foreach` (partial) |
| B9.6 | Shell integration | Exit codes, stdout/stderr discipline, `-` convention | Unix tradition |
| B9.7 | Batch mode | Non-interactive execution of viewer/scripts | VisiData `-b`, qsv |

### 4.10 Quality, safety, and operations

| ID | Feature | Details | Found in |
|---|---|---|---|
| B10.1 | Schema inference | JSON Schema/Polars schema generation | qsv `schema` |
| B10.2 | Validation | Schema, type/domain/format/unique/foreign-key checks | qsv `validate`, Great Expectations |
| B10.3 | Diff | Row/cell differences, key-based comparison | qsv `diff`, daff |
| B10.4 | Dry-run/preview | Show plan, sample, and warnings before writing | Some tools via `--limit`/preview; full support is N21 |
| B10.5 | Atomic writes | Temp file + rename; backup on overwrite | Rare; full support is N21 |
| B10.6 | Deterministic seeds | Reproducible sampling/shuffling/synthesis | qsv, xan |
| B10.7 | Logging/audit | Structured logs, timings, row counts, warnings | qsv, xan |
| B10.8 | Checksums | BLAKE3/SHA-256 for inputs/outputs | qsv `blake3` |

### 4.11 Performance

| ID | Feature | Details | Found in |
|---|---|---|---|
| B11.1 | Streaming | Constant memory for select/filter/map/head | Miller, xan, csvkit |
| B11.2 | Index/mmap | Constant-time slice/count/sample on indexed files | xsv/qsv |
| B11.3 | Parallel scan | Multi-threaded parsing/aggregation | qsv, xan, Polars |
| B11.4 | External sort | Sort larger-than-RAM files with spill | qsv `extsort` |
| B11.5 | External dedup/group | Disk-backed hash/aggregation | qsv `extdedup`, DuckDB |
| B11.6 | Columnar interchange | Arrow/Parquet without row-materialization | DuckDB, Polars |
| B11.7 | Predicate/projection pushdown | Push filters/columns into readers/SQL | DuckDB, Polars |
| B11.8 | Compression-aware I/O | Stream decompress/compress without temp files | csvtk, trdsql, xan |

---

## 5. New features (idea inventory)

> **Only items marked “Core v1” are committed.** The rest are retained for context; “Discontinued” items are out of the product direction, and “Deferred” items require a new scope decision after v1.
>
> **Discontinued sections:** N6, N7, N8, N10, N12, N14, N15, N16, N19, N22. The detailed text for those sections is an idea record only, not a plan.

These are not commonly available in the analogues, or are available only in isolation. They are the reason to choose `tabmux` over a shell pipeline of existing tools.

### 5.1 Summary

| ID | Feature | One-line description | Status |
|---|---|---|---|
| N1 | Reproducible pipeline | Declarative, replayable conversion manifest with dry-run and pre-write validation | **Core v1** |
| N2 | Messy-source ingestion | CSV/TSV, XLSX, ODS, HTML tables, JSON/JSONL, directories, ZIP → clean table set | **Core v1** |
| N3 | Workbook structure/value fidelity | Sheets, values, types, dates, merged-cell expansion, basic formatting | **Core v1** |
| N4 | Table diff/patch/merge | Git-style diff, patch, and three-way merge for tables | Deferred |
| N5 | Data contracts & drift | Declarative contracts, validation, quarantine, drift detection | Deferred |
| N6 | Privacy & synthetic data | PII detection, masking, k-anonymity, differential privacy, synthesis | **Discontinued** |
| N7 | Natural-language copilot | Deterministic plan generation from natural language | **Discontinued** |
| N8 | Federated query & cloud I/O | Join files, databases, HTTP, S3/GCS/Azure in one query | **Discontinued** |
| N9 | Streaming engine | Bounded-memory CSV path in v1; external-memory sort/join/group deferred | **Core v1 (basic)** |
| N10 | Watch/serve/MCP modes | Continuous pipelines, HTTP API, AI-agent tool server | **Discontinued** |
| N11 | Time-series & event tables | Resampling, sessionization, as-of joins, gap filling | Deferred |
| N12 | Geospatial tables | GeoJSON/Shapefile/GPX/KML, CRS, spatial joins, maps | **Discontinued** |
| N13 | Text/log mining | Log parsing, tokenization, fuzzy joins, entity extraction | Deferred |
| N14 | Graph/network analytics | Edge lists, adjacency matrices, graph metrics, path queries | **Discontinued** |
| N15 | Stats/ML-lite | Tests, regression, clustering, PCA, outliers, forecasting | **Discontinued** |
| N16 | WASM plugin system | Sandboxed readers/writers/transforms with a stable ABI | **Discontinued** |
| N17 | Loss report | Predict and report exactly what a conversion will lose | **Core v1** |
| N18 | Semantic data dictionary & lineage | Inferred semantics, data dictionary, lineage, packaging | Deferred |
| N19 | Template/mail-merge reports | Per-row HTML/DOCX/PDF/email generation | **Discontinued** |
| N20 | Pipeline test harness | Golden outputs, schema assertions, performance budgets, fuzzing | Deferred (basic quality gate in v1) |
| N21 | Safe-by-default execution | Dry-run, atomic writes, backups, audit log, provenance | **Core v1** |
| N22 | Terminal dashboards | Compose tables/charts into a dashboard; export HTML | **Discontinued** |

---

### N1. Reproducible pipeline

> **v1 scope:** a declarative TOML manifest with named inputs, a **linear chain** of conversion steps, and multiple named outputs. Arbitrary fan-out/fan-in DAGs and the FFmpeg `-filter_complex` syntax below are deferred until the conversion core is stable.

**What.** A declarative pipeline of table operations with named inputs and outputs. This is more than a shell pipeline: the engine knows the schema at every edge, validates the plan before writing, and can replay the same conversion deterministically.

**Why it is new.** Existing tools are either shell pipelines (`csvkit`, `Miller`, `xan`) or SQL queries (`DuckDB`). Neither gives a first-class, schema-aware graph with multiple outputs and reusable pipeline files. FFmpeg's `-filter_complex` proves the model works for media; tables are simpler and benefit even more from static validation.

**Sketch.**

```sh
tabmux \
  -i sales.xlsx#Sheet1 \
  -i regions.csv \
  -i targets.parquet \
  -filter_complex "
    [0:t0] filter 'amount > 100'                         -> [sales];
    [sales] join [1:t0] type=left on=region_id=id        -> [enriched];
    [enriched] aggregate by=region sum=amount            -> [actual];
    [2:t0] rename 'region->region_name'                  -> [targets];
    [actual] join [targets] type=left on=region=region_name -> [report];
    [report] sort 'amount:desc'                          -> [sorted];
    [sorted] tee [csv_out][xlsx_out]
  " \
  -map '[csv_out]'  -f csv  -o report.csv \
  -map '[xlsx_out]' -f xlsx -o report.xlsx#Summary
```

A pipeline file (YAML or TOML) is the reproducible form:

```yaml
version: 1
inputs:
  sales:  { path: sales.xlsx, sheet: Sheet1 }
  regions: { path: regions.csv }
  targets: { path: targets.parquet }
steps:
  - id: sales_filtered
    op: filter
    input: sales
    expr: amount > 100
  - id: enriched
    op: join
    left: sales_filtered
    right: regions
    type: left
    on: [region_id, id]
  - id: actual
    op: aggregate
    input: enriched
    by: [region]
    aggs: { amount: sum }
outputs:
  csv:   { input: actual, path: report.csv, format: csv }
  excel: { input: actual, path: report.xlsx, sheet: Summary, format: xlsx }
```

**Requirements.**

- Static schema/type checking at every edge.
- Cycle detection; deterministic execution order.
- Fan-out and fan-in with explicit buffering/spill policies.
- Named streams and `-map`.
- Parallel execution of independent branches.
- Pipeline serialization to YAML/TOML/JSON; `tabmux run pipeline.yaml`.
- `tabmux graph --dot` to visualize the DAG.
- `--dry-run` prints the resolved graph, schemas, and estimated row counts.

**Status:** Core v1 (linear manifest; arbitrary DAG deferred).

---

### N2. Anything-to-table ingestion and workbook assembly

> **v1 scope:** HTML tables, CSV/TSV, XLSX, ODS, JSON/JSONL, recursive directories, and ZIP archives. PDF, DOCX/ODT, and EML/MSG ingestion are deferred.

**What.** Turn messy real-world sources into clean tables:

- **HTML:** all `<table>` elements, including nested tables, `colspan`/`rowspan`, `<caption>`, `<thead>`/`<tbody>`/`<tfoot>`, repeated headers, and multi-page tables.
- **PDF:** text extraction plus table heuristics (column alignment, ruling lines, whitespace); confidence scores.
- **DOCX/ODT:** tables from word-processing documents, including merged cells.
- **Email (EML/MSG):** body tables and CSV/XLSX attachments.
- **Archives/directories:** recursively read CSV/TSV/XLSX/ODS/JSON/JSONL/HTML files from ZIP archives and directories, with deterministic ordering, include/exclude filters, path-traversal protection, and archive size/ratio limits. TAR and 7z are deferred.
- **Workbook assembly:** one sheet per table, an optional index sheet, caption/file/page metadata, deduplicated sheet names, and a warning report.

This directly generalizes the user's idea #5 (HTML tables → XLSX with one sheet per table) to any document source.

**Why it is new.** Existing tools handle one source type at a time. VisiData can open many formats but does not assemble a workbook. Pandoc converts documents but not data operations. LibreOffice converts spreadsheets but not HTML tables from a document. `tabmux` can do the whole job in one command, with provenance.

**Sketch.**

```sh
tabmux ingest report.pdf \
  --tables all \
  --format xlsx \
  --sheet-per-table \
  --index-sheet \
  --report ingestion.json \
  -o report.xlsx

tabmux ingest 'inbox/*.eml' \
  --include-attachments \
  --tables all \
  -o mail.xlsx --sheet-per-table
```

**Requirements.**

- Table boundary detection and header inference.
- Merged-cell expansion policy (`--merge-policy expand|first|error`).
- Confidence score per table; `--min-confidence`.
- Sheet-name sanitization and collision handling.
- Index sheet columns: source, page/section, caption, rows, columns, confidence.
- Dry-run report with previews.
- Never silently drop a table: warn and include it or fail with `--strict`.

**Status:** Core v1 for HTML/CSV/XLSX/ODS/JSON/directories/ZIP; PDF/DOCX/EML deferred.

---

### N3. Workbook structure/value fidelity

> **v1 scope:** sheets, values, types, dates, merged-cell expansion, basic header/freeze/width/autofilter formatting, and cached formula values. Charts, pivot caches, macros, full formula recalculation, conditional formatting, and style cloning are discontinued.

**What.** Treat a workbook as a container of tables **and** presentation metadata:

- Sheets, hidden sheets, named ranges, tables, pivot caches.
- Cell styles, number formats, fonts, fills, borders, alignment, column widths, row heights.
- Merged cells, hyperlinks, comments, data validation, conditional formatting, autofilter, freeze panes.
- Formulas (read cached values, preserve formula text, optionally recalculate).
- Charts and images (preserve where possible; export where requested).
- Defined names, external links, macros (preserve only; never execute).

**Why it is new.** CSV tools throw metadata away. LibreOffice preserves it but is not a data pipeline. `tabmux` can offer two explicit modes:

- `--preserve`: keep as much workbook metadata as possible; only change the requested tables.
- `--normalize`: intentionally flatten to plain tables; report what was dropped.

**Sketch.**

```sh
# Edit one sheet in place, preserve everything else
tabmux workbook update book.xlsx \
  --sheet Sales \
  --input new_sales.csv \
  --preserve \
  -o book.updated.xlsx

# Inspect a workbook
tabmux workbook info book.xlsx --json
tabmux workbook sheets book.xlsx
tabmux workbook extract book.xlsx --sheet Sales -o sales.csv

# Assemble a new workbook from tables
tabmux workbook assemble \
  --sheet Sales=sales.csv \
  --sheet Regions=regions.csv \
  --sheet Summary=summary.csv \
  -o report.xlsx
```

**Requirements.**

- Clear fidelity matrix per format (what is preserved, translated, or dropped).
- Style cloning for new sheets/cells.
- Formula preservation and translation between formats where feasible.
- Optional formula evaluation for a documented subset; otherwise read cached values and warn.
- Round-trip test corpus covering common Excel/ODS features.
- `--strict` fails if a requested feature cannot be preserved.

**Status:** Core v1 for sheets/values/types/dates/merged cells/basic formatting; advanced fidelity discontinued.

---

### N4. Table diff, patch, and merge ("git for tables")

**What.** Schema-aware, key-based comparison and merging:

- **Diff:** added/removed/renamed columns, type changes, row additions/deletions, cell modifications, moved rows.
- **Patch:** a portable JSON/YAML patch describing changes; apply/revert.
- **Three-way merge:** base + ours + theirs; conflict detection per cell/row; conflict markers.
- **Reports:** terminal, Markdown, HTML, XLSX with highlighted changes.
- **Git integration:** register `tabmux` as a `textconv`/diff driver for CSV/XLSX.

**Why it is new.** `diff` is line-based and useless for reordered columns or key-based row changes. qsv `diff` and daff are steps in this direction, but no tool combines schema diff, cell patch, and three-way merge with workbook awareness.

**Sketch.**

```sh
tabmux diff old.csv new.csv --key id --report diff.html
tabmux diff old.xlsx new.xlsx --sheet Sales --key id --format terminal

tabmux patch apply data.csv change.patch -o data.new.csv
tabmux merge3 base.csv ours.csv theirs.csv --key id -o merged.csv
```

**Requirements.**

- Primary-key inference and explicit `--key`.
- Composite keys; null-safe comparison; numeric tolerance; date/time normalization.
- Column rename detection (similarity + position + values).
- Deterministic patch format with schema version.
- Conflict resolution strategies: `ours`, `theirs`, `manual`, `fail`.
- Large-file mode with streaming and external sort.

**Status:** Deferred.

---

### N5. Data contracts, validation, and drift detection

**What.** Declarative contracts that describe what a table must look like, plus runtime validation, quarantine, and drift detection across runs.

**Why it is new.** qsv has JSON Schema validation; Great Expectations/dbt tests live in the Python/data-engineering world. A single-binary tool that validates at ingestion, quarantines bad rows, and detects schema drift is missing.

**Sketch.**

```yaml
# contract.yaml
table: sales
columns:
  id:        { type: integer, required: true, unique: true }
  email:     { type: string, format: email, pii: true }
  amount:    { type: decimal, min: 0, max: 1000000 }
  region:    { type: string, enum: [EU, US, APAC] }
  closed_at: { type: datetime, required: false }
constraints:
  - row_count: { min: 1000 }
  - freshness: { column: updated_at, max_age: 24h }
```

```sh
tabmux validate sales.csv --contract contract.yaml \
  --quarantine bad_rows.csv \
  --report validation.json

tabmux contract infer sales.csv -o contract.yaml
tabmux contract drift baseline.yaml sales.csv --fail-on breaking
```

**Requirements.**

- Types, nullability, ranges, regex, enums, uniqueness, composite keys, foreign keys.
- Cross-column rules (e.g., `closed_at >= opened_at`).
- Freshness/row-count/freshness checks.
- Quarantine output with error reasons.
- Drift classes: compatible, widening, breaking.
- CI-friendly exit codes and JSON reports.

**Status:** Deferred.

---

### N6. Privacy, PII, and synthetic data

**What.** A privacy toolkit for tables:

- **Detect:** emails, phones, IPs, URLs, credit cards (Luhn), IBANs, national IDs, addresses, names (dictionary-based), dates of birth, coordinates.
- **Protect:** mask, redact, hash, tokenize, pseudonymize consistently, generalize (age bands, ZIP prefixes), suppress, format-preserving pseudonyms.
- **Measure:** k-anonymity, l-diversity, t-closeness warnings.
- **Aggregate safely:** Laplace/Gaussian noise for differential privacy on counts/sums/means.
- **Synthesize:** generate statistically faithful fake tables preserving schema, distributions, null rates, and column relationships; deterministic seed.

**Why it is new.** PII tools are usually SaaS or Python libraries. A local, single-binary, format-aware privacy pipeline that works on CSV/XLSX/JSON and produces a report is a strong differentiator for regulated industries.

**Sketch.**

```sh
tabmux pii scan customers.csv --report pii.json
tabmux pii mask customers.csv --columns email,phone --strategy tokenize -o safe.csv
tabmux anonymize customers.csv --k-anonymity 5 --quasi-ids age,zip,gender -o k5.csv
tabmux synthesize customers.csv --rows 100000 --seed 42 -o synthetic.csv
```

**Requirements.**

- Local-only by default; no network.
- Confidence scores and false-positive review output.
- Consistent tokenization across runs (keyed hash).
- Referential integrity across multiple tables.
- Clear documentation that detection is heuristic, not a legal compliance guarantee.

**Status:** Discontinued.

---

### N7. Natural-language copilot (deterministic execution)

**What.** Ask for a result in plain language; `tabmux` proposes a **typed pipeline plan** (not arbitrary code), shows a preview, and executes only after confirmation.

**Why it is new.** Text-to-SQL tools exist, but they usually target databases and produce unverified SQL. A local-first planner that emits the same validated pipeline AST as hand-written commands is safer and more general: it can choose verbs, SQL, or a mix.

**Sketch.**

```sh
tabmux ask sales.xlsx \
  "monthly revenue by region, excluding refunds, as an Excel report"

# Output:
# Proposed plan:
#   1. filter status != "refund"
#   2. derive month = date_trunc(created_at, 'month')
#   3. aggregate by [month, region], sum(amount) as revenue
#   4. sort [month, revenue:desc]
#   5. write report.xlsx#Monthly
# Preview: 1,234 rows -> 36 rows
# [e]xecute, [e]dit, [c]ancel?
```

**Requirements.**

- Model-agnostic: OpenAI-compatible HTTP endpoint, local GGUF (optional), or no-LLM rule-based suggestions.
- The LLM never touches raw data by default; only schema, sample values (redacted), and column statistics.
- Plans are validated against the schema and executed by the deterministic engine.
- `--dry-run` always available; no implicit writes.
- "Explain this table" and "suggest cleaning steps" modes.
- Audit the prompt/response (opt-in) for reproducibility.

**Status:** Discontinued.

---

### N8. Federated query and cloud/remote I/O

**What.** One query across files, databases, HTTP endpoints, and object storage:

```sh
tabmux query \
  "select s.*, c.name
   from 's3://bucket/sales/*.parquet' s
   join postgres://prod/customers c on s.customer_id = c.id
   where s.date >= current_date - interval 30 day"
```

**Why it is new.** DuckDB can do much of this with extensions; `tabmux` can make it a first-class, credential-aware, cached, schema-discovering workflow with a consistent table model and output layer.

**Requirements.**

- HTTP(S) with auth, redirects, retries, caching, and range reads.
- S3/GCS/Azure via `object_store` (optional SDKs).
- PostgreSQL/MySQL/SQLite/DuckDB attachments.
- Credential resolution from env/config/OS keychain (never log secrets).
- Predicate/projection pushdown where possible.
- Query result caching with TTL and explicit invalidation.

**Status:** Discontinued.

---

### N9. Streaming and external-memory engine

> **v1 scope:** streaming CSV read/write and constant-memory select/filter/head conversion. External-memory sort/join/group, spill files, and resumable checkpoints are deferred.

**What.** A single execution engine that automatically chooses between in-memory, streaming, and spill-to-disk strategies, with explicit controls.

**Why it is new.** Some tools stream; some materialize. Few expose a coherent cost model and let the user set memory limits, spill directories, and resume checkpoints across all operations.

**Requirements.**

- Constant-memory operators: select, filter, map, head, sample (reservoir), regex search, format conversion.
- Bounded-memory operators: sort, join, group-by, dedup, pivot, distinct, window.
- Spill formats: Arrow IPC, Parquet, or sorted runs; configurable compression.
- `--memory-limit`, `--temp-dir`, `--spill-compression`.
- Automatic strategy selection plus `--explain` estimates.
- Resumable jobs for long pipelines (`--checkpoint`).
- Backpressure between pipeline stages; parallel readers/writers.
- Progress with rows/sec, bytes/sec, ETA, and spill usage.

**Status:** Core v1 for streaming CSV/select/filter/head; external-memory sort/join deferred.

---

### N10. Watch, serve, and MCP modes

**What.** Turn pipelines into services:

- **Watch:** re-run a pipeline when inputs change; tail append-only CSV/JSONL; watermarking.
- **Serve:** HTTP API to upload, inspect, query, convert, and download; OpenAPI schema.
- **MCP:** expose table operations as Model Context Protocol tools for AI agents.
- **Chat:** an interactive REPL for exploring a dataset and building a pipeline.

**Why it is new.** Existing CLI tools are batch-only. A local, single-binary service with the same pipeline format is a natural extension for dashboards, automation, and AI agents.

**Sketch.**

```sh
tabmux watch pipeline.yaml --debounce 500ms --on-error continue
tabmux serve --bind 127.0.0.1:8080 --auth token
tabmux mcp --stdio
```

**Requirements.**

- Watch: file watching, debounce, ignore patterns, append/tail mode, failure policy.
- Serve: auth, upload limits, sandboxed temp dirs, request timeouts, OpenAPI, health endpoint.
- MCP: tool schemas, resource listing, safe read-only defaults.
- No network listener by default; explicit `--bind`.

**Status:** Discontinued.

---

### N11. Time-series and event-table operations

**What.** Table operations for timestamped data:

- Parse timestamps/timezones; normalize to UTC or a target zone.
- Resample (1m/5m/1h/1d), align, fill gaps, aggregate.
- Rolling windows, expanding windows, lag/lead, percent change.
- As-of joins (attach the latest record before a timestamp).
- Sessionization (group events into sessions by inactivity gap).
- Fiscal calendars and business-day arithmetic.
- Anomaly detection and forecasting hooks.

**Why it is new.** SQL engines can do this with window functions, but the ergonomics are poor for common table workflows; CSV tools rarely support time semantics at all.

**Sketch.**

```sh
tabmux resample events.csv --time ts --every 5m \
  --group user_id --agg 'count=*, sum=amount' -o 5m.csv

tabmux asof-join trades.csv quotes.csv \
  --left-time ts --right-time ts --by symbol -o trades_enriched.csv
```

**Status:** Deferred.

---

### N12. Geospatial tables

**What.** Read, transform, analyze, and write geospatial tables:

- Formats: GeoJSON, Shapefile, GPX, KML, WKT/WKB columns, CSV with lat/lon.
- CRS transforms (via optional PROJ).
- Spatial join, distance, area, buffer, contains/intersects, simplify.
- Export to GeoJSON/Shapefile/CSV; terminal/HTML/SVG maps.
- Geocoding/reverse geocoding (optional, network).

**Why it is new.** Geospatial tooling is usually separate (GDAL, QGIS, PostGIS). A table CLI that treats geometry as a column type is a strong niche differentiator.

**Status:** Discontinued.

---

### N13. Text, log, and fuzzy-data mining

**What.**

- Parse logfmt, LTSV, nginx/apache access logs, JSON logs, syslog.
- Grok-like pattern extraction into columns.
- Tokenization, n-grams, vocabulary, TF-IDF.
- Fuzzy matching/joining (Levenshtein, Jaro-Winkler, normalized keys).
- Entity extraction: URLs, emails, dates, money, phone numbers, IPs.
- Language detection and optional sentiment/profanity flags.
- Deduplicate near-duplicate names/addresses.

**Why it is new.** Log and text tools exist separately (`lnav`, `angle-grinder`), and fuzzy joins are usually custom Python. Bringing them into the table model is valuable for data cleaning.

**Status:** Deferred.

---

### N14. Graph and network analytics

**What.** Treat an edge list as a graph:

- Edge list ↔ adjacency matrix ↔ node/edge tables.
- Degree, centrality, components, communities, shortest paths, PageRank.
- Filter/aggregate by graph metrics; join metrics back to tables.
- Export to Graphviz/DOT, JSON, CSV, or a graph database.

**Why it is new.** Graph tools (NetworkX, igraph) are libraries; CSV tools do not understand graphs. A pipeline that can go from edge list to metrics and back is a useful extension.

**Status:** Discontinued.

---

### N15. Statistics, ML-lite, and forecasting

**What.** Reproducible, small-to-medium-data analytics inside the pipeline:

- Hypothesis tests (t-test, chi-square, ANOVA, Mann-Whitney).
- Regression (linear, logistic, regularized).
- Clustering (k-means, DBSCAN), PCA, outlier detection.
- Forecasting (ETS, ARIMA-like, seasonal decomposition).
- Model scoring and cross-validation on tables.
- Export model coefficients and predictions as columns.

**Why it is new.** R/Python are the default for this; a single binary that can do quick, reproducible analytics without a runtime is attractive for CI and field work.

**Status:** Discontinued.

---

### N16. WASM plugin system

**What.** A stable, sandboxed plugin ABI for custom readers, writers, transforms, and validators:

```sh
tabmux plugin install ./plugins/parquet-crypto.wasm
tabmux plugin list
tabmux convert secret.parquet out.csv --plugin parquet-crypto
```

**Why it is new.** Existing tools are either closed sets of commands or require compiling Rust code. A WASM plugin system keeps the single-binary distribution while enabling extension.

**Requirements.**

- WASM component model via Extism/Wasmtime.
- Manifest: name, version, capabilities (`read`, `write`, `network`, `env`), input/output schemas.
- Sandboxed by default; explicit capability grants.
- Versioned ABI; host functions for table batches, schema, logging.
- Plugin registry/marketplace optional; local install first.
- `tabmux plugin test` with fixtures.

**Status:** Discontinued.

---

### N17. Fidelity and loss reports

**What.** Every conversion produces a machine-readable fidelity report:

- What was preserved, translated, approximated, or dropped.
- Examples: styles, formulas, merged cells, comments, data validation, charts, pivot tables, macros, precision, timezone, encoding, column widths.
- `--strict` fails if loss exceeds a threshold; `--lossy` proceeds; `--report` writes JSON/Markdown.
- `tabmux convert --verify` round-trips and compares.

**Why it is new.** Converters silently lose data. A trust-building fidelity report is a clear differentiator, especially for Excel/ODS workflows.

**Status:** Core v1.

---

### N18. Semantic data dictionary, lineage, and packaging

**What.**

- Infer semantic types: email, phone, URL, country, currency, UUID, IP, date, geo, identifier.
- Generate a data dictionary in Markdown/HTML/JSON Schema/Frictionless/Croissant/DCAT.
- Suggest join keys, foreign keys, and column relationships (functional dependencies).
- Track lineage: which input/operation produced each output column.
- Export a data package with metadata, checksums, and a README.

**Why it is new.** qsv has schema/profile; data catalogs are separate products. A CLI that emits a portable data dictionary and lineage from the same pipeline is valuable for open-data and governance workflows.

**Status:** Deferred.

---

### N19. Template, mail-merge, and report generation

**What.** Render one document per row (or one document for the whole table) using HTML/Markdown/DOCX templates:

```sh
tabmux render invoice.html \
  --data invoices.csv \
  --out 'out/invoice-{{ id }}.html' \
  --per-row

tabmux render report.md --data summary.csv -o report.md
```

- MiniJinja templates with filters, conditionals, loops, and chart embeds.
- Batch output naming patterns.
- Optional DOCX/PDF generation (PDF via external engine or HTML-to-PDF).
- Email generation (EML) with attachments.

**Why it is new.** Pandoc and templating tools exist, but not as a table-aware batch renderer with data validation and workbook inputs.

**Status:** Discontinued.

---

### N20. Pipeline test and benchmark harness

**What.** Treat pipelines as code:

```sh
tabmux test pipeline.yaml --fixtures fixtures/ --update
tabmux bench pipeline.yaml --runs 5 --memory
```

- Golden output comparison (CSV/JSON/XLSX-aware, not byte-for-byte).
- Schema assertions, row counts, checksums, null checks.
- Property-based tests for transforms.
- Fuzz testing for parsers (CSV/XLSX/HTML/JSON).
- Performance budgets and regression detection.
- CI output in JUnit/JSON.

**Why it is new.** Data pipelines are notoriously untested. A built-in test harness makes `tabmux` suitable for production ETL, not just ad-hoc commands.

**Status:** Deferred (basic quality gate in v1).

---

### N21. Safe-by-default execution

**What.** Make destructive operations hard to do accidentally:

- `--dry-run` prints the resolved plan, input/output schemas, row estimates, sample, and loss warnings.
- Atomic writes: write to a temp file in the same directory, fsync, rename.
- Backups on overwrite (`file.csv.bak` or timestamped).
- `--no-clobber` default for multi-output operations.
- Confirmation prompts only on a TTY; never block scripts.
- Audit log: command, version, inputs, outputs, checksums, timings, warnings.
- Sandbox permissions for plugins and network access.
- Crash-safe checkpoints for long jobs.

**Why it is new.** Most CLI tools overwrite silently and provide no audit trail. Safety is a differentiator for data work, where mistakes are expensive.

**Status:** Core v1.

---

### N22. Terminal dashboards and shareable reports

**What.** Compose tables and charts into a dashboard definition:

```yaml
# dashboard.yaml
title: Sales overview
rows:
  - [ {table: summary}, {chart: bar, data: by_region, x: region, y: revenue} ]
  - [ {chart: line, data: daily, x: date, y: revenue, width: full} ]
```

```sh
tabmux dashboard dashboard.yaml --watch sales.csv
tabmux dashboard dashboard.yaml --export report.html
```

- Terminal rendering with sparklines, bars, heatmaps, and tables.
- Self-contained HTML export (no CDN) with interactive charts.
- PNG/SVG export.
- Auto-refresh in watch mode.

**Why it is new.** Terminal charting exists (xan, csvtk, qsv viz), but composing multiple views into a dashboard and exporting a shareable report is a product-level feature.

**Status:** Discontinued.

---

## 6. Roadmap and prioritization

The authoritative phase plan is [`devplan.md`](./devplan.md). Each phase is one focused implementation pass and ends with a manual demo.

| Phase | Outcome |
|---|---|
| 0 | CLI skeleton, format registry, fixtures, smoke script |
| 1 | Core table model, CSV/TSV/JSON, safe writes, select/head/count |
| 2 | XLSX read, sheet selection, CSV conversion |
| 3 | ODS read, all-sheets/per-sheet, row/column terminal output |
| 4 | XLSX/ODS write, multi-sheet assembly, basic formatting |
| 5 | Merge/concat, schema diff, basic transforms |
| 6 | HTML table ingestion, HTML → workbook |
| 7 | Directory/ZIP batch ingestion |
| 8 | Reproducible pipeline manifest, dry-run, replay |
| 9 | Loss report, provenance manifest, strict mode |
| 10 | Basic DuckDB export and simple query |
| 11 | Quality gate, fixtures, release |

**Release sequence:** `0.1.0-alpha.1` (phases 0–1), `alpha.2` (2–3), `alpha.3` (4–5), `beta.1` (6–7), `beta.2` (8–9), `0.1.0` (10–12).

Anything not in this table is either **deferred** (N4, N5, advanced N9, N11, N13, N18, full N20) or **discontinued** (see the scope reset at the top of this document). Do not add discontinued features to the roadmap without a new product decision.

---

## 7. Non-goals

To keep the product coherent, `tabmux` will **not** aim to be:

- A full spreadsheet editor or collaborative Office replacement.
- A pixel-perfect Office renderer.
- A general document typesetting system (Pandoc's job) or a mail-merge/reporting engine.
- A BI server, dashboard platform, or scheduled reporting service.
- A distributed compute engine, data warehouse, or federated query layer.
- A geospatial, graph, statistics, ML, or forecasting toolkit.
- A PII detection, anonymization, or legal-compliance product.
- A natural-language assistant or AI agent platform.
- A plugin platform or embedded scripting runtime.
- A TUI file browser or interactive data explorer.
- A replacement for domain-specific scientific software (GIS, bioinformatics, statistics suites).
- A tool that promises advanced Excel fidelity (charts, pivot caches, macros, full formula recalculation, conditional formatting, style cloning). It reports loss instead.

**Explicitly discontinued in this product direction:** N6, N7, N8, N10, N12, N14, N15, N16, N19, N22. Do not reintroduce them as “small additions”; they require a separate product decision.

---

## 8. Acceptance criteria and success metrics

### 8.1 Functional acceptance

1. **CSV round-trip:** RFC 4180 edge cases (quotes, commas, newlines, BOM, CRLF) survive read → write.
2. **XLSX read:** values, types, dates, merged cells, and cached formula values convert correctly; unsupported features appear in the loss report.
3. **ODS read:** one sheet and all-sheets conversion produce correctly named CSV files.
4. **Workbook write:** multiple tables become a valid XLSX/ODS with one sheet per table, native types, bold header, freeze row, and autofilter.
5. **HTML:** a page with 20 tables becomes an XLSX with 20 uniquely named sheets plus an index sheet.
6. **Merge:** two tables with identical headers merge exactly; mismatched headers produce an actionable schema diff and no partial output.
7. **Row/column output:** selecting row 42/column `email` is exact, including quoted/multiline cells.
8. **Pipeline:** a TOML manifest replays deterministically; two runs produce identical output checksums; `--dry-run` writes nothing.
9. **Loss report:** every conversion emits a report; `--strict` fails before writing when warnings/errors exceed the policy.
10. **Safety:** overwrite requires `--force`; atomic writes leave no partial file on failure; provenance includes input/output checksums.
11. **DuckDB:** export a table, query it, and get an identical row count; import failure rolls back.
12. **Batch ingestion:** a directory and a ZIP archive convert deterministically; unsupported members are reported, not silently skipped.
13. **Streaming:** a large CSV can be filtered/selected with bounded memory, or fails with a clear memory error.

### 8.2 Performance targets (initial)

| Scenario | Target |
|---|---|
| Startup (`--version`) | < 50 ms |
| CSV scan/filter throughput | ≥ 100 MB/s per core on typical hardware |
| CSV → CSV/TSV round-trip (100 MB) | < 5 s, bounded memory |
| XLSX read (100k rows × 20 cols) | < 5 s |
| XLSX write (100k rows × 20 cols) | < 10 s |
| ODS read/write (100k rows × 20 cols) | < 15 s |
| HTML 20-table workbook | < 2 s |
| DuckDB import (1M rows) | < 10 s |
| Memory for streaming select/filter | < 200 MB regardless of input size |

### 8.3 Quality targets

- Parser fuzzing for CSV, XLSX, HTML, JSON, and XML.
- Property-based tests for type inference and round-trip conversions.
- Golden-file tests for every command and pipeline.
- Cross-platform CI: Linux, macOS, Windows; x86_64 and aarch64.
- No panics on malformed user input; all errors have file/sheet/row/column context.
- Reproducible builds; dependency audit and license checks in CI.

### 8.4 Product metrics

- Time-to-first-result: a new user can convert a spreadsheet to CSV in under 60 seconds after install.
- Time-to-pipeline: a user can build and replay a 5-step pipeline in under 10 minutes.
- Conversion coverage: the supported formats cover the five starter scenarios without a second tool.
- Trust: every lossy conversion produces a report; zero silent data-loss bugs in the tracker.
- Honesty: no conversion claims fidelity it does not have; `--strict` is the default recommendation for archival work.

---

## 9. Feature-to-analogue quick reference

| Feature area | Strongest analogue(s) | tabmux v1 target |
|---|---|---|
| CSV/TSV conversion | xsv/qsv, xan, Miller, csvtk | Correct, safe, streaming, loss-aware |
| Spreadsheet conversion | xlsx2csv, in2csv, qsv, ssconvert, LibreOffice | Pure-Rust default + optional LibreOffice fidelity bridge |
| HTML table extraction | Pandoc, VisiData, xan `scrape` | All tables → workbook with provenance |
| Workbook assembly | csvtk, ssconvert, LibreOffice | Multi-sheet, native types, basic formatting |
| Reproducible pipeline | xan `run`, shell pipelines | TOML manifest, dry-run, deterministic replay |
| Loss reporting | None in the analogues | First-class, per-conversion, machine-readable |
| Safe execution | None in the analogues | Atomic writes, no-clobber, checksums, strict mode |
| Basic SQL | DuckDB, trdsql | DuckDB import + simple read-only query |
| Batch ingestion | qsv, xan, trdsql | Directory/ZIP, deterministic, report skipped members |
