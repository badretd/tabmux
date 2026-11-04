# Dependencies and build plan

> **Status:** dependency plan, 2026-11-04  
> **Companion documents:** [`analogues.md`](./analogues.md), [`features.md`](./features.md), and [`devplan.md`](./devplan.md).  
> **Rule:** every dependency in this document must be justified by a feature in `features.md`; heavy or uncertain dependencies are feature-gated and never part of the minimal build.

> **Scope reset (2026-11-04).** The v1 dependency set is the conversion core only:
>
> **CSV/TSV, XLSX read/write, ODS read/write, HTML table input, JSON/JSONL input, directories/ZIP, safe writes, loss report, reproducible pipeline, and basic DuckDB export/query.**
>
> **Discontinued dependency areas:** geo/GDAL, graph, stats/ML/forecasting, NL/AI, PII/synthetic, plugins/WASM, scripting/jq, server/MCP/watch, cloud/federation, TUI, dashboards, mail merge/document rendering, DBF/SAS/SPSS/Stata, Avro/ORC, 7z write, exotic compression, and locale-aware formatting.
>
> Sections below are retained as an architecture catalog. The **v1 Cargo skeleton and phase sets** are authoritative; crates for discontinued areas are marked **CUT** or **DEFERRED**.

---

## 1. Principles

1. **Single binary by default.** The default build should install as one executable with no runtime.
2. **Pure Rust where practical.** The only C/C++ dependency in v1 is DuckDB (optional). LibreOffice is an optional external bridge, not a linked dependency.
3. **Feature-gate everything heavy.** A user who only converts CSV should not compile DuckDB or spreadsheet writers.
4. **One conversion engine, two interfaces.** Verb mode and pipeline/batch mode share the same table model and operation implementations.
5. **Stable versions only.** Prefer stable releases; beta/RC crates are opt-in and documented.
6. **License hygiene.** Default build should be MIT/Apache-2.0 compatible. GPL tools are optional runtime bridges, never linked into the default binary.
7. **Reproducibility.** Commit `Cargo.lock`, audit dependencies, and support `cargo vendor` for offline builds.
8. **MSRV:** Rust **1.88** (required by current DuckDB; edition 2024 requires at least 1.85).

---

## 2. Feature flags (v1)

| Feature | What it enables | Main crates | Approx. build weight |
|---|---|---|---|
| `core` *(default)* | CSV/TSV, JSON/JSONL, inspection, select/head/count, safe writes, directory/ZIP batch ingestion, loss report, pipeline, ZIP container support, terminal output | clap, csv, serde, serde_json, toml, comfy-table, indicatif, tempfile, blake3, globset, walkdir, zip | Small |
| `excel` | XLS/XLSX/XLSB/ODS read; XLSX/ODS write; multi-sheet; basic formatting | calamine, rust_xlsxwriter, spreadsheet-ods, quick-xml | Medium |
| `html` | HTML table extraction, HTML table inventory | built-in parser (no external crate) | Small |
| `sql-duckdb` | DuckDB export and read-only query | duckdb, arrow | **Large** (bundled C++) |
| `full` | All v1 features | `core` + `excel` + `html` + `sql-duckdb` | Medium–large |

**Recommended presets**

```toml
# Fast, useful default: conversion without DuckDB
default = ["core", "excel", "html"]

# Everything in v1
full = ["default", "sql-duckdb"]
```

### 2.1 Cut and deferred feature flags

| Old flag | Status | Reason |
|---|---|---|
| `excel-preserve` | Deferred | Advanced style/formula preservation is out of v1; use loss reporting |
| `yaml`, `xml`, `json-query` | Cut from base | YAML/TOML table write, XML XPath, jq are out of scope |
| `sql-sqlite`, `sql-postgres`, `sql-mysql`, `sql-datafusion` | Cut/deferred | Only DuckDB export + simple query remains |
| `dataframe-polars`, `columnar`, `research-formats` | Cut/deferred | No analytics engine, Avro/ORC, or SAS/SPSS/Stata in v1 |
| `tui`, `charts` | Cut | TUI and dashboards are discontinued |
| `stats`, `timeseries`, `geo`, `geo-proj`, `geo-gdal`, `text-mining` | Cut | Analytics/geo/text mining discontinued or deferred |
| `validate`, `diff` | Deferred | N4/N5 are post-v1 |
| `privacy`, `synthetic` | Cut | PII/synthetic data discontinued |
| `plugins`, `scripting` | Cut | WASM/scripting discontinued |
| `pdf`, `pdf-render`, `ocr`, `ocr-tesseract`, `office` | Deferred | PDF/DOCX/EML ingestion is post-v1 |
| `cloud`, `serve`, `watch`, `nlp` | Cut | Cloud/federation/server/NL discontinued |

---

## 3. Dependency catalog

> **Catalog status.** This is the full architecture catalog, including deferred and discontinued areas. For v1, use only the rows marked `core`, `excel`, `html`, or `sql-duckdb` in the “Feature” column. Sections marked **CUT** or **DEFERRED** are not part of the v1 build.

Versions were checked against crates.io on **2026-11-04**. Use caret requirements for libraries and commit the resulting `Cargo.lock` for the binary.

**Version notes**

- The `duckdb` crate uses its own release numbering; `1.20000.1` is the current release at the time of writing and corresponds to the DuckDB 1.2.x line.
- `umya-spreadsheet` 3.1 is the current stable line; 4.0 is still beta and should not be used for the default build.
- `apache-avro` 0.22 is stable; 0.23 is a release candidate.
- `notify` 8.2 is stable; 9.x is a release candidate.
- `serde_yaml` is deprecated; use `serde_yaml_ng` for serde and `saphyr` for strict YAML 1.2.

### 3.1 CLI, terminal, and UX

> **v1:** keep `clap*`, `owo-colors`, `anstream`, `comfy-table`, `unicode-width`, `indicatif`, `console`, `dialoguer`, `ctrlc`, `terminal_size`, `supports-color`, `bytesize`, `num-format`, `natord`. **CUT:** `ratatui`, `crossterm`, `tui-textarea`, `rustyline`, `arboard`, `textplots`, `plotters`, `charts-rs`, `syntect`.

