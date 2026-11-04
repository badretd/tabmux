#!/usr/bin/env python3
"""Generate fixtures/rich.xlsx for Phase 9 loss-report tests.

The workbook deliberately contains every OOXML feature the reader reports as
lost or approximated: a chart, cell styles, a cached formula, a merged range,
conditional formatting, data validation, a comment, a pivot cache, and a VBA
project. It also has a plain pivot-like sheet so the fixture is still useful as
a conversion source.

Run it from the repository root:

    python3 scripts/make-rich-xlsx.py
"""

from __future__ import annotations

import datetime as dt
import pathlib
import zipfile
from xml.sax.saxutils import escape

ROOT = pathlib.Path(__file__).resolve().parent.parent
FIXTURE = ROOT / "fixtures" / "rich.xlsx"

EXCEL_EPOCH = dt.date(1899, 12, 30)
DATE_STYLE = 2
NUMBER_STYLE = 3
HEADER_STYLE = 1
STAMP = (1980, 1, 1, 0, 0, 0)


def add(archive: zipfile.ZipFile, name: str, data: str | bytes) -> None:
    info = zipfile.ZipInfo(name, date_time=STAMP)
    info.compress_type = zipfile.ZIP_DEFLATED
    info.external_attr = 0o644 << 16
    archive.writestr(info, data)


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


def sheet_xml(
    rows: list[str],
    merges: list[str] | None = None,
    extra: str = "",
) -> str:
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
        f"{extra}"
        '<pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/>'
        "</worksheet>"
    )


def build_sheets() -> list[str]:
    rich_rows = [
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
                formula_cell("D3", "SUM(D2:D2)", 120),
                date_cell("E3", dt.date(2026, 2, 1)),
                inline_cell("F3", "cached formula"),
            ],
        ),
        row(
            4,
            [
                inline_cell("A4", "Total", HEADER_STYLE),
                formula_cell("D4", "SUM(D2:D3)", 240),
            ],
        ),
    ]

    rich_extra = (
        '<conditionalFormatting sqref="D2:D4">'
        '<cfRule type="cellIs" dxfId="0" priority="1" operator="greaterThan">'
        "<formula>100</formula></cfRule></conditionalFormatting>"
        '<dataValidations count="1">'
        '<dataValidation type="whole" sqref="A2:A4">'
        "<formula1>1</formula1></dataValidation></dataValidations>"
    )

    pivot_rows = [
        row(
            1,
            [
                inline_cell("A1", "region", HEADER_STYLE),
                inline_cell("B1", "amount", HEADER_STYLE),
            ],
        ),
        row(2, [inline_cell("A2", "North"), number_cell("B2", 200)]),
        row(3, [inline_cell("A3", "South"), number_cell("B3", 80.5)]),
    ]

    return [
        sheet_xml(rich_rows, ["A4:B4"], rich_extra),
        sheet_xml(pivot_rows),
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
  <dxfs count="1"><dxf><font><b/></font></dxf></dxfs>
</styleSheet>"""

CHART = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
  <c:chart><c:plotArea><c:layout/></c:plotArea></c:chart>
</c:chartSpace>"""

COMMENTS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <authors><author>tabmux fixture</author></authors>
  <commentList>
    <comment ref="B2" authorId="0"><text><t>reviewed</t></text></comment>
  </commentList>
</comments>"""

PIVOT_CACHE = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<pivotCacheDefinition xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" recordCount="1">
  <cacheSource type="worksheet"><worksheetSource ref="A1:B3" sheet="Pivot"/></cacheSource>
</pivotCacheDefinition>"""

CONTENT_TYPES = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Default Extension="bin" ContentType="application/vnd.ms-office.vbaProject"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
  <Override PartName="/xl/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>
  <Override PartName="/xl/comments1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml"/>
  <Override PartName="/xl/pivotCache/pivotCacheDefinition1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml"/>
</Types>"""

ROOT_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"""

WORKBOOK = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Rich" sheetId="1" r:id="rId1"/>
    <sheet name="Pivot" sheetId="2" r:id="rId2"/>
  </sheets>
</workbook>"""

WORKBOOK_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
  <Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="charts/chart1.xml"/>
</Relationships>"""


def main() -> None:
    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    sheets = build_sheets()
    with zipfile.ZipFile(FIXTURE, "w") as archive:
        add(archive, "[Content_Types].xml", CONTENT_TYPES)
        add(archive, "_rels/.rels", ROOT_RELS)
        add(archive, "xl/workbook.xml", WORKBOOK)
        add(archive, "xl/_rels/workbook.xml.rels", WORKBOOK_RELS)
        add(archive, "xl/styles.xml", STYLES)
        add(archive, "xl/charts/chart1.xml", CHART)
        add(archive, "xl/comments1.xml", COMMENTS)
        add(archive, "xl/pivotCache/pivotCacheDefinition1.xml", PIVOT_CACHE)
        add(archive, "xl/vbaProject.bin", b"fake vba project")
        for index, sheet in enumerate(sheets, start=1):
            add(archive, f"xl/worksheets/sheet{index}.xml", sheet)
    print(f"wrote {FIXTURE.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
