#!/usr/bin/env python3
"""Secure Code Check: the per-release report, as JSON and HTML.

Built in CI from the run's own results, never by hand:

  report.py --repo . --scan-dir scc --record-scan
      Notes when, where and on which commit the checks ran
      (scc/scan-meta.json). Runs in the job that collects the results.

  report.py --repo . --scan-dir scc --out-dir out [release options]
      Writes <base>.json and <base>.html to out/. Without release options
      it is a preview of the scans alone (every push). With them it is
      the release report: the scans, the release's own tests and build,
      every release file with its SHA-256 and Windows signature, and the
      SBOMs copied next to it under release names.

  Release options:
      --tag vX.Y.Z           the release tag
      --released-at ISO      when the GitHub release was published
      --release-needs FILE   toJSON(needs) of the release's evidence job
      --assets-dir DIR       the release's files, as downloaded
      --authenticode FILE    Windows signature status per file (build job)

The scan results come from scc/: scan-needs.json (toJSON(needs) of the
results job), semgrep.json and sbom/*.cdx.json.

What it never says: that anything is certified or compliant. It lists
evidence, mapped to the ISO 27001:2022 Annex A controls it supports.
Only the Python standard library is used, so it runs on any runner.
"""
from __future__ import annotations

import argparse
import base64
import datetime as dt
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tomllib

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import render  # noqa: E402

# One product per repo for now. Step 2 (Suite, WettBoi, Analyser) moves
# this into a small config file per repo.
PRODUCT = {
    "id": "hardwave-daw",
    "name": "Hardwave DAW",
    "platforms": "Windows x64",
    "default_repository": "Dishairano/hardwave-daw",
}

TOOLS = {
    "cargo_deny": "cargo deny (cargo-deny-action 2.1.1)",
    "npm_audit": "npm audit",
    "gitleaks": "gitleaks 8.30.1",
    "semgrep": "Semgrep CE 1.182.1",
    "semgrep_rules": "16b9e6d45f10649f6a531ec4838ed9924372c3bc",
    "cargo_fuzz": "cargo fuzz 0.13.2 (libFuzzer)",
    "playwright": "Playwright",
    "cargo_cyclonedx": "cargo-cyclonedx 0.5.9",
    "cyclonedx_npm": "cyclonedx-npm 6.0.1",
}

SIGNING = {
    "method": "SSH signature, Ed25519 (ssh-keygen -Y sign)",
    "namespace": "hardwave-secure-code-check",
    "identity": "secure-code-check@hardwavestudios.com",
}

NOTICE = (
    "This is not a certificate. Hardwave Studios is not ISO 27001 certified, and this "
    "report does not say that the product or our company is ISO 27001 compliant. Only an "
    "accredited certification body can certify an organisation's information security "
    "management system, after auditing the whole organisation. This is evidence for one "
    "release, produced automatically by our build and mapped to ISO 27001:2022 controls, "
    "that you can use in your own supplier assessment (Annex A 5.19 to 5.22)."
)

SBOM_PARTS = {
    "app-crates.cdx.json": "DAW app: Rust crates",
    "plugin-bridge-crates.cdx.json": "Plug-in bridge: Rust crates",
    "launcher-crates.cdx.json": "Launcher and uninstaller: Rust crates",
    "ui-npm.cdx.json": "DAW interface: npm packages",
    "launcher-npm.cdx.json": "Launcher interface: npm packages",
}

CONTROL_TITLES = {
    "8.8": "Management of technical vulnerabilities",
    "8.25": "Secure development life cycle",
    "8.28": "Secure coding",
    "8.29": "Security testing in development and acceptance",
    "8.32": "Change management",
    "5.21": "Managing information security in the ICT supply chain",
    "5.17": "Authentication information",
    "5.32": "Intellectual property rights",
    "8.24": "Use of cryptography",
}

PASSING = {"passed", "written", "signed"}


# ---------------------------------------------------------------- inputs

def read_json(path: pathlib.Path, default=None):
    try:
        # utf-8-sig: PowerShell may write a byte order mark.
        return json.loads(path.read_text(encoding="utf-8-sig"))
    except (FileNotFoundError, json.JSONDecodeError):
        return default


def git(repo: pathlib.Path, *args: str) -> str:
    try:
        return subprocess.run(["git", "-C", str(repo), *args], capture_output=True,
                              text=True, check=True).stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return ""


