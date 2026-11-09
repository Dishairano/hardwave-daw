"""HTML for the Secure Code Check report and its public page.

report_html(report)  one self-contained file (styles inline, no scripts,
                     nothing loaded from elsewhere) for a supplier file.
page_html(...)       the public page at
                     hardwavestudios.com/security/releases/<product>/<version>/,
                     with its styles in style.css next to it, so it works
                     under a strict content policy (no inline style, no
                     script).
index_html(...)      the product's list of releases with evidence.

Every value from the report is escaped. Dates are shown in Amsterdam time.
"""
from __future__ import annotations

import datetime as dt
from html import escape
from zoneinfo import ZoneInfo

SITE = "https://hardwavestudios.com"
AMS = ZoneInfo("Europe/Amsterdam")

CSS = """
:root {
  --bg: #0a0a0a; --panel: #121214; --panel2: #1a1a1e; --line: rgba(255,255,255,.08); --line2: rgba(255,255,255,.14);
  --text: #fafafa; --muted: #a1a1aa; --faint: #8b8b94; --accent: #dc2626; --ok: #4ade80; --okbg: rgba(74,222,128,.1);
  --warn: #fbbf24; --warnbg: rgba(251,191,36,.1); --bad: #f87171; --badbg: rgba(248,113,113,.12);
  --na: #a1a1aa; --nabg: rgba(161,161,170,.12); --code: #0f0f12;
}
@media (prefers-color-scheme: light) {
  :root:not([data-theme="dark"]) {
    --bg: #ffffff; --panel: #fafafa; --panel2: #f1f1f3; --line: #e4e4e7; --line2: #d4d4d8;
    --text: #18181b; --muted: #52525b; --faint: #63636b; --accent: #c81e1e; --ok: #15803d; --okbg: #dcfce7;
    --warn: #a16207; --warnbg: #fef3c7; --bad: #b91c1c; --badbg: #fee2e2; --na: #52525b; --nabg: #f1f1f3; --code: #f4f4f5;
  }
}
@media print {
  :root { --bg: #fff; --panel: #fff; --panel2: #f1f1f3; --line: #d4d4d8; --line2: #d4d4d8; --text: #000; --muted: #333;
          --faint: #555; --ok: #15803d; --okbg: #fff; --warn: #a16207; --warnbg: #fff; --bad: #b91c1c; --badbg: #fff;
          --na: #333; --nabg: #fff; --code: #fff; }
  header.site { position: static; }
}
* { box-sizing: border-box; }
html { -webkit-text-size-adjust: 100%; }
body { margin: 0; background: var(--bg); color: var(--text); font: 15px/1.6 Inter, system-ui, -apple-system, "Segoe UI", sans-serif; overflow-wrap: anywhere; }
a { color: inherit; }
.wrap { max-width: 1120px; margin: 0 auto; padding: 0 16px; }
header.site { border-bottom: 1px solid var(--line); background: var(--bg); position: sticky; top: 0; z-index: 5; }
header.site .wrap { display: flex; align-items: center; gap: 24px; height: 60px; }
.logo { font-weight: 800; letter-spacing: .2em; font-size: 14px; text-decoration: none; }
.logo b { color: var(--accent); }
nav.main { display: flex; gap: 20px; font-size: 14px; color: var(--muted); }
nav.main a { text-decoration: none; } nav.main a.on { color: var(--text); }
.crumbs { font-size: 13px; color: var(--faint); margin: 28px 0 10px; }
.crumbs a { text-decoration: none; color: var(--muted); }
.hero { display: grid; grid-template-columns: 1.4fr 1fr; gap: 28px; align-items: start; padding: 8px 0 28px; border-bottom: 1px solid var(--line); }
.eyebrow { font-size: 12px; font-weight: 700; letter-spacing: .14em; text-transform: uppercase; color: var(--accent); }
h1 { font-size: 40px; line-height: 1.1; margin: 8px 0 12px; letter-spacing: -.02em; }
h2 { font-size: 22px; margin: 0 0 6px; letter-spacing: -.01em; }
h3 { margin: 26px 0 8px; font-size: 16px; }
.sub { color: var(--muted); font-size: 15px; max-width: 560px; }
.facts { display: grid; grid-template-columns: auto 1fr; gap: 6px 14px; font-size: 13.5px; margin-top: 16px; }
.facts dt { color: var(--faint); } .facts dd { margin: 0; }
.mono, code { font-family: "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: 12.5px; }
.summary { background: var(--panel); border: 1px solid var(--line); border-radius: 14px; padding: 18px; }
.summary .big { display: flex; align-items: center; gap: 10px; font-weight: 700; font-size: 17px; }
.dot { width: 10px; height: 10px; border-radius: 50%; background: var(--ok); flex: none; }
.dot.warn { background: var(--warn); }
.stats { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; margin-top: 14px; }
.stat { background: var(--panel2); border-radius: 10px; padding: 10px 12px; }
.stat b { display: block; font-size: 22px; line-height: 1.2; }
.stat span { font-size: 12px; color: var(--muted); }
.badge { display: inline-flex; margin-top: 14px; border-radius: 6px; overflow: hidden; font-weight: 600; font-size: 11.5px; }
.badge span { padding: 4px 8px; } .badge span:first-child { background: #27272a; color: #fafafa; } .badge span:last-child { background: #166534; color: #f0fdf4; }
section { padding: 36px 0; border-bottom: 1px solid var(--line); }
section.last { border-bottom: 0; }
.lead { color: var(--muted); margin: 0 0 18px; max-width: 760px; }
.notcert { display: flex; gap: 14px; background: var(--warnbg); border-radius: 12px; padding: 16px 18px; margin-top: 24px; }
.notcert .i { flex: none; width: 26px; height: 26px; border-radius: 50%; border: 2px solid var(--warn); color: var(--warn); display: grid; place-items: center; font-weight: 800; font-size: 14px; }
.notcert p { margin: 0; } .notcert b { color: var(--warn); }
.tbl { width: 100%; overflow-x: auto; border: 1px solid var(--line); border-radius: 12px; }
table { border-collapse: collapse; width: 100%; font-size: 14px; }
th, td { text-align: left; vertical-align: top; padding: 11px 14px; border-bottom: 1px solid var(--line); }
th { white-space: nowrap; background: var(--panel2); font-size: 11.5px; text-transform: uppercase; letter-spacing: .07em; color: var(--muted); font-weight: 600; }
tr:last-child td { border-bottom: 0; }
td small { display: block; color: var(--muted); font-size: 12.5px; }
.st { display: inline-flex; align-items: center; gap: 6px; font-size: 12px; font-weight: 600; padding: 2px 9px; border-radius: 999px; white-space: nowrap; }
.st.pass { background: var(--okbg); color: var(--ok); }
.st.exc { background: var(--warnbg); color: var(--warn); }
.st.fail { background: var(--badbg); color: var(--bad); }
.st.na { background: var(--nabg); color: var(--na); }
.cards { display: grid; grid-template-columns: repeat(3, 1fr); gap: 14px; }
.card { background: var(--panel); border: 1px solid var(--line); border-radius: 12px; padding: 16px; display: flex; flex-direction: column; gap: 6px; }
.card h3 { margin: 0; font-size: 15px; }
.card p { margin: 0; color: var(--muted); font-size: 13.5px; }
.card .dl { margin-top: auto; padding-top: 10px; display: flex; gap: 8px; flex-wrap: wrap; }
.btn { display: inline-block; font-size: 13px; font-weight: 600; padding: 7px 12px; border-radius: 8px; text-decoration: none; border: 1px solid var(--line2); }
.btn.primary { background: var(--accent); border-color: var(--accent); color: #fff; }
pre { background: var(--code); border: 1px solid var(--line); border-radius: 10px; padding: 12px 14px; overflow-x: auto; font: 500 12.5px/1.6 "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; margin: 8px 0 0; white-space: pre; }
pre .c { color: var(--faint); }
.steps { counter-reset: s; list-style: none; padding: 0; margin: 0; display: grid; gap: 18px; }
.steps > li { counter-increment: s; position: relative; padding-left: 40px; }
.steps > li::before { content: counter(s); position: absolute; left: 0; top: 0; width: 26px; height: 26px; border-radius: 50%; background: var(--panel2); border: 1px solid var(--line2); display: grid; place-items: center; font-weight: 700; font-size: 13px; }
.steps b { display: block; }
.steps p { margin: 2px 0 0; color: var(--muted); font-size: 14px; }
.hist { display: flex; gap: 10px; flex-wrap: wrap; }
.hist a { text-decoration: none; background: var(--panel); border: 1px solid var(--line); border-radius: 10px; padding: 10px 14px; font-size: 13.5px; }
.hist a b { display: block; } .hist a span { color: var(--muted); font-size: 12px; }
.hist a.on { border-color: var(--accent); }
.findings { margin: 10px 0 0; padding-left: 18px; color: var(--muted); font-size: 13px; }
.hero > *, .cards > *, .steps > li, .facts > * { min-width: 0; }
footer { padding: 36px 0 56px; color: var(--faint); font-size: 13px; }
footer .wrap { display: flex; gap: 18px; flex-wrap: wrap; }
@media (max-width: 860px) {
  .hero { grid-template-columns: 1fr; }
  .cards { grid-template-columns: 1fr; }
  nav.main { display: none; }
  h1 { font-size: 32px; }
}
@media (max-width: 520px) {
  h1 { font-size: 27px; }
  table { font-size: 13px; } th, td { padding: 9px 10px; }
  .facts { grid-template-columns: 1fr; gap: 0; } .facts dd { margin-bottom: 6px; }
}
"""

