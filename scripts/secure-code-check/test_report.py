"""Tests for the Secure Code Check report and page.

Run: python3 -m unittest discover -s scripts/secure-code-check -p 'test_*.py'

They build a release report from made-up scan results against this
repository's real deny.toml and lock files, then the public page, and
check what a buyer would rely on: the result, the exceptions, the
hashes, and that nothing claims a certificate or compliance.
"""
import hashlib
import html
import json
import pathlib
import re
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(HERE))
import page  # noqa: E402
import report  # noqa: E402

GREEN = {"result": "success", "outputs": {}}


def scan_needs(**overrides):
    needs = {job: dict(GREEN) for job in (
        "rust-dependencies", "launcher-dependencies", "ui-dependencies", "ui-content-policy",
        "secrets", "static-analysis", "fuzz", "sbom")}
    needs["secrets"] = {"result": "success", "outputs": {"commits": "4321"}}
    needs["static-analysis"] = {"result": "success", "outputs": {
        "high": "0", "medium": "2", "low": "1", "rules": "180", "files": "590"}}
    needs["fuzz"] = {"result": "success", "outputs": {"targets": "aiff flp osc", "seconds": "60"}}
    needs.update(overrides)
    return needs


def bom(n):
    return {"bomFormat": "CycloneDX", "specVersion": "1.5",
            "components": [{"name": f"c{i}", "components": [{"name": "nested"}] if i == 0 else []}
                           for i in range(n)]}


class ReportTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = pathlib.Path(self.tmp.name)
        self.scan, self.assets, self.out, self.site = (root / d for d in ("scc", "assets", "out", "site"))
        (self.scan / "sbom").mkdir(parents=True)
        self.assets.mkdir()
        for name in ("app-crates.cdx.json", "ui-npm.cdx.json"):
            (self.scan / "sbom" / name).write_text(json.dumps(bom(3)))
        (self.scan / "semgrep.json").write_text(json.dumps({"results": [
            {"check_id": "rust.lang.security.temp-dir", "path": "src/<x>.rs", "start": {"line": 3},
             "extra": {"severity": "WARNING"}}]}))
        (self.scan / "scan-meta.json").write_text(json.dumps({
            "scanned_at": "2026-11-02T15:01:00+00:00", "commit": "7fa4ac5d" + "0" * 32,
            "ref": "refs/tags/v0.266.37", "repository": "Dishairano/hardwave-daw",
            "run_url": "https://github.com/Dishairano/hardwave-daw/actions/runs/1", "runner": "github-hosted"}))
        self.exe = self.assets / "HardwaveDAW-Setup.exe"
        self.exe.write_bytes(b"MZ installer")
        (self.assets / "latest.json").write_text("{}")
        (self.assets / "x.exe.sig").write_text("sig")
        # An evidence file from an earlier attempt is not a release file.
        (self.assets / "hardwave-daw-0.266.37-secure-code-check.json").write_text("{}")
        self.authenticode = root / "authenticode.json"
        self.authenticode.write_text(json.dumps([{
            "file": "HardwaveDAW-Setup.exe", "sha256": hashlib.sha256(b"MZ installer").hexdigest().upper(),
            "status": "Valid", "signer": "CN=Hardwave Studios"}]))
        self.release_needs = root / "release-needs.json"
        self.release_needs.write_text(json.dumps({"lint-and-test": GREEN, "ui-tests": GREEN,
                                                  "build": GREEN, "secure-code-check": GREEN}))

    def tearDown(self):
        self.tmp.cleanup()

    def build(self, needs=None):
        (self.scan / "scan-needs.json").write_text(json.dumps(needs or scan_needs()))
        report.main(["--repo", str(REPO), "--scan-dir", str(self.scan), "--out-dir", str(self.out),
                     "--tag", "v0.266.37", "--released-at", "2026-11-02T15:56:00Z",
                     "--release-needs", str(self.release_needs), "--assets-dir", str(self.assets),
                     "--authenticode", str(self.authenticode)])
        return json.loads((self.out / "hardwave-daw-0.266.37-secure-code-check.json").read_text())

    def test_green_release(self):
        data = self.build()
        self.assertEqual(data["result"], "passed")
        self.assertEqual(data["summary"]["checks"], 11)
        names = [f["name"] for f in data["release_files"]]
        self.assertEqual(names, ["HardwaveDAW-Setup.exe", "latest.json", "x.exe.sig"])
        exe = data["release_files"][0]
        self.assertEqual(exe["sha256"], hashlib.sha256(b"MZ installer").hexdigest())
        self.assertEqual(exe["windows_signature"]["status"], "Valid")
        self.assertEqual(sum(s["components"] for s in data["sbom"]), 8)
        self.assertTrue((self.out / "hardwave-daw-0.266.37-sbom-app-crates.cdx.json").exists())

    def test_exceptions_come_from_deny_toml(self):
        data = self.build()
        ids = [a["id"] for a in data["exceptions"]["advisories"]]
        self.assertIn("RUSTSEC-2026-0429", ids)
        for advisory in data["exceptions"]["advisories"]:
            self.assertTrue(advisory["reason"], advisory["id"])
        self.assertGreaterEqual(data["exceptions"]["licences"]["count"], 1)
        html = (self.out / "hardwave-daw-0.266.37-secure-code-check.html").read_text()
        # Licence exceptions are a count and a link, never the crate names.
        self.assertNotIn("rusty_link", html)

    def test_a_failed_check_is_reported(self):
        data = self.build(scan_needs(fuzz={"result": "failure", "outputs": {}}))
        self.assertEqual(data["result"], "not_all_passed")
        self.assertIn("fuzzing", data["summary"]["not_passed"])
        html = (self.out / "hardwave-daw-0.266.37-secure-code-check.html").read_text()
        self.assertIn("10 of 11 checks passed", html)

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

    def test_html_is_escaped(self):
        self.build()
        html = (self.out / "hardwave-daw-0.266.37-secure-code-check.html").read_text()
        self.assertIn("src/&lt;x&gt;.rs", html)
        self.assertNotIn("src/<x>.rs", html)

    def test_page(self):
        self.build()
        base = self.out / "hardwave-daw-0.266.37-secure-code-check"
        for suffix in (".json.sig", ".html.sig"):
            pathlib.Path(str(base) + suffix).write_text("sig")
        history = self.out / "history.json"
        history.write_text(json.dumps([{"version": "0.266.38", "released_at": "2026-11-10T10:00:00Z"}]))
        self.assertEqual(page.main(["--report", str(base) + ".json", "--files", str(self.out),
                                    "--repo", str(REPO), "--site-dir", str(self.site),
                                    "--history", str(history)]), 0)
        out = self.site / "security" / "releases" / "hardwave-daw" / "0.266.37"
        html = (out / "index.html").read_text()
        for name in ("style.css", "badge.svg", "allowed_signers", "signing_key.pub",
                     "hardwave-daw-0.266.37-sbom-ui-npm.cdx.json"):
            self.assertTrue((out / name).exists(), name)
        self.assertNotIn("<script", html)
        self.assertNotIn("<style", html)
        self.assertIn('href="../0.266.38/"', html)
        self.assertTrue((out.parent / "index.html").exists())

    def test_no_certificate_or_compliance_claims(self):
        self.build()
        base = self.out / "hardwave-daw-0.266.37-secure-code-check"
        for path in (pathlib.Path(str(base) + ".html"), pathlib.Path(str(base) + ".json")):
            text = html.unescape(path.read_text()) if path.suffix == ".html" else \
                json.dumps(json.loads(path.read_text()), ensure_ascii=False)
            text = text.replace(report.NOTICE, "")
            text = text.replace(report.NOTICE.split(". ", 1)[1], "").replace("This is not a certificate.", "")
            text = text.replace("not a certificate", "")
            for word in ("certificate", "certified", "compliant", "compliance"):
                self.assertIsNone(re.search(word, text, re.I), f"{path.name} says {word!r}")


if __name__ == "__main__":
    unittest.main()
