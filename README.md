# tabmux

**Safe, reproducible conversion of messy tabular sources into clean tables and workbooks.**

`tabmux` reads CSV/TSV, XLSX/XLS/XLSB, ODS, HTML tables, JSON/JSONL, directories, and ZIP
archives; lets you select, filter, sort, rename, merge, and concatenate tables; and writes
clean CSV/TSV or multi-sheet XLSX/ODS workbooks. Every conversion reports what was lost,
and every write is atomic and non-destructive by default.

The five v1 commitments are:

| Commitment | What it means in practice |
|---|---|
| **Reproducible pipeline** | A declarative `pipeline.toml` can be replayed byte-for-byte, with dry-run and validation before writing |
| **Messy-source ingestion** | CSV/TSV, XLSX, ODS, HTML, JSON/JSONL, directories, and ZIP archives become one clean table set |
| **Workbook fidelity** | Sheets, native values, dates, merged-cell expansion, and basic formatting, with honest limits |
| **Loss report** | A versioned JSON report lists dropped, approximated, and unsupported content |
| **Safe by default** | No silent overwrite, atomic writes, dry-run, checksums, provenance manifest, and strict mode |

`tabmux` is a conversion engine, not an analytics platform. Geo, graph, statistics/ML,
NL copilots, PII generation, dashboards, MCP servers, plugins, and cloud/federated I/O are
explicitly out of scope. See [`docs/devplan.md`](docs/devplan.md) for the authoritative v1
scope.

## Install

### From source

Requires Rust **1.88** or newer.

```sh
git clone https://github.com/badretd/tabmux.git
cd tabmux
cargo build --release
./target/release/tabmux --version
```

The default build includes CSV/TSV/JSON, XLSX/ODS read/write, HTML tables, ZIP/directory
batch mode, the pipeline runner, and loss reports. The optional SQL export/query surface is
enabled with `--features sql-duckdb`.

### Feature flags

| Feature | Default | Enables |
|---|---|---|
| `core` | yes | CSV/TSV/JSON, inspection and transforms, safe writes, batch ZIP/directory, pipeline, loss reports |
| `excel` | yes | XLSX/XLS/XLSB/ODS read and XLSX/ODS write |
| `html` | yes | HTML table extraction |
| `rust_xlsxwriter` | yes | Library-backed XLSX writer; without it the build uses the built-in minimal writer |
| `sql-duckdb` | no | `to duckdb` and read-only `query` commands |
| `full` | no | `default` + `sql-duckdb` |

Minimal build:

```sh
cargo build --release --no-default-features --features core
```

Workbook + HTML build without SQL:

```sh
cargo build --release --no-default-features --features core,excel,html
```

## Quick start

### Convert a workbook sheet to CSV

```sh
tabmux sheets book.xlsx
tabmux convert book.xlsx sales.csv --sheet Sales
```

Dates are exported as ISO-8601 text, merged cells are expanded, and cached formula values
are used with an `excel.formula_cached_value_only` warning.

### Convert every sheet to its own file

```sh
tabmux convert book.ods out/ --all-sheets --split-sheets
ls out
```

### Combine sheets into one table

```sh
tabmux convert book.xlsx stacked.csv --all-sheets --stack --add-source-column sheet
```

### Turn every HTML table into a sheet

```sh
tabmux convert page.html tables.xlsx \
  --tables all --sheet-per-table --use-captions --index-sheet
```

### Build a workbook from several CSVs

```sh
tabmux convert january.csv february.csv -o report.xlsx --sheet-name-template '{stem}'
```

### Inspect data

```sh
tabmux count sales.csv
tabmux head sales.csv --rows 5
tabmux select sales.csv --columns name,amount
tabmux row sales.xlsx --sheet Sales --where 'id == 3'
tabmux cell sales.xlsx --sheet Sales --row 2 --column email
```

`--pretty` renders an aligned terminal table; the default `--raw` output is pipe-friendly.