| Crate | Version | Features / notes | Used for | Feature |
|---|---|---|---|---|
| `clap` | `4.6` | `derive`, `env`, `wrap_help`, `cargo`, `unicode`, `string` | CLI parsing, subcommands, help | core |
| `clap_complete` | `4.6` | bash/zsh/fish/powershell | Shell completions (including column names) | core |
| `clap_mangen` | `0.3` | — | Man pages | core |
| `owo-colors` | `4.4` | — | Terminal styling | core |
| `anstream` | `1.0` | — | ANSI-aware output that degrades when piped | core |
| `comfy-table` | `8.0` | `custom_styling`, `tty` | Pretty tables, wrapping, alignment | core |
| `unicode-width` | `0.2` | — | Display width for CJK/emoji | core |
| `unicode-segmentation` | `1.13` | — | Grapheme-aware truncation | core |
| `indicatif` | `0.18` | `unicode`, `console` | Progress bars/ETA | core |
| `console` | `0.16` | — | Terminal abstraction (direct use optional) | core |
| `dialoguer` | `0.12` | `fuzzy-select` | Interactive prompts (sheet/column picker, overwrite) | core |
| `inquire` | `0.9` | — | Alternative to `dialoguer`; pick one | optional |
| `ctrlc` | `3.5` | — | Graceful shutdown, checkpoint on Ctrl-C | core |
| `terminal_size` | `0.4` | — | Responsive table widths | core |
| `supports-color` | `3.0` | — | Color capability detection (may be covered by `anstream`) | core |
| `bytesize` | `2.7` | — | Human-readable byte counts | core |
| `humansize` | `2.1` | — | Alternative to `bytesize`; pick one | optional |
| `num-format` | `0.4` | — | Thousands separators, locale-ish formatting | core |
| `natord` | `1.0` | — | Natural sort (`file2` < `file10`) | core |
| `ratatui` | `0.30` | — | TUI framework | tui |
| `crossterm` | `0.29` | `event-stream` | Terminal backend/events | tui |
| `tui-textarea` | `0.7` | — | Editable cell/textarea widget | tui |
| `rustyline` | `19.0` | `derive`, `with-file-history` | SQL/pipeline REPL | tui / serve |
| `arboard` | `3.7` | `wayland-data-control` (Linux) | Clipboard copy | tui |
| `textplots` | `0.8` | — | Terminal line/scatter charts | charts |
| `plotters` | `0.3` | `svg`, `bitmap`, `chrono` | SVG/PNG charts | charts |
| `charts-rs` | `2.3` | — | Alternative nicer charts; optional | charts |
| `syntect` | `5.3` | `default-fancy` | SQL/JSON syntax highlighting in TUI | tui (optional) |

**Decision:** use `dialoguer` for simple prompts. No TUI in v1; the rows above marked `tui` are cut.

### 3.2 Core model, serialization, and utilities

| Crate | Version | Features / notes | Used for | Feature |
|---|---|---|---|---|
| `serde` | `1.0` | `derive` | Table schema, configs, pipeline files | core |
| `serde_json` | `1.0` | `preserve_order` | JSON/JSONL, reports, API | core |
| `indexmap` | `2.14` | `serde` | Ordered columns/rows | core |
| `bytes` | `1.10` | — | Zero-copy buffers | core |
| `memchr` | `2.7` | — | Fast byte scanning | core |
| `itertools` | `0.15` | — | Iterator utilities | core |
| `once_cell` | `1.21` | `std` | Lazy globals | core |
| `smallvec` | `1.16` | — | Small-vector optimization | core |
| `ahash` | `0.8` | `runtime-rng` | Fast hashing for joins/group-by | core |
| `hashbrown` | `0.17` | `serde`, `rayon` | Hash maps/sets | core |
| `rayon` | `1.12` | — | Parallel scans/aggregations | core |
| `crossbeam-channel` | `0.5` | — | Pipeline stages, backpressure | core |
| `parking_lot` | `0.13` | — | Faster locks | core |
| `tempfile` | `3.27` | — | Spill files, atomic temp outputs | core |
| `memmap2` | `0.9` | — | Memory-mapped large files | core |
| `globset` | `0.4` | — | Glob matching for inputs | core |
| `walkdir` | `2.5` | — | Recursive directory ingestion | core |
| `same-file` | `1.0` | — | Avoid input/output self-overwrite | core |
| `dunce` | `1.0` | — | Windows path normalization | core |
| `uuid` | `1.29` | `v4`, `v7`, `serde` | IDs, audit, temp names | core |
| `semver` | `1.0` | `serde` | Plugin ABI versioning | plugins |
| `blake3` | `1.8` | `rayon` | Fast checksums, audit | core |
| `sha2` | `0.11` | — | SHA-256 compatibility, keyed hashing | core |
| `hex` | `0.4` | — | Hex encoding | core |
| `base64` | `0.23` | — | Plugin payloads, embeddings | plugins |

### 3.3 CSV, delimited text, encoding

| Crate | Version | Features / notes | Used for | Feature |
|---|---|---|---|---|
| `csv` | `1.4` | — | RFC 4180 reader/writer | core |
| `csv-core` | `0.1` | — | Low-level parser for custom fast paths | core |
| `csv-sniffer` | `0.3` | — | Delimiter/quote/header detection | core |
| `simd-csv` | `0.14` | — | Optional SIMD CSV fast path | core (optional) |
| `encoding_rs` | `0.8` | — | UTF-8/UTF-16/Latin-1/Windows encodings | core |
| `chardetng` | `1.0` | — | Encoding detection | core |
| `encoding_rs_io` | `0.1` | — | Streaming transcoding | core |
| `unicode-bom` | `2.0` | — | BOM detection/stripping | core |
| `csv-index` | `0.1` | — | Optional on-disk index; may implement custom index | core (optional) |

**Notes**

- `csv` is the correctness baseline. `simd-csv` is only a fast path and must pass the same conformance suite.
- Index format should be versioned and checksummed; consider a custom index rather than committing to `csv-index`.
- Never assume UTF-8; use `chardetng` + `encoding_rs` and expose `--encoding`.

### 3.4 Spreadsheets

> **v1:** `calamine`, `rust_xlsxwriter`, `spreadsheet-ods`, `zip`, `quick-xml`. **DEFERRED:** `umya-spreadsheet` (preserve/edit), `formula` (evaluation).

| Crate | Version | Features / notes | Used for | Feature |
|---|---|---|---|---|
| `calamine` | `0.38` | `dates`, `picture` (optional) | Read XLS/XLSX/XLSB/ODS, sheet listing, cached formula values | excel |
| `rust_xlsxwriter` | `0.102` | `chrono`, `zlib`, `serde`, `constant_memory`, `rust_decimal` | Write XLSX: types, styles, number formats, tables, conditional formatting, charts, freeze panes, autofilter, formulas | excel |
| `spreadsheet-ods` | `1.0` | `rust_decimal`, `all_locales` | Read/write ODS | excel |
| `umya-spreadsheet` | `3.1` | — | Read/write existing XLSX with style/formula preservation | excel-preserve |
| `zip` | `9.3` | `deflate`, `bzip2`, `zstd`, `time` | OOXML/ODS containers, archive input | excel / core |
| `quick-xml` | `0.42` | `serialize`, `encoding` | OOXML/ODS XML, DOCX/ODT tables | excel / office |
| `formula` | `0.1` | — | Optional lightweight formula parser/evaluator; immature | excel (optional) |
| `chrono` | `0.4` | `serde`, `clock` | Excel serial dates, timestamps | core |
| `jiff` | `0.2` | `serde` | Modern timezone-aware date/time; preferred for new code | core |

**Decisions**

- **Read:** `calamine` is the primary reader; it is fast and covers XLS/XLSX/XLSB/ODS.
- **Write new XLSX:** `rust_xlsxwriter` gives the best feature/quality trade-off.
- **Write ODS:** `spreadsheet-ods`.
- **Edit existing XLSX:** deferred. `umya-spreadsheet` is not part of v1; use loss reporting instead of pretending to preserve advanced formatting.
- **Formula evaluation:** do not promise full Excel compatibility. Read cached values by default; use `formula` for a documented subset; offer an optional LibreOffice bridge for recalculation.
- **Date handling:** use `jiff` for new timezone-aware code, `chrono` where crates require it (calamine, duckdb, rust_xlsxwriter).

