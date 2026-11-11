#!/usr/bin/env python3
"""Secure Code Check: the public page for one release, as static files.

Usage:
  page.py --report out/<product>-<version>-secure-code-check.json \\
          --files out --repo . --site-dir site [--history history.json]

Writes, ready to publish as they are:

  site/security/releases/<product>/<version>/
      index.html        the page (no script, styles in style.css)
      style.css
      badge.svg         "Secure Code Check | v<version> · <date>"
      <report>.html/.json and their .sig, the SBOM files
      allowed_signers, signing_key.pub
  site/security/releases/<product>/index.html
      the product's releases with evidence, newest first

history.json is a list of {"version", "released_at"} for earlier
releases that have a report; the release workflow reads it from GitHub.
Nothing is fetched from elsewhere when the page is viewed.
"""
from __future__ import annotations

import argparse
import json
import pathlib
import shutil
import sys
from html import escape

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import render  # noqa: E402


def badge_svg(text_right: str) -> str:
    left = "Secure Code Check"
    lw, rw = 7 * len(left) + 16, int(6.6 * len(text_right)) + 16
    return f"""<svg xmlns="http://www.w3.org/2000/svg" width="{lw + rw}" height="22" role="img" aria-label="{escape(left)}: {escape(text_right)}">
<title>{escape(left)}: {escape(text_right)}</title>
<rect width="{lw}" height="22" rx="4" fill="#27272a"/><rect x="{lw - 4}" width="{rw + 4}" height="22" rx="4" fill="#166534"/>
<g fill="#fff" font-family="Inter,system-ui,sans-serif" font-size="11.5" font-weight="600">
<text x="8" y="15">{escape(left)}</text><text x="{lw + 8}" y="15" fill="#f0fdf4">{escape(text_right)}</text></g>
</svg>
"""


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--report", required=True)
    parser.add_argument("--files", required=True, help="folder with the report, signatures and SBOMs")
    parser.add_argument("--repo", default=".")
    parser.add_argument("--site-dir", required=True)
    parser.add_argument("--history")
    args = parser.parse_args(argv)

    report = json.loads(pathlib.Path(args.report).read_text())
    if not report.get("release"):
        parser.error("the public page is made from a release report, not a preview")
    product, version = report["product"], report["release"]["version"]
    history = json.loads(pathlib.Path(args.history).read_text()) if args.history else []
    history = [h for h in history if h.get("version") != version]

    product_dir = pathlib.Path(args.site_dir) / "security" / "releases" / product["id"]
    out = product_dir / version
    out.mkdir(parents=True, exist_ok=True)

    files = pathlib.Path(args.files)
    base = render.base_name(report)
    wanted = [f"{base}.json", f"{base}.html", f"{base}.json.sig", f"{base}.html.sig"]
    wanted += [s["file"] for s in report["sbom"]]
    missing = [name for name in wanted if not (files / name).is_file()]
    if missing:
        print(f"page.py: missing {', '.join(missing)}", file=sys.stderr)
        return 1
    for name in wanted:
        shutil.copyfile(files / name, out / name)
    keys = pathlib.Path(args.repo) / "security" / "secure-code-check"
    for name in ("allowed_signers", "signing_key.pub"):
        shutil.copyfile(keys / name, out / name)

    (out / "style.css").write_text(render.CSS.lstrip())
    (out / "index.html").write_text(render.page_html(report, history))
    when = render.short_date(report["release"].get("released_at"))
    (out / "badge.svg").write_text(badge_svg(f"v{version} · {when}"))

    releases = [{"version": version, "released_at": report["release"].get("released_at")}] + history
    (product_dir / "index.html").write_text(render.index_html(product, releases))
    (product_dir / "style.css").write_text(render.CSS.lstrip())
    print(f"page.py: wrote {out} ({len(list(out.iterdir()))} files) and {product_dir / 'index.html'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
