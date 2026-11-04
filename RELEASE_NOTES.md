# tabmux v0.1.0

The first release of `tabmux`: a safe, reproducible conversion engine for messy tabular
sources.

## Highlights

- **Ingest** CSV/TSV, XLSX/XLS/XLSB, ODS, HTML tables, JSON/JSONL, directories, and ZIP
  archives.
- **Select and shape** sheets, tables, rows, and columns; filter, sort, rename, dedupe,
  merge, and concatenate.
- **Write** clean CSV/TSV and multi-sheet XLSX/ODS workbooks with bold headers, frozen top
  rows, autofilter, native types, and sensible column widths.
- **Replay** conversions from a declarative `pipeline.toml` with dry-run and deterministic
  output.
- **Report loss** with a versioned JSON/Markdown loss report covering dropped charts,
  styles, macros, pivot definitions, comments, conditional formatting, data validation,
  merged-cell expansion, cached formulas, ragged rows, encoding replacement, and more.
- **Write safely** with atomic temp-file commits, no-clobber defaults, `--force`,
  `--backup`, `--dry-run`, `--strict`, checksums, and a provenance manifest.
- **Verify** outputs with `--verify` and inspect them with `sheets`, `count`, `head`,
  `select`, `row`, and `cell`.
- **Export and query** with the optional `sql-duckdb` feature (backed by bundled SQLite in
  this build).

## Scope

This release is the v1 conversion core. It is not an analytics platform, TUI, plugin host,
cloud service, or general SQL engine. See `docs/devplan.md` for the authoritative scope and
`docs/features.md` for the feature inventory.

## Known fidelity limits

- Workbook output is basic fidelity. Charts, pivot definitions, macros, comments,
  conditional formatting, data validation, and style cloning are not preserved; they are
  reported.
- Formulas are never recalculated. Cached values are exported and the report explains the
  approximation.
- HTML parsing is tolerant but heuristic. Layout tables may be skipped and headers may be
  inferred; both are reported.
- v1 materializes tables in memory. External-memory spill for very large inputs is
  deferred.
- The `sql-duckdb` feature in this build uses bundled SQLite behind the DuckDB command
  contract. Files with DuckDB's `DUCK` magic are rejected rather than misread.
- No command performs network access.

## Install

Download the archive for your platform, verify it with `SHA256SUMS`, extract it, and run:

```sh
./tabmux --version
./tabmux formats
```

Build from source with Rust 1.88 or newer:

```sh
cargo build --release --locked
```

## Verification

The release archive includes `SHA256SUMS` and a CycloneDX SBOM. The CI release workflow
runs formatting, clippy, tests, the release-binary smoke suite, and checksum generation
before publishing.

## License

MIT OR Apache-2.0. See `LICENSE-MIT`, `LICENSE-APACHE`, and `NOTICE`.