**Build note (Phase 4)**

`rust_xlsxwriter 0.102` is part of the default build: the `default` feature set
includes `rust_xlsxwriter`, and `formats/xlsx_writer.rs` uses the library. The
built-in `formats/xlsx_manual.rs` writer remains available when the crate is
intentionally disabled with `--no-default-features --features core,excel`.
`spreadsheet-ods` is not used; ODS writing remains the built-in ODF writer. Both
XLSX paths expose the same CLI and table-model contract.

### 3.5 Structured documents: JSON, YAML, XML, HTML, Markdown, DOCX, email

> **v1:** `serde_json`, `toml` (config/pipeline), built-in HTML table parser. **CUT:** YAML table I/O, XML/XPath, jq/JSONPath, Markdown/RST/LaTeX output, DOCX/EML/PDF ingestion (deferred). **DEFERRED:** `minijinja` is used only for internal naming, not user templates in v1.

| Crate | Version | Features / notes | Used for | Feature |
|---|---|---|---|---|
| `quick-xml` | `0.42` | `serialize`, `encoding` | XML read/write, XPath-like traversal (manual), DOCX/ODT XML | xml / office |
| `scraper` | `0.29` | — | HTML parsing, CSS selectors, table extraction (not used; built-in parser ships instead) | html (optional alternative) |
| `html5ever` | `0.35` | — | Lower-level HTML parsing (optional direct use) | html |
| `html-escape` | `0.3` | — | HTML escaping/unescaping | html |
| `table-extract` | `0.2` | — | Alternative HTML table extractor; optional | html |
| `pulldown-cmark` | `0.13` | `simd` | Markdown parsing (tables, pipe/grid) | html |
| `serde_yaml_ng` | `0.10` | `serde` | Maintained YAML 1.1/1.2 serde fork; replaces deprecated `serde_yaml` | yaml |
| `saphyr` | `0.3` | — | Full YAML 1.2 parser for edge cases; optional alternative | yaml |
| `toml` | `1.2` | `parse`, `display`, `serde` | TOML config/pipeline files | core |
| `json5` | `1.3` | `serde` | JSON5 input | json-query (optional) |
| `jaq` | `3.1` | — | jq-compatible JSON query engine | json-query |
| `jsonpath-rust` | `1.0` | — | JSONPath queries | json-query |
| `minijinja` | `3.0` | `loader`, `json`, `urlencode` | Template rendering (reports, sheet names, filenames) | core |
| `docx-rs` | `0.4` | — | DOCX generation | office |
| `dotext` | `0.1` | — | DOCX text extraction; immature, use `zip` + `quick-xml` as fallback | office |
| `mail-parser` | `1.0` | `serde`, `full_encoding` | EML parsing, attachments, body tables | office |
| `printpdf` | `0.12` | — | Optional PDF generation | office (optional) |
| `typst` | `0.15` | — | Optional high-quality PDF typesetting; very heavy | office (optional) |
| `headless_chrome` | `1.0` | — | Optional HTML → PDF via Chromium | office (optional) |
| `chromiumoxide` | `0.9` | — | Alternative Chromium driver | office (optional) |

**Decisions**

- Use `serde_yaml_ng` for YAML serde workflows; `serde_yaml` is deprecated. Keep `saphyr` optional for strict YAML 1.2.
- Use the built-in tolerant HTML parser for tables; `scraper` remains an optional alternative if it becomes available.
- For DOCX reading, unzip with `zip` and parse `word/document.xml` with `quick-xml` if `dotext` is insufficient.
- Never execute macros or scripts from Office documents.

**Build note (Phase 6)**

`scraper` was not available in the offline registry. HTML table ingestion uses
the dependency-free parser in `formats/html.rs`; the `html` feature therefore
has no external crate and is safe to include in the default build. If `scraper`
becomes available later it can replace the tokenizer without changing the CLI,
table model, or loss-event contract.

### 3.6 SQL and databases

> **v1:** `duckdb` only (export + read-only query). **CUT:** `rusqlite`, `postgres`, `mysql`, `datafusion`, async drivers. **DEFERRED:** `sqlparser` (only if query validation needs it).

| Crate | Version | Features / notes | Used for | Feature |
|---|---|---|---|---|
| `duckdb` | `1.20000.1` | `bundled`, `chrono`, `serde_json`, `rust_decimal` | DuckDB export and read-only query | sql-duckdb |
| `libduckdb-sys` | `1.20000.1` | Build dependency; do not depend directly | DuckDB C API | sql-duckdb |
| `rusqlite` | `0.41` | `bundled`, `chrono`, `serde_json`, `functions`, `backup`, `blob` | SQLite read/write/query, FTS | sql-sqlite |
| `postgres` | `0.19` | `with-chrono-0_4`, `with-serde_json-1`, `with-jiff-0_2` | PostgreSQL read/write | sql-postgres |
| `mysql` | `28.0` | `chrono`, `rust_decimal`, `rustls` | MySQL read/write | sql-mysql |
| `sqlparser` | `0.63` | — | Parse/validate SQL, dialect translation, safe query inspection | core / sql |
| `datafusion` | `55.2` | `parquet`, `sql`, `serde` | Pure-Rust SQL alternative to DuckDB | sql-datafusion |
| `r2d2` | `0.8` | — | Connection pooling for serve mode | serve (optional) |
| `tokio-postgres` | `0.7` | — | Async PostgreSQL for serve mode | serve (optional) |
| `mysql_async` | `0.36` | — | Async MySQL for serve mode | serve (optional) |

**Decision: DuckDB only, optional**

- DuckDB is the only SQL engine, used for `to duckdb` and read-only `query`.
- No Polars, DataFusion, SQLite, PostgreSQL, MySQL, federation, upsert, or query cache.
- Cost: bundled C++ build (large binary, slow compile), so it is behind `sql-duckdb` and never in the default build.

**DuckDB feature checklist**

- `bundled` — compile DuckDB from source; required for a single-binary `full` build.
- `chrono` — date/time types.
- `serde_json` — parameter/result JSON.
- `rust_decimal` — decimal types.
- Do **not** enable `polars`/`modern-full` (they pull an unnecessary DataFrame engine) or `httpfs` (no network/federation in v1).

### 3.7 Analytical engine, dataframes, and statistical packages

> **CUT/DEFERRED.** No Polars, SAS/SPSS/Stata, NumPy, ML, or statistical packages in v1. DuckDB handles the only supported SQL path.

