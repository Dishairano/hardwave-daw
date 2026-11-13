"""Tests for the Secure Code Check report, page and config.

Run: python3 -m unittest discover -s scripts -p 'test_*.py'

They build a release report from made-up scan results against a small
made-up product repository (deny.toml, lock files, config, history,
public key, in a temporary git repository), then the public page, and
check what a buyer would rely on: the result, the exceptions, the
hashes, and that nothing claims a certificate or compliance.
"""
import hashlib
import html
import json
import pathlib
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import config  # noqa: E402
import page  # noqa: E402
import report  # noqa: E402

GREEN = {"result": "success", "outputs": {}}
PUBLIC_KEY = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIIT+m3BVUiJBA9lAoQXXvrL2j8+jRmwIJqQPM+PHieeQ"

CONFIG = """
[product]
id = "hardwave-example"
name = "Hardwave Example"
platforms = "Windows x64, macOS, Linux x64"
repository = "Dishairano/hardwave-example"

sbom_target = "all"
sbom_note = "Rust crates for every platform the plug-in is built for."

[[sbom]]
file = "plugin-crates.cdx.json"
part = "Example plug-in: Rust crates"
cargo = "Cargo.toml"
output = "hardwave-example.cdx.json"

[[sbom]]
file = "ui-npm.cdx.json"
part = "Example interface: npm packages"
npm = "ui"

[release]
test_jobs = ["lint-and-test"]
tests_tool = "cargo fmt, Clippy, cargo test"
tests_what = "Formatting; Clippy with warnings as errors; the tests"
windows_signing = "Azure Trusted Signing"
files_note = "Built on GitHub-hosted runners."

[not_checked]
fuzzing = "No fuzz targets yet."
"""

DENY = """
[advisories]
ignore = [{ id = "RUSTSEC-2026-0001", reason = "Only reachable from a build tool." }]

[[licenses.exceptions]]
crate = "vst3-sys"
allow = ["GPL-3.0-or-later"]
"""

HISTORY = {
    "about": "Every RustSec advisory that affected a shipped build.",
    "sources": ["release tags and their Cargo.lock"],
    "entries": [
        {"id": "RUSTSEC-2026-0002", "aliases": ["CVE-2026-1"], "crate": "ring", "versions": ["0.17.1"],
         "kind": "vulnerability", "summary": "Some flaw in ring", "published": "2026-10-01",
         "first_shipped": "v1.0.0", "first_shipped_date": "2026-09-01", "platforms": ["Windows"],
         "status": "fixed", "resolved": "2026-10-04", "resolved_by": "abcdef12", "fixed_release": "v1.0.1"},
        {"id": "RUSTSEC-2026-0003", "aliases": [], "crate": "time", "versions": ["0.3.1"],
         "kind": "unsound", "summary": "Another flaw", "published": "2026-11-01",
         "first_shipped": "v1.0.0", "first_shipped_date": "2026-09-01", "platforms": ["Linux"],
         "status": "open", "reason": "Waiting for the upstream fix"},
    ],
}


def scan_needs(**overrides):
    needs = {job: dict(GREEN) for job in (
        "rust-dependencies", "npm-dependencies", "secrets", "static-analysis", "sbom")}
    needs["rust-dependencies"] = {"result": "success", "outputs": {"advisories": "success", "licences": "success"}}
    needs["secrets"] = {"result": "success", "outputs": {"commits": "4321"}}
    needs["static-analysis"] = {"result": "success", "outputs": {
        "high": "0", "medium": "2", "low": "1", "rules": "180", "files": "59"}}
    needs["fuzz-targets"] = {"result": "skipped", "outputs": {}}
    needs["fuzz"] = {"result": "skipped", "outputs": {}}
    needs["content-policy"] = {"result": "skipped", "outputs": {}}
    needs.update(overrides)
    return needs


SCAN_INPUTS = {"cargo-manifests": "Cargo.toml\nxtask/Cargo.toml", "npm-audit": "ui --omit=dev",
               "fuzz": False, "content-policy-dir": "", "semgrep-languages": "rust typescript"}


def bom(n):
    return {"bomFormat": "CycloneDX", "specVersion": "1.5",
            "components": [{"name": f"c{i}", "components": [{"name": "nested"}] if i == 0 else []}
                           for i in range(n)]}


