#!/usr/bin/env python3
"""Fail the run on a high Semgrep finding; count the rest.

Usage: semgrep_gate.py <semgrep.json> <rules dir>

Semgrep marks a rule's severity ERROR, WARNING or INFO (newer rules may
say CRITICAL, HIGH, MEDIUM or LOW). ERROR, CRITICAL and HIGH are "high"
here and fail; the others are listed for review in the report. Writes
the counts to $GITHUB_OUTPUT for the report.
"""
import json
import os
import pathlib
import re
import sys

HIGH = {"ERROR", "CRITICAL", "HIGH"}
MEDIUM = {"WARNING", "MEDIUM"}


def level(severity: str) -> str:
    severity = (severity or "").upper()
    if severity in HIGH:
        return "high"
    if severity in MEDIUM:
        return "medium"
    return "low"


SOURCE = (".rs", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs")


def rule_id(finding: dict) -> str:
    """The rule's own id, without the folder the rules were copied into."""
    return re.sub(r"^rules\.", "", finding.get("check_id", ""))


def count_rules(rules_dir: pathlib.Path) -> int:
    total = 0
    for path in rules_dir.rglob("*"):
        if path.suffix in (".yaml", ".yml"):
            total += len(re.findall(r"^\s*-\s+id:", path.read_text(errors="replace"), re.M))
    return total


def main() -> int:
    results_path, rules_dir = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
    data = json.loads(results_path.read_text())
    counts = {"high": 0, "medium": 0, "low": 0}
    for finding in data.get("results", []):
        lvl = level(finding.get("extra", {}).get("severity", ""))
        counts[lvl] += 1
        where = f"{finding.get('path')}:{finding.get('start', {}).get('line')}"
        if lvl == "high":
            print(f"::error file={finding.get('path')},line={finding.get('start', {}).get('line')}::"
                  f"{rule_id(finding)}: {finding.get('extra', {}).get('message', '').strip()[:300]}")
        else:
            print(f"{lvl:6} {rule_id(finding)}  {where}")
    files = sum(1 for path in data.get("paths", {}).get("scanned", []) if path.endswith(SOURCE))
    rules = count_rules(rules_dir)
    errors = data.get("errors", [])
    print(f"Semgrep: {rules} rules over {files} source files; "
          f"{counts['high']} high, {counts['medium']} medium, {counts['low']} low; "
          f"{len(errors)} files it could not fully read")
    out = os.environ.get("GITHUB_OUTPUT")
    if out:
        with open(out, "a") as handle:
            for key, value in {**counts, "rules": rules, "files": files}.items():
                handle.write(f"{key}={value}\n")
    if files == 0 or rules == 0:
        print("::error::Semgrep scanned nothing; the rules or the source did not load")
        return 1
    return 1 if counts["high"] else 0


if __name__ == "__main__":
    sys.exit(main())
