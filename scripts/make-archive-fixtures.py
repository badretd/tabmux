#!/usr/bin/env python3
"""Regenerate the checked-in ZIP batch fixtures.

The archives are intentionally tiny and deterministic so they can live in
git. `archive.zip` mixes supported table formats with an unsupported text
file, `traversal.zip` contains a path-escape member, and `bomb.zip` contains a
highly compressible 2 MiB member used to exercise the archive limits.

Usage:
    python3 scripts/make-archive-fixtures.py
"""

from pathlib import Path
import zipfile

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "fixtures"
STAMP = (1980, 1, 1, 0, 0, 0)


def entry(name: str, data: bytes) -> zipfile.ZipInfo:
    info = zipfile.ZipInfo(name, date_time=STAMP)
    info.compress_type = zipfile.ZIP_DEFLATED
    info.external_attr = 0o644 << 16
    return info


def write_archive(path: Path, members: list[tuple[str, bytes]]) -> None:
    with zipfile.ZipFile(path, "w") as archive:
        for name, data in members:
            archive.writestr(entry(name, data), data)


def main() -> None:
    basic = (FIXTURES / "basic.csv").read_bytes()
    sales_xlsx = (FIXTURES / "sales.xlsx").read_bytes()
    tables_html = (FIXTURES / "tables.html").read_bytes()
    regions = (FIXTURES / "b.csv").read_bytes()

    write_archive(
        FIXTURES / "archive.zip",
        [
            ("sales.csv", basic),
            ("book.xlsx", sales_xlsx),
            ("page.html", tables_html),
            ("nested/regions.csv", regions),
            ("notes.txt", b"this file is intentionally unsupported\n"),
        ],
    )

    write_archive(
        FIXTURES / "traversal.zip",
        [
            ("safe.csv", basic),
            ("../escape.csv", b"id,name\n99,Escape\n"),
        ],
    )

    write_archive(
        FIXTURES / "bomb.zip",
        [
            ("bomb.csv", b"0" * (2 * 1024 * 1024) + b"\n"),
        ],
    )

    # Deliberately not a ZIP archive. The batch reader must fail with a clear
    # error instead of panicking or creating an output directory.
    (FIXTURES / "malformed.zip").write_bytes(
        b"PK\x03\x04this is not a valid zip central directory\n"
    )


if __name__ == "__main__":
    main()
