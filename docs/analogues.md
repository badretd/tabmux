# Analogues to tabmux

> **Status:** research snapshot, 2026-11-04  
> **Scope:** command-line and terminal tools that read, inspect, transform, query, convert, or visualize tabular data.  
> **Question:** what already exists, what does each tool do well, and where is the gap for a unified "FFmpeg for tables"?
>
> **Product note:** this document records the broad landscape. The current product scope is narrower: safe, reproducible, honest conversion (N1 + N2 + N3 + N17 + N21). See the scope reset in [`features.md`](./features.md) and the phase plan in [`devplan.md`](./devplan.md).

---

## 1. Executive summary

The tabular CLI landscape is mature but fragmented. The strongest tools fall into five groups:

1. **Fast delimited-text toolkits** — `csvkit`, `xsv`, `qsv`, `xan`, `Miller`, `csvtk`, `tsv-utils`, `zsv`. They excel at composable verbs over CSV/TSV and increasingly over JSON and Parquet.
2. **SQL-over-files engines** — DuckDB, SQLite, `trdsql`, `q`, `textql`, `csvq`, `dsq`, `octosql`, `clickhouse-local`. They provide a full query language but are weak at spreadsheet fidelity and document extraction.
3. **Spreadsheet converters** — `xlsx2csv`, csvkit's `in2csv`, Gnumeric's `ssconvert`, LibreOffice headless. They handle XLS/XLSX/ODS but are usually one-way, one-file, and not composable.
4. **Document converters** — Pandoc and office suites. They understand document tables but not data operations, SQL, or large-file streaming.
5. **Interactive viewers** — VisiData, `csvlens`, `tabiew`. They are excellent for exploration but are not scriptable ETL engines.

No single tool covers **all** of: multi-format ingestion, workbook-aware output, streaming operations, SQL, interactive exploration, table diff/validation, and document-table extraction. That is the opening for `tabmux`.

The closest direct analogues are:

- **qsv** — the broadest Rust CSV toolkit; spreadsheet import, SQL, validation, stats, TUI, and many data-cleaning commands.
- **xan** — the most composable modern CSV tool; expression language, charts, scraping, and a very large verb set.
- **DuckDB** — the strongest embedded SQL engine for files and the natural backend for "convert to DuckDB and query".
- **VisiData** — the broadest format coverage in an interactive tool.
- **LibreOffice / Gnumeric** — the reference for spreadsheet fidelity.
- **Pandoc** — the reference for document-table conversion.

`tabmux` should not try to out-feature every one of these on its home turf. It should combine their strengths behind one consistent I/O layer and one pipeline model.

---

## 2. Capability comparison

Legend: **●** native / first-class · **◐** partial, plugin, external tool, or limited · **○** not a meaningful feature.

| Tool | Delimited I/O | Spreadsheets (XLS/XLSX/ODS) | HTML / document tables | JSON / YAML / XML | Binary / columnar | SQL engine | DB export | Joins / reshape | Stats / plots | Interactive TUI | Scripting / DSL |
|---|---|---|---|---|---|---|---|---|---|---|---|
| **csvkit** | ● | ◐ read XLS/XLSX, no write | ○ | ◐ JSON in/out | ○ | ● `csvsql` | ● via SQLAlchemy | ◐ join, stack | ● stats, pretty | ○ | ◐ Python `csvpy` |
| **xsv** | ● | ○ | ○ | ○ | ○ | ○ | ○ | ● join, split | ● stats, frequency | ○ | ○ |
| **qsv** | ● | ● read XLS/XLSX/ODS, write XLSX/ODS | ○ | ● JSON/JSONL | ● Parquet/Arrow/Avro via Polars | ● `sqlp` | ● SQLite/Postgres/XLSX/ODS/Parquet | ● joins, pivot, explode | ● stats, schema, viz | ● `lens` | ● Luau, Python, templates |
| **xan** | ● | ◐ XLSX via `from`/`to` | ◐ HTML scraping | ● JSON/JSONL | ● Parquet, NumPy | ○ | ○ | ● joins, pivot, unpivot | ● stats, hist, plot, heatmap | ◐ `view`/`flatten` | ● expression language |
| **Miller** | ● | ○ | ○ | ● JSON/JSONL | ○ | ○ | ○ | ● join, reshape, nest | ● stats1/stats2 | ○ | ● DSL (`put`, `filter`) |
| **csvtk** | ● | ◐ XLSX read/write, split | ● HTML output | ● JSON | ○ | ○ | ○ | ● join, concat, transpose | ● summary, plots | ○ | ◐ mutate expressions |
| **VisiData** | ● | ● XLS/XLSX, ODS partial | ● HTML | ● JSON/YAML | ● HDF5, NumPy, Parquet? | ◐ SQLite | ◐ SQLite/CSV | ● join, pivot, melt | ● describe, graphs | ● | ● Python |
| **DuckDB CLI** | ● | ◐ XLSX via extension | ○ | ● JSON/JSONL | ● Parquet/Arrow | ● | ● | ● full SQL | ● SQL aggregates, `SUMMARIZE` | ◐ shell | ● SQL |
| **sqlite-utils** | ● | ○ | ○ | ● JSON | ○ | ● | ● | ● SQL joins | ◐ | ○ | ◐ Python |
| **trdsql** | ● | ◐ via `xlsxsql` sibling | ○ | ● JSON/JSONL/YAML | ○ | ● | ● SQLite/Postgres/MySQL | ● SQL joins | ● SQL aggregates | ○ | ◐ SQL + jq |
| **dasel / jq / yq** | ◐ CSV in dasel | ○ | ○ | ● JSON/YAML/TOML/XML | ○ | ○ | ○ | ○ | ○ | ○ | ● query language |
| **xlsx2csv / in2csv** | ● | ● XLS/XLSX read | ○ | ● in2csv JSON | ○ | ○ | ○ | ○ | ○ | ○ | ○ |
| **ssconvert / LibreOffice** | ● | ● XLS/XLSX/ODS read/write | ● HTML | ○ | ◐ DBF | ○ | ◐ DBF | ○ | ◐ formulas | ○ | ● macros |
| **Pandoc** | ◐ via filters | ○ | ● HTML/DOCX/ODT/LaTeX | ◐ JSON filters | ○ | ○ | ○ | ○ | ○ | ○ | ● Lua filters |
| **csvlens / tabiew** | ● | ◐ tabiew | ○ | ◐ tabiew | ◐ tabiew | ◐ tabiew | ◐ export | ○ | ◐ | ● | ○ |
| **tsv-utils / zsv** | ● | ○ | ○ | ◐ zsv JSON | ○ | ◐ zsv SQL | ○ | ● tsv-join | ● tsv-summarize | ○ | ○ |

