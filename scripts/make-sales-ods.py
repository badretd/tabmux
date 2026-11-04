#!/usr/bin/env python3
"""Generate fixtures/sales.ods with only the Python standard library.

The fixture exercises the Phase 3 ODS reader: two sheets, native dates, a
merged range, a cached formula, and a styled header. Run it from the repository
root:

    python3 scripts/make-sales-ods.py
"""

from __future__ import annotations

import pathlib
import zipfile
from xml.sax.saxutils import escape

ROOT = pathlib.Path(__file__).resolve().parent.parent
FIXTURE = ROOT / "fixtures" / "sales.ods"

HEADER_STYLE = "ce1"


def string_cell(value: str, style: str = "") -> str:
    style_attr = f' table:style-name="{style}"' if style else ""
    return (
        f'<table:table-cell{style_attr} office:value-type="string">'
        f"<text:p>{escape(value)}</text:p></table:table-cell>"
    )


def number_cell(value: float | int, style: str = "") -> str:
    style_attr = f' table:style-name="{style}"' if style else ""
    return (
        f'<table:table-cell{style_attr} office:value-type="float" '
        f'office:value="{value}"><text:p>{value}</text:p></table:table-cell>'
    )


def date_cell(value: str) -> str:
    return (
        '<table:table-cell office:value-type="date" '
        f'office:date-value="{value}"><text:p>{value}</text:p></table:table-cell>'
    )


def formula_cell(formula: str, cached: float | int) -> str:
    return (
        f'<table:table-cell table:formula="{escape(formula)}" '
        f'office:value-type="float" office:value="{cached}">'
        f"<text:p>{cached}</text:p></table:table-cell>"
    )


def merged_cell(value: str, columns: int) -> str:
    covered = '<table:covered-table-cell/>' * (columns - 1)
    return (
        f'<table:table-cell table:number-columns-spanned="{columns}" '
        'table:number-rows-spanned="1" office:value-type="string">'
        f"<text:p>{escape(value)}</text:p></table:table-cell>{covered}"
    )


def row(cells: list[str]) -> str:
    return f"<table:table-row>{''.join(cells)}</table:table-row>"


def build_sales() -> str:
    rows = [
        row(
            [
                string_cell("id", HEADER_STYLE),
                string_cell("name", HEADER_STYLE),
                string_cell("region", HEADER_STYLE),
                string_cell("amount", HEADER_STYLE),
                string_cell("sold_on", HEADER_STYLE),
                string_cell("note", HEADER_STYLE),
            ]
        ),
        row(
            [
                number_cell(1),
                string_cell("Alice"),
                string_cell("North"),
                number_cell(120),
                date_cell("2026-01-15"),
                string_cell("first order"),
            ]
        ),
        row(
            [
                number_cell(2),
                string_cell("Bob"),
                string_cell("South"),
                number_cell(80.5),
                date_cell("2026-02-01"),
                string_cell("repeat, with comma"),
            ]
        ),
        row(
            [
                number_cell(3),
                string_cell("Carol"),
                string_cell("North"),
                number_cell(200),
                date_cell("2026-03-10"),
                string_cell('quoted "text"'),
            ]
        ),
        row(
            [
                merged_cell("Total", 2),
                "<table:table-cell/>",
                formula_cell("of:=SUM([.D2:.D4])", 400.5),
            ]
        ),
    ]
    return (
        '<table:table table:name="Sales" table:style-name="ta1">'
        '<table:table-column table:number-columns-repeated="6"/>'
        f"{''.join(rows)}</table:table>"
    )


def build_regions() -> str:
    rows = [
        row([string_cell("region", HEADER_STYLE), string_cell("manager", HEADER_STYLE)]),
        row([string_cell("North"), string_cell("Ada")]),
        row([string_cell("South"), string_cell("Grace")]),
    ]
    return (
        '<table:table table:name="Regions" table:style-name="ta1">'
        '<table:table-column table:number-columns-repeated="2"/>'
        f"{''.join(rows)}</table:table>"
    )


CONTENT = (
    '<?xml version="1.0" encoding="UTF-8"?>'
    '<office:document-content '
    'xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" '
    'xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" '
    'xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" '
    'xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" '
    'xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" '
    'office:version="1.2">'
    "<office:automatic-styles>"
    '<style:style style:name="ce1" style:family="table-cell">'
    '<style:text-properties fo:font-weight="bold"/>'
    "</style:style>"
    '<style:style style:name="ta1" style:family="table">'
    '<style:table-properties table:display="true"/>'
    "</style:style>"
    "</office:automatic-styles>"
    "<office:body><office:spreadsheet>"
    + build_sales()
    + build_regions()
    + "</office:spreadsheet></office:body></office:document-content>"
)

STYLES = (
    '<?xml version="1.0" encoding="UTF-8"?>'
    '<office:document-styles '
    'xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" '
    'xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" '
    'office:version="1.2">'
    "<office:styles>"
    '<style:style style:name="Default" style:family="table-cell"/>'
    "</office:styles>"
    "</office:document-styles>"
)

META = (
    '<?xml version="1.0" encoding="UTF-8"?>'
    '<office:document-meta '
    'xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" '
    'xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" '
    'office:version="1.2">'
    "<office:meta><meta:generator>tabmux fixture generator</meta:generator></office:meta>"
    "</office:document-meta>"
)

SETTINGS = (
    '<?xml version="1.0" encoding="UTF-8"?>'
    '<office:document-settings '
    'xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" '
    'office:version="1.2"><office:settings/></office:document-settings>'
)

MANIFEST = (
    '<?xml version="1.0" encoding="UTF-8"?>'
    '<manifest:manifest '
    'xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" '
    'manifest:version="1.2">'
    '<manifest:file-entry manifest:full-path="/" '
    'manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/>'
    '<manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>'
    '<manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/>'
    '<manifest:file-entry manifest:full-path="meta.xml" manifest:media-type="text/xml"/>'
    '<manifest:file-entry manifest:full-path="settings.xml" manifest:media-type="text/xml"/>'
    "</manifest:manifest>"
)


def main() -> None:
    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(FIXTURE, "w") as archive:
        archive.writestr(
            "mimetype",
            "application/vnd.oasis.opendocument.spreadsheet",
            compress_type=zipfile.ZIP_STORED,
        )
        archive.writestr("content.xml", CONTENT, compress_type=zipfile.ZIP_DEFLATED)
        archive.writestr("styles.xml", STYLES, compress_type=zipfile.ZIP_DEFLATED)
        archive.writestr("meta.xml", META, compress_type=zipfile.ZIP_DEFLATED)
        archive.writestr("settings.xml", SETTINGS, compress_type=zipfile.ZIP_DEFLATED)
        archive.writestr(
            "META-INF/manifest.xml", MANIFEST, compress_type=zipfile.ZIP_DEFLATED
        )
    print(f"wrote {FIXTURE.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