### Merge and transform

```sh
# Strict: identical column names and order are required.
tabmux merge a.csv b.csv -o merged.csv --mode strict

# Loose: union columns by name and fill missing cells with null.
tabmux merge a.csv c.csv -o merged.csv --mode loose --add-source-column origin --dedup

tabmux select sales.csv --columns name,amount \
  | tabmux filter - --where 'amount > 100' \
  | tabmux sort - --by amount:desc \
  | tabmux rename - --map amount=total
```

### Replay a pipeline

```toml
# pipeline.toml
version = 1

[options]
force = true

[[inputs]]
id = "sales"
path = "book.xlsx"
sheet = "Sales"

[[steps]]
op = "select"
input = "sales"
columns = ["id", "name", "region", "amount"]

[[steps]]
op = "filter"
input = "selected"
expr = "amount > 100"

[[outputs]]
id = "csv"
input = "filtered"
path = "out.csv"
format = "csv"
```

```sh
tabmux run pipeline.toml --dry-run --save-plan plan.json
tabmux run pipeline.toml
```

The same manifest and inputs produce identical data output checksums on every run.

### Batch directories and ZIP archives

```sh
tabmux convert fixtures/batch/ out/ --recursive \
  --include '*.csv,*.xlsx,*.html' --name-template '{stem}.csv'

tabmux convert archive.zip out/ --recursive
```

Relative directories are preserved unless `--flatten` is used. Unsupported members produce
`format.unsupported` events instead of disappearing silently. Path traversal, entry-count,
uncompressed-size, and compression-ratio limits are enforced.

### Export to SQL and query

```sh
cargo build --release --features sql-duckdb

tabmux to duckdb book.xlsx --sheet Sales --table sales -o warehouse.duckdb
tabmux query warehouse.duckdb "select region, sum(amount) as total \
  from sales group by region order by total desc" -f csv
```

> **Backend note.** In this build the `sql-duckdb` feature uses bundled SQLite with the
> same CLI, transaction, verification, and read-only-query contract. A file with DuckDB's
> `DUCK` magic is rejected rather than misread. See
> [`docs/implementation-notes.md`](docs/implementation-notes.md).

## Safety model

- **No silent overwrite.** Existing outputs require `--force`. `--backup` keeps a
  timestamped `.bak` copy.
- **Atomic writes.** All table outputs are written to a temporary file in the destination
  directory, flushed, fsynced, and renamed. A failed conversion leaves the old file intact.
- **Dry-run first.** `--dry-run` validates inputs, schemas, and output names without
  creating data files.
- **Multi-output staging.** Split-sheet and batch writes stage every output, then commit.
- **Loss reporting.** Every conversion builds a loss report, even when it is empty.
- **Strict mode.** `--strict` fails with exit code 5 before writing if errors exist or
  warnings exceed `--max-warnings N` (default 0). `--lossy` explicitly allows warnings.
- **Provenance.** `--manifest PATH` records tool version/commit, argv, options, RFC 3339
  start time, duration, input/output byte counts and SHA-256 checksums, and the loss
  summary.
- **Verification.** `--verify` re-reads each output and compares columns, sheet names, row
  counts, and rendered values.
- **Formula-injection warning.** CSV/TSV exports warn when text begins with a spreadsheet
  formula prefix. Values are never rewritten implicitly; sanitize the source or use
  `--strict` to block the export.

Example:

```sh
tabmux convert rich.xlsx out.xlsx --all-sheets \
  --report loss.json --report-markdown loss.md \
  --manifest manifest.json --verify
```

See [`docs/loss-report.md`](docs/loss-report.md) for the schema, event codes, and strict
policy.

## Format support