**Takeaway:** the columns are rarely covered by the same tool. Spreadsheet fidelity lives in converters, SQL lives in engines, interactive exploration lives in TUIs, and composable data operations live in CSV toolkits. `tabmux` can win by making these one coherent system.

---

## 3. Detailed profiles

### 3.1 csvkit (Python)

**What it is.** A suite of small, Unix-style commands for converting to and working with CSV. It is the classic "Swiss Army knife" of CSV on the command line and a direct inspiration for many later tools.

**Functionality**

| Area | Commands |
|---|---|
| Input | `in2csv` (CSV, DBF, fixed-width, GeoJSON, JSON, NDJSON, XLS, XLSX), `sql2csv` |
| Cleaning | `csvclean` (fix/repair, report errors, null values) |
| Selection | `csvcut`, `csvgrep` |
| Combining | `csvjoin`, `csvstack` |
| Ordering | `csvsort` |
| Formatting | `csvformat`, `csvjson`, `csvlook` |
| Analysis | `csvstat`, `csvsql` |
| Scripting | `csvpy` |

**Strengths**

- Very approachable, stable, well documented, and widely available.
- Good CSV dialect handling: delimiter sniffing, quoting, encoding, line endings.
- `in2csv` is a useful multi-format front door: DBF, fixed-width, GeoJSON, JSON, XLS/XLSX.
- `csvsql` can run SQL against CSV files and load results into a database.
- `csvstat` gives a quick statistical profile of every column.

**Limitations**

- Python runtime and dependency footprint; slower than Rust/Go tools.
- No ODS support in the current `in2csv` format list.
- No writing to spreadsheet formats.
- No HTML-table extraction, no TUI, no workbook/sheet model.
- Each operation is a separate binary; pipelines are shell pipelines, not a table-aware graph.

**Lessons for tabmux**

- Small composable verbs are valuable; keep a verb mode alongside a pipeline mode.
- `in2csv`'s format list is a good baseline for "ingest anything tabular".
- `csvstat` and `csvlook` set the bar for quick inspection output.

---

### 3.2 xsv and qsv (Rust)

**xsv** is BurntSushi's fast CSV toolkit and the ancestor of qsv and xan. It is now unmaintained, but its command design still defines the genre.

**xsv functionality**

`cat`, `count`, `fixlengths`, `flatten`, `fmt`, `frequency`, `headers`, `index`, `input`, `join`, `partition`, `sample`, `reverse`, `search`, `select`, `slice`, `sort`, `split`, `stats`, `table`.

**qsv** is a much larger fork/superset with a data-wrangling focus. Its command surface is one of the broadest in the ecosystem.

**qsv functionality (grouped)**

- **Inspect:** `count`, `headers`, `stats`, `frequency`, `schema`, `sniff`, `flatten`, `table`, `lens`, `color`.
- **Select/shape:** `select`, `slice`, `head`/`tail` (via `slice`), `sample`, `sort`, `reverse`, `dedup`, `extdedup`, `extsort`, `split`, `partition`, `transpose`, `explode`, `implode`, `fill`, `enum`, `edit`.
- **Combine:** `cat`, `join`, `joinp`, `exclude`, `diff`.
- **Transform:** `apply`, `applydp`, `replace`, `rename`, `safenames`, `datefmt`, `fmt`, `fixedwidth`, `input`, `fixlengths`, `behead`, `pseudo`, `denull`.
- **Aggregate:** `stats`, `moarstats`, `pragmastat`, `frequency`, `pivotp`, `template`.
- **SQL/analytics:** `sqlp` (Polars SQL), `luau`, `py`, `foreach`.
- **Spreadsheets/data:** `excel`, `to`, `json`, `jsonl`, `tojsonl`, `readstat`, `writestat`, `geocode`, `geoconvert`.
- **Quality/validation:** `validate`, `schema`, `scoresql`, `diff`.
- **Ops/UX:** `index`, `clean`, `get`, `fetch`, `fetchpost`, `clipboard`, `prompt`, `snappy`, `blake3`, `describegpt`, `profile`, `synthesize`, `viz`.

