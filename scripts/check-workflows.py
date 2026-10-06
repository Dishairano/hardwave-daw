#!/usr/bin/env python3
"""Every workflow file has to be YAML a machine will read.

A colon followed by a space inside an unquoted command is the trap:
it reads as a mapping, the file stops parsing, and GitHub shows the
run as "this run likely failed because of a workflow file issue"
with no log to read. That cost us a gate run and an evening, so it
is checked here before a push instead.
"""
import pathlib
import sys

import yaml

def main() -> int:
    root = pathlib.Path(__file__).resolve().parent.parent / ".github" / "workflows"
    bad = 0
    for path in sorted(root.glob("*.yml")):
        try:
            yaml.safe_load(path.read_text())
        except yaml.YAMLError as error:
            bad += 1
            mark = getattr(error, "problem_mark", None)
            where = f"line {mark.line + 1}" if mark else "somewhere"
            line = path.read_text().split("\n")[mark.line] if mark else ""
            print(f"{path.name}: {where}: {error.problem}")
            if line:
                print(f"  {line.strip()[:120]}")
                print("  quote the value, or write it as a block scalar with >-")
    if bad == 0:
        print(f"{len(list(root.glob('*.yml')))} workflow files parse")
    return 1 if bad else 0

if __name__ == "__main__":
    sys.exit(main())