def make_repo(root: pathlib.Path) -> pathlib.Path:
    repo = root / "repo"
    (repo / "security" / "secure-code-check").mkdir(parents=True)
    (repo / "xtask").mkdir()
    (repo / "ui").mkdir()
    (repo / "security" / "secure-code-check" / "config.toml").write_text(CONFIG)
    (repo / "security" / "secure-code-check" / "signing_key.pub").write_text(
        PUBLIC_KEY + " secure-code-check@hardwavestudios.com\n")
    (repo / "security" / "secure-code-check" / "allowed_signers").write_text(
        'secure-code-check@hardwavestudios.com namespaces="hardwave-secure-code-check" ' + PUBLIC_KEY + "\n")
    (repo / "security" / "vulnerability-history.json").write_text(json.dumps(HISTORY))
    (repo / "Cargo.toml").write_text("[workspace]\nmembers = ['xtask']\n")
    (repo / "xtask" / "Cargo.toml").write_text("[package]\nname = 'xtask'\n")
    (repo / "Cargo.lock").write_text("".join(f'[[package]]\nname = "c{i}"\nversion = "1.0.0"\n\n' for i in range(7)))
    (repo / "ui" / "package-lock.json").write_text(json.dumps({"packages": {
        "": {}, "node_modules/react": {}, "node_modules/vite": {"dev": True}}}))
    (repo / "deny.toml").write_text(DENY)
    for args in (["init", "-q"], ["add", "-A"],
                 ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "init",
                  "--date", "2026-10-17T10:00:00"]):
        subprocess.run(["git", "-C", str(repo), *args], check=True,
                       env={"GIT_COMMITTER_DATE": "2026-10-17T10:00:00", "PATH": "/usr/bin:/bin"})
    return repo


class ReportTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = pathlib.Path(self.tmp.name)
        self.repo = make_repo(root)
        self.scan, self.assets, self.out, self.site = (root / d for d in ("scc", "assets", "out", "site"))
        (self.scan / "sbom").mkdir(parents=True)
        self.assets.mkdir()
        (self.scan / "sbom" / "plugin-crates.cdx.json").write_text(json.dumps(bom(3)))
        (self.scan / "sbom" / "ui-npm.cdx.json").write_text(json.dumps(bom(2)))
        (self.scan / "scan-inputs.json").write_text(json.dumps(SCAN_INPUTS))
        (self.scan / "semgrep.json").write_text(json.dumps({"results": [
            {"check_id": "rules.rust.lang.security.temp-dir", "path": "src/<x>.rs", "start": {"line": 3},
             "extra": {"severity": "WARNING"}}]}))
        (self.scan / "scan-meta.json").write_text(json.dumps({
            "scanned_at": "2026-11-02T15:01:00+00:00", "commit": "7fa4ac5d" + "0" * 32,
            "ref": "refs/tags/v1.0.2", "repository": "Dishairano/hardwave-example",
            "run_url": "https://github.com/Dishairano/hardwave-example/actions/runs/1",
            "runner": "github-hosted"}))
        self.exe = self.assets / "Example-Setup.exe"
        self.exe.write_bytes(b"MZ installer")
        (self.assets / "example-macos.zip").write_bytes(b"PK zip")
        (self.assets / "x.exe.sig").write_text("sig")
        # An evidence file from an earlier attempt is not a release file.
        (self.assets / "hardwave-example-1.0.2-secure-code-check.json").write_text("{}")
        self.authenticode = root / "authenticode.json"
        self.authenticode.write_text(json.dumps([{
            "file": "Example-Setup.exe", "sha256": hashlib.sha256(b"MZ installer").hexdigest().upper(),
            "status": "Valid", "signer": "CN=Hardwave Studios"}]))
        self.release_needs = root / "release-needs.json"
        self.release_needs.write_text(json.dumps({"lint-and-test": GREEN, "secure-code-check": GREEN}))

    def tearDown(self):
        self.tmp.cleanup()

    def build(self, needs=None):
        (self.scan / "scan-needs.json").write_text(json.dumps(needs or scan_needs()))
        report.main(["--repo", str(self.repo), "--scan-dir", str(self.scan), "--out-dir", str(self.out),
                     "--tag", "v1.0.2", "--released-at", "2026-11-02T15:56:00Z",
                     "--release-needs", str(self.release_needs), "--assets-dir", str(self.assets),
                     "--authenticode", str(self.authenticode)])
        return json.loads((self.out / "hardwave-example-1.0.2-secure-code-check.json").read_text())

    def html(self):
        return (self.out / "hardwave-example-1.0.2-secure-code-check.html").read_text()

    def test_green_release(self):
        data = self.build()
        self.assertEqual(data["result"], "passed")
        ids = [c["id"] for c in data["checks"]]
        # Fuzzing and the content policy are off for this product: not listed as checks.
        self.assertEqual(ids, ["rust-dependencies", "licences", "npm-dependencies", "secrets",
                               "static-analysis", "sbom", "tests", "code-signing"])
        self.assertEqual(data["product"]["id"], "hardwave-example")
        self.assertEqual(data["summary"]["crates"], 7)
        names = [f["name"] for f in data["release_files"]]
        self.assertEqual(names, ["Example-Setup.exe", "example-macos.zip", "x.exe.sig"])
        exe = data["release_files"][0]
        self.assertEqual(exe["sha256"], hashlib.sha256(b"MZ installer").hexdigest())
        self.assertEqual(exe["windows_signature"]["status"], "Valid")
        self.assertEqual(sum(s["components"] for s in data["sbom"]), 7)
        self.assertTrue((self.out / "hardwave-example-1.0.2-sbom-plugin-crates.cdx.json").exists())
        npm = next(c for c in data["checks"] if c["id"] == "npm-dependencies")
        self.assertIn("1 in ui (what ships)", npm["what"])
        sa = next(c for c in data["checks"] if c["id"] == "static-analysis")
        self.assertIn("Rust and TypeScript files", sa["what"])
        self.assertIn("<b>Fuzzing</b>: No fuzz targets yet.", self.html())

    def test_exceptions_come_from_deny_toml(self):
        data = self.build()
        [advisory] = data["exceptions"]["advisories"]
        self.assertEqual(advisory["id"], "RUSTSEC-2026-0001")
        self.assertEqual(advisory["reason"], "Only reachable from a build tool.")
        self.assertEqual(advisory["recorded"]["date"], "2026-10-17")
        self.assertEqual(data["exceptions"]["licences"]["count"], 1)
        # Licence exceptions are a count and a link, never the crate names.
        self.assertNotIn("vst3-sys", self.html())

    def test_a_failed_check_is_reported(self):
        data = self.build(scan_needs(secrets={"result": "failure", "outputs": {}}))
        self.assertEqual(data["result"], "not_all_passed")
        self.assertIn("secrets", data["summary"]["not_passed"])
        self.assertIn("7 of 8 checks passed", self.html())

    def test_fuzzing_is_listed_when_on(self):
        inputs = dict(SCAN_INPUTS, fuzz=True)
        (self.scan / "scan-inputs.json").write_text(json.dumps(inputs))
        data = self.build(scan_needs(**{"fuzz-targets": {"result": "success", "outputs": {
            "targets": '["state","osc"]', "seconds": "60"}}, "fuzz": GREEN}))
        fuzz = next(c for c in data["checks"] if c["id"] == "fuzzing")
        self.assertEqual(fuzz["status"], "passed")
        self.assertIn("2 readers of outside input, 60 s each: state, osc", fuzz["what"])

    def test_licences_fail_on_their_own(self):
        data = self.build(scan_needs(**{"rust-dependencies": {"result": "failure", "outputs": {
            "advisories": "failure", "licences": "success"}}}))
        status = {c["id"]: c["status"] for c in data["checks"]}
        self.assertEqual(status["rust-dependencies"], "failed")
        self.assertEqual(status["licences"], "passed")

    def test_unsigned_installer_is_not_called_signed(self):
        self.authenticode.write_text("[]")
        data = self.build()
        signing = next(c for c in data["checks"] if c["id"] == "code-signing")
        self.assertEqual(signing["status"], "not_verified")
        self.assertNotIn("Windows code signing", json.dumps(data["iso_controls"]))

    def test_a_product_without_windows_signing(self):
        cfg = (self.repo / "security/secure-code-check/config.toml")
        cfg.write_text(CONFIG.replace('windows_signing = "Azure Trusted Signing"', 'windows_signing = ""'))
        for f in self.assets.glob("*.exe*"):
            f.unlink()
        data = self.build()
        self.assertNotIn("code-signing", [c["id"] for c in data["checks"]])
        self.assertEqual(data["result"], "passed")
        self.assertNotIn("Windows signature", self.html())
        self.assertNotIn("Get-AuthenticodeSignature", self.html())

    def test_html_is_escaped(self):
        self.build()
        self.assertIn("src/&lt;x&gt;.rs", self.html())
        self.assertNotIn("src/<x>.rs", self.html())

    def test_page(self):
        self.build()
        base = self.out / "hardwave-example-1.0.2-secure-code-check"
        for suffix in (".json.sig", ".html.sig"):
            pathlib.Path(str(base) + suffix).write_text("sig")
        history = self.out / "history.json"
        history.write_text(json.dumps([{"version": "1.0.3", "released_at": "2026-11-10T10:00:00Z"}]))
        self.assertEqual(page.main(["--report", str(base) + ".json", "--files", str(self.out),
                                    "--repo", str(self.repo), "--site-dir", str(self.site),
                                    "--history", str(history)]), 0)
        out = self.site / "security" / "releases" / "hardwave-example" / "1.0.2"
        text = (out / "index.html").read_text()
        for name in ("style.css", "badge.svg", "allowed_signers", "signing_key.pub",
                     "hardwave-example-1.0.2-sbom-ui-npm.cdx.json"):
            self.assertTrue((out / name).exists(), name)
        self.assertNotIn("<script", text)
        self.assertNotIn("<style", text)
        self.assertIn('href="../1.0.3/"', text)
        self.assertTrue((out.parent / "index.html").exists())

    def test_vulnerability_history(self):
        hist = report.history(self.repo, report.dt.date(2026, 11, 9))
        s = hist["summary"]
        self.assertEqual((s["total"], s["open"], s["fixed"]), (2, 1, 1))
        self.assertEqual(s["last_published"], "2026-11-01")
        self.assertEqual(s["days_since_last"], 8)
        ring = next(e for e in hist["entries"] if e["id"] == "RUSTSEC-2026-0002")
        self.assertEqual(ring["days_open"], 3)
        later = report.history(self.repo, report.dt.date(2026, 12, 9))
        time_then = next(e for e in later["entries"] if e["id"] == "RUSTSEC-2026-0003")
        time_now = next(e for e in hist["entries"] if e["id"] == "RUSTSEC-2026-0003")
        self.assertEqual(time_then["days_open"], time_now["days_open"] + 30)
        self.build()
        self.assertIn("Vulnerability history", self.html())
        self.assertIn("RUSTSEC-2026-0002", self.html())

    def test_an_empty_history_is_shown_as_empty(self):
        (self.repo / "security" / "vulnerability-history.json").write_text(json.dumps(
            {"about": "Every RustSec advisory that affected a shipped build.", "entries": []}))
        self.build()
        self.assertIn("No advisory has affected a shipped build.", self.html())

    def test_no_certificate_or_compliance_claims(self):
        self.build()
        base = self.out / "hardwave-example-1.0.2-secure-code-check"
        for path in (pathlib.Path(str(base) + ".html"), pathlib.Path(str(base) + ".json")):
            text = html.unescape(path.read_text()) if path.suffix == ".html" else \
                json.dumps(json.loads(path.read_text()), ensure_ascii=False)
            text = text.replace(report.NOTICE, "")
            text = text.replace(report.NOTICE.split(". ", 1)[1], "").replace("This is not a certificate.", "")
            text = text.replace("not a certificate", "")
            for word in ("certificate", "certified", "compliant", "compliance"):
                self.assertIsNone(re.search(word, text, re.I), f"{path.name} says {word!r}")