| Crate | Version | Features / notes | Used for | Feature |
|---|---|---|---|---|
| `polars` | `0.55` | `lazy`, `sql`, `csv`, `json`, `parquet`, `ipc`, `avro`, `dtype-full`, `temporal`, `strings`, `regex`, `pivot`, `dynamic_group_by`, `asof_join`, `cross_join`, `semi_anti_join`, `streaming`, `describe`, `hist`, `random`, `is_in`, `rows`, `round_series`, `mode`, `rank`, `cum_agg`, `diff`, `moment`, `cov` | Joins, group-by, pivot, window functions, Arrow I/O | dataframe-polars |
| `polars-io` | `0.55` | — | Direct readers/writers if needed | dataframe-polars |
| `polars-readstat-rs` | `0.27` | `row_reader` | Pure-Rust SAS/SPSS/Stata readers → Polars/Arrow | research-formats |
| `dta` | `0.6` | `chrono` | Stata DTA reader/writer | research-formats |
| `dbase` | `0.8` | `chrono`, `encoding_rs` | DBF read/write | research-formats |
| `npyz` | `0.9` | `npz` | NumPy `.npy`/`.npz` read/write | research-formats |
| `ndarray` | `0.17` | `serde`, `rayon`, `blas` (optional) | Numeric arrays, PCA/ML | stats |
| `ndarray-stats` | `0.7` | — | Order statistics, correlation, histograms | stats |
| `statrs` | `0.19` | — | Distributions, hypothesis tests, descriptive stats | stats |
| `augurs` | `0.10` | `ets`, `forecaster`, `outlier`, `clustering`, `mstl`, `seasons` | Time-series forecasting/outlier detection | timeseries |
| `linfa` | `0.8` | `serde` | ML framework core | stats |
| `linfa-linear` | `0.8` | — | Linear regression | stats |
| `linfa-logistic` | `0.8` | — | Logistic regression | stats |
| `linfa-clustering` | `0.8` | — | K-means, DBSCAN | stats |
| `linfa-reduction` | `0.8` | — | PCA | stats |
| `linfa-preprocessing` | `0.8` | — | Scaling/normalization | stats |
| `smartcore` | `0.7` | — | Alternative lean ML library with built-in algorithms; pick one | stats (optional) |
| `petgraph` | `0.8` | `serde-1` | Pipeline DAG, graph analytics | core / graph |
| `graphviz-rust` | `0.9` | — | Export DAG/graph as DOT/SVG | graph |

**No Polars in v1.** The DataFrame/analytics engine is cut. DuckDB covers the only SQL path.

### 3.8 Columnar and binary formats

> **CUT/DEFERRED.** No Avro, ORC, HDF5, NetCDF, or NumPy in v1. Parquet/Arrow are pulled only by `sql-duckdb` if needed for import/export.

| Crate | Version | Features / notes | Used for | Feature |
|---|---|---|---|---|
| `arrow` | `60.1` | `prettyprint`, `ffi`, `ipc`, `chrono-tz` | Arrow memory/IPC | columnar |
| `arrow-csv` | `60.1` | — | Arrow ↔ CSV | columnar |
| `arrow-json` | `60.1` | — | Arrow ↔ JSON | columnar |
| `parquet` | `60.1` | `arrow`, `lz4`, `zstd`, `flate2-rust_backend`, `crc` | Parquet read/write | columnar |
| `apache-avro` | `0.22` | `snappy`, `zstandard`, `bzip`, `xz` | Avro read/write | columnar |
| `orcrs` | `0.5` | — | ORC read-only; experimental and old | columnar (optional) |
| `hdf5` | `0.8` | — | HDF5 read/write; requires system HDF5 | columnar (optional) |
| `netcdf` | `0.12` | — | NetCDF read/write; requires system NetCDF | columnar (optional) |
| `npyz` | `0.9` | — | NumPy arrays | columnar |

**Notes**

- `arrow`/`parquet` 60.1 matches the Arrow version used by `duckdb-rs` 1.20000.1, which keeps interchange cheap.
- ORC support is **not in v1**. `orcrs` is listed only as historical research; it is old and read-only.
- HDF5/NetCDF are behind features and require system libraries; do not include them in the default build.

### 3.9 Statistics, math, time, and privacy primitives

> **v1:** keep date/time, decimal, checksum, and basic numeric utilities. **CUT:** rand/fake/privacy/text-mining rows (PII/synthetic/NLP discontinued).

| Crate | Version | Features / notes | Used for | Feature |
|---|---|---|---|---|
| `rust_decimal` | `1.43` | `serde`, `maths` | Exact decimal arithmetic, money | core |
| `bigdecimal` | `0.4` | `serde` | Arbitrary precision | stats (optional) |
| `num-traits` | `0.2` | — | Generic numeric code | core |
| `num-format` | `0.4` | — | Number formatting | core |
| `chrono` | `0.4` | `serde`, `clock` | Dates/timestamps | core |
| `jiff` | `0.2` | `serde` | Timezone-aware date/time | core |
| `dateparser` | `0.3` | — | Parse common human date formats | core |
| `iso8601` | `0.6` | — | Strict ISO-8601 parsing | core (optional) |
| `fundu` | `2.1` | — | Parse durations (`24h`, `5m`) for freshness/watch | core |
| `rand` | `0.10` | `std`, `std_rng` | Sampling, synthetic data | core / synthetic |
| `rand_distr` | `0.6` | — | Distributions, differential privacy noise | synthetic / privacy |
| `fake` | `5.1` | `derive` | Realistic synthetic values | synthetic |
| `strsim` | `0.11` | — | Levenshtein/Jaro-Winkler for fuzzy joins | text-mining |
| `aho-corasick` | `1.1` | — | Multi-pattern PII/text scanning | text-mining / privacy |
| `regex` | `1.13` | `unicode`, `perf` | Regex filters, PII, parsing | core |
| `fancy-regex` | `0.20` | — | Lookaround/backreferences; optional | text-mining (optional) |
| `whatlang` | `0.18` | — | Language detection | text-mining |
| `rust-stemmers` | `1.2` | — | Snowball stemming for text mining | text-mining |
| `grok` | `2.4` | — | Log pattern extraction | text-mining |
| `censor` | `0.3` | — | Profanity detection | text-mining (optional) |
| `vader_sentiment` | `0.1` | — | Sentiment scoring | text-mining (optional) |
| `tokenizers` | `0.23` | — | HuggingFace tokenizers; heavy, optional | text-mining (optional) |
| `unicode-normalization` | `0.1` | — | Unicode normalization for matching | text-mining |

### 3.10 Geospatial

> **CUT.** Geo analytics (N12) is discontinued. No geo crates in v1.

| Crate | Version | Features / notes | Used for | Feature |
|---|---|---|---|---|
| `geo` | `0.33` | `use-proj` (optional) | Geometry algorithms: distance, area, buffer, contains | geo |
| `geo-types` | `0.7` | `serde` | Geometry primitives | geo |
| `geozero` | `0.15` | `with-geojson`, `with-shp`, `with-gpkg` (optional) | Zero-copy geo format conversion | geo |
| `geojson` | `1.0` | `serde` | GeoJSON read/write | geo |
| `shapefile` | `0.9` | — | Shapefile read/write | geo |
| `gpx` | `0.10` | — | GPX read/write | geo |
| `kml` | `0.15` | — | KML read/write | geo |
| `proj` | `0.31` | — | CRS transforms; requires PROJ | geo-proj |
| `gdal` | `0.19` | — | Broad geospatial formats; requires GDAL | geo-gdal |

### 3.11 Compression, archives, and filesystem