STATUS_CLASS = {"passed": "pass", "written": "pass", "signed": "pass", "failed": "fail"}


# --------------------------------------------------------------- helpers

def parse_time(value: str | None) -> dt.datetime | None:
    if not value:
        return None
    try:
        return dt.datetime.fromisoformat(value.replace("Z", "+00:00")).astimezone(AMS)
    except ValueError:
        return None


def long_date(value: str | None) -> str:
    when = parse_time(value)
    return f"{when.day} {when:%B %Y}" if when else "not recorded"


def short_date(value: str | None) -> str:
    when = parse_time(value)
    return f"{when.day} {when:%b %Y}" if when else "not recorded"


def date_time(value: str | None) -> str:
    when = parse_time(value)
    return f"{when.day} {when:%b %Y, %H:%M} {when.tzname()}" if when else "not recorded"


def plain_date(value: str | None) -> str:
    """A YYYY-MM-DD date as '17 Oct 2026'."""
    if not value:
        return "not recorded"
    try:
        day = dt.date.fromisoformat(value)
    except ValueError:
        return escape(value)
    return f"{day.day} {day:%b %Y}"


def size(n: int) -> str:
    if n >= 1 << 20:
        return f"{n / (1 << 20):.1f} MB"
    if n >= 1 << 10:
        return f"{n / (1 << 10):.0f} KB"
    return f"{n} bytes"


