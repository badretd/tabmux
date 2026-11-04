#!/usr/bin/env python3
"""Generate fixtures/sales.xlsx with only the Python standard library.

The fixture exercises the Phase 2 workbook reader: three sheets, native dates,
a merged range, a cached formula, a styled header, and a chart part that must be
reported as dropped. Run it from the repository root:

    python3 scripts/make-sales-xlsx.py
"""

from __future__ import annotations

import datetime as dt
import pathlib
import zipfile
from xml.sax.saxutils import escape

ROOT = pathlib.Path(__file__).resolve().parent.parent
FIXTURE = ROOT / "fixtures" / "sales.xlsx"

EXCEL_EPOCH = dt.date(1899, 12, 30)
DATE_STYLE = 2
NUMBER_STYLE = 3
HEADER_STYLE = 1


def serial(value: dt.date) -> int:
    return (value - EXCEL_EPOCH).days


def inline_cell(ref: str, value: str, style: int = 0) -> str:
    style_attr = f' s="{style}"' if style else ""
    return (
        f'<c r="{ref}"{style_attr} t="inlineStr"><is><t>{escape(value)}</t></is></c>'
    )


def number_cell(ref: str, value: float | int, style: int = 0) -> str:
    style_attr = f' s="{style}"' if style else ""
    return f'<c r="{ref}"{style_attr}><v>{value}</v></c>'


def date_cell(ref: str, value: dt.date) -> str:
    return f'<c r="{ref}" s="{DATE_STYLE}"><v>{serial(value)}</v></c>'


def formula_cell(ref: str, formula: str, cached: float | int) -> str:
    return f'<c r="{ref}" s="{NUMBER_STYLE}"><f>{escape(formula)}</f><v>{cached}</v></c>'


def row(number: int, cells: list[str]) -> str:
    return f'<row r="{number}">{"".join(cells)}</row>'


def sheet_xml(rows: list[str], merges: list[str] | None = None) -> str:
    merge_xml = ""
    if merges:
        merge_xml = (
            f'<mergeCells count="{len(merges)}">'
            + "".join(f'<mergeCell ref="{ref}"/>' for ref in merges)
            + "</mergeCells>"
        )
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
        '<dimension ref="A1:F5"/>'
        f'<sheetData>{"".join(rows)}</sheetData>'
        f"{merge_xml}"
        '<pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/>'
        "</worksheet>"
    )


def build_sheets() -> list[str]:
    sales = [
        row(
            1,
            [
                inline_cell("A1", "id", HEADER_STYLE),
                inline_cell("B1", "name", HEADER_STYLE),
                inline_cell("C1", "region", HEADER_STYLE),
                inline_cell("D1", "amount", HEADER_STYLE),
                inline_cell("E1", "sold_on", HEADER_STYLE),
                inline_cell("F1", "note", HEADER_STYLE),
            ],
        ),
        row(
            2,
            [
                number_cell("A2", 1),
                inline_cell("B2", "Alice"),
                inline_cell("C2", "North"),
                number_cell("D2", 120, NUMBER_STYLE),
                date_cell("E2", dt.date(2026, 1, 15)),
                inline_cell("F2", "first order"),
            ],
        ),
        row(
            3,
            [
                number_cell("A3", 2),
                inline_cell("B3", "Bob"),
                inline_cell("C3", "South"),
                number_cell("D3", 80.5, NUMBER_STYLE),
                date_cell("E3", dt.date(2026, 2, 1)),
                inline_cell("F3", "repeat, with comma"),
            ],
        ),
        row(
            4,
            [
                number_cell("A4", 3),
                inline_cell("B4", "Carol"),
                inline_cell("C4", "North"),
                number_cell("D4", 200, NUMBER_STYLE),
                date_cell("E4", dt.date(2026, 3, 10)),
                inline_cell("F4", 'quoted "text"'),
            ],
        ),
        row(
            5,
            [
                inline_cell("A5", "Total", HEADER_STYLE),
                formula_cell("D5", "SUM(D2:D4)", 400.5),
            ],
        ),
    ]

    regions = [
        row(
            1,
            [
                inline_cell("A1", "region", HEADER_STYLE),
                inline_cell("B1", "manager", HEADER_STYLE),
            ],
        ),
        row(2, [inline_cell("A2", "North"), inline_cell("B2", "Ada")]),
        row(3, [inline_cell("A3", "South"), inline_cell("B3", "Grace")]),
    ]

    notes = [
        row(1, [inline_cell("A1", "note", HEADER_STYLE)]),
        row(2, [inline_cell("A2", "Phase 2 workbook fixture")]),
        row(3, [inline_cell("A3", "Styling is not preserved; values are exported.")]),
    ]

    return [
        sheet_xml(sales, ["A5:B5"]),
        sheet_xml(regions),
        sheet_xml(notes),
    ]


STYLES = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <numFmts count="1"><numFmt numFmtId="164" formatCode="yyyy-mm-dd"/></numFmts>
  <fonts count="2">
    <font><sz val="11"/><name val="Calibri"/><family val="2"/></font>
    <font><b/><sz val="11"/><name val="Calibri"/><family val="2"/></font>
  </fonts>
  <fills count="2">
    <fill><patternFill patternType="none"/></fill>
    <fill><patternFill patternType="gray125"/></fill>
  </fills>
  <borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
  <cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
  <cellXfs count="4">
    <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
    <xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/>
    <xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
    <xf numFmtId="4" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
  </cellXfs>
  <cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>
</styleSheet>"""

CHART = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
  <c:chart><c:plotArea><c:layout/></c:plotArea></c:chart>
</c:chartSpace>"""

CONTENT_TYPES = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet3.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
  <Override PartName="/xl/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>
</Types>"""

ROOT_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"""

WORKBOOK = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Sales" sheetId="1" r:id="rId1"/>
    <sheet name="Regions" sheetId="2" r:id="rId2"/>
    <sheet name="Notes" sheetId="3" r:id="rId3"/>
  </sheets>
</workbook>"""

WORKBOOK_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet3.xml"/>
  <Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
  <Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="charts/chart1.xml"/>
</Relationships>"""


def main() -> None:
    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    sheets = build_sheets()
    with zipfile.ZipFile(FIXTURE, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("[Content_Types].xml", CONTENT_TYPES)
        archive.writestr("_rels/.rels", ROOT_RELS)
        archive.writestr("xl/workbook.xml", WORKBOOK)
        archive.writestr("xl/_rels/workbook.xml.rels", WORKBOOK_RELS)
        archive.writestr("xl/styles.xml", STYLES)
        archive.writestr("xl/charts/chart1.xml", CHART)
        for index, sheet in enumerate(sheets, start=1):
            archive.writestr(f"xl/worksheets/sheet{index}.xml", sheet)
    print(f"wrote {FIXTURE.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
