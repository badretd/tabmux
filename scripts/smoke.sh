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

section "Phase 2: XLSX read, sheet selection, CSV conversion"

sheet_list="$("$BIN" sheets fixtures/sales.xlsx)"
[[ "$sheet_list" == $'Sales\nRegions\nNotes' ]] \
  || fail "sheets output was '$sheet_list'"

"$BIN" convert fixtures/sales.xlsx "$TMP_DIR/sales.csv" --sheet Sales \
  --fidelity-report 2>"$TMP_DIR/xlsx.err"
grep -q '^id,name,region,amount,sold_on,note' "$TMP_DIR/sales.csv" \
  || fail "xlsx -> csv produced the wrong header"
grep -q 'Alice,North,120,2026-01-15' "$TMP_DIR/sales.csv" \
  || fail "xlsx dates were not exported as ISO-8601"
grep -q '400.5' "$TMP_DIR/sales.csv" \
  || fail "cached formula value was not exported"
grep -q 'excel.merged_cell_expanded' "$TMP_DIR/xlsx.err" \
  || fail "merged cells were not reported"
grep -q 'excel.formula_cached_value_only' "$TMP_DIR/xlsx.err" \
  || fail "formula cached values were not reported"
grep -q 'excel.chart_dropped' "$TMP_DIR/xlsx.err" \
  || fail "charts were not reported"
echo "xlsx -> csv: ok"

mkdir "$TMP_DIR/all-sheets"
"$BIN" convert fixtures/sales.xlsx "$TMP_DIR/all-sheets/" \
  --all-sheets --split-sheets >/dev/null 2>&1
[[ "$(find "$TMP_DIR/all-sheets" -type f | wc -l | tr -d ' ')" == "3" ]] \
  || fail "--all-sheets --split-sheets did not write three files"
grep -q '^region,manager' "$TMP_DIR/all-sheets/sales.Regions.csv" \
  || fail "the Regions sheet was not written"
echo "split sheets: ok"

set +e
"$BIN" convert fixtures/sales.xlsx "$TMP_DIR/missing.csv" \
  --sheet Missing >/dev/null 2>"$TMP_DIR/missing.err"
missing_status=$?
set -e
[[ "$missing_status" -eq 1 ]] \
  || fail "missing sheet exited $missing_status, expected 1"
grep -q 'Sales' "$TMP_DIR/missing.err" \
  || fail "missing-sheet error did not list available sheets"
[[ ! -e "$TMP_DIR/missing.csv" ]] \
  || fail "missing-sheet error created an output file"
echo "missing sheet: ok"

section "Phase 3: ODS read, split/stack sheets, row/cell output"

ods_sheets="$("$BIN" sheets fixtures/sales.ods)"
[[ "$ods_sheets" == $'Sales\nRegions' ]] \
  || fail "ODS sheets output was '$ods_sheets'"

"$BIN" convert fixtures/sales.ods "$TMP_DIR/ods-sales.csv" --sheet Sales \
  --fidelity-report 2>"$TMP_DIR/ods.err"
grep -q '^id,name,region,amount,sold_on,note' "$TMP_DIR/ods-sales.csv" \
  || fail "ODS -> CSV produced the wrong header"
grep -q 'Alice,North,120,2026-01-15' "$TMP_DIR/ods-sales.csv" \
  || fail "ODS dates were not exported as ISO-8601"
grep -q 'Total,Total,,400.5,,' "$TMP_DIR/ods-sales.csv" \
  || fail "ODS merged cells were not expanded"
grep -q 'excel.merged_cell_expanded' "$TMP_DIR/ods.err" \
  || fail "ODS merged cells were not reported"
grep -q 'excel.formula_cached_value_only' "$TMP_DIR/ods.err" \
  || fail "ODS cached formula values were not reported"
echo "ODS -> CSV: ok"

mkdir "$TMP_DIR/ods-split"
"$BIN" convert fixtures/sales.ods "$TMP_DIR/ods-split/" \
  --all-sheets --split-sheets \
  --name-template '{stem}.{index}.{sheet}.{ext}' >/dev/null 2>&1
[[ "$(find "$TMP_DIR/ods-split" -type f | wc -l | tr -d ' ')" == "2" ]] \
  || fail "ODS split did not write exactly two files"