def pill(check: dict) -> str:
    status = check["status"]
    cls = STATUS_CLASS.get(status, "na")
    if status == "passed" and (check.get("exceptions") or check.get("lower")):
        cls = "exc" if check.get("exceptions") else "pass"
    return f'<span class="st {cls}">{escape(check["status_text"])}</span>'


def label(report: dict) -> str:
    if report.get("release"):
        return f"v{report['release']['version']}"
    return f"commit {report['source']['commit'][:8]}"


def base_name(report: dict) -> str:
    tail = report["release"]["version"] if report.get("release") else report["source"]["commit"][:8]
    return f"{report['product']['id']}-{tail}-secure-code-check"


def repo_url(report: dict) -> str:
    return f"https://github.com/{report['product']['repository']}"


# -------------------------------------------------------------- sections

def hero(report: dict, eyebrow: str, badge: bool) -> str:
    product, src = report["product"], report["source"]
    release = report.get("release")
    summary = report["summary"]
    passed = report["result"] == "passed"
    facts = []
    if release:
        facts.append(("Released", f"{long_date(release.get('released_at'))}, {escape(product['platforms'])}"))
        facts.append(("Source", f'<a href="{escape(repo_url(report))}">github.com/{escape(product["repository"])}</a>, '
                                f'tag {escape(release["tag"])}'))
    else:
        facts.append(("Source", f'<a href="{escape(repo_url(report))}">github.com/{escape(product["repository"])}</a>, '
                                f'{escape(src.get("ref", ""))}'))
    facts.append(("Commit", f'<a class="mono" href="{escape(repo_url(report))}/commit/{escape(src["commit"])}">'
                            f'{escape(src["commit"][:8])}</a>'))
    runner = "GitHub-hosted runners" if src.get("runner", "github-hosted") == "github-hosted" else "self-hosted runners"
    facts.append(("Checks ran", f"{date_time(report.get('scanned_at'))}, on {runner}"))
    if src.get("run_url"):
        facts.append(("Workflow run", f'<a href="{escape(src["run_url"])}">{escape(src["run_url"].split("/")[-1])}</a>'))
    fact_html = "".join(f"<dt>{k}</dt><dd>{v}</dd>" for k, v in facts)
    big = "All checks passed" if passed else f"{summary['passed']} of {summary['checks']} checks passed"
    when = short_date(release.get("released_at") if release else report.get("scanned_at"))
    badge_html = (f'<div class="badge"><span>Secure Code Check</span><span>{escape(label(report))} · {when}</span></div>'
                  if badge else "")
    title = f"{escape(product['name'])} {escape(label(report))}"
    sub = ("The security checks run on this release before it was published, what went into it, and the "
           "SHA-256 of every file, in a report signed by our build. You can check every line yourself; "
           "the commands are at the bottom.") if release else (
           "A preview of the checks on one commit. Release reports are signed and list every release file.")
    return f"""
  <div class="hero">
    <div>
      <div class="eyebrow">{escape(eyebrow)}</div>
      <h1>{title}</h1>
      <p class="sub">{sub}</p>
      <dl class="facts">{fact_html}</dl>
    </div>
    <aside class="summary">
      <div class="big"><span class="dot{'' if passed else ' warn'}"></span>{big}</div>
      <div class="stats">
        <div class="stat"><b>{summary['checks']}</b><span>checks run</span></div>
        <div class="stat"><b>{summary['checks'] - summary['passed']}</b><span>checks not passed</span></div>
        <div class="stat"><b>{summary['advisory_exceptions']}</b><span>recorded exceptions, reasons below</span></div>
        <div class="stat"><b>{summary['crates']}</b><span>Rust crates checked in the DAW</span></div>
      </div>
      {badge_html}
    </aside>
  </div>
  <div class="notcert"><div class="i">i</div><p><b>This is not a certificate.</b> {escape(report['notice'].split('. ', 1)[1])}</p></div>"""


