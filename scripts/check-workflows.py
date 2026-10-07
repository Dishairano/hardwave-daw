#!/usr/bin/env python3
"""Every workflow file has to be YAML a machine will read.

A colon followed by a space inside an unquoted command is the trap:
it reads as a mapping, the file stops parsing, and GitHub shows the
run as "this run likely failed because of a workflow file issue"
with no log to read. That cost us a gate run and an evening, so it
is checked here before a push instead.

It also catches the YAML that parses but that GitHub refuses: a step
whose `env:` or `with:` is left empty (null), which happens when the
last line under it is removed, and an action used by tag instead of
a pinned commit.
"""
import pathlib
import re
import sys

import yaml


def steps_of(workflow):
    for job in (workflow.get("jobs") or {}).values():
        for step in (job or {}).get("steps") or []:
            yield step


def problems(workflow):
    """What parses but GitHub would refuse, or we do not allow."""
    found = []
    for step in steps_of(workflow):
        name = step.get("name") or step.get("uses") or "a step"
        for key in ("env", "with"):
            if key in step and step[key] is None:
                found.append(f"{name}: `{key}:` is empty")
        uses = step.get("uses") or ""
        if uses and not uses.startswith("./") and not re.search(r"@[0-9a-f]{40}$", uses):
            found.append(f"{name}: {uses} is not pinned to a commit")
    return found

def main() -> int:
    root = pathlib.Path(__file__).resolve().parent.parent / ".github" / "workflows"
    bad = 0
    for path in sorted(root.glob("*.yml")):
        try:
            workflow = yaml.safe_load(path.read_text())
            for problem in problems(workflow or {}):
                bad += 1
                print(f"{path.name}: {problem}")
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
        print(f"{len(list(root.glob('*.yml')))} workflow files parse, every action pinned")
    return 1 if bad else 0

if __name__ == "__main__":
    sys.exit(main())