**Strengths**

- Very fast, memory-mapped indexing, streaming and external-memory modes for large files.
- Breadth: import/export, SQL, stats, validation, charts, geocoding, TUI viewer, spreadsheet conversion.
- `to` already writes XLSX, ODS, Parquet, SQLite, PostgreSQL, and Data Packages.
- `excel` already extracts a selected sheet from XLS/XLSX/ODS to CSV.
- Strong data-quality and schema tooling.

**Limitations**

- The command surface is enormous and uneven; discoverability is hard.
- CSV-centric naming and semantics; many commands assume well-formed rectangular data.
- No HTML-table extraction, no document ingestion, no workbook style/formula preservation.
- SQL is Polars-based (`sqlp`), not a persistent DuckDB file by default.
- Some advanced features are feature-gated and add significant build complexity.

**Lessons for tabmux**

- Speed and streaming must be table stakes, not premium features.
- A persistent index and external-memory algorithms are necessary for huge files.
- A single `to`/`from` conversion surface is a good abstraction.
- The breadth is a warning: tabmux needs a curated default surface plus a discoverable extension mechanism.

---

### 3.3 xan (Rust)

**What it is.** A modern, nearly rewritten fork of xsv designed as a composable CSV "magician". It is probably the closest philosophical analogue to a general-purpose table CLI.

**Functionality (grouped)**

- **Explore/visualize:** `count`, `headers`, `view`, `flatten`, `hist`, `plot`, `heatmap`, `spark`, `progress`.
- **Search/filter:** `search`, `filter`, `head`, `tail`, `slice`, `top`, `sample`, `bisect`.
- **Sort/dedup:** `sort`, `dedup`, `shuffle`.
- **Aggregate:** `frequency`, `groupby`, `stats`, `agg`, `bins`, `window`.
- **Combine:** `cat`, `join`, `merge`.
- **Columns:** `select`, `drop`, `map`, `transform`, `enum`, `fill`, `complete`, `blank`, `separate`.
- **Format/convert:** `behead`, `rename`, `input`, `fixlengths`, `fmt`, `explode`, `implode`, `from`, `to`, `scrape`, `reverse`, `transpose`, `pivot`, `unpivot`.
- **Split/parallel:** `split`, `partition`, `parallel`, `range`.
- **Text/graph:** `tokenize`, `vocab`, `matrix`, `network`.
- **Scripting:** `run`, `eval`, expression language ("moonblade").

**Formats**

- CSV/TSV/SSV/PSV and CSV-adjacent formats such as CDX, VCF, GTF, SAM, BED, Matrix Market.
- JSON/NDJSON, Excel (via `from`/`to`), Parquet, NumPy arrays.
- gzip and zstd input.

**Strengths**

- Excellent composability and a very expressive, fast expression language.
- Strong terminal visualization (histograms, scatter, heatmaps, sparklines).
- Handles large files with streaming and parallelism.
- `scrape` brings HTML-to-CSV into the same pipeline.
- `from`/`to` is a clean conversion abstraction.

**Limitations**

- CSV is the conceptual center; workbook semantics are minimal.
- No ODS, no database connectors, no persistent SQL engine.
- No full-screen TUI; `view`/`flatten` are preview commands.
- No data validation/diffing/contracts.

**Lessons for tabmux**

- A pipeline of verbs plus a compact expression language is a proven UX.
- Terminal charts are a real differentiator and should be first-class.
- `from`/`to` should be extended into a full format registry with capability metadata.

---

### 3.4 Miller (Go)

**What it is.** "Like awk, sed, cut, join, and sort for name-indexed data" — a streaming record processor for CSV, TSV, JSON, JSON Lines, and several line-oriented formats.

**Functionality (verbs)**

`altkv`, `bar`, `cat`, `check`, `count`, `count-distinct`, `count-similar`, `cut`, `decimate`, `fill-down`, `filter`, `flatten`, `format-values`, `fraction`, `grep`, `group-by`, `group-like`, `having-fields`, `head`, `histogram`, `join`, `json-parse`, `json-stringify`, `label`, `latin1-to-utf8`, `merge-fields`, `nest`, `nothing`, `put`, `regularize`, `rename`, `reorder`, `repeat`, `reshape`, `sec2gmt`, `sec2gmtdate`, `seqgen`, `shuffle`, `sort`, `sort-within-records`, `sparsify`, `split`, `split-lines`, `stats1`, `stats2`, `step`, `tac`, `tail`, `tee`, `unflatten`, `uniq`, `unspace`, `unsparsify`, `utf8-to-latin1`.

