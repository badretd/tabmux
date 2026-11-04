#!/usr/bin/env bash
#
# Phase 11 performance smoke test.
#
# This is deliberately separate from scripts/smoke.sh: it generates ~100 MB of
# input and a 100k x 20 workbook, so it takes minutes rather than seconds.
#
# Usage:
#   ./scripts/perf-smoke.sh                    # build release and measure
#   ./scripts/perf-smoke.sh target/release/tabmux
#
# Budgets (configurable with the environment variables below):
#   CSV:  100 MB select/filter in under 60 s and 2048 MiB peak RSS
#   XLSX: 100k x 20 write in under 120 s and read in under 60 s, 4096 MiB RSS
#
# The budgets are intentionally generous: v1 materializes tables in memory, and
# the goal is to catch pathological regressions, not to compete with streaming
# engines. Documented in docs/performance.md.

set -euo pipefail

cd "$(dirname "$0")/.."

BIN="${1:-}"
if [[ -z "$BIN" ]]; then
  cargo build --quiet --release
  BIN="target/release/tabmux"
fi
[[ -x "$BIN" ]] || {
  echo "perf-smoke: error: binary is not executable: $BIN" >&2
  exit 1
}

CSV_BYTES="${TABMUX_PERF_CSV_BYTES:-104857600}"   # 100 MiB
CSV_SECONDS="${TABMUX_PERF_CSV_SECONDS:-60}"
CSV_RSS_MB="${TABMUX_PERF_CSV_RSS_MB:-2048}"
XLSX_ROWS="${TABMUX_PERF_XLSX_ROWS:-100000}"
XLSX_COLS="${TABMUX_PERF_XLSX_COLS:-20}"
XLSX_WRITE_SECONDS="${TABMUX_PERF_XLSX_WRITE_SECONDS:-120}"
XLSX_READ_SECONDS="${TABMUX_PERF_XLSX_READ_SECONDS:-60}"
XLSX_RSS_MB="${TABMUX_PERF_XLSX_RSS_MB:-4096}"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

TIME_BIN="$(command -v /usr/bin/time || true)"
if [[ -z "$TIME_BIN" ]]; then
  echo "perf-smoke: error: /usr/bin/time is required" >&2
  exit 1
fi

# Run a command and print "SECONDS RSS_MB" on stdout. Command output is
# discarded; diagnostics are kept for failures.
measure() {
  local label="$1"
  shift
  local out="$TMP_DIR/time.txt"
  "$TIME_BIN" -f '%e %M' -o "$out" "$@" >/dev/null 2>"$TMP_DIR/$label.err" || {
    cat "$TMP_DIR/$label.err" >&2
    echo "perf-smoke: FAIL: $label failed" >&2
    exit 1
  }
  local seconds rss_kb
  read -r seconds rss_kb <"$out"
  awk -v s="$seconds" -v k="$rss_kb" 'BEGIN { printf "%.2f %.1f\n", s, k / 1024 }'
}

assert_budget() {
  local label="$1" seconds="$2" rss_mb="$3" max_seconds="$4" max_rss_mb="$5"
  awk -v s="$seconds" -v m="$max_seconds" 'BEGIN { exit !(s <= m) }' || {
    echo "perf-smoke: FAIL: $label took ${seconds}s, budget ${max_seconds}s" >&2
    exit 1
  }
  awk -v r="$rss_mb" -v m="$max_rss_mb" 'BEGIN { exit !(r <= m) }' || {
    echo "perf-smoke: FAIL: $label used ${rss_mb} MiB RSS, budget ${max_rss_mb} MiB" >&2
    exit 1
  }
}

echo "perf-smoke: binary $BIN"
echo "perf-smoke: generating ${CSV_BYTES} byte CSV and ${XLSX_ROWS}x${XLSX_COLS} workbook"

python3 - "$TMP_DIR/large.csv" "$CSV_BYTES" "$TMP_DIR/large.xlsx.csv" "$XLSX_ROWS" "$XLSX_COLS" <<'PY'
import sys

csv_path, csv_bytes, xlsx_csv, rows, cols = sys.argv[1], int(sys.argv[2]), sys.argv[3], int(sys.argv[4]), int(sys.argv[5])

header = "id,name,region,amount,note\n"
chunk_rows = 10_000
lines = []
for index in range(1, chunk_rows + 1):
    lines.append(f"{index},user{index},region{index % 7},{index % 1000}.5,row {index} note\n")
chunk = "".join(lines)
with open(csv_path, "w", encoding="utf-8", newline="") as handle:
    handle.write(header)
    written = len(header)
    while written < csv_bytes:
        handle.write(chunk)
        written += len(chunk)

columns = [f"c{index}" for index in range(1, cols + 1)]
with open(xlsx_csv, "w", encoding="utf-8", newline="") as handle:
    handle.write(",".join(columns) + "\n")
    for row in range(1, rows + 1):
        handle.write(",".join(str((row * column) % 1000) for column in range(1, cols + 1)) + "\n")
PY

echo "perf-smoke: CSV size: $(wc -c <"$TMP_DIR/large.csv") bytes"
read -r secs rss < <(measure select "$BIN" select "$TMP_DIR/large.csv" --columns id,amount)
echo "perf-smoke: select: ${secs}s ${rss} MiB"
assert_budget "select" "$secs" "$rss" "$CSV_SECONDS" "$CSV_RSS_MB"

read -r secs rss < <(
  measure filter bash -c \
    "'$BIN' select '$TMP_DIR/large.csv' --columns id,amount | '$BIN' filter - --where 'amount > 100'"
)
echo "perf-smoke: select+filter: ${secs}s ${rss} MiB"
assert_budget "select+filter" "$secs" "$rss" "$CSV_SECONDS" "$CSV_RSS_MB"

read -r secs rss < <(
  measure xlsx-write "$BIN" convert "$TMP_DIR/large.xlsx.csv" "$TMP_DIR/large.xlsx" --force
)
echo "perf-smoke: XLSX write: ${secs}s ${rss} MiB"
assert_budget "XLSX write" "$secs" "$rss" "$XLSX_WRITE_SECONDS" "$XLSX_RSS_MB"

read -r secs rss < <(
  measure xlsx-read "$BIN" convert "$TMP_DIR/large.xlsx" "$TMP_DIR/large-back.csv" --force
)
echo "perf-smoke: XLSX read: ${secs}s ${rss} MiB"
assert_budget "XLSX read" "$secs" "$rss" "$XLSX_READ_SECONDS" "$XLSX_RSS_MB"

[[ "$("$BIN" count "$TMP_DIR/large-back.csv")" == "$XLSX_ROWS" ]] || {
  echo "perf-smoke: FAIL: round-tripped XLSX row count changed" >&2
  exit 1
}

echo "perf-smoke: all budgets passed"