> **v1:** `flate2` (gzip), `zstd`, `zip`, `walkdir`, `globset`, `same-file`, `dunce`, `fs4`, `atomicwrites`/`tempfile`. **CUT:** bzip2, xz, lz4, snappy, tar (optional), sevenz-rust, 7z write, exotic compression.

| Crate | Version | Features / notes | Used for | Feature |
|---|---|---|---|---|
| `flate2` | `1.1` | `rust_backend` or `zlib-ng` | gzip/zlib | core |
| `zstd` | `0.14` | `zstdmt` | Zstandard | core |
| `bzip2` | `0.6` | `static` | bzip2 | CUT |
| `xz2` | `0.1` | — | xz/lzma | CUT |
| `lz4_flex` | `0.14` | `frame` | LZ4 | CUT |
| `snap` | `1.1` | — | Snappy | CUT |
| `zip` | `9.3` | `deflate`, `zstd`, `time` | ZIP batch input and OOXML/ODS containers | core |
| `tar` | `0.4` | — | TAR archives | DEFERRED (optional read) |
| `sevenz-rust` | `0.6` | — | 7z archives | CUT |
| `walkdir` | `2.5` | — | Recursive ingestion | core |
| `globset` | `0.4` | — | Include/exclude patterns | core |
| `same-file` | `1.0` | — | Prevent self-overwrite | core |
| `dunce` | `1.0` | — | Windows paths | core |
| `fs4` | `1.1` | — | File locks, single-writer safety | core |
| `atomicwrites` | `0.4` | — | Atomic file replacement | core |
| `filetime` | `0.2` | — | Preserve timestamps on copy | core (optional) |

**Build note (Phase 7)**

`walkdir` and `globset` are enabled by the `core` feature. `zip` moved from
the `excel` feature to `core` because directory/ZIP batch ingestion is part of
the conversion core. The batch reader enforces entry-count, total-uncompressed
size, and compression-ratio limits and rejects absolute paths, `..` components,
and Windows-style separators before writing anything.

### 3.12 Scripting, templating, and plugins

> **CUT.** Embedded scripting, jq, Lua, and WASM plugins are discontinued. `minijinja` may remain only for internal file/sheet-name templates, not user scripting.

| Crate | Version | Features / notes | Used for | Feature |
|---|---|---|---|---|
| `evalexpr` | `13.1` | — | Typed expressions | CUT (use a tiny built-in comparison parser for v1 filters) |
| `rhai` | `1.27` | `serde`, `sync` | Embedded scripting for advanced transforms | scripting |
| `mlua` | `0.12` | `lua54`, `luau`, `vendored`, `serde` | Lua/Luau scripting compatibility | scripting |
| `jaq` | `3.1` | — | jq-compatible query language | json-query |
| `minijinja` | `3.0` | `loader`, `json`, `urlencode` | Internal file/sheet-name templates only | core |
| `extism` | `1.41` | — | Sandboxed WASM plugins (primary) | plugins |
| `wasmtime` | `50.0` | `component-model` | Alternative WASM runtime, more control | plugins (alternative) |
| `libloading` | `0.9` | — | Native dynamic plugins; unsafe, optional | plugins (optional) |
| `abi_stable` | `0.11` | — | Stable Rust ABI for native plugins | plugins (optional) |
| `semver` | `1.0` | `serde` | Plugin version compatibility | plugins |
| `wat` | `1.240` | — | Compile WAT for plugin tests | plugins (dev) |

**Decision:** no embedded expression engine or plugin runtime in v1. Filtering uses a tiny built-in parser for simple comparisons (`col == value`, `col != value`, numeric comparisons); no `evalexpr`, `rhai`, `mlua`, jq, or WASM.

### 3.13 Validation, diff, schema, and contracts

> **DEFERRED.** N4/N5 are post-v1. Keep `similar`/`json-patch` out of the default build; `jsonschema`/`schemars` only when contracts return.

| Crate | Version | Features / notes | Used for | Feature |
|---|---|---|---|---|
| `jsonschema` | `0.60` | `resolve-http` (optional); Draft 2020-12 is supported by default | Validate tables against JSON Schema | validate |
| `schemars` | `1.2` | `derive` | Generate JSON Schema from Rust types | validate |
| `similar` | `3.2` | `inline`, `unicode` | Table/text diff engine | diff |
| `json-patch` | `4.2` | `diff` | RFC 6902 patch format | diff |
| `serde_json` | `1.0` | `preserve_order` | Diff/patch serialization | diff |
| `regex` | `1.13` | — | Contract patterns (deferred) | validate |
| `fundu` | `2.1` | — | Freshness durations | validate |
| `blake3` / `sha2` | `1.8` / `0.11` | — | Checksums, audit | core |

**Notes**

- Table diff is not a JSON diff; build a row/cell-aware engine on top of `similar`'s algorithms or a custom LCS.
- JSON Patch is used for pipeline definitions and portable patch files, not as the only table patch format.
- Contracts can compile to JSON Schema where possible; keep a native contract model for cross-column and freshness rules.

### 3.14 Configuration, logging, errors, and observability

| Crate | Version | Features / notes | Used for | Feature |
|---|---|---|---|---|
| `figment` | `0.10` | `env`, `json`, `yaml` | Layered config (file + env + CLI) | core |
| `confy` | `2.0` | — | Simple config management; alternative to figment | core (optional) |
| `directories` | `6.0` | — | Config/cache/data directories | core |
| `toml` | `1.2` | `serde` | Config/pipeline files | core |
| `tracing` | `0.1` | `max_level_info` | Structured logging/spans | core |
| `tracing-subscriber` | `0.3` | `env-filter`, `json`, `fmt` | Log output | core |
| `tracing-appender` | `0.2` | — | Rotating file logs | core (optional) |
| `anyhow` | `1.0` | — | Application errors | core |
| `thiserror` | `2.0` | — | Library error types | core |
| `miette` | `7.6` | `fancy` | Rich diagnostics with source snippets | core |
| `vergen` | `10.0` | `build`, `cargo` | Version/build metadata | core |
| `human-panic` | `2.0` | — | Friendly crash reports; optional | core (optional) |
| `dotenvy` | `0.15` | — | `.env` loading for dev; optional | core (optional) |

### 3.15 HTTP, cloud, server, and watch

> **CUT.** No network, cloud, server, MCP, watch, or federation in v1.

| Crate | Version | Features / notes | Used for | Feature |
|---|---|---|---|---|
| `ureq` | `3.5` | `rustls`, `gzip`, `brotli` | Blocking HTTP(S) input | core / cloud |
| `reqwest` | `0.13` | `rustls`, `json`, `stream`, `gzip`, `brotli` | Async HTTP, cloud, serve | cloud / serve |
| `url` | `2.5` | `serde` | URL parsing, database URLs | core |
| `object_store` | `0.14` | `aws`, `gcp`, `azure`, `http` | Unified S3/GCS/Azure/HTTP object storage | cloud |
| `aws-sdk-s3` | `1.157` | — | AWS-specific features; heavy, optional | cloud (optional) |
| `google-cloud-storage` | `1.21` | — | GCS-specific features; optional | cloud (optional) |
| `azure_storage_blobs` | `0.21` | — | Azure-specific features; optional | cloud (optional) |
| `tokio` | `1.54` | `rt-multi-thread`, `macros`, `fs`, `io-util`, `net`, `signal` | Async runtime for serve/cloud | serve / cloud |
| `axum` | `0.8` | `json`, `multipart`, `tokio` | HTTP API | serve |
| `tower-http` | `0.7` | `cors`, `trace`, `limit`, `compression-full` | HTTP middleware | serve |
| `rmcp` | `3.8` | `server`, `transport-io` | MCP server | serve |
| `jsonrpsee` | `0.26` | `server`, `client` | Alternative JSON-RPC | serve (optional) |
| `notify` | `8.2` | `macos_kqueue` (optional) | Watch mode | watch |
| `rustls` | `0.23` | `ring` (or the aws-lc backend) | TLS backend | cloud / serve |
| `webpki-roots` | `1.0` | — | Root certificates for static builds | cloud / serve |

