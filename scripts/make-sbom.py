#!/usr/bin/env python3
"""Generate a CycloneDX 1.5 SBOM from Cargo.lock.

Usage:
    python3 scripts/make-sbom.py [--lock Cargo.lock] [--output SBOM.cdx.json]

The SBOM is intentionally dependency-focused: it records every locked crate,
its version, Cargo checksum, and package URL. It does not claim to reproduce
license texts; those remain in each crate's package metadata and in Cargo.lock.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import pathlib
import re
import sys
import uuid

ROOT = pathlib.Path(__file__).resolve().parent.parent


def parse_lock(path: pathlib.Path) -> list[dict[str, str]]:
    text = path.read_text(encoding="utf-8")
    packages: list[dict[str, str]] = []
    for block in text.split("[[package]]")[1:]:
        name = re.search(r'^name\s*=\s*"([^"]+)"', block, re.M)
        version = re.search(r'^version\s*=\s*"([^"]+)"', block, re.M)
        checksum = re.search(r'^checksum\s*=\s*"([^"]+)"', block, re.M)
        source = re.search(r'^source\s*=\s*"([^"]+)"', block, re.M)
        if not name or not version:
            continue
        packages.append(
            {
                "name": name.group(1),
                "version": version.group(1),
                "checksum": checksum.group(1) if checksum else "",
                "source": source.group(1) if source else "",
            }
        )
    return packages


def package_version(path: pathlib.Path) -> tuple[str, str]:
    text = path.read_text(encoding="utf-8")
    name = re.search(r'^name\s*=\s*"([^"]+)"', text, re.M)
    version = re.search(r'^version\s*=\s*"([^"]+)"', text, re.M)
    if not name or not version:
        raise SystemExit("could not read package name/version from Cargo.toml")
    return name.group(1), version.group(1)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lock", default=str(ROOT / "Cargo.lock"))
    parser.add_argument("--manifest", default=str(ROOT / "Cargo.toml"))
    parser.add_argument("--output", default=str(ROOT / "SBOM.cdx.json"))
    args = parser.parse_args()

    lock = pathlib.Path(args.lock)
    manifest = pathlib.Path(args.manifest)
    output = pathlib.Path(args.output)

    name, version = package_version(manifest)
    packages = [p for p in parse_lock(lock) if p["name"] != name]

    components = []
    for package in sorted(packages, key=lambda item: (item["name"], item["version"])):
        purl = f"pkg:cargo/{package['name']}@{package['version']}"
        component: dict[str, object] = {
            "type": "library",
            "name": package["name"],
            "version": package["version"],
            "purl": purl,
        }
        if package["checksum"]:
            component["hashes"] = [
                {"alg": "SHA-256", "content": package["checksum"]}
            ]
        components.append(component)

    # Deterministic serial per package version and dependency set; timestamp is
    # the only intentionally run-dependent field.
    digest = hashlib.sha256(
        json.dumps(components, sort_keys=True).encode("utf-8")
    ).hexdigest()
    serial = uuid.uuid5(uuid.NAMESPACE_URL, f"{name}@{version}:{digest}")

    bom = {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "serialNumber": f"urn:uuid:{serial}",
        "version": 1,
        "metadata": {
            "timestamp": dt.datetime.now(dt.timezone.utc)
            .replace(microsecond=0)
            .isoformat()
            .replace("+00:00", "Z"),
            "component": {
                "type": "application",
                "name": name,
                "version": version,
                "licenses": [
                    {"license": {"id": "MIT"}},
                    {"license": {"id": "Apache-2.0"}},
                ],
            },
        },
        "components": components,
    }

    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(bom, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {output} ({len(components)} components)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
