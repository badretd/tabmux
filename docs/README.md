# tabmux documentation

`tabmux` is planned as a single-binary, FFmpeg-style Swiss Army knife for tabular data: read messy sources, convert them into clean tables/workbooks with a reproducible pipeline, and report exactly what was lost.

The product is deliberately focused on the conversion core:

> **N1 reproducible pipeline + N2 messy-source ingestion + N3 workbook structure/value fidelity + N17 loss report + N21 safe-by-default execution.**

Analytics, TUI, plugins, cloud/federation, geo/graph/ML, PII, AI, dashboards, and mail merge are out of scope. See the scope reset in `features.md`.

## Documents

| Document | What it covers |
|---|---|
| [`devplan.md`](./devplan.md) | **Authoritative v1 plan:** 13 phases (0–12), each completable in one focused pass, with exact manual tests after every phase |
| [`features.md`](./features.md) | Feature inventory and scope: vision, CLI model, five starter scenarios, basic parity research, new-feature status, non-goals, and acceptance criteria |
| [`analogues.md`](./analogues.md) | Competitive research: the main CLI tools for tabular data, their functionality, strengths, limitations, and the gap `tabmux` fills |
| [`dependencies.md`](./dependencies.md) | Rust crate and external-tool plan: feature flags, dependency catalog, feature map, build/CI strategy, risks, and policy |
| [`implementation-notes.md`](./implementation-notes.md) | Deliberate deviations from the plan and behavior notes for shipped phases |

## The five starter scenarios

1. Convert a table to DuckDB for basic SQL queries.
2. Print a specific row or column directly to the terminal.
3. Convert ODS and XLSX files to CSV.
4. Merge two tables with identical columns.
5. Convert every table in an HTML document to XLSX, one sheet per table.

## Guiding idea

> **FFmpeg for tables:** demux any supported container, decode into one typed table model, apply a replayable conversion pipeline, and mux the result into a clean target format — with honest loss reporting and safe defaults.