**Decision:** `ureq` keeps the core blocking and small. `reqwest` + `tokio` are only compiled for `cloud`/`serve`. Never enable network features by default; network access is explicit per input/output.

### 3.16 PDF, OCR, and document extraction

> **DEFERRED.** PDF/DOCX/EML ingestion and OCR are post-v1. HTML table ingestion is the only v1 document source.

| Crate | Version | Features / notes | Used for | Feature |
|---|---|---|---|---|
| `pdf-extract` | `0.12` | — | Text extraction from PDFs | pdf |
| `lopdf` | `0.46` | — | PDF object traversal, page text | pdf |
| `pdfium-render` | `0.9` | — | Render PDF pages; requires PDFium | pdf-render |
| `ocrs` | `0.13` | `rten` (models downloaded separately) | Pure-Rust OCR | ocr |
| `image` | `0.25` | `png`, `jpeg`, `tiff`, `webp` | Image decode for OCR/rendering | ocr / pdf-render |
| `leptess` | `0.14` | — | Tesseract bridge; requires libtesseract | ocr-tesseract |
| `docx-rs` | `0.4` | — | DOCX output | office |
| `dotext` | `0.1` | — | DOCX text extraction | office |
| `mail-parser` | `1.0` | — | EML parsing | office |
| `printpdf` | `0.12` | — | PDF output | office (optional) |
| `typst` | `0.15` | — | High-quality PDF typesetting; very heavy | office (optional) |

**Notes**

- PDF table extraction is heuristic. Always emit a confidence score and preserve the source page/coordinates.
- OCR models can be large; do not bundle them by default. Provide a `tabmux models download` command or document manual installation.
- `pdfium-render` and `leptess` require system libraries; keep them behind features.

### 3.17 Natural-language copilot

> **CUT.** N7 is discontinued. No LLM or inference dependencies.

| Crate | Version | Features / notes | Used for | Feature |
|---|---|---|---|---|
| `ureq` / `reqwest` | `3.5` / `0.13` | JSON | OpenAI-compatible HTTP API | nlp |
| `serde_json` | `1.0` | — | Request/response payloads | nlp |
| `llama-cpp-2` | `0.1` | `metal` (macOS), `cuda` (optional) | Local GGUF inference; optional | nlp-local |
| `candle-core` | `0.11` | — | Alternative local inference; optional | nlp-local |
| `mistralrs` | `0.8` | — | Alternative local inference; heavy | nlp-local |

**Decision:** default to an OpenAI-compatible HTTP endpoint configured by the user. Local inference is strictly opt-in and feature-gated. The LLM never receives raw data by default; only schema, statistics, and redacted samples.

### 3.18 Testing, benchmarking, fuzzing

| Crate | Version | Features / notes | Used for |
|---|---|---|---|
| `assert_cmd` | `2.2` | — | CLI integration tests |
| `predicates` | `3.1` | — | Assertions on command output |
| `insta` | `1.49` | `json`, `redactions` | Snapshot tests |
| `proptest` | `1.12` | — | Property-based tests |
| `criterion` | `0.8` | `html_reports` | Benchmarks |
| `tempfile` | `3.27` | — | Test fixtures |
| `wiremock` | `0.6` | `json` | HTTP mock tests |
| `pretty_assertions` | `1.4` | — | Readable diffs |
| `rstest` | `0.26` | — | Fixture/parameterized tests (optional) |
| `libfuzzer-sys` | `0.4` | — | Fuzz targets (nightly) |
| `cargo-fuzz` | external | — | Fuzzing harness |
| `cargo-nextest` | external | — | Faster test runner |
| `cargo-deny` | external | — | License/advisory/duplicate checks |
| `cargo-audit` | external | — | Security advisories |
| `cargo-about` | external | — | License notices |

### 3.19 Build and packaging

| Crate / tool | Version | Used for |
|---|---|---|
| `cc` | `1.0` | Build DuckDB/HDF5/NetCDF C/C++ components |
| `bindgen` | `0.72` | Generate bindings for C libraries |
| `pkg-config` | `0.3` | Locate system HDF5/NetCDF/GDAL/PROJ |
| `cmake` | `0.1` | Build DuckDB with CMake-backed features |
| `vergen` | `10.0` | Embed git hash/build date |
| `mdbook` | `0.5` | Documentation site |
| `cargo-dist` | external | Release binaries/installers |
| `cross` | external | Cross-compilation |
| `cargo-vendor` | external | Offline/vendored builds |
| `just` | external | Task runner (optional) |

---

## 4. Feature-to-dependency map (v1)

| Feature area | Primary dependencies |
|---|---|
| CSV/TSV/JSON I/O | `csv`, `csv-sniffer`, `serde_json`, `encoding_rs`, `chardetng`, `encoding_rs_io`, `unicode-bom` |
| XLSX/XLS/XLSB/ODS read | `calamine`, `zip`, `quick-xml`, `chrono`/`jiff` |
| XLSX/ODS write | `rust_xlsxwriter`, `spreadsheet-ods`, `zip`, `quick-xml` |
| HTML tables | built-in tolerant parser in `formats/html.rs` |
| Directory/ZIP ingestion | `walkdir`, `globset`, `zip`, `same-file`, `dunce` |
| Basic transforms | `indexmap`, `itertools`, `regex`, `natord` |
| Merge/schema diff | `indexmap`, `hashbrown`, `ahash`, `serde_json` |
| Pipeline manifest | `toml`, `serde`, `serde_json`, `petgraph` (validation only) |
| Loss report | `serde`, `serde_json`, `blake3`/`sha2`, `chrono` |
| Safe writes | `tempfile`, `fs4`, `atomicwrites` (or `tempfile` persist), `blake3`/`sha2`, `tracing` |
| DuckDB export/query | `duckdb`, `arrow` (optional) |
| CLI/terminal | `clap`, `clap_complete`, `clap_mangen`, `comfy-table`, `owo-colors`, `indicatif`, `dialoguer` |
| Errors/logging | `anyhow`, `thiserror`, `miette`, `tracing`, `tracing-subscriber` |
| Config | `figment`, `directories`, `toml` |
| Compression | `flate2` (gzip), `zstd`, `zip` |
| Testing | `assert_cmd`, `predicates`, `insta`, `proptest`, `tempfile` |

---

## 5. Proposed `Cargo.toml` skeleton (v1)

