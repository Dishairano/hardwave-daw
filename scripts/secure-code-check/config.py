#!/usr/bin/env python3
"""The per-product config of the Secure Code Check.

Each product repository keeps one small file,
security/secure-code-check/config.toml, with what the report says about
the product (what the checks themselves cover is set by the workflow's
inputs):

  [product]
  id = "hardwave-wettboi"           # page: /security/releases/<id>/<version>/
  name = "Hardwave WettBoi"
  platforms = "Windows x64, macOS (Apple silicon and Intel), Linux x64"
  repository = "Dishairano/hardwave-wettboi-vst"

  sbom_target = "all"               # cargo-cyclonedx --target: a triple, or "all"
  sbom_note = "..."                 # one sentence under the SBOM table

  [[sbom]]                          # one per SBOM file
  file = "plugin-crates.cdx.json"   # name in the report
  part = "WettBoi plug-in: Rust crates"
  cargo = "Cargo.toml"              # manifest cargo-cyclonedx runs on ...
  output = "hardwave-wettboi.cdx.json"   # ... and the file it writes
  # or: npm = "installer"           # folder with package-lock.json; runtime packages

  [release]
  test_jobs = ["lint-and-test"]     # release jobs whose result is "Lint and tests"
  tests_tool = "cargo fmt, Clippy, cargo test"
  tests_what = "..."
  windows_signing = ""              # signing service, or "" when nothing is signed
  files_note = "..."                # one sentence above the release files table

  [not_checked]                     # optional: checks this product does not run, and why
  fuzzing = "..."

Usage from the workflow:
  config.py check FILE        validates it (fails the run on a mistake)
  config.py sbom-cargo FILE   prints "<manifest>\\t<output>\\t<file>" per cargo part
  config.py sbom-npm FILE     prints "<folder>\\t<file>" per npm part
  config.py get FILE KEY      prints one top-level value, e.g. sbom_target
"""
from __future__ import annotations

import pathlib
import re
import sys
import tomllib

DEFAULT_PATH = "security/secure-code-check/config.toml"

PRODUCT_KEYS = ("id", "name", "platforms", "repository")
RELEASE_KEYS = ("test_jobs", "tests_tool", "tests_what", "windows_signing", "files_note")


class ConfigError(ValueError):
    pass


def load(path: pathlib.Path) -> dict:
    try:
        data = tomllib.loads(pathlib.Path(path).read_text())
    except FileNotFoundError:
        raise ConfigError(f"{path}: missing; every product needs its config") from None
    except tomllib.TOMLDecodeError as err:
        raise ConfigError(f"{path}: {err}") from None
    product = data.get("product", {})
    for key in PRODUCT_KEYS:
        if not isinstance(product.get(key), str) or not product[key].strip():
            raise ConfigError(f"{path}: [product] {key} is missing")
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]*", product["id"]):
        raise ConfigError(f"{path}: [product] id must be lower case letters, digits and dashes")
    if not re.fullmatch(r"[\w.-]+/[\w.-]+", product["repository"]):
        raise ConfigError(f"{path}: [product] repository must be owner/name")
    parts = data.get("sbom", [])
    if not parts:
        raise ConfigError(f"{path}: at least one [[sbom]] part is needed")
    seen = set()
    for part in parts:
        for key in ("file", "part"):
            if not part.get(key):
                raise ConfigError(f"{path}: an [[sbom]] part has no {key}")
        if not part["file"].endswith(".cdx.json") or "/" in part["file"]:
            raise ConfigError(f"{path}: sbom file {part['file']!r} must be a plain name ending .cdx.json")
        if part["file"] in seen:
            raise ConfigError(f"{path}: sbom file {part['file']!r} is listed twice")
        seen.add(part["file"])
        if bool(part.get("cargo")) == bool(part.get("npm")):
            raise ConfigError(f"{path}: sbom part {part['file']!r} needs either cargo (with output) or npm")
        if part.get("cargo") and not part.get("output"):
            raise ConfigError(f"{path}: sbom part {part['file']!r} needs the output cargo-cyclonedx writes")
    release = data.get("release", {})
    for key in RELEASE_KEYS:
        if key not in release:
            raise ConfigError(f"{path}: [release] {key} is missing")
    if not release["test_jobs"] or not all(isinstance(j, str) for j in release["test_jobs"]):
        raise ConfigError(f"{path}: [release] test_jobs must list job ids")
    return {
        "product": {k: product[k].strip() for k in PRODUCT_KEYS},
        "sbom": parts,
        "sbom_target": data.get("sbom_target", "all"),
        "sbom_note": data.get("sbom_note", ""),
        "release": release,
        "not_checked": [{"check": k, "why": v} for k, v in data.get("not_checked", {}).items()],
    }


def main(argv: list[str]) -> int:
    if len(argv) < 2 or argv[0] not in ("check", "sbom-cargo", "sbom-npm", "get"):
        print(__doc__, file=sys.stderr)
        return 2
    try:
        cfg = load(pathlib.Path(argv[1]))
    except ConfigError as err:
        print(f"::error::{err}")
        return 1
    if argv[0] == "check":
        print(f"{argv[1]}: {cfg['product']['name']} ({cfg['product']['id']}), "
              f"{len(cfg['sbom'])} SBOM parts")
    elif argv[0] == "sbom-cargo":
        for part in cfg["sbom"]:
            if part.get("cargo"):
                print(f"{part['cargo']}\t{part['output']}\t{part['file']}")
    elif argv[0] == "sbom-npm":
        for part in cfg["sbom"]:
            if part.get("npm"):
                print(f"{part['npm']}\t{part['file']}")
    elif argv[0] == "get":
        print(cfg.get(argv[2], ""))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