**Formats**

CSV, TSV, JSON, JSON Lines, DKVP, NIDX, PPRINT, XTAB, Markdown-like pretty output.

**Strengths**

- True streaming: most operations need only one record in memory.
- Handles heterogeneous records with different fields — rare and useful.
- A real DSL for computed fields and filters.
- Format-aware operations (headers stay headers under sort/tac).
- Single static binary, very portable.

**Limitations**

- No spreadsheet, HTML, Parquet, or database formats.
- Limited reshaping compared with SQL/Polars.
- No TUI and no persistent query engine.
- The DSL is powerful but has a learning curve.

**Lessons for tabmux**

- Streaming and heterogeneous-record support are architectural, not add-ons.
- A single binary with zero runtime dependencies is a strong distribution advantage.
- The record/stream abstraction should sit underneath all format readers.

---

### 3.5 csvtk (Go)

**What it is.** A cross-platform, practical CSV/TSV toolkit with a strong bioinformatics heritage.

**Functionality (grouped)**

- **Information:** `corr`, `dim`, `headers`, `ncol`, `nrow`, `summary`, `watch`.
- **Format conversion:** `csv2html`, `csv2json`, `csv2md`, `csv2rst`, `csv2tab`, `csv2xlsx`, `pretty`, `space2tab`, `splitxlsx`, `tab2csv`, `xlsx2csv`.
- **Set operations:** `comb`, `concat`, `cut`, `filter`, `filter2`, `freq`, `grep`, `head`, `inter`, `join`, `sample`, `split`, `uniq`.
- **Edit:** `add-header`, `comma`, `del-header`, `del-quotes`, `fix`, `fix-quotes`, `fmtdate`, `mutate`, `mutate2`, `mutate3`, `rename`, `rename2`, `replace`, `round`.
- **Transform:** `fold`, `gather`, `long2matrix`, `matrix2long`, `sep`, `spread`, `transpose`, `unfold`.
- **Ordering:** `shuf`, `sort`.
- **Plotting:** `plot` (hist, box, line, bar).
- **Misc:** `cat`, `genautocomplete`, `version`.

**Formats**

CSV/TSV, gzip/xz/zstd/bzip2, XLSX read/write.

**Strengths**

- Practical, easy-to-learn command names.
- Good compression support and fast gzip writing.
- Excel conversion in both directions.
- Plotting and a rich set of reshape operations.
- Works well for scientific and bioinformatics data.

**Limitations**

- No ODS, no HTML input, no SQL, no TUI.
- Per-command flags are numerous; pipelines are shell-only.
- XLSX support is basic (no styling/formulas).

**Lessons for tabmux**

- Friendly aliases and clear command names matter.
- Bidirectional XLSX conversion is expected even in a "CSV" tool.
- Reshape operations (gather/spread/long2matrix) are high-value and often missing from SQL-first tools.

---

### 3.6 VisiData (Python)

**What it is.** An interactive terminal multitool for tabular data — a spreadsheet-like TUI with Python under the hood.

**Functionality**

- **Supported inputs:** CSV, TSV, fixed-width text, XLS/XLSX, JSON/JSON Lines, HTML, YAML, HDF5, NumPy `.npy`/`.npz`, pandas DataFrames, SAS/SPSS/Stata (`.xpt`/`.sas7bdat`/`.sav`/`.dta`), Shapefile, pcap, SQLite, ZIP archives, git repositories, and more.
- **Supported outputs:** TSV, CSV, fixed-width, plain text, JSON, NDJSON, GeoJSON, HTML `<table>`, SQLite, Markdown, org-mode, NumPy.
- **Operations:** sort, filter, select, frequency, pivot/melt, join, describe, graph, Python expressions per row, macros, save/restore sessions, batch mode (`-b`), plugins.

**Strengths**

- Unmatched breadth of input formats in one interactive tool.
- Excellent for exploratory data analysis with millions of rows.
- One-keystroke frequency tables, histograms, and graphs.
- Python scripting and a plugin ecosystem.
- Batch mode makes it usable in scripts.

**Limitations**

- TUI-first; not designed as a composable CLI pipeline.
- Python runtime and memory overhead; performance varies by loader.
- No workbook style/formula fidelity on output.
- No SQL engine beyond SQLite-ish use.
- Not a conversion or ETL tool in the traditional sense.

**Lessons for tabmux**

- Format breadth is a headline feature; users do not want to install five tools.
- The TUI must be optional and scriptable via batch mode.
- "Open anything, inspect quickly, export cleanly" is a strong wedge.

---

### 3.7 DuckDB CLI

**What it is.** An embedded analytical SQL database with a CLI, strong file readers, and an extension ecosystem.

**Functionality**

