#!/usr/bin/env python3
"""Generate fixtures/twenty-tables.html for Phase 11 stress tests.

The document contains 20 data tables plus one layout table. Two data tables
share the caption ``Sales`` so caption-based sheet naming has to deduplicate
them, and every fifth table uses a ``colspan`` header. The final one-row table
must be skipped by the default ``--min-rows 2`` policy.

Run it from the repository root:

    python3 scripts/make-twenty-tables.py
"""

from __future__ import annotations

import pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
FIXTURE = ROOT / "fixtures" / "twenty-tables.html"


def table(index: int) -> str:
    caption = "Sales" if index in (5, 15) else f"Region/2026 {index}"
    if index % 5 == 0:
        header = (
            '<tr><th colspan="2">group</th></tr>'
            f'<tr><th>id</th><th>amount</th></tr>'
        )
    else:
        header = "<tr><th>id</th><th>amount</th></tr>"
    rows = (
        f"<tr><td>{index}</td><td>{index * 10}</td></tr>"
        f"<tr><td>{index + 100}</td><td>{index * 10 + 5}</td></tr>"
    )
    return f"<table><caption>{caption}</caption>{header}{rows}</table>"


LAYOUT = '<table><tr><td>logo</td><td>navigation</td></tr></table>'


def main() -> None:
    body = "\n".join(table(index) for index in range(1, 21))
    document = (
        "<!doctype html>\n<html><body>\n"
        "<h1>Twenty tables</h1>\n"
        f"{body}\n"
        f"{LAYOUT}\n"
        "</body></html>\n"
    )
    FIXTURE.write_text(document, encoding="utf-8")
    print(f"wrote {FIXTURE.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