```toml
[package]
name = "tabmux"
version = "0.1.0"
edition = "2024"
rust-version = "1.88"
license = "MIT OR Apache-2.0"
description = "Safe, reproducible conversion of messy tabular sources into clean tables and workbooks"

[features]
default = ["core", "excel", "html"]

core = [
  # CLI
  "dep:clap", "dep:clap_complete", "dep:clap_mangen",
  # serialization / model
  "dep:serde", "dep:serde_json", "dep:indexmap",
  # delimited text and encodings
  "dep:csv", "dep:csv-sniffer",
  "dep:encoding_rs", "dep:chardetng", "dep:encoding_rs_io", "dep:unicode-bom",
  # compression and archive input
  "dep:flate2", "dep:zstd", "dep:zip",
  # filesystem
  "dep:globset", "dep:walkdir", "dep:same-file", "dep:dunce",
  # utilities
  "dep:itertools", "dep:memchr", "dep:tempfile", "dep:fs4",
  "dep:chrono", "dep:jiff", "dep:rust_decimal",
  "dep:num-format", "dep:natord",
  # terminal output
  "dep:comfy-table", "dep:unicode-width", "dep:unicode-segmentation",
  "dep:owo-colors", "dep:anstream", "dep:indicatif", "dep:dialoguer",
  "dep:terminal_size", "dep:bytesize",
  # text / internal naming
  "dep:regex", "dep:minijinja",
  # config, errors, logging
  "dep:anyhow", "dep:thiserror", "dep:miette",
  "dep:figment", "dep:directories", "dep:toml",
  "dep:tracing", "dep:tracing-subscriber",
  # integrity
  "dep:blake3", "dep:hex",
]

excel = [
  "dep:calamine", "dep:rust_xlsxwriter", "dep:spreadsheet-ods",
  "dep:zip", "dep:quick-xml",
]

html = []

sql-duckdb = ["dep:duckdb"]

full = ["default", "sql-duckdb"]

[dependencies]
# Add dependencies with `optional = true` for every `dep:` above.
clap = { version = "4.6", features = ["derive", "env", "wrap_help", "cargo", "unicode", "string"], optional = true }
csv = { version = "1.4", optional = true }
serde = { version = "1.0", features = ["derive"], optional = true }
serde_json = { version = "1.0", features = ["preserve_order"], optional = true }
calamine = { version = "0.38", features = ["dates"], optional = true }
rust_xlsxwriter = { version = "0.102", features = ["chrono", "zlib", "constant_memory", "rust_decimal"], optional = true }
spreadsheet-ods = { version = "1.0", features = ["rust_decimal"], optional = true }
duckdb = { version = "1.20000.1", default-features = false, features = ["bundled", "chrono", "serde_json", "rust_decimal"], optional = true }

[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1
strip = "debuginfo"
panic = "unwind"
```

> **Note:** this is intentionally minimal. Add a dependency only when a phase in `devplan.md` needs it, and add it behind the matching feature.

---

## 6. External/system dependencies (optional)

The v1 default build needs **no external tools**. Only one optional bridge is recommended:

| Tool | Used for | Status | License / caveat |
|---|---|---|---|
| **LibreOffice headless** | Maximum-fidelity XLSX/ODS → PDF/HTML/DOCX and formula recalculation | Optional, post-v1 nice-to-have | MPL-2.0/LGPL; large install; slow startup |
| **Gnumeric `ssconvert`** | Legacy spreadsheet formats (Lotus, Quattro, DIF, SYLK) | CUT from v1 | GPL-2.0/3.0 |
| **Pandoc** | Document conversion/rendering | CUT | GPL-2.0+ |
| **Tesseract / PDFium / OCR** | PDF/OCR ingestion | DEFERRED | Apache-2.0 / BSD-3-Clause |
| **PROJ / GDAL** | Geospatial formats | CUT | MIT/X11 |
| **HDF5 / NetCDF** | Scientific arrays | CUT | BSD-3-Clause |
| **Python 3** | Scripting bridge | CUT | PSF |
| **Chromium** | HTML → PDF | CUT | BSD-3-Clause |
| **llama.cpp / GGUF** | NL copilot | CUT | MIT; model licenses vary |
| **7-Zip** | 7z extraction | CUT (7z write cut; read deferred) | LGPL |

If a LibreOffice bridge is added, it must be:

- explicitly opt-in (`--use-external=libreoffice`),
- capability-probed and version-detected,
- invoked without shell interpolation,
- reported in the provenance manifest,
- treated as a fidelity fallback, not a dependency of the core.

---

## 7. Build profiles, CI, and release

### 7.1 Profiles

```toml
[profile.dev]
opt-level = 0
debug = true

[profile.dev.package."*"]
opt-level = 2

[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1
strip = "debuginfo"
panic = "unwind"

[profile.dist]
inherits = "release"
lto = "fat"
codegen-units = 1
```

- Use `panic = "unwind"` (not `abort`) so the CLI can report a diagnostic and the test harness can catch panics.
- `lto = "fat"` only for release artifacts; it significantly increases link time.
- Build DuckDB with `bundled` in release CI; cache the build across jobs.
- Consider `sccache` or `cargo-cache` for CI.

### 7.2 CI matrix

| Dimension | Values |
|---|---|
| OS | Ubuntu, macOS, Windows |
| Arch | x86_64, aarch64 |
| Rust | MSRV 1.88, stable, nightly (fuzz only) |
| Feature sets | `core`, `default`, `excel`, `html`, `sql-duckdb`, `full` |
| Checks | `fmt`, `clippy -D warnings`, `test`, `doc`, `deny`, `audit`, `fuzz` (nightly), `bench` (main only) |

**Suggested jobs**

```sh
cargo fmt --check
cargo clippy --all-targets --features core -- -D warnings
cargo test --features core,excel,html
cargo test --features sql-duckdb --test duckdb
cargo check --no-default-features --features core
cargo check --all-features
cargo deny check
cargo audit
```

`--all-features` is heavy because of bundled DuckDB. Run it in a separate, cached job; do not block every PR on it.

### 7.3 Release artifacts

- Linux: `x86_64-unknown-linux-gnu`, `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-gnu`, `aarch64-unknown-linux-musl`.
- macOS: `x86_64-apple-darwin`, `aarch64-apple-darwin`.
- Windows: `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`.
- Provide a `minimal` binary (`core` + `excel` + `html`) and a `full` binary (adds `sql-duckdb`).
- Ship shell completions and man pages in the archive.
- Publish checksums and a software bill of materials (SBOM).
- Consider `cargo-dist` for installers and Homebrew/Scoop/Winget manifests.

---

## 8. Risks, alternatives, and decisions