def lock_crates(path: pathlib.Path) -> int:
    try:
        return len(tomllib.loads(path.read_text()).get("package", []))
    except FileNotFoundError:
        return 0


def npm_packages(path: pathlib.Path, shipped_only: bool) -> int:
    data = read_json(path, {})
    packages = [v for k, v in data.get("packages", {}).items() if k]
    if shipped_only:
        packages = [p for p in packages if not p.get("dev")]
    return len(packages)


def recorded(repo: pathlib.Path, needle: str) -> dict:
    """When an exception first appeared in deny.toml, from git history."""
    line = git(repo, "log", "--format=%as %H", f"-S{needle}", "--", "deny.toml").splitlines()
    if not line:
        return {"date": None, "commit": None}
    date, commit = line[-1].split()
    return {"date": date, "commit": commit}


def exceptions(repo: pathlib.Path) -> dict:
    deny = tomllib.loads((repo / "deny.toml").read_text())
    advisories = []
    for entry in deny.get("advisories", {}).get("ignore", []):
        if isinstance(entry, str):
            entry = {"id": entry}
        key = entry.get("id") or entry.get("crate")
        kind = "advisory" if entry.get("id") else "yanked crate"
        advisories.append({
            "id": key,
            "kind": kind,
            "reason": entry.get("reason", ""),
            "recorded": recorded(repo, key),
        })
    licences = deny.get("licenses", {}).get("exceptions", [])
    return {"advisories": advisories, "licences": {"count": len(licences)}}


def scan_meta_from_env() -> dict:
    env = os.environ
    server = env.get("GITHUB_SERVER_URL", "https://github.com")
    repo = env.get("GITHUB_REPOSITORY", PRODUCT["default_repository"])
    run_id = env.get("GITHUB_RUN_ID")
    return {
        "scanned_at": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds"),
        "commit": env.get("GITHUB_SHA", ""),
        "ref": env.get("GITHUB_REF", ""),
        "event": env.get("GITHUB_EVENT_NAME", ""),
        "repository": repo,
        "workflow_ref": env.get("GITHUB_WORKFLOW_REF", ""),
        "run_url": f"{server}/{repo}/actions/runs/{run_id}" if run_id else "",
        "runner": env.get("RUNNER_ENVIRONMENT", "github-hosted"),
    }