- **Readers:** `read_csv_auto`, `read_json`/`read_ndjson`, `read_parquet`, `read_xlsx` (Excel extension), `read_arrow`, plus extensions for SQLite, PostgreSQL, MySQL, Iceberg, Delta Lake, and HTTP/S3.
- **SQL:** full analytical SQL — joins, window functions, `PIVOT`/`UNPIVOT`, `QUALIFY`, `GROUP BY ALL`, `SUMMARIZE`, `DESCRIBE`.
- **Writers:** `COPY ... TO` CSV/Parquet/JSON, `EXPORT DATABASE`, `ATTACH`/`CREATE TABLE AS`, Excel export via extension.
- **CLI:** interactive shell, dot commands, `.read`, `.mode`, `.output`, `.import`, `.timer`.
- **Extensions:** `httpfs`, `spatial`, `json`, `parquet`, `excel`, `icu`, `postgres_scanner`, `sqlite_scanner`, `mysql_scanner`.

**Strengths**

- Extremely fast analytical SQL over files without a server.
- Zero-copy Arrow integration.
- Persistent `.duckdb` files are ideal for the "convert a table to DuckDB" use case.
- Rich SQL semantics and a large function library.
- Extensions cover databases, cloud storage, and geospatial data.

**Limitations**

- SQL-first; not a friendly format converter for non-SQL users.
- No workbook styling, formulas, charts, or sheet-level fidelity.
- ODS is not a native reader.
- HTML tables and document extraction are out of scope.
- Large bundled build; not lightweight to embed.

**Lessons for tabmux**

- DuckDB should be the SQL/persistence backend, not the whole product.
- A `to duckdb` command plus `query` subcommand covers the user's DuckDB use case directly.
- Use Arrow as the interchange layer to avoid duplicating data between Rust and DuckDB.

---

### 3.8 SQLite and sqlite-utils

**What it is.** SQLite is the ubiquitous embedded relational database; `sqlite-utils` is a Python CLI/library that makes importing, querying, and transforming data pleasant.

**Functionality**

- `sqlite-utils insert`, `upsert`, `insert-files`, `query`, `memory`, `bulk`, `convert`, `extract`, `transform`, `create-index`, `enable-fts`, `tables`, `views`, `triggers`, `vacuum`.
- Import CSV/TSV/JSON/NDJSON; export CSV/JSON.
- Full-text search, JSON1 functions, generated columns.

**Strengths**

- Ubiquitous, durable, single-file database.
- Excellent for small/medium datasets and local applications.
- SQLite's JSON and FTS extensions are surprisingly capable.
- Easy to embed in Rust via `rusqlite`.

**Limitations**

- Not an analytical engine; large joins/aggregations are slower than DuckDB.
- No spreadsheet or document formats.
- `sqlite-utils` is Python-specific.
- No native columnar/Parquet support.

**Lessons for tabmux**

- Support SQLite both as an input/output format and as a lightweight query backend.
- SQLite is the right default for "I just need a local database file"; DuckDB is the right default for analytics.

---

### 3.9 SQL-over-files family: trdsql, q, textql, csvq, dsq, octosql, clickhouse-local

These tools share one idea: treat files as tables and let the user write SQL.

| Tool | Language | Inputs | Outputs | Engine | Notable |
|---|---|---|---|---|---|
| **trdsql** | Go | CSV, TSV, LTSV, JSON, JSONL, YAML, TBLN, logfmt, fixed-width, text | CSV, TSV, JSON, JSONL, LTSV, ASCII table, Markdown, raw, vertical, YAML, TBLN | SQLite, PostgreSQL, MySQL | `-a` analyzes a file and suggests SQL; jq expressions; compression |
| **q** | Python | CSV, TSV | CSV/TSV | SQLite | `-H`, `-d`, `-c`; multiple files |
| **textql** | Go | CSV, TSV | CSV/TSV | SQLite | Simple and fast |
| **csvq** | Go | CSV | CSV, TSV, JSON, JSONL, LTSV, GFM, org, ASCII, XML | SQLite-like | SQL dialect with CSV-specific functions |
| **dsq** | Rust | CSV, JSON, Excel, Parquet | CSV, JSON | SQLite/DuckDB | Multi-engine; good format breadth |
| **octosql** | Go | CSV, JSON, Parquet | Table/JSON | Custom | Streaming, `EXPLAIN`, join optimization |
| **clickhouse-local** | C++ | 70+ formats | 70+ formats | ClickHouse | Extremely fast; huge format matrix |

**Strengths**

- SQL is the most expressive interface most users already know.
- Easy to join heterogeneous files and database tables.
- trdsql in particular has excellent input/output format coverage and a helpful `-a` analyzer.

**Limitations**

- No workbook styling, sheet manipulation, or document extraction.
- SQL is a poor fit for simple one-off conversions and quick inspection.
- Engine-specific dialects and quoting rules.
- Usually no TUI, no diff/validation, no plugin model.

**Lessons for tabmux**

- Provide SQL as an optional power tool, not the only interface.
- Auto-generate SQL examples from a file's schema (trdsql's `-a` is a good model).
- Allow multiple files and database tables in one query.

---

### 3.10 dasel, jq, and yq

**What they are.** Structured-data query and conversion tools.

- **dasel v3:** JSON, YAML, TOML, XML, CSV, HCL, INI, KDL. Unified selectors, recursive descent, search, in-place modification, format conversion.
- **jq:** the standard JSON query/transform language.
- **yq (mikefarah):** YAML/JSON/XML/TOML/CSV/properties query and conversion with a jq-like syntax.