| Risk | Impact | Mitigation |
|---|---|---|
| DuckDB bundled build is slow/large | Long CI, large binary | Feature-gate `sql-duckdb`; offer `minimal` build; cache builds |
| Excel fidelity expectations | User disappointment/support burden | Publish a fidelity matrix; emit loss reports; `--strict` |
| Merged cells/date edge cases | Silent corruption | Fixtures, explicit policies, round-trip tests |
| HTML table heuristics | Wrong headers/sheets | Confidence warnings, `--min-rows/cols`, preview/dry-run |
| CSV/Excel formula injection | Security | Warn/sanitize on export (`=`, `+`, `-`, `@`) |
| Zip bombs / path traversal | DoS / file escape | Entry/size/ratio limits; canonicalize; reject `..` |
| SQL injection in `query` | Data exfiltration | Read-only guard, parameterized queries, identifier quoting |
| Secret leakage in logs | Credential exposure | No network in v1; still redact paths/tokens in diagnostics |
| Large dependency tree | Supply-chain risk | `cargo-deny`, `cargo-audit`; minimize default features |
| License incompatibility | Distribution blocked | Default MIT/Apache-2.0; GPL tools external-only |
| Non-deterministic output | Pipeline replay fails | Stable ordering, stable sheet names, checksum tests |
| Loss report noise | Users ignore warnings | Stable codes, severity, counts, concise Markdown summary |

### 8.1 Key decisions

1. **DuckDB is the only SQL engine, and it is optional.** It is used for export and read-only queries; it is not the conversion engine.
2. **calamine + rust_xlsxwriter + spreadsheet-ods is the spreadsheet stack.** No `umya-spreadsheet`, no style cloning, no formula evaluation in v1.
3. **No full Excel formula evaluator.** Read cached values, report `formula_cached_value_only`; optional LibreOffice recalculation later.
4. **TOML is config only.** YAML/TOML table write is cut.
5. **No PDF/OCR/DOCX/EML ingestion in v1.** HTML tables are the only document source.
6. **No plugins, scripting, TUI, server, or network.** They are discontinued, not feature-gated.
7. **Loss reporting is mandatory.** Every conversion emits a report; `--strict` fails before writing.
8. **Safe writes are mandatory.** Atomic temp-and-rename, no-clobber by default, checksums in provenance.
9. **LibreOffice is an optional fidelity bridge, not a dependency.**

---

## 9. Dependency policy

### 9.1 Licensing

- Default build: only MIT, Apache-2.0, BSD, ISC, Zlib, MPL-2.0-compatible crates.
- GPL/AGPL tools (Gnumeric, Pandoc, Git) are **external runtime bridges**, never linked. LibreOffice is MPL-2.0/LGPL and remains optional.
- Generate a `THIRD_PARTY_NOTICES.md` with `cargo-about` for every release.
- Run `cargo deny check licenses` in CI.

### 9.2 Security

- `cargo audit` on every CI run; `cargo deny check advisories`.
- Pin `Cargo.lock`; review dependency updates with `cargo update --dry-run` and diff review.
- Disable unused default features; prefer `default-features = false`.
- Fuzz all v1 parsers: CSV, XLSX, ODS, HTML, JSON.
- Enforce resource limits: max file size, max rows, max columns, max cell size, max archive entries, max decompression ratio.
- Sanitize spreadsheet formula injection on export.
- Never execute macros, formulas, or scripts from input documents.
- Reject path traversal in ZIP members; canonicalize output paths.
- Use parameterized SQL and quoted identifiers; keep `query` read-only.
- Redact secrets in logs and error messages.

### 9.3 Reproducibility

- Commit `Cargo.lock`.
- Use `cargo vendor` for offline/air-gapped builds.
- Record tool versions, git commit, and feature flags in `--version --verbose` and provenance manifests.
- Pin external tool versions in integration tests.
- Use deterministic ordering and stable sheet names; no timestamps in data outputs.

### 9.4 Performance budgets

| Build | Target binary size (stripped) | Target clean build time |
|---|---|---|
| `core` | < 15 MB | < 2 min |
| `default` (core + excel + html) | < 30 MB | < 5 min |
| `full` (+ DuckDB) | < 60 MB | < 20 min (cached DuckDB) |

These are guardrails, not hard limits; measure on CI and adjust.

---

## 10. Dependency sets per devplan phase

| Phase | New dependencies | Result |
|---|---|---|
| **0 Skeleton** | `clap`, `clap_complete`, `clap_mangen`, `anyhow`, `thiserror`, `miette`, `tracing` | Buildable CLI + format matrix |
| **1 Core** | `csv`, `csv-sniffer`, `serde`, `serde_json`, `indexmap`, `encoding_rs`, `chardetng`, `encoding_rs_io`, `unicode-bom`, `flate2`, `zstd`, `zip`, `globset`, `walkdir`, `same-file`, `dunce`, `tempfile`, `fs4`, `chrono`, `jiff`, `rust_decimal`, `comfy-table`, `owo-colors`, `indicatif`, `dialoguer`, `regex`, `blake3`, `hex` | CSV/TSV/JSON conversion, safe writes, select/head/count |
| **2 XLSX read** | `calamine`, `quick-xml` | XLSX/XLS/XLSB read, sheets, CSV conversion |
| **3 ODS read** | `calamine` (ODS support) | ODS read, all-sheets, row/cell output |
| **4 Workbook write** | `rust_xlsxwriter`, `spreadsheet-ods` | XLSX/ODS write, multi-sheet, basic formatting |
| **5 Merge/transforms** | `hashbrown`/std collections, `natord` | Merge, schema diff, filter/sort/rename/dedupe |
| **6 HTML tables** | none (built-in parser) | HTML table extraction, HTML → workbook |
| **7 Batch ingestion** | `walkdir`, `globset`, `zip` | Directory/ZIP batch conversion |
| **8 Pipeline** | `toml`, `serde` | TOML manifest, dry-run, replay |
| **9 Loss/provenance** | `serde_json`, `blake3` | Loss report, manifest, strict mode |
| **10 DuckDB** | `duckdb` | Export + read-only query |
| **11 Quality gate** | `assert_cmd`, `predicates`, `insta`, `proptest`, `criterion` (dev) | Tests, fuzz targets, release checks |

---

## 11. Dependency review checklist

Before adding a crate:

- [ ] Is it required by a phase in `devplan.md`?
- [ ] Is it in scope (not a discontinued feature)?
- [ ] Is it actively maintained (release within the last 12 months, responsive issues)?
- [ ] Is the license compatible with the default build?
- [ ] Can it be feature-gated without contaminating the core API?
- [ ] Does it duplicate an existing dependency's functionality?
- [ ] Does it pull in a conflicting Arrow/DuckDB version?
- [ ] Does it require system libraries or network access at build time?
- [ ] Is there a pure-Rust alternative that is good enough?
- [ ] Are its transitive dependencies audited?
- [ ] Is there a fallback if it becomes unmaintained?

---

## 12. References

- crates.io: <https://crates.io/>
- docs.rs: <https://docs.rs/>
- DuckDB Rust: <https://github.com/duckdb/duckdb-rs>
- calamine: <https://github.com/tafia/calamine>
- rust_xlsxwriter: <https://github.com/jmcnamara/rust_xlsxwriter>
- spreadsheet-ods: <https://github.com/thscharler/spreadsheet-ods>
- `csv` crate: <https://github.com/BurntSushi/rust-csv>
- `serde`/`serde_json`: <https://serde.rs/>
- `scraper`: <https://github.com/rust-scraper/scraper>
- LibreOffice headless (optional fidelity bridge): <https://help.libreoffice.org/latest/en-US/text/shared/guide/start_parameters.html>
