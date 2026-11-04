# Performance smoke budgets

> **Status:** Phase 11 quality gate. These are regression budgets, not
> marketing benchmarks. v1 materializes tables in memory, so the limits are
> deliberately generous.

Run the performance smoke test separately from the normal smoke suite because
it generates roughly 100 MB of input:

```sh
./scripts/perf-smoke.sh
# or test an existing release binary:
./scripts/perf-smoke.sh target/release/tabmux
```

The script builds the release binary when no path is supplied, generates the
inputs in a temporary directory, and removes them on exit. It uses
`/usr/bin/time -f '%e %M'`, so it currently targets Linux; macOS users can run
the same commands manually with `/usr/bin/time -l`.

## Budgets

| Workload | Default budget | Environment override |
|---|---|---|
| 100 MB CSV `select` | 60 s, 2048 MiB peak RSS | `TABMUX_PERF_CSV_SECONDS`, `TABMUX_PERF_CSV_RSS_MB` |
| 100 MB CSV `select` + `filter` pipeline | 60 s, 2048 MiB peak RSS | same as above |
| 100k x 20 XLSX write | 120 s, 4096 MiB peak RSS | `TABMUX_PERF_XLSX_WRITE_SECONDS`, `TABMUX_PERF_XLSX_RSS_MB` |
| 100k x 20 XLSX read | 60 s, 4096 MiB peak RSS | `TABMUX_PERF_XLSX_READ_SECONDS`, `TABMUX_PERF_XLSX_RSS_MB` |

Workload sizes can be reduced for a quick local check:

```sh
TABMUX_PERF_CSV_BYTES=1048576 \
TABMUX_PERF_XLSX_ROWS=1000 \
TABMUX_PERF_XLSX_COLS=10 \
./scripts/perf-smoke.sh target/debug/tabmux
```

## What is measured

- **CSV:** the full command, including encoding detection, delimiter sniffing,
  materialization, column selection, and serialization. The filter case is a
  two-process pipeline, so it includes pipe overhead.
- **XLSX write:** type inference, workbook assembly, formatting, ZIP
  compression, and the atomic commit.
- **XLSX read:** calamine extraction, type/date conversion, and CSV
  serialization.

The script also verifies that the XLSX round-trip preserves the row count.

## Known limits

- v1 has no external-memory spill. A file substantially larger than available
  RAM is out of scope (advanced N9 is deferred).
- The CSV capability advertises streaming as a target; the current
  implementation still materializes. The budgets reflect that.
- Performance numbers vary with disk, CPU, and compression settings. Treat a
  budget failure as a signal to investigate, not as a precise product promise.