**Strengths**

- Excellent for nested and semi-structured data.
- Conversion between JSON/YAML/TOML/XML is a commodity.
- jq's language is widely known and easy to embed conceptually.

**Limitations**

- CSV support is usually shallow (no schema, types, joins, stats, or large-file streaming).
- Not table-aware: no column semantics, no aggregation, no spreadsheet I/O.
- XML/HTML table extraction is awkward.

**Lessons for tabmux**

- Embed a jq-compatible engine for nested fields rather than inventing one.
- Support JSON/YAML/TOML/XML as first-class table sources and outputs.
- Keep the table abstraction primary; structured data becomes columns/rows at the boundary.

---

### 3.11 xlsx2csv and csvkit in2csv

**xlsx2csv** converts XLSX to CSV with a focused, practical feature set:

- Sheet selection by index or name; `-a` for all sheets; `-s 0` to export all sheets to a directory.
- Custom delimiter, line terminator, encoding, date format, float format, quoting mode.
- Skip empty lines, escape `\r\n\t`, merge cells, include hyperlinks.
- Include/exclude sheet patterns.
- Directory mode for batch conversion.

**in2csv** is csvkit's multi-format front door:

- CSV, DBF, fixed-width (with a schema file), GeoJSON, JSON, NDJSON, XLS, XLSX.
- Sheet listing/selection, writing all sheets to files, encoding override, type inference controls, skip lines.

**Strengths**

- Focused and reliable for the common "spreadsheet to CSV" job.
- Good handling of real-world quirks: dates, floats, merged cells, hyperlinks, malformed dimensions.
- Batch and all-sheets modes.

**Limitations**

- One-way conversion only.
- No ODS in in2csv's current format list; xlsx2csv is XLSX-only.
- No styling, formulas, or multi-format workbook output.
- No composable data operations.

**Lessons for tabmux**

- Spreadsheet conversion must handle messy real-world files, not just clean ones.
- All-sheets and per-sheet output modes are essential.
- Date/float/merged-cell behavior needs explicit, documented policies.

---

### 3.12 Gnumeric ssconvert and LibreOffice headless

**ssconvert** is Gnumeric's spreadsheet converter. It reads and writes a wide range of spreadsheet and tabular formats:

- Read: Gnumeric XML, ODS/SXC, XLS, XLSX, XLS 2003 XML, CSV/TSV, HTML, DBF, DIF, SYLK, Lotus, Quattro Pro, and more.
- Write: ODS, XLS/XLSX, CSV, HTML/XHTML, PDF, LaTeX, TROFF, DBF, and more.
- Features: sheet selection, export one file per sheet (`%n`, `%s` templates), merge workbooks, recalculate formulas, export charts as SVG/PNG, configurable text export (separator, quoting, locale, encoding, date formatting).

**LibreOffice headless** provides similar conversion through `--convert-to`, with high-fidelity Office format support, formula recalculation, PDF export, and filter options. It can also run macros for more complex transformations.

**Strengths**

- Best-in-class format fidelity for legacy and modern office formats.
- Formula recalculation and chart export.
- Handles ODS, XLS, XLSX, HTML, CSV, PDF, and more in one tool.
- Mature handling of styles, merged cells, and locale-specific formats.

**Limitations**

- Heavy external dependency; slow startup; not a single static binary.
- Poor fit for streaming or large-file processing.
- CLI is conversion-oriented, not data-operation-oriented.
- Licensing and packaging complexity (GPL, office runtime).

**Lessons for tabmux**

- Pure-Rust spreadsheet I/O will not match LibreOffice fidelity immediately; document the fidelity boundary.
- Offer an optional LibreOffice bridge for "maximum fidelity" conversions while keeping the default self-contained.
- Formula recalculation is a hard problem; support reading cached values first, and optionally delegate recalculation.

---

### 3.13 Pandoc

**What it is.** A universal document converter with excellent support for document tables.

**Functionality**

- Input: Markdown, HTML, DOCX, ODT, LaTeX, reStructuredText, EPUB, Org, and many more.
- Output: the same family plus PDF (via an engine), slides, and custom templates.
- Tables: pipe tables, grid tables, simple tables, and DOCX/HTML table conversion.
- Filters: Lua/JSON filters for custom transformations.
- Templates, citation processing, syntax highlighting.

**Strengths**

- Best-in-class document-format conversion.
- Handles complex table markup across document formats.
- Extensible via filters and templates.

**Limitations**

- Not a data tool: no SQL, joins, stats, or large-file streaming.
- No XLS/XLSX/ODS spreadsheet support.
- Table extraction is document-structure-oriented, not data-cleaning-oriented.
- PDF output requires an external engine.

**Lessons for tabmux**

- Document tables (HTML, DOCX, Markdown) should be ingested into the same table model.
- For document output, generate HTML/Markdown/DOCX rather than trying to replace Pandoc.
- A template/filter mechanism is a good escape hatch for bespoke reports.

---

