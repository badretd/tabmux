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

section "Phase 4: XLSX/ODS write, multi-sheet assembly, basic formatting"

"$BIN" convert fixtures/a.csv fixtures/b.csv -o "$TMP_DIR/report.xlsx"
report_sheets="$("$BIN" sheets "$TMP_DIR/report.xlsx")"
[[ "$report_sheets" == $'a\nb' ]] \
  || fail "multi-input XLSX sheets were '$report_sheets'"
"$BIN" convert "$TMP_DIR/report.xlsx" - --sheet a >"$TMP_DIR/report-a.csv"
grep -q '^id,name,amount,active,joined$' "$TMP_DIR/report-a.csv" \
  || fail "round-tripped XLSX sheet has the wrong header"
grep -q '^1,Alice,120.5,true,2026-01-15$' "$TMP_DIR/report-a.csv" \
  || fail "round-tripped XLSX values or types changed"
echo "multi-input XLSX: ok"

"$BIN" convert fixtures/a.csv fixtures/b.csv -o "$TMP_DIR/report.ods"
ods_report_sheets="$("$BIN" sheets "$TMP_DIR/report.ods")"
[[ "$ods_report_sheets" == $'a\nb' ]] \
  || fail "multi-input ODS sheets were '$ods_report_sheets'"
"$BIN" convert "$TMP_DIR/report.ods" - --sheet b >"$TMP_DIR/report-b.csv"
grep -q '^4,Dave,42.25,false,2026-04-02$' "$TMP_DIR/report-b.csv" \
  || fail "round-tripped ODS values changed"
echo "multi-input ODS: ok"

"$BIN" convert fixtures/typed.csv -o "$TMP_DIR/typed.xlsx" --sheet-name Types
"$BIN" convert "$TMP_DIR/typed.xlsx" - --sheet Types >"$TMP_DIR/typed.csv"
grep -q '^1,1.5,true,2026-01-15,2026-01-15T10:30:00,10:30:00,007$' "$TMP_DIR/typed.csv" \
  || fail "native types were not preserved in XLSX"
echo "native workbook types: ok"

"$BIN" convert fixtures/sales.ods -o "$TMP_DIR/sales-roundtrip.xlsx" --all-sheets \
  >/dev/null 2>&1
roundtrip_sheets="$("$BIN" sheets "$TMP_DIR/sales-roundtrip.xlsx")"
[[ "$roundtrip_sheets" == $'Sales\nRegions' ]] \
  || fail "ODS -> XLSX all-sheets produced '$roundtrip_sheets'"
grep -q 'Total,Total,,400.5,,' <("$BIN" convert "$TMP_DIR/sales-roundtrip.xlsx" - --sheet Sales) \
  || fail "ODS -> XLSX round-trip lost merged or cached values"
echo "workbook -> workbook round-trip: ok"

"$BIN" convert fixtures/a.csv fixtures/b.csv -o "$TMP_DIR/templated.xlsx" \
  --sheet-name-template 'sheet-{index}'
templated_sheets="$("$BIN" sheets "$TMP_DIR/templated.xlsx")"
[[ "$templated_sheets" == $'sheet-1\nsheet-2' ]] \
  || fail "sheet-name-template produced '$templated_sheets'"

"$BIN" convert fixtures/a.csv fixtures/a.csv -o "$TMP_DIR/collision.xlsx" \
  2>"$TMP_DIR/collision.err"
collision_sheets="$("$BIN" sheets "$TMP_DIR/collision.xlsx")"
[[ "$collision_sheets" == $'a\na-2' ]] \
  || fail "sheet-name collisions produced '$collision_sheets'"
grep -q 'output.sheet_name_sanitized' "$TMP_DIR/collision.err" \
  || fail "sheet-name collision was not reported"
echo "sheet naming: ok"

"$BIN" convert fixtures/a.csv -o "$TMP_DIR/dry.xlsx" --dry-run \
  >/dev/null 2>"$TMP_DIR/dry-workbook.err"
