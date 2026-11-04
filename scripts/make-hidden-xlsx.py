#!/usr/bin/env python3
"""Generate fixtures/hidden.xlsx for the Phase 11 hardening corpus.

The workbook exercises sheet metadata and edge cases:
- a normal visible sheet,
- a hidden sheet and a very-hidden sheet,
- two sheet names that collide when compared case-insensitively,
- an entirely empty sheet.

Run it from the repository root:

    python3 scripts/make-hidden-xlsx.py
"""

from __future__ import annotations

import pathlib
import zipfile
from xml.sax.saxutils import escape

ROOT = pathlib.Path(__file__).resolve().parent.parent
FIXTURE = ROOT / "fixtures" / "hidden.xlsx"
STAMP = (1980, 1, 1, 0, 0, 0)


def add(archive: zipfile.ZipFile, name: str, data: str) -> None:
    info = zipfile.ZipInfo(name, date_time=STAMP)
    info.compress_type = zipfile.ZIP_DEFLATED
    info.external_attr = 0o644 << 16
    archive.writestr(info, data)


def inline_cell(ref: str, value: str) -> str:
    return f'<c r="{ref}" t="inlineStr"><is><t>{escape(value)}</t></is></c>'


def row(number: int, cells: list[str]) -> str:
    return f'<row r="{number}">{"".join(cells)}</row>'


def sheet_xml(rows: list[str]) -> str:
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
        f'<sheetData>{"".join(rows)}</sheetData>'
        '<pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/>'
        "</worksheet>"
    )


def data_sheet(column: str, values: list[str]) -> str:
    rows = [row(1, [inline_cell("A1", column)])]
    rows.extend(
        row(index, [inline_cell(f"A{index}", value)])
        for index, value in enumerate(values, start=2)
    )
    return sheet_xml(rows)


SHEETS = [
    ("Visible", "visible", data_sheet("name", ["Alice", "Bob"])),
    ("Hidden", "hidden", data_sheet("secret", ["hidden-value"])),
    ("VeryHidden", "veryHidden", data_sheet("secret", ["very-hidden-value"])),
    ("Data", "visible", data_sheet("value", ["first"])),
    ("DATA", "visible", data_sheet("value", ["second"])),
    ("Empty", "visible", sheet_xml([])),
]

CONTENT_TYPES = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
    '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
    '  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
    '  <Default Extension="xml" ContentType="application/xml"/>'
    '  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>'
    + "".join(
        f'  <Override PartName="/xl/worksheets/sheet{index}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
        for index in range(1, len(SHEETS) + 1)
    )
    + "</Types>"
)

ROOT_RELS = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
    '  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>'
    "</Relationships>"
)

WORKBOOK = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
    '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"'
    ' xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">'
    "<sheets>"
    + "".join(
        f'<sheet name="{escape(name)}" sheetId="{index}" state="{state}" r:id="rId{index}"/>'
        for index, (name, state, _) in enumerate(SHEETS, start=1)
    )
    + "</sheets></workbook>"
)

WORKBOOK_RELS = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
    + "".join(
        f'  <Relationship Id="rId{index}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{index}.xml"/>'
        for index in range(1, len(SHEETS) + 1)
    )
    + "</Relationships>"
)


def main() -> None:
    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(FIXTURE, "w") as archive:
        add(archive, "[Content_Types].xml", CONTENT_TYPES)
        add(archive, "_rels/.rels", ROOT_RELS)
        add(archive, "xl/workbook.xml", WORKBOOK)
        add(archive, "xl/_rels/workbook.xml.rels", WORKBOOK_RELS)
        for index, (_, _, sheet) in enumerate(SHEETS, start=1):
            add(archive, f"xl/worksheets/sheet{index}.xml", sheet)
    print(f"wrote {FIXTURE.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