### 3.14 csvlens and tabiew

**csvlens** is a `less`-like CSV viewer:

- Scroll rows/columns, page up/down, go to line, go to top/bottom.
- Regex find/highlight, filter rows by regex, filter columns by regex.
- Sort by column (lexical or natural), toggle line wrapping, freeze columns.
- Row/cell selection, copy to clipboard, print marked rows to stdout.
- Optional delimiter, tab-separated mode, no-headers mode, initial column/filter/find regex, echo a selected column.

**tabiew** is a Rust TUI for tabular data with SQL query support and multiple input formats.

**Strengths**

- Fast, focused, keyboard-driven exploration.
- Excellent "peek at a file" workflow.
- Scriptable output selection via stdout.
- Low startup cost.

**Limitations**

- Viewers, not ETL engines.
- Limited transformation and output format support.
- No schema validation, joins across many files, or workbook authoring.

**Lessons for tabmux**

- A built-in `view`/TUI is a natural extension of the same reader stack.
- Keyboard-driven selection plus "print selection to stdout" is a powerful pattern.
- Keep the viewer lightweight and optional.

---

### 3.15 tsv-utils and zsv

**tsv-utils** (eBay) is a set of focused TSV tools:

`tsv-select`, `tsv-filter`, `tsv-join`, `tsv-summarize`, `tsv-sort`, `tsv-sample`, `tsv-uniq`, `csv2tsv`, `tsv-pretty`, `number-lines`, `keep-header`, `tsv-split`, `tsv-append`.

**zsv** is a fast C CSV parser and CLI with commands such as `select`, `count`, `sql`, `2json`, `2tsv`, `flatten`, `stack`, `pretty`, and `paste`.

**Strengths**

- Extremely fast and memory-efficient on large delimited files.
- Small binaries, low overhead, Unix-friendly.
- tsv-utils has excellent documentation of performance trade-offs.

**Limitations**

- Delimited text only.
- No spreadsheet, document, database, or TUI support.
- Limited reshaping and analytics.

**Lessons for tabmux**

- Performance on large files is a credibility issue.
- Simple, well-named verbs are still the best UX for common tasks.
- TSV deserves first-class support, not "CSV with a different delimiter".

---

## 4. Cross-cutting findings

### 4.1 Table stakes

Any credible entrant needs:

- Correct CSV/TSV parsing and writing: RFC 4180, custom delimiters/quotes/escapes, BOM, encodings, CRLF/LF, malformed-row policies.
- Column selection, row filtering, sorting, deduplication, joining, concatenation, and aggregation.
- Format conversion with stdin/stdout support and compression.
- Streaming or bounded-memory processing for large files.
- Clear type inference and a way to override it.
- Good error messages with row/column context.
- Shell completions and a stable, scriptable exit-code contract.

### 4.2 What is fragmented

| User need | Where it lives today | Gap |
|---|---|---|
| Excel/ODS to CSV | xlsx2csv, in2csv, csvtk, qsv, ssconvert, LibreOffice | One-way, per-tool, inconsistent date/float/merged-cell behavior |
| CSV to Excel/ODS | qsv `to`, csvtk `csv2xlsx`, ssconvert, LibreOffice | Little styling, formulas, multi-sheet authoring, or workbook editing |
| HTML tables to data | VisiData, Pandoc, xan `scrape`, custom scripts | No robust "all tables to one workbook with a sheet per table" workflow |
| SQL over files | DuckDB, trdsql, q, dsq, clickhouse-local | Weak conversion/workbook output; engine-specific dialects |
| Interactive exploration | VisiData, csvlens, tabiew | Not scriptable ETL; limited conversion |
| Data quality | qsv `validate`/`schema`, Great Expectations, dbt tests | Not integrated with conversion or exploration |
| Table diff/merge | `diff`, qsv `diff`, daff (separate tool) | No schema-aware, cell-level, three-way merge with conflict resolution |
| Document tables | Pandoc, LibreOffice, PDF extractors | Not connected to tabular data operations |

### 4.3 What users actually combine

Real workflows are pipelines of different tools:

```sh
# Typical today
xlsx2csv book.xlsx | csvcut -c name,email | csvsql --query "select ..." | csvlook
```

or:

```sh
# Another typical stack
in2csv data.xls | mlr --icsv --ojson put '$total = $a + $b' | jq ...
```

The friction is at the boundaries: format conversion, type coercion, null semantics, encoding, sheet selection, and repeated parsing. `tabmux` can remove most of that friction by keeping one table model across the whole pipeline.

### 4.4 Performance patterns worth adopting

- **Streaming first:** Miller and xan prove that most operations can be single-pass.
- **Indexes for random access:** xsv/qsv indexes make `slice`, `count`, and sampling instant.
- **Memory mapping:** useful for large files and index-backed reads.
- **SIMD CSV parsing:** xan's `simd-csv` shows the ceiling for delimited parsing.
- **Arrow/columnar interchange:** DuckDB and Polars avoid serialization overhead.
- **External-memory algorithms:** qsv's `extsort`/`extdedup` and Miller's streaming stats handle files larger than RAM.
- **Pushdown:** SQL engines should push filters/projections into readers when possible.