[[ -f "$TMP_DIR/ods-split/sales.1.Sales.csv" ]] \
  || fail "the name template did not expand {index}/{sheet}/{ext}"
[[ -f "$TMP_DIR/ods-split/sales.2.Regions.csv" ]] \
  || fail "the Regions sheet was not written"
echo "ODS split with name template: ok"

"$BIN" convert fixtures/sales.ods "$TMP_DIR/stacked.csv" \
  --all-sheets --stack --add-source-column sheet >/dev/null 2>&1
grep -q '^id,name,region,amount,sold_on,note,manager,sheet$' "$TMP_DIR/stacked.csv" \
  || fail "stacked sheet output did not union the columns"
[[ "$(wc -l <"$TMP_DIR/stacked.csv" | tr -d ' ')" == "7" ]] \
  || fail "stacked sheet output did not contain all data rows"
grep -q ',Sales$' "$TMP_DIR/stacked.csv" \
  || fail "stacked output is missing the Sales provenance"
grep -q ',Regions$' "$TMP_DIR/stacked.csv" \
  || fail "stacked output is missing the Regions provenance"
echo "stack sheets: ok"

row_two="$("$BIN" row fixtures/sales.xlsx --sheet Sales --number 2 2>/dev/null)"
grep -q '^2,Bob,South,80.5,2026-02-01,' <<<"$row_two" \
  || fail "row --number 2 produced '$row_two'"

row_key="$("$BIN" row fixtures/sales.xlsx --sheet Sales --where 'id == 3' 2>/dev/null)"
grep -q '^3,Carol,North,200,2026-03-10,' <<<"$row_key" \
  || fail "row --where 'id == 3' produced '$row_key'"

cell_name="$("$BIN" cell fixtures/sales.xlsx --sheet Sales --row 2 --column name 2>/dev/null)"
[[ "$cell_name" == "Bob" ]] || fail "cell output was '$cell_name'"

cell_note="$("$BIN" cell fixtures/wide.csv --row 2 --column note)"
[[ "$cell_note" == 'contains, comma and "quotes"' ]] \
  || fail "multiline cell output was '$cell_note'"

"$BIN" select fixtures/wide.csv --columns note >"$TMP_DIR/wide-note.csv"
grep -q 'second line' "$TMP_DIR/wide-note.csv" \
  || fail "select lost a multiline value"

pretty="$("$BIN" head fixtures/basic.csv --rows 1 --pretty)"
grep -q '|' <<<"$pretty" || fail "--pretty did not print an aligned table"
grep -q -- '---+' <<<"$pretty" || fail "--pretty did not print a separator"
echo "row/cell and pretty output: ok"

section "Diagnostics policy"

row_default_err="$("$BIN" row fixtures/sales.xlsx --sheet Sales --number 2 2>&1 >/dev/null)"
[[ -z "$row_default_err" ]] \
  || fail "read-only row printed source-fidelity noise: $row_default_err"

row_fidelity_err="$("$BIN" row fixtures/sales.xlsx --sheet Sales --number 2 \
  --fidelity-report 2>&1 >/dev/null)"
grep -q 'excel.chart_dropped' <<<"$row_fidelity_err" \
  || fail "--fidelity-report did not restore workbook fidelity events"

ragged_default="$("$BIN" row fixtures/ragged.csv --ragged pad --where 'name == Bob' \
  2>&1 >/dev/null)"
grep -q 'row 2' <<<"$ragged_default" \
  || fail "the matched row's ragged-row diagnostic was not printed"
if grep -q 'row 3' <<<"$ragged_default"; then
  fail "a filtered-out row's diagnostic leaked into summary output"
fi

ragged_debug="$("$BIN" row fixtures/ragged.csv --ragged pad --where 'name == Bob' \
  --debug 2>&1 >/dev/null)"
grep -q 'row 3' <<<"$ragged_debug" \
  || fail "--debug did not include filtered-out row diagnostics"

quiet_err="$("$BIN" select fixtures/ragged.csv --ragged pad --columns name --quiet \
  2>&1 >/dev/null)"
[[ -z "$quiet_err" ]] || fail "--quiet still printed diagnostics: $quiet_err"
echo "summary/fidelity/debug/quiet: ok"

printf '\nsmoke: all checks passed\n'