| Format | Read | Write | Multiple tables | Notes |
|---|---|---|---|---|
| CSV | yes | yes | no | RFC 4180, delimiter sniffing, BOM/UTF-16/Latin-1, ragged-row policy |
| TSV | yes | yes | no | Tab-separated dialect |
| XLSX / XLS / XLSB | yes | XLSX | yes | Basic fidelity; cached formulas, merged-cell expansion, loss events |
| ODS | yes | yes | yes | Basic fidelity; native values, bold headers, column widths |
| HTML | yes | no | yes | `<table>` extraction, captions, colspan/rowspan, nested tables |
| JSON / JSONL | yes | no | no | Arrays of objects, nested objects flattened to `a.b` |

Writing JSON/YAML/TOML tables, PDF/DOCX ingestion, DBF, Avro/ORC, 7z write, and exotic
compression are out of scope for v1.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success |
| `1` | User or data error |
| `2` | CLI usage error |
| `3` | I/O error |
| `4` | Unsupported format or feature |
| `5` | Lossy conversion blocked by strict mode |
| `6` | Partial success |
| `130` | Interrupted |

## Development

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo test --all-targets --all-features
./scripts/smoke.sh
TABMUX_FEATURES=sql-duckdb ./scripts/smoke.sh
```

Larger performance checks:

```sh
./scripts/perf-smoke.sh
```

Nightly, non-blocking fuzz targets live in `fuzz/` and are documented in
[`fuzz/README.md`](fuzz/README.md).

### Project layout

```
src/cli/         command-line definitions, dispatch, batch and pipeline glue
src/model/       Table, TableSet, Column, CellValue, SourceRef
src/formats/     CSV, JSON, XLSX, ODS, HTML, archives, SQL backend
src/ops/         select, filter, sort, rename, dedupe, merge, schema diff
src/pipeline/    manifest parsing and deterministic execution
src/report/      loss report, provenance manifest, verification
src/io/          safe atomic writes and encoding detection
tests/           CLI, pipeline, report, batch, hardening, and property tests
fixtures/        checked-in deterministic inputs
scripts/         smoke, performance, fixture generators, release tooling
```

## Documentation

| Document | Contents |
|---|---|
| [`docs/devplan.md`](docs/devplan.md) | Authoritative v1 phase plan and scope |
| [`docs/features.md`](docs/features.md) | Feature inventory, CLI model, and non-goals |
| [`docs/implementation-notes.md`](docs/implementation-notes.md) | Deliberate deviations and shipped behavior |
| [`docs/loss-report.md`](docs/loss-report.md) | Loss-report schema, event codes, strict policy |
| [`docs/performance.md`](docs/performance.md) | Performance smoke budgets |
| [`docs/release.md`](docs/release.md) | Independent release checklist and instructions |
| [`docs/dependencies.md`](docs/dependencies.md) | Dependency and feature plan |
| [`fixtures/README.md`](fixtures/README.md) | Fixture catalog and regeneration scripts |

## Known limits

- v1 materializes tables in memory. Very large inputs (well beyond available RAM) are out
  of scope; external-memory spill is deferred.
- Workbook output is basic fidelity: bold headers, frozen top row, autofilter, sensible
  column widths, and native types. Charts, pivot definitions, macros, comments,
  conditional formatting, data validation, and style cloning are not preserved. They are
  reported.
- Formulas are never recalculated. Cached values are exported and the report explains the
  approximation.
- HTML parsing is tolerant but heuristic. Skipped layout tables and inferred headers are
  reported; there is no JavaScript rendering or CSS layout reconstruction.
- The `sql-duckdb` feature in this build is backed by bundled SQLite. Replacing the backend
  with native DuckDB later does not change the command contract.
- **LibreOffice is not bundled** and is not a build or runtime dependency. It is only an
  optional external viewer for generated XLSX/ODS files; `tabmux` validates workbooks by
  re-reading them with calamine.
- No network access is performed by any command.

## License

Licensed under either of:

- Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE))
- MIT license ([`LICENSE-MIT`](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in this project by you, as defined in the Apache-2.0 license, shall be dual
licensed as above, without any additional terms or conditions.