### 4.5 UX patterns worth adopting

- **Two modes:** small composable verbs (Unix style) and a single pipeline command (FFmpeg style).
- **`-` means stdin/stdout**, consistently.
- **Format inference from extension + magic + content**, with explicit overrides.
- **`--dry-run`/preview** before destructive writes.
- **Progress on stderr**, data on stdout.
- **Actionable errors:** file, sheet, row, column, expected vs actual.
- **Sensible defaults** with an `explain` command that shows what will happen.
- **Shell completions** including column-name completion from the input file.
- **Color only when interactive**, respecting `NO_COLOR` and `CLICOLOR`.

### 4.6 Licensing and distribution

- Rust tools (xsv, qsv, xan, tabiew, csvlens, dsq) demonstrate that a single static binary is a major distribution advantage.
- Python tools (csvkit, VisiData) are easier to extend but harder to ship.
- Go tools (Miller, csvtk, trdsql, q, octosql) are portable but often less memory-efficient than Rust.
- Office-suite bridges (LibreOffice is MPL-2.0/LGPL; Gnumeric is GPL) are heavy; they should be optional runtime integrations, not linked dependencies.

---

## 5. Positioning for tabmux

### 5.1 The wedge

`tabmux` should be:

> **An FFmpeg-style multiplexer for tables: one binary that reads any tabular source, applies a composable pipeline of typed operations, and writes any tabular target — with SQL, a terminal viewer, and workbook-aware output when you need them.**

The three strongest differentiators are:

1. **Unified I/O registry** — one `formats` command and one capability model for CSV, TSV, XLSX, XLS, ODS, JSON, YAML, XML, HTML, Markdown, Parquet, Arrow, SQLite, DuckDB, and databases.
2. **Table-aware pipeline** — not just shell pipes: named streams, multiple inputs/outputs, fan-out/fan-in, joins as graph edges, replayable pipeline files.
3. **Workbook and document awareness** — sheets as first-class tables, multi-sheet output, HTML/PDF/DOCX table extraction, and honest fidelity reporting.

### 5.2 What to build vs. bridge

| Capability | Build in Rust | Bridge to external tool |
|---|---|---|
| CSV/TSV/JSON/YAML/XML parsing | Yes | — |
| XLSX/XLS/ODS reading | Yes (`calamine`) | — |
| XLSX/ODS writing | Yes (`rust_xlsxwriter`, `spreadsheet-ods`) | LibreOffice for maximum fidelity |
| Formula evaluation | Read cached values; limited evaluator | LibreOffice/Gnumeric for recalculation |
| SQL engine | DuckDB (`duckdb` crate) | External DuckDB CLI optional |
| PDF table extraction | Basic text/table heuristics | `pdfium`/OCR or Camelot-like Python optional |
| OCR | `ocrs` optional | Tesseract optional |
| Geospatial | `geo`/`geozero` | GDAL for exotic formats/CRS |
| Document conversion | HTML/Markdown/DOCX output | Pandoc for full document typesetting |

### 5.3 What not to become

- A full spreadsheet editor or collaborative Office replacement.
- A BI server or dashboard platform.
- A general document typesetting system (Pandoc's job).
- A distributed compute engine.
- A pixel-perfect Office renderer.

---

## 6. References

- csvkit: <https://csvkit.readthedocs.io/>
- xsv: <https://github.com/BurntSushi/xsv>
- qsv: <https://github.com/dathere/qsv> and <https://qsv.dathere.com/>
- xan: <https://github.com/medialab/xan>
- Miller: <https://miller.readthedocs.io/>
- csvtk: <https://github.com/shenwei356/csvtk>
- VisiData: <https://www.visidata.org/>
- DuckDB: <https://duckdb.org/docs/>
- SQLite: <https://sqlite.org/>
- sqlite-utils: <https://sqlite-utils.datasette.io/>
- trdsql: <https://github.com/noborus/trdsql>
- q: <https://github.com/harelba/q>
- textql: <https://github.com/dinedal/textql>
- csvq: <https://github.com/mithrandie/csvq>
- dsq: <https://github.com/multiprocessio/dsq>
- octosql: <https://github.com/benbjohnson/octosql>
- clickhouse-local: <https://clickhouse.com/docs/en/operations/utilities/clickhouse-local>
- dasel: <https://github.com/TomWright/dasel>
- jq: <https://jqlang.github.io/jq/>
- yq: <https://github.com/mikefarah/yq>
- xlsx2csv: <https://github.com/dilshod/xlsx2csv>
- Gnumeric ssconvert: <https://manpages.debian.org/bookworm/gnumeric/ssconvert.1.en.html>
- LibreOffice headless: <https://help.libreoffice.org/latest/en-US/text/shared/guide/start_parameters.html>
- Pandoc: <https://pandoc.org/>
- csvlens: <https://github.com/YS-L/csvlens>
- tabiew: <https://github.com/shshemi/tabiew>
- tsv-utils: <https://github.com/eBay/tsv-utils>
- zsv: <https://github.com/liquidaty/zsv>