[[ ! -e "$TMP_DIR/dry.xlsx" ]] || fail "--dry-run wrote a workbook"
grep -q 'dry-run' "$TMP_DIR/dry-workbook.err" \
  || fail "workbook --dry-run printed no plan"
echo "workbook dry-run: ok"

set +e
"$BIN" convert fixtures/a.csv fixtures/b.csv -o "$TMP_DIR/multi.csv" \
  >/dev/null 2>"$TMP_DIR/multi.err"
multi_status=$?
set -e
[[ "$multi_status" -eq 2 ]] \
  || fail "multi-input delimited output exited $multi_status, expected 2"
[[ ! -e "$TMP_DIR/multi.csv" ]] || fail "validation failure wrote an output"
echo "workbook output validation: ok"

section "Phase 5: merge/concat, schema diff, basic transforms"

"$BIN" merge fixtures/a.csv fixtures/b.csv -o "$TMP_DIR/merged.csv" --mode strict
[[ "$(wc -l <"$TMP_DIR/merged.csv" | tr -d ' ')" == "6" ]] \
  || fail "strict merge did not produce five data rows"
[[ "$(head -n 1 "$TMP_DIR/merged.csv")" == "id,name,amount,active,joined" ]] \
  || fail "strict merge produced the wrong header"
grep -q '^4,Dave,' "$TMP_DIR/merged.csv" \
  || fail "strict merge lost the second input"
echo "strict merge: ok"

"$BIN" concat fixtures/a.csv fixtures/b.csv -o "$TMP_DIR/concat.csv" --mode loose
[[ "$(wc -l <"$TMP_DIR/concat.csv" | tr -d ' ')" == "6" ]] \
  || fail "concat alias did not merge both inputs"
echo "concat alias: ok"

set +e
"$BIN" merge fixtures/a.csv fixtures/c.csv -o "$TMP_DIR/schema-bad.csv" \
  --mode strict >/dev/null 2>"$TMP_DIR/schema.err"
schema_status=$?
set -e
[[ "$schema_status" -eq 1 ]] \
  || fail "strict schema mismatch exited $schema_status, expected 1"
grep -q "missing column 'joined'" "$TMP_DIR/schema.err" \
  || fail "schema diff did not report the missing column"
grep -q 'column order differs' "$TMP_DIR/schema.err" \
  || fail "schema diff did not report the reordered columns"
[[ ! -e "$TMP_DIR/schema-bad.csv" ]] \
  || fail "strict schema mismatch wrote an output file"

set +e
"$BIN" merge fixtures/a.csv fixtures/c.csv -o "$TMP_DIR/schema-json.csv" \
  --mode strict --json >"$TMP_DIR/schema.json" 2>/dev/null
json_status=$?
set -e
[[ "$json_status" -eq 1 ]] \
  || fail "JSON schema mismatch exited $json_status, expected 1"
grep -q '"missing"' "$TMP_DIR/schema.json" \
  || fail "--json did not produce a machine-readable schema diff"
grep -q '"joined"' "$TMP_DIR/schema.json" \
  || fail "--json schema diff omitted the missing column"

set +e
"$BIN" merge fixtures/a.csv fixtures/d.csv -o "$TMP_DIR/type-bad.csv" \
  --mode strict >/dev/null 2>"$TMP_DIR/type.err"
type_status=$?
set -e
[[ "$type_status" -eq 1 ]] \
  || fail "strict type conflict exited $type_status, expected 1"
grep -q "type conflict on 'amount': float vs text" "$TMP_DIR/type.err" \
  || fail "schema diff did not report the type conflict"
echo "schema diff: ok"

"$BIN" merge fixtures/a.csv fixtures/c.csv -o "$TMP_DIR/loose.csv" \
  --mode loose --add-source-column origin --dedup
