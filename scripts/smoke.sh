#!/usr/bin/env bash
#
# tabmux smoke test.
#
# Usage:
#   ./scripts/smoke.sh                 # build debug and test it
#   ./scripts/smoke.sh path/to/tabmux  # test an existing binary
#
# Every phase adds one section. The script must stay runnable with one command
# and must use only checked-in fixtures or throwaway temporary files.

set -euo pipefail

cd "$(dirname "$0")/.."

BIN="${1:-}"
if [[ -z "$BIN" ]]; then
  cargo build --quiet
  BIN="target/debug/tabmux"
fi

if [[ ! -x "$BIN" ]]; then
  echo "smoke: error: binary is not executable: $BIN" >&2
  exit 1
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

section() {
  printf '\n== %s ==\n' "$1"
}

fail() {
  echo "smoke: FAIL: $*" >&2
  exit 1
}

section "Phase 0: CLI skeleton and format matrix"

version="$("$BIN" --version)"
[[ "$version" =~ ^tabmux\ [0-9]+\.[0-9]+\.[0-9]+$ ]] \
  || fail "--version output was '$version'"
printf 'version: %s\n' "$version"

help="$("$BIN" --help)"
grep -q '^Usage: tabmux' <<<"$help" || fail "--help has no usage line"
grep -q 'formats' <<<"$help" || fail "--help does not list formats"
echo "help: ok"

matrix="$("$BIN" formats)"
expected_header="FORMAT   READ  WRITE  MULTI-TABLE  NOTES"
[[ "$(head -n 1 <<<"$matrix")" == "$expected_header" ]] \
  || fail "formats header was not '$expected_header'"

for format in csv tsv xlsx ods html json; do
  grep -q "^${format} " <<<"$matrix" || fail "formats is missing '${format}'"
done
echo "formats: ok"

set +e
"$BIN" definitely-not-a-command >/dev/null 2>&1
unknown_status=$?
set -e
[[ "$unknown_status" -eq 2 ]] \
  || fail "unknown command exited ${unknown_status}, expected 2"
echo "unknown command exit code: ok"

section "Phase 1: delimited conversion, selection, and safe writes"

"$BIN" convert fixtures/quoted.csv "$TMP_DIR/quoted.tsv"
"$BIN" convert "$TMP_DIR/quoted.tsv" "$TMP_DIR/quoted.back.csv"
"$BIN" convert fixtures/quoted.csv - >"$TMP_DIR/quoted.stdout.csv"
cmp "$TMP_DIR/quoted.back.csv" "$TMP_DIR/quoted.stdout.csv" \
  || fail "quoted CSV -> TSV -> CSV did not round-trip"
echo "quoted round-trip: ok"

header="$("$BIN" convert fixtures/semicolon.csv - | head -n 1)"
[[ "$header" == "name,email,amount" ]] \
  || fail "semicolon sniffing produced header '$header'"
echo "delimiter sniffing: ok"

"$BIN" convert fixtures/latin1.csv "$TMP_DIR/utf8.csv"
grep -q 'café' "$TMP_DIR/utf8.csv" || fail "latin1 conversion lost café"
grep -q 'Zürich' "$TMP_DIR/utf8.csv" || fail "latin1 conversion lost Zürich"
echo "latin1 -> utf8: ok"

"$BIN" select fixtures/basic.csv --columns name,email >"$TMP_DIR/selected.csv"
[[ "$(head -n 1 "$TMP_DIR/selected.csv")" == "name,email" ]] \
  || fail "select produced the wrong header"
[[ "$("$BIN" count fixtures/basic.csv)" == "3" ]] \
  || fail "count basic.csv was not 3"
[[ "$("$BIN" count fixtures/data.jsonl 2>/dev/null)" == "2" ]] \
  || fail "count data.jsonl was not 2"
head_rows="$("$BIN" head fixtures/basic.csv --rows 2 | wc -l | tr -d ' ')"
[[ "$head_rows" == "3" ]] || fail "head --rows 2 printed $head_rows lines"
echo "count/head/select: ok"

"$BIN" convert fixtures/data.jsonl "$TMP_DIR/data.csv" 2>"$TMP_DIR/json.err"
grep -q '^id,user.name,user.tags,amount,active' "$TMP_DIR/data.csv" \
  || fail "JSONL flattening produced the wrong header"
grep -q 'json.flattened' "$TMP_DIR/json.err" \
  || fail "JSONL conversion did not report flattened objects"
echo "JSONL flattening: ok"

printf 'sentinel\n' >"$TMP_DIR/existing.csv"
set +e
"$BIN" convert fixtures/basic.csv "$TMP_DIR/existing.csv" >/dev/null 2>&1
overwrite_status=$?
set -e
[[ "$overwrite_status" -eq 1 ]] \
  || fail "overwriting without --force exited $overwrite_status, expected 1"
[[ "$(cat "$TMP_DIR/existing.csv")" == "sentinel" ]] \
  || fail "existing output changed without --force"

"$BIN" convert fixtures/basic.csv "$TMP_DIR/existing.csv" --force
grep -q '^name,email,amount' "$TMP_DIR/existing.csv" \
  || fail "--force did not replace the existing output"

"$BIN" convert fixtures/basic.csv "$TMP_DIR/nothing.csv" --dry-run \
  >/dev/null 2>"$TMP_DIR/dry.err"
[[ ! -e "$TMP_DIR/nothing.csv" ]] || fail "--dry-run created an output file"
grep -q 'dry-run' "$TMP_DIR/dry.err" || fail "--dry-run printed no plan"

"$BIN" convert fixtures/basic.csv "$TMP_DIR/existing.csv" --force --backup
compgen -G "$TMP_DIR/existing.csv.*.bak" >/dev/null \
  || fail "--backup did not create a .bak file"
echo "safe writes: ok"

printf '\nsmoke: all checks passed\n'