def results(report: dict, public: bool) -> str:
    rows = []
    for check in report["checks"]:
        extra = ""
        if check["id"] == "static-analysis" and check.get("lower"):
            extra = f"<small>{check['lower']} lower findings for review, listed in the report</small>" if public else \
                    f"<small>{check['lower']} lower findings for review, listed below</small>"
        rows.append(f"<tr><td><b>{escape(check['name'])}</b><small>{escape(check['tool'])}</small></td>"
                    f"<td>{escape(check['what'])}{extra}</td><td>{pill(check)}</td></tr>")
    exc = report["exceptions"]
    exc_rows = "".join(
        f"<tr><td class=\"mono\">{escape(a['id'])}"
        f"{'<small>yanked crate</small>' if a['kind'] != 'advisory' else ''}</td>"
        f"<td>{escape(a['reason'])}</td>"
        f"<td>{plain_date(a['recorded']['date'])}</td></tr>"
        for a in exc["advisories"])
    licences = exc["licences"]
    licence_line = (f'<p class="lead">Licence exceptions: {licences["count"]}, each with its reason in '
                    f'<a href="{escape(licences["source"])}"><span class="mono">deny.toml</span></a>.</p>'
                    if licences["count"] else "")
    findings = ""
    if not public:
        sa = next((c for c in report["checks"] if c["id"] == "static-analysis"), {})
        items = "".join(f"<li><span class=\"mono\">{escape(f['path'])}:{escape(str(f['line']))}</span> "
                        f"{escape(f['rule'])} ({escape(f['severity'].lower())})</li>"
                        for f in sa.get("findings", []))
        if items:
            findings = (f"<h3>Static analysis findings for review</h3><p class=\"lead\">Below high severity, so "
                        f"they do not stop the release; each is looked at by a developer.</p>"
                        f"<ul class=\"findings\">{items}</ul>")
    return f"""
  <section>
    <h2>Results per check</h2>
    <p class="lead">Every check runs on each push and again on the release tag. A failed check stops the release: no build, no installer.</p>
    <div class="tbl"><table>
      <thead><tr><th>Check</th><th>What was checked</th><th>Result</th></tr></thead>
      <tbody>{''.join(rows)}</tbody>
    </table></div>
    <h3>Recorded exceptions</h3>
    <p class="lead">An exception is allowed only with a written reason, and is looked at again whenever the dependency that brings it in updates. "Recorded" is the day it was added to <a href="{escape(licences['source'])}"><span class="mono">deny.toml</span></a>.</p>
    <div class="tbl"><table>
      <thead><tr><th>Advisory</th><th>Why it cannot be reached</th><th>Recorded</th></tr></thead>
      <tbody>{exc_rows or '<tr><td colspan="3">None</td></tr>'}</tbody>
    </table></div>
    {licence_line}
    {findings}
  </section>"""