[[ "$(head -n 1 "$TMP_DIR/loose.csv")" == "id,name,amount,active,joined,origin" ]] \
  || fail "loose merge did not union the columns"
[[ "$(wc -l <"$TMP_DIR/loose.csv" | tr -d ' ')" == "5" ]] \
  || fail "loose merge did not keep the expected rows"
grep -q ',a.csv$' "$TMP_DIR/loose.csv" \
  || fail "loose merge is missing a.csv provenance"
grep -q ',c.csv$' "$TMP_DIR/loose.csv" \
  || fail "loose merge is missing c.csv provenance"

"$BIN" merge fixtures/a.csv fixtures/a.csv -o "$TMP_DIR/deduped.csv" \
  --mode loose --add-source-column origin --dedup
[[ "$(wc -l <"$TMP_DIR/deduped.csv" | tr -d ' ')" == "4" ]] \
  || fail "merge --dedup did not collapse identical rows"
echo "loose merge and dedup: ok"

"$BIN" select fixtures/a.csv --columns name,amount \
  | "$BIN" filter - --where 'amount > 100' >"$TMP_DIR/filtered.csv"
[[ "$(wc -l <"$TMP_DIR/filtered.csv" | tr -d ' ')" == "3" ]] \
  || fail "filter numeric comparison kept the wrong number of rows"
grep -q '^Alice,120.5$' "$TMP_DIR/filtered.csv" \
  || fail "filter dropped Alice"
grep -q '^Carol,200$' "$TMP_DIR/filtered.csv" \
  || fail "filter dropped Carol"
echo "filter pipeline: ok"

"$BIN" sort fixtures/a.csv --by amount:desc >"$TMP_DIR/sorted.csv"
[[ "$(sed -n '2p' "$TMP_DIR/sorted.csv" | cut -d, -f2)" == "Carol" ]] \
  || fail "sort --by amount:desc did not put Carol first"
[[ "$(sed -n '4p' "$TMP_DIR/sorted.csv" | cut -d, -f2)" == "Bob" ]] \
  || fail "sort --by amount:desc did not put Bob last"
echo "sort: ok"

"$BIN" rename fixtures/a.csv --map amount=total,active=enabled >"$TMP_DIR/renamed.csv"
[[ "$(head -n 1 "$TMP_DIR/renamed.csv")" == "id,name,total,enabled,joined" ]] \
  || fail "rename did not rewrite the selected headers"
echo "rename: ok"

printf 'id,name\n1,Alice\n2,Bob\n1,Alice\n' >"$TMP_DIR/duplicates.csv"
"$BIN" dedupe "$TMP_DIR/duplicates.csv" >"$TMP_DIR/deduped-rows.csv"
[[ "$(wc -l <"$TMP_DIR/deduped-rows.csv" | tr -d ' ')" == "3" ]] \
  || fail "dedupe did not keep exactly the first occurrences"
echo "dedupe: ok"

section "Phase 6: HTML table ingestion, HTML -> workbook"

"$BIN" convert fixtures/tables.html "$TMP_DIR/tables.xlsx" \
  --tables all --sheet-per-table --use-captions --index-sheet \
  2>"$TMP_DIR/html.err"
html_sheets="$("$BIN" sheets "$TMP_DIR/tables.xlsx")"
[[ "$html_sheets" == $'_index\nSales\nRegions\nNotes' ]] \
  || fail "HTML workbook sheets were '$html_sheets'"
grep -q 'html.merged_cell_expanded' "$TMP_DIR/html.err" \
  || fail "HTML colspan/rowspan was not reported"
grep -q 'html.header_colspan_expanded' "$TMP_DIR/html.err" \
  || fail "HTML header colspan expansion was not reported"
if grep -q 'html.duplicate_header' "$TMP_DIR/html.err"; then
  fail "HTML colspan expansion was misreported as a duplicate header"