def sha256_of(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def public_key(repo: pathlib.Path) -> dict:
    path = repo / "security" / "secure-code-check" / "signing_key.pub"
    try:
        key_type, blob = path.read_text().split()[:2]
    except (FileNotFoundError, ValueError):
        return {}
    fingerprint = base64.b64encode(hashlib.sha256(base64.b64decode(blob)).digest()).decode().rstrip("=")
    return {"public_key": f"{key_type} {blob}", "fingerprint": f"SHA256:{fingerprint}"}


def history(repo: pathlib.Path, today: dt.date) -> dict:
    """The product's vulnerability history, from the checked-in
    security/vulnerability-history.json, with the days worked out for
    today: days open run from the later of the day the advisory was
    published and the first release that shipped it, to the day it was
    resolved (or today, while it is open)."""
    data = read_json(repo / "security" / "vulnerability-history.json", {})
    entries = []
    for entry in data.get("entries", []):
        start = max(dt.date.fromisoformat(entry["published"]),
                    dt.date.fromisoformat(entry.get("first_shipped_date") or entry["published"]))
        end = dt.date.fromisoformat(entry["resolved"]) if entry.get("resolved") else today
        entries.append({**entry, "days_open": max(0, (end - start).days)})
    entries.sort(key=lambda e: (e["published"], e["id"]), reverse=True)
    fixed = sorted(e["days_open"] for e in entries if e["status"] == "fixed")
    median = None
    if fixed:
        mid = len(fixed) // 2
        median = fixed[mid] if len(fixed) % 2 else (fixed[mid - 1] + fixed[mid]) / 2
    last = max((dt.date.fromisoformat(e["published"]) for e in entries), default=None)
    return {
        "about": data.get("about", ""),
        "sources": data.get("sources", []),
        "summary": {
            "total": len(entries),
            "open": sum(1 for e in entries if e["status"] == "open"),
            "fixed": len(fixed),
            "accepted": sum(1 for e in entries if e["status"] == "accepted"),
            "no_longer_shipped": sum(1 for e in entries if e["status"] == "no_longer_shipped"),
            "median_days_to_fix": median,
            "last_published": last.isoformat() if last else None,
            "days_since_last": (today - last).days if last else None,
            "as_of": today.isoformat(),
        },
        "entries": entries,
        "not_counted": data.get("not_counted", []),
    }


# ---------------------------------------------------------------- checks

def combined(results: list) -> str:
    """One status from several GitHub job results or step outcomes."""
    if results and all(r == "success" for r in results):
        return "passed"
    if any(r == "failure" for r in results):
        return "failed"
    if any(r == "cancelled" for r in results):
        return "cancelled"
    return "not_run"


def job_status(needs: dict, *jobs: str) -> str:
    return combined([needs.get(job, {}).get("result", "missing") for job in jobs])


def step_status(needs: dict, parts: list) -> str:
    """Status from step outcomes a job reports as outputs, e.g.
    [("rust-dependencies", "licences")]. Falls back to the job's own
    result when a job does not report that step."""
    return combined([outputs(needs, job).get(key) or needs.get(job, {}).get("result", "missing")
                     for job, key in parts])


def outputs(needs: dict, job: str) -> dict:
    return needs.get(job, {}).get("outputs", {}) or {}


def as_int(value, default=0) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return default


def scan_checks(repo: pathlib.Path, needs: dict, exc: dict, sbom: list, semgrep: dict) -> list:
    daw_crates = lock_crates(repo / "Cargo.lock")
    launcher_crates = lock_crates(repo / "installer" / "src-tauri" / "Cargo.lock")
    launcher_npm = npm_packages(repo / "installer" / "package-lock.json", shipped_only=True)
    ui_npm = npm_packages(repo / "packages" / "daw-ui" / "package-lock.json", shipped_only=False)
    advisory_count = len(exc["advisories"])
    licence_count = exc["licences"]["count"]

    fuzz = outputs(needs, "fuzz-targets") or outputs(needs, "fuzz")
    raw = fuzz.get("targets", "")
    try:
        targets = json.loads(raw) if raw.startswith("[") else raw.split()
    except json.JSONDecodeError:
        targets = []
    seconds = as_int(fuzz.get("seconds"), 60)
    secrets = outputs(needs, "secrets")
    sa = outputs(needs, "static-analysis")

    checks = [
        {
            "id": "rust-dependencies", "name": "Rust dependencies", "tool": TOOLS["cargo_deny"],
            "what": f"{daw_crates} crates in the DAW's Cargo.lock against the RustSec advisory database: "
                    "known vulnerabilities, yanked crates, download sources",
            "status": step_status(needs, [("rust-dependencies", "advisories")]),
            "exceptions": advisory_count,
            "controls": ["8.8", "5.21"],
        },
        {
            "id": "launcher-dependencies", "name": "Launcher dependencies",
            "tool": f"{TOOLS['cargo_deny']}, {TOOLS['npm_audit']}",
            "what": f"{launcher_crates} crates of the launcher and uninstaller, and the {launcher_npm} "
                    "npm packages it ships",
            "status": step_status(needs, [("launcher-dependencies", "advisories"),
                                          ("launcher-dependencies", "npm")]),
            "controls": ["8.8", "5.21"],
        },
        {
            "id": "ui-dependencies", "name": "Interface dependencies", "tool": TOOLS["npm_audit"],
            "what": f"{ui_npm} npm packages of the interface, build tools included; "
                    "moderate severity and up fails",
            "status": job_status(needs, "ui-dependencies"),
            "controls": ["8.8", "5.21"],
        },
        {
            "id": "licences", "name": "Licences", "tool": TOOLS["cargo_deny"],
            "what": "Every crate's licence against our allow list",
            "status": step_status(needs, [("rust-dependencies", "licences"),
                                          ("launcher-dependencies", "licences")]),
            "exceptions": licence_count,
            "controls": ["5.32"],
        },
        {
            "id": "secrets", "name": "Secrets", "tool": TOOLS["gitleaks"],
            "what": (f"All {secrets['commits']} commits in the repository's history"
                     if secrets.get("commits") else "Every commit in the repository's history")
                    + ", for keys, tokens and passwords",
            "status": job_status(needs, "secrets"),
            "controls": ["5.17"],
        },
        {
            "id": "static-analysis", "name": "Static analysis", "tool": TOOLS["semgrep"],
            "what": (f"{as_int(sa.get('files'))} Rust, TypeScript and JavaScript files against "
                     f"{as_int(sa.get('rules'))} security rules from semgrep-rules "
                     f"{TOOLS['semgrep_rules'][:7]} (all but unsafe-usage, which flags every unsafe "
                     "block); a high finding fails"),
            "status": job_status(needs, "static-analysis"),
            "high": as_int(sa.get("high")),
            "lower": as_int(sa.get("medium")) + as_int(sa.get("low")),
            "controls": ["8.28"],
        },
        {
            "id": "fuzzing", "name": "Fuzzing", "tool": TOOLS["cargo_fuzz"],
            "what": (f"{len(targets)} readers of outside input, {seconds} s each: {', '.join(targets)}"
                     if targets else "Every reader of outside input"),
            "status": job_status(needs, "fuzz-targets", "fuzz") if "fuzz-targets" in needs
                      else job_status(needs, "fuzz"),
            "controls": ["8.29"],
        },
        {
            "id": "content-policy", "name": "Window content policy", "tool": TOOLS["playwright"],
            "what": "The built interface loaded under the app's content policy; anything blocked fails",
            "status": job_status(needs, "ui-content-policy"),
            "controls": ["8.29"],
        },
        {
            "id": "sbom", "name": "SBOM", "tool": f"{TOOLS['cargo_cyclonedx']}, {TOOLS['cyclonedx_npm']}",
            "what": (f"CycloneDX 1.5 JSON of what ships: {sum(s['components'] for s in sbom)} components "
                     f"in {len(sbom)} files"),
            "status": "written" if job_status(needs, "sbom") == "passed" and sbom else job_status(needs, "sbom"),
            "controls": ["5.21"],
        },
    ]
    for check in checks:
        check["status_text"] = status_text(check)
    for check in checks:
        if check["id"] == "static-analysis" and semgrep.get("findings"):
            check["findings"] = semgrep["findings"]
    return checks


def release_checks(needs: dict, files: list) -> list:
    exes = [f for f in files if f["name"].lower().endswith(".exe")]
    signed = [f for f in exes if (f.get("windows_signature") or {}).get("status") == "Valid"]
    if exes and len(signed) == len(exes):
        signing = "signed"
    else:
        signing = "not_verified"
    checks = [
        {
            "id": "tests", "name": "Lint and tests", "tool": "cargo fmt, Clippy, cargo test, Playwright",
            "what": "Formatting; Clippy with warnings as errors; the whole workspace's tests including the "
                    "long engine tests; the interface suite in a browser",
            "status": job_status(needs, "lint-and-test", "ui-tests"),
            "controls": ["8.28", "8.29"],
        },
        {
            "id": "code-signing", "name": "Code signing", "tool": "Azure Trusted Signing",
            "what": (f"Windows signature on {len(exes)} installer files, checked on the build machine "
                     f"right after signing ({len(signed)} of {len(exes)} valid)"),
            "status": signing,
            "controls": ["8.24"],
        },
    ]
    for check in checks:
        check["status_text"] = status_text(check)
    return checks


def status_text(check: dict) -> str:
    status = check["status"]
    if status == "passed":
        if check.get("id") == "static-analysis":
            return f"Passed, {check.get('high', 0)} high findings"
        if check.get("exceptions"):
            n = check["exceptions"]
            return f"Passed, {n} exception{'s' if n != 1 else ''}"
        return "Passed"
    return {
        "failed": "Failed",
        "cancelled": "Cancelled",
        "not_run": "Did not run",
        "written": "Written",
        "signed": "Signed",
        "not_verified": "Not verified",
    }.get(status, status)


# ------------------------------------------------------------ ISO table

def iso_controls(checks: list, exc: dict, release: bool, files: list) -> list:
    ok = {c["id"] for c in checks if c["status"] in PASSING}
    evidence: dict[str, list[str]] = {k: [] for k in CONTROL_TITLES}
    if ok & {"rust-dependencies", "launcher-dependencies", "ui-dependencies"}:
        evidence["8.8"].append("dependency checks against the advisory databases on this commit")
        if exc["advisories"]:
            evidence["8.8"].append("recorded exceptions with their reasons")
    evidence["8.8"].append("the product's vulnerability history with days to fix")
    evidence["8.25"].append("these checks run on every push and again on the release tag; "
                            "a failed check stops the release")
    if "static-analysis" in ok:
        evidence["8.28"].append("static analysis with security rules")
    if "tests" in ok:
        evidence["8.28"].append("Clippy with warnings as errors")
    evidence["8.28"].append("build tools and actions pinned to exact versions, hashes or commits")
    if "fuzzing" in ok:
        evidence["8.29"].append("fuzzing of every reader of outside input")
    if "content-policy" in ok:
        evidence["8.29"].append("window content policy check")
    if "tests" in ok:
        evidence["8.29"].append("full test suite and interface suite")
    if release:
        evidence["8.32"].append("this signed report ties each release file's SHA-256 to one commit "
                                "and one workflow run")
    else:
        evidence["8.32"].append("every change runs these checks before it can be released")
    if "sbom" in ok:
        evidence["5.21"].append("SBOM of what ships")
    if ok & {"rust-dependencies", "launcher-dependencies"}:
        evidence["5.21"].append("dependency source checks (crates.io only, unless listed)")
    if "secrets" in ok:
        evidence["5.17"].append("secret scan over the full history")
    if "licences" in ok:
        evidence["5.32"].append("licence check against an allow list")
    if "code-signing" in ok:
        evidence["8.24"].append("Windows code signing, checked after signing")
    if any(f["name"].endswith(".sig") for f in files):
        evidence["8.24"].append("update packages signed; the app checks the signature before installing")
    if release:
        evidence["8.24"].append("this report's Ed25519 signature")
    rows = []
    for control, items in evidence.items():
        if items:
            text = "; ".join(items)
            rows.append({"control": control, "title": CONTROL_TITLES[control],
                         "evidence": text[0].upper() + text[1:]})
    return rows


# ---------------------------------------------------------------- build

def sbom_entries(sbom_dir: pathlib.Path) -> list:
    entries = []
    for name, part in SBOM_PARTS.items():
        path = sbom_dir / name
        if not path.exists():
            continue
        bom = read_json(path, {})

        def count(components):
            return sum(1 + count(c.get("components")) for c in components or [])

        entries.append({"source": name, "part": part, "components": count(bom.get("components")),
                        "spec": bom.get("specVersion", ""), "path": path})
    return entries


def semgrep_findings(path: pathlib.Path) -> dict:
    data = read_json(path, {})
    findings = []
    for finding in data.get("results", [])[:200]:
        findings.append({
            "rule": re.sub(r"^rules\.", "", finding.get("check_id", "")),
            "severity": finding.get("extra", {}).get("severity", ""),
            "path": finding.get("path", ""),
            "line": finding.get("start", {}).get("line"),
        })
    return {"findings": findings}


def release_files(assets_dir: pathlib.Path | None, own_prefix: str, authenticode: list) -> list:
    if not assets_dir or not assets_dir.is_dir():
        return []
    by_hash = {a.get("sha256", "").lower(): a for a in authenticode or []}
    files = []
    for path in sorted(assets_dir.iterdir()):
        if not path.is_file() or path.name.startswith(own_prefix):
            continue
        digest = sha256_of(path)
        sig = by_hash.get(digest)
        files.append({
            "name": path.name,
            "size": path.stat().st_size,
            "sha256": digest,
            "windows_signature": ({"status": sig.get("status"), "signer": sig.get("signer", "")}
                                  if sig else None),
        })
    return files


def build(args) -> dict:
    repo = pathlib.Path(args.repo).resolve()
    scan_dir = pathlib.Path(args.scan_dir)
    needs = read_json(scan_dir / "scan-needs.json", {})
    meta = read_json(scan_dir / "scan-meta.json", {}) or scan_meta_from_env()
    release = bool(args.tag)
    version = args.tag[1:] if release and args.tag.startswith("v") else args.tag
    commit = meta.get("commit") or git(repo, "rev-parse", "HEAD")
    label = version if release else commit[:8]
    prefix = f"{PRODUCT['id']}-{label}"

    exc = exceptions(repo)
    sbom = sbom_entries(scan_dir / "sbom")
    semgrep = semgrep_findings(scan_dir / "semgrep.json")
    checks = scan_checks(repo, needs, exc, sbom, semgrep)

    files: list = []
    if release:
        release_needs = read_json(pathlib.Path(args.release_needs), {}) if args.release_needs else {}
        authenticode = read_json(pathlib.Path(args.authenticode), []) if args.authenticode else []
        if isinstance(authenticode, dict):
            authenticode = authenticode.get("files", [authenticode])
        files = release_files(pathlib.Path(args.assets_dir) if args.assets_dir else None,
                              prefix, authenticode)
        checks += release_checks(release_needs, files)

    out_dir = pathlib.Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    sbom_out = []
    for entry in sbom:
        name = f"{prefix}-sbom-{entry['source']}"
        shutil.copyfile(entry["path"], out_dir / name)
        sbom_out.append({"file": name, "part": entry["part"], "components": entry["components"],
                         "spec_version": entry["spec"], "sha256": sha256_of(out_dir / name),
                         "size": (out_dir / name).stat().st_size})

    failed = [c for c in checks if c["status"] not in PASSING]
    repository = meta.get("repository") or PRODUCT["default_repository"]
    report = {
        "schema": "https://hardwavestudios.com/schemas/secure-code-check/v1",
        "kind": "release" if release else "preview",
        "product": {"id": PRODUCT["id"], "name": PRODUCT["name"], "repository": repository,
                    "platforms": PRODUCT["platforms"]},
        "release": {"version": version, "tag": args.tag, "prerelease": "-" in (args.tag or ""),
                    "released_at": args.released_at} if release else None,
        "source": {"commit": commit, "ref": meta.get("ref", ""), "run_url": meta.get("run_url", ""),
                   "workflow_ref": meta.get("workflow_ref", ""), "event": meta.get("event", ""),
                   "runner": meta.get("runner", "")},
        "scanned_at": meta.get("scanned_at"),
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds"),
        "result": "passed" if not failed else "not_all_passed",
        "summary": {
            "checks": len(checks),
            "passed": len(checks) - len(failed),
            "not_passed": [c["id"] for c in failed],
            "advisory_exceptions": len(exc["advisories"]),
            "licence_exceptions": exc["licences"]["count"],
            "crates": lock_crates(repo / "Cargo.lock"),
        },
        "checks": checks,
        "exceptions": {
            "advisories": exc["advisories"],
            "licences": {"count": exc["licences"]["count"],
                         "source": f"https://github.com/{repository}/blob/{commit}/deny.toml"},
        },
        "sbom": sbom_out,
        "release_files": files,
        "iso_controls": iso_controls(checks, exc, release, files),
        "vulnerability_history": history(repo, dt.datetime.now(dt.timezone.utc).date()),
        "signing": {**SIGNING, **public_key(repo)},
        "tools": TOOLS,
        "notice": NOTICE,
    }
    base = f"{prefix}-secure-code-check"
    (out_dir / f"{base}.json").write_text(json.dumps(report, indent=2) + "\n")
    (out_dir / f"{base}.html").write_text(render.report_html(report))
    print(f"{out_dir / base}.json: {report['summary']['passed']} of {len(checks)} checks passed")
    for check in checks:
        print(f"  {check['name']:24} {check['status_text']}")
    return report


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--repo", default=".")
    parser.add_argument("--scan-dir", required=True)
    parser.add_argument("--record-scan", action="store_true")
    parser.add_argument("--out-dir")
    parser.add_argument("--tag")
    parser.add_argument("--released-at")
    parser.add_argument("--release-needs")
    parser.add_argument("--assets-dir")
    parser.add_argument("--authenticode")
    args = parser.parse_args(argv)
    if args.tag and not re.fullmatch(r"v\d+\.\d+\.\d+(-[a-z0-9.]+)?", args.tag):
        parser.error(f"{args.tag!r} is not a release tag")
    if args.record_scan:
        path = pathlib.Path(args.scan_dir) / "scan-meta.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(scan_meta_from_env(), indent=2) + "\n")
        return 0
    if not args.out_dir:
        parser.error("--out-dir is needed to write the report")
    build(args)
    return 0


if __name__ == "__main__":
    sys.exit(main())