def sbom_table(report: dict, link: bool) -> str:
    if not report["sbom"]:
        return ""
    rows = []
    for s in report["sbom"]:
        name = escape(s["file"])
        shown = f'<a href="{name}">{name}</a>' if link else name
        rows.append(f'<tr><td>{escape(s["part"])}<small class="mono">{shown}</small></td>'
                    f'<td>{s["components"]}</td><td class="mono">{escape(s["sha256"])}</td></tr>')
    rows = "".join(rows)
    return f"""
  <section>
    <h2>What went into it (SBOM)</h2>
    <p class="lead">Every component in what ships, with version and licence, in CycloneDX {escape(report['sbom'][0]['spec_version'])} JSON. Rust crates for the Windows build with every feature on; npm packages that the interface carries at run time.</p>
    <div class="tbl"><table>
      <thead><tr><th>Part</th><th>Components</th><th>SHA-256</th></tr></thead>
      <tbody>{rows}</tbody>
    </table></div>
  </section>"""


def files_table(report: dict) -> str:
    if not report.get("release"):
        return ""
    rows = []
    for f in report["release_files"]:
        sig = f.get("windows_signature")
        if sig:
            cls = "pass" if sig.get("status") == "Valid" else "fail"
            sig_html = f'<span class="st {cls}">{escape(sig.get("status") or "")}</span>'
            if sig.get("signer"):
                sig_html += f"<small>{escape(sig['signer'])}</small>"
        else:
            sig_html = '<span class="st na">Not an executable</span>' if not f["name"].lower().endswith(".exe") \
                else '<span class="st na">Not verified</span>'
        rows.append(f"<tr><td class=\"mono\">{escape(f['name'])}<small>{f['size']:,} bytes</small></td>"
                    f"<td class=\"mono\">{escape(f['sha256'])}</td><td>{sig_html}</td></tr>")
    tag = report["release"]["tag"]
    return f"""
  <section>
    <h2>Release files</h2>
    <p class="lead">Every file on the GitHub release for {escape(tag)}, built by <span class="mono">.github/workflows/release.yml</span> from commit <span class="mono">{escape(report['source']['commit'][:8])}</span> on GitHub-hosted runners. The hashes are part of the signed report, so a file that matches is the one our build published. GitHub build attestations (SLSA provenance) are not part of this release yet; this page will say so when they are.</p>
    <div class="tbl"><table>
      <thead><tr><th>File</th><th>SHA-256</th><th>Windows signature</th></tr></thead>
      <tbody>{''.join(rows) or '<tr><td colspan="3">No files recorded</td></tr>'}</tbody>
    </table></div>
  </section>"""


def iso_table(report: dict) -> str:
    rows = "".join(f"<tr><td><b>{escape(c['control'])}</b> {escape(c['title'])}</td><td>{escape(c['evidence'])}</td></tr>"
                   for c in report["iso_controls"])
    return f"""
  <section>
    <h2>ISO 27001:2022 controls this evidence supports</h2>
    <p class="lead">Each check is listed only against an Annex A control it gives evidence for. "Supports" means this release shows the control working in our code and build. Whether a control is met is decided by an auditor, for a whole organisation. Not listed on purpose: 8.26 and 8.27 need design documents we do not publish yet.</p>
    <div class="tbl"><table>
      <thead><tr><th>Annex A control</th><th>Evidence in this report</th></tr></thead>
      <tbody>{rows}</tbody>
    </table></div>
  </section>"""