fi
grep -q 'html.table_skipped' "$TMP_DIR/html.err" \
  || fail "the HTML layout table was not reported as skipped"
grep -q 'html.nested_table_extracted' "$TMP_DIR/html.err" \
  || fail "the nested HTML table was not reported"
grep -q 'wrote 4 sheet(s)' "$TMP_DIR/html.err" \
  || fail "the workbook summary was not printed"
grep -q 'skipped table-4 (1 row(s) < --min-rows 2)' "$TMP_DIR/html.err" \
  || fail "the workbook summary did not describe the skipped table"

"$BIN" convert "$TMP_DIR/tables.xlsx" - --sheet _index >"$TMP_DIR/html-index.csv"
[[ "$(head -n 1 "$TMP_DIR/html-index.csv")" \
  == "source,table,caption,rows,columns,status,reason" ]] \
  || fail "the HTML index sheet has the wrong header"
[[ "$(wc -l <"$TMP_DIR/html-index.csv" | tr -d ' ')" == "5" ]] \
  || fail "the HTML index sheet must contain included and skipped tables"
grep -q '^fixtures/tables.html,1,Sales,4,5,included,$' "$TMP_DIR/html-index.csv" \
  || fail "the HTML index sheet is missing Sales metadata"
grep -q '^fixtures/tables.html,3,Notes,1,2,included,nested table extracted from table-2' \
  "$TMP_DIR/html-index.csv" \
  || fail "the HTML index sheet is missing the nested-table note"
grep -q '^fixtures/tables.html,4,,1,2,skipped,1 row(s) < --min-rows 2$' \
  "$TMP_DIR/html-index.csv" \
  || fail "the HTML index sheet is missing the skipped layout table"

"$BIN" convert "$TMP_DIR/tables.xlsx" - --sheet Sales >"$TMP_DIR/html-sales.csv"
grep -q '^id,name,territory,territory_2,amount$' "$TMP_DIR/html-sales.csv" \
  || fail "HTML Sales has the wrong header"
grep -q '^2,Bob,North,US,80.5$' "$TMP_DIR/html-sales.csv" \
  || fail "HTML rowspan expansion is missing"
grep -q '^,Total,,,400.5$' "$TMP_DIR/html-sales.csv" \
  || fail "HTML tfoot rows were lost"
echo "HTML -> workbook: ok"

"$BIN" convert fixtures/tables.html "$TMP_DIR/html-stacked.csv" \
  --tables all --stack --add-source-column table >/dev/null 2>&1
[[ "$(wc -l <"$TMP_DIR/html-stacked.csv" | tr -d ' ')" == "7" ]] \
  || fail "HTML stack did not contain all data rows"
grep -q ',table-1$' "$TMP_DIR/html-stacked.csv" \
  || fail "HTML stack is missing table provenance"
grep -q ',table-3$' "$TMP_DIR/html-stacked.csv" \
  || fail "the nested HTML table was not stacked"
if grep -q 'Logo' "$TMP_DIR/html-stacked.csv"; then
  fail "the HTML layout table leaked into the output"
fi
echo "HTML stack: ok"

"$BIN" convert fixtures/header-row.html - --header-row 2 >"$TMP_DIR/html-header.csv"
[[ "$(cat "$TMP_DIR/html-header.csv")" == $'id,amount\n1,10\n2,20' ]] \
  || fail "--header-row 2 produced the wrong table"

set +e
"$BIN" convert fixtures/tables.html - --tables 1 --merge-policy error \
  >/dev/null 2>"$TMP_DIR/html-merge.err"
html_merge_status=$?
set -e
[[ "$html_merge_status" -eq 1 ]] \
  || fail "--merge-policy error exited $html_merge_status, expected 1"
grep -q -- '--merge-policy' "$TMP_DIR/html-merge.err" \
  || fail "--merge-policy error did not explain the policy choice"
echo "HTML header/merge policy: ok"

printf '\nsmoke: all checks passed\n'
