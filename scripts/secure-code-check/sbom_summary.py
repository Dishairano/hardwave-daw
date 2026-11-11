#!/usr/bin/env python3
"""Count the components in each CycloneDX file of a folder.

Usage: sbom_summary.py <sbom dir>

Prints one line per file and writes `summary={"file": count, ...}` to
$GITHUB_OUTPUT. A file without components fails: an empty SBOM would
read as "nothing ships", which is never true.
"""
import json
import os
import pathlib
import sys


def count_components(components) -> int:
    total = 0
    for component in components or []:
        total += 1 + count_components(component.get("components"))
    return total


def summarise(folder: pathlib.Path) -> dict:
    summary = {}
    for path in sorted(folder.glob("*.cdx.json")):
        bom = json.loads(path.read_text())
        summary[path.name] = count_components(bom.get("components"))
    return summary


def main() -> int:
    summary = summarise(pathlib.Path(sys.argv[1]))
    for name, count in summary.items():
        print(f"{name}: {count} components")
    out = os.environ.get("GITHUB_OUTPUT")
    if out:
        with open(out, "a") as handle:
            handle.write(f"summary={json.dumps(summary, separators=(',', ':'))}\n")
    if not summary or 0 in summary.values():
        print("::error::An SBOM is missing or has no components")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