def verify_commands(report: dict) -> str:
    base = base_name(report)
    signing = report["signing"]
    first_exe = next((f["name"] for f in report["release_files"] if f["name"].lower().endswith(".exe")),
                     "HardwaveDAW-Setup.exe")
    ident, ns = escape(signing["identity"]), escape(signing["namespace"])
    return f"""
    <ol class="steps">
      <li><b>Get our public key</b>
        <p>Download <a href="allowed_signers"><span class="mono">allowed_signers</span></a> here and compare it with the copy in the source repository: <a href="{escape(repo_url(report))}/blob/{escape(report['source']['commit'])}/security/secure-code-check/allowed_signers"><span class="mono">security/secure-code-check/allowed_signers</span></a>. Key fingerprint: <span class="mono">{escape(signing.get('fingerprint', ''))}</span>.</p>
      </li>
      <li><b>Check the report is the one our build signed</b>
        <p>Needs only OpenSSH 8.1 or newer, which macOS, Linux and Windows 10 and 11 include. Use it on a copy that reached you by email too.</p>
<pre>ssh-keygen -Y verify -f allowed_signers \\
  -I {ident} -n {ns} \\
  -s {escape(base)}.json.sig &lt; {escape(base)}.json

<span class="c"># Windows (Command Prompt keeps the bytes exactly as they are)</span>
cmd /c "ssh-keygen -Y verify -f allowed_signers -I {ident} -n {ns} -s {escape(base)}.json.sig &lt; {escape(base)}.json"</pre>
        <p>It prints <span class="mono">Good "{ns}" signature for {ident}</span>. The same works for the HTML report with its own <span class="mono">.html.sig</span>.</p>
      </li>
      <li><b>Compare a file's hash with the report</b>
        <p>The hash must match the one in the release files table; the SBOM files work the same way.</p>
<pre><span class="c"># macOS or Linux</span>
shasum -a 256 {escape(first_exe)}

<span class="c"># Windows PowerShell</span>
Get-FileHash .\\{escape(first_exe)} -Algorithm SHA256</pre>
      </li>
      <li><b>Check the Windows signature</b>
<pre>Get-AuthenticodeSignature .\\{escape(first_exe)}</pre>
      </li>
    </ol>"""


def page_shell(title: str, description: str, body: str, inline_css: bool) -> str:
    style = f"<style>{CSS}</style>" if inline_css else '<link rel="stylesheet" href="style.css">'
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{escape(title)}</title>
<meta name="description" content="{escape(description)}">
{style}
</head>
<body>
{body}
</body>
</html>
"""


SITE_HEADER = f"""
<header class="site">
  <div class="wrap">
    <a class="logo" href="{SITE}/">HARD<b>WAVE</b></a>
    <nav class="main">
      <a href="{SITE}/plugins">Plug-ins</a><a href="{SITE}/daw">DAW</a><a href="{SITE}/learn">Learn</a><a href="{SITE}/pricing">Pricing</a><a class="on" href="{SITE}/security">Security</a>
    </nav>
  </div>
</header>"""

SITE_FOOTER = f"""
<footer>
  <div class="wrap">
    <span>Hardwave Studios</span><a href="{SITE}/security">Security</a><a href="{SITE}/privacy">Privacy</a><a href="{SITE}/terms">Terms</a>
  </div>
</footer>"""


# ------------------------------------------------------------- documents

def report_html(report: dict) -> str:
    kind = "release report" if report.get("release") else "preview, not a release report"
    signing = report["signing"]
    body = f"""<main class="wrap">
  {hero(report, f"Secure Code Check · {kind}", badge=False)}
  {results(report, public=False)}
  {sbom_table(report, link=False)}
  {files_table(report)}
  {iso_table(report)}
  <section class="last">
    <h2>Signature</h2>
    <p class="lead">{'This file and its JSON twin are signed by our build with an Ed25519 key that only the release workflow holds. The public key is in the source repository and on the public page.' if report.get('release') else 'Release reports are always signed by our build. A preview of a push or pull request is signed too when the run holds the key: look for the .sig file next to this one.'}</p>
    <dl class="facts">
      <dt>Method</dt><dd>{escape(signing['method'])}</dd>
      <dt>Identity</dt><dd class="mono">{escape(signing['identity'])}</dd>
      <dt>Namespace</dt><dd class="mono">{escape(signing['namespace'])}</dd>
      <dt>Public key</dt><dd class="mono">{escape(signing.get('public_key', ''))}</dd>
      <dt>Fingerprint</dt><dd class="mono">{escape(signing.get('fingerprint', ''))}</dd>
    </dl>
    {verify_commands(report) if report.get('release') else ''}
  </section>