class ConfigTest(unittest.TestCase):
    def write(self, text):
        tmp = tempfile.NamedTemporaryFile("w", suffix=".toml", delete=False)
        tmp.write(textwrap.dedent(text))
        tmp.close()
        self.addCleanup(pathlib.Path(tmp.name).unlink)
        return pathlib.Path(tmp.name)

    def test_valid(self):
        cfg = config.load(self.write(CONFIG))
        self.assertEqual(cfg["product"]["id"], "hardwave-example")
        self.assertEqual(cfg["not_checked"], [{"check": "fuzzing", "why": "No fuzz targets yet."}])

    def test_mistakes_are_caught(self):
        for broken in (CONFIG.replace('id = "hardwave-example"', 'id = "Hardwave Example"'),
                       CONFIG.replace('repository = "Dishairano/hardwave-example"', ""),
                       CONFIG.replace('output = "hardwave-example.cdx.json"', ""),
                       CONFIG.replace('npm = "ui"', ""),
                       CONFIG.replace('test_jobs = ["lint-and-test"]', "test_jobs = []")):
            with self.assertRaises(config.ConfigError):
                config.load(self.write(broken))

    def test_lines_for_the_workflow(self):
        path = self.write(CONFIG)
        out = subprocess.run([sys.executable, str(HERE / "config.py"), "sbom-cargo", str(path)],
                             capture_output=True, text=True, check=True).stdout
        self.assertEqual(out, "Cargo.toml\thardwave-example.cdx.json\tplugin-crates.cdx.json\n")
        out = subprocess.run([sys.executable, str(HERE / "config.py"), "sbom-npm", str(path)],
                             capture_output=True, text=True, check=True).stdout
        self.assertEqual(out, "ui\tui-npm.cdx.json\n")


if __name__ == "__main__":
    unittest.main()