</main>
<footer><div class="wrap"><span>Hardwave Studios</span><span>Generated {date_time(report['generated_at'])} by the build, schema {escape(report['schema'])}</span></div></footer>"""
    title = f"{report['product']['name']} {label(report)} Secure Code Check"
    return page_shell(title, f"Security evidence for {report['product']['name']} {label(report)}", body, inline_css=True)


def downloads(report: dict) -> str:
    base = base_name(report)
    sboms = "".join(f'<a class="btn" href="{escape(s["file"])}">{escape(s["part"])}</a>' for s in report["sbom"])
    total = sum(s["components"] for s in report["sbom"])
    return f"""
  <section>
    <h2>Downloads</h2>
    <p class="lead">The report is signed by our build. A copy forwarded by email can still be checked with the commands further down.</p>
    <div class="cards">
      <div class="card">
        <h3>Release report</h3>
        <p>All results, exceptions, hashes and the ISO control table, as one document for your supplier file, and the same as JSON for your tools.</p>
        <div class="dl"><a class="btn primary" href="{escape(base)}.html">HTML</a><a class="btn" href="{escape(base)}.json">JSON</a><a class="btn" href="{escape(base)}.html.sig">HTML signature</a><a class="btn" href="{escape(base)}.json.sig">JSON signature</a></div>
      </div>
      <div class="card">
        <h3>SBOM</h3>
        <p>Every component in what ships with version and licence. CycloneDX JSON, {total} components in {len(report['sbom'])} files.</p>
        <div class="dl">{sboms}</div>
      </div>
      <div class="card">
        <h3>Signing key</h3>
        <p>The public half of the key our build signs reports with. Fingerprint <span class="mono">{escape(report['signing'].get('fingerprint', ''))}</span>.</p>
        <div class="dl"><a class="btn" href="allowed_signers">allowed_signers</a><a class="btn" href="signing_key.pub">signing_key.pub</a></div>
      </div>
    </div>
  </section>"""


def history_links(report: dict, history: list) -> str:
    current = report["release"]["version"]
    links = []
    seen = set()
    for item in [{"version": current, "released_at": report["release"].get("released_at")}] + list(history):
        if item["version"] in seen:
            continue
        seen.add(item["version"])
        cls = ' class="on"' if item["version"] == current else ""
        links.append(f'<a{cls} href="../{escape(item["version"])}/"><b>v{escape(item["version"])}</b>'
                     f'<span>{short_date(item.get("released_at"))}</span></a>')
        if len(links) >= 6:
            break
    links.append('<a href="../"><b>All releases</b><span>with evidence</span></a>')
    return "".join(links)


def page_html(report: dict, history: list) -> str:
    product = report["product"]
    version = report["release"]["version"]
    body = f"""{SITE_HEADER}
<main class="wrap">
  <div class="crumbs"><a href="{SITE}/security">Security</a> / <a href="../../">Release evidence</a> / <a href="../">{escape(product['name'])}</a> / v{escape(version)}</div>
  {hero(report, "Secure Code Check · release evidence", badge=True)}
  {results(report, public=True)}
  {downloads(report)}
  {files_table(report)}
  {iso_table(report)}
  <section>
    <h2>Verify it yourself</h2>
    <p class="lead">You do not have to trust this page. It is made from the signed report; these steps check the report and the files you downloaded against our build's signature.</p>
    {verify_commands(report)}
  </section>
  <section class="last">
    <h2>Other releases</h2>
    <p class="lead">Every release since Secure Code Check started has its own page.</p>
    <div class="hist">{history_links(report, history)}</div>
  </section>
</main>
{SITE_FOOTER}"""
    title = f"{product['name']} v{version} security evidence"
    return page_shell(title, f"Security checks, SBOM and signed report for {product['name']} v{version}.",
                      body, inline_css=False)


def index_html(product: dict, releases: list) -> str:
    links = "".join(f'<a href="{escape(r["version"])}/"><b>v{escape(r["version"])}</b>'
                    f'<span>{short_date(r.get("released_at"))}</span></a>' for r in releases)
    body = f"""{SITE_HEADER}
<main class="wrap">
  <div class="crumbs"><a href="{SITE}/security">Security</a> / <a href="../">Release evidence</a> / {escape(product['name'])}</div>
  <section class="last">
    <div class="eyebrow">Secure Code Check · release evidence</div>
    <h1>{escape(product['name'])}</h1>
    <p class="lead">Each release with its security checks, SBOM and signed report. Evidence mapped to ISO 27001:2022 controls, for your own supplier assessment; not a certificate.</p>
    <div class="hist">{links}</div>
  </section>
</main>
{SITE_FOOTER}"""
    return page_shell(f"{product['name']} release evidence",
                      f"Security evidence for every {product['name']} release.", body, inline_css=False)
