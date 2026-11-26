#!/usr/bin/env python3
"""The DAW's two update feeds, checked and written for one release.

The DAW updater signing key changed after incident 2026-11-12. An installed
DAW only accepts an update signed with the key built into it, so releases
are chained through v0.266.47, the bridge release:

  latest.json     read by every DAW up to v0.266.46. They trust only the
                  old key. Up to v0.266.47 it is this release's own feed.
                  From v0.266.48 on it is frozen: v0.266.47's own
                  latest.json, byte for byte, so those copies install
                  v0.266.47 (old key) and carry on from there.
  latest-v2.json  read from v0.266.47 on (the endpoint in tauri.conf.json).
                  v0.266.47's copy is the same as its latest.json (old key:
                  the GitHub secret switches after v0.266.47 is out). From
                  v0.266.48 on it is signed with the key in tauri.conf.json.

Tauri does not write either feed (includeUpdaterJson is off in release.yml):
both are built here from the release's own files and .sig files, in the
shape tauri-action wrote them, with the release's publication time as
pub_date, so a re-run writes the same bytes.

So a DAW that skipped v0.266.47 is offered v0.266.47 by latest.json, and
the newest release by latest-v2.json once it runs v0.266.47: no manual
reinstall on Windows. Nothing here falls back: a feed that does not follow
these rules stops the release, and latest.json never carries anything that
is not signed with the old key.

The launcher (installer/src-tauri/src/install.rs) moves with the feeds: up to
v0.266.47 it trusts the old key and finds the installer through the GitHub
API; after it, it trusts the key in tauri.conf.json and reads latest-v2.json.

Everything here works on local files. scripts/updater-feeds.sh downloads
and uploads with gh; the release workflow runs that. Commands:

  plan --tag vX.Y.Z --conf src-tauri/tauri.conf.json --launcher installer/src-tauri/src/install.rs
      Checks tauri.conf.json and the launcher against the rules for this
      version and prints the step output frozen=true|false (latest.json is
      the frozen copy of v0.266.47's).
  bridge-tag
      Prints the bridge release's tag.
  signing-key --tag vX.Y.Z --conf ... --sig FILE
      FILE is a .sig made with the signing secret of the release run.
      Refuses when the secret is not the key this version must carry.
  frozen --conf ... --feed FILE --sigs DIR --bridge-conf FILE --out FILE
      Checks v0.266.47's latest.json, the .sig files of that release and
      v0.266.47's tauri.conf.json, then writes the frozen latest.json.
  feeds --tag vX.Y.Z --conf ... --sigs DIR --release FILE --out DIR
        [--bridge-feed FILE --bridge-sigs DIR --bridge-conf FILE]
      Writes this release's latest-v2.json and latest.json into DIR.
      FILE is `gh release view --json assets,body,publishedAt,createdAt`.
      The bridge arguments (v0.266.47's latest.json, .sig files and
      tauri.conf.json) are needed from v0.266.48 on.
"""
import argparse
import base64
import binascii
import json
import pathlib
import re
import sys
import urllib.parse

REPO = "Dishairano/hardwave-daw"
DOWNLOAD = f"https://github.com/{REPO}/releases/download/"
V2_ENDPOINT = f"https://github.com/{REPO}/releases/latest/download/latest-v2.json"
# What the launcher asks for the installer up to v0.266.47 (and on macOS and
# Linux after it): the latest release on the GitHub API.
LAUNCHER_API = f"https://api.github.com/repos/{REPO}/releases/latest"

# The last release signed with the old key and the first that reads
# latest-v2.json. Every DAW up to v0.266.46 reaches the new key through it.
BRIDGE = (0, 266, 47)
BRIDGE_TAG = "v0.266.47"

# The public half of the old key, stolen in incident 2026-11-12. Every DAW up
# to v0.266.46 trusts only this one. Nothing after v0.266.47 is signed with it.
OLD_PUBKEY_FILE = pathlib.Path(__file__).resolve().parent.parent / "src-tauri" / "updater-key-until-v0.266.47.pub"

# The updater files a Windows release carries, and the feed entry each one
# fills, in the order and shape tauri-action wrote them up to v0.266.46:
# `windows-x86_64` is the archive a DAW falls back to when it does not know it
# was installed with NSIS, `windows-x86_64-nsis` the bare installer.
UPDATER_FILES = (
    ("windows-x86_64", "_x64-setup.nsis.zip"),
    ("windows-x86_64-nsis", "_x64-setup.exe"),
)

# Signature files on a release that are not updater files: the Secure Code
# Check report signatures (release-evidence.yml).
NOT_UPDATER_SIGS = ("-secure-code-check.json.sig", "-secure-code-check.html.sig")

FEED_NAMES = ("latest-v2.json", "latest.json")

TAG = re.compile(r"^v(\d+)\.(\d+)\.(\d+)(-[a-z0-9.]+)?$")
RFC3339 = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$")


class FeedError(Exception):
    """A feed, key or config that breaks the rules. Stops the release."""


def version_of_tag(tag: str) -> tuple:
    match = TAG.match(tag)
    if not match:
        raise FeedError(f"{tag!r} is not a release tag (vX.Y.Z or vX.Y.Z-rcN)")
    return tag[1:], tuple(int(part) for part in match.groups()[:3])


def minisign_lines(text_b64: str, what: str) -> list:
    """The lines of a minisign key or signature, which Tauri stores base64 encoded."""
    try:
        text = base64.b64decode(text_b64.strip(), validate=True).decode("utf-8")
    except (binascii.Error, ValueError, UnicodeDecodeError):
        raise FeedError(f"{what} is not a base64 encoded minisign text") from None
    lines = text.splitlines()
    if len(lines) < 2 or not lines[0].startswith("untrusted comment:"):
        raise FeedError(f"{what} is not a minisign text")
    return lines


def key_id(raw: bytes) -> str:
    """The key id as minisign prints it."""
    return raw[2:10][::-1].hex().upper()


def pubkey_id(pubkey_b64: str, what: str = "the public key") -> str:
    lines = minisign_lines(pubkey_b64, what)
    try:
        raw = base64.b64decode(lines[1].strip(), validate=True)
    except (binascii.Error, ValueError):
        raise FeedError(f"{what} is damaged") from None
    if len(raw) != 42 or raw[:2] != b"Ed":
        raise FeedError(f"{what} is not an Ed25519 minisign public key")
    return key_id(raw)


def signature_info(signature_b64: str, what: str) -> tuple:
    """(key id, signed version) of a Tauri updater signature.

    The signed version is the `version:` field of the trusted comment, which
    the signature covers; the updater refuses a feed that announces another.
    """
    lines = minisign_lines(signature_b64, what)
    try:
        raw = base64.b64decode(lines[1].strip(), validate=True)
    except (binascii.Error, ValueError):
        raise FeedError(f"{what} is damaged") from None
    if len(raw) != 74 or raw[:2] not in (b"Ed", b"ED"):
        raise FeedError(f"{what} is not an Ed25519 minisign signature")
    if len(lines) < 3 or not lines[2].startswith("trusted comment: "):
        raise FeedError(f"{what} has no trusted comment")
    fields = lines[2][len("trusted comment: "):].split("\t")
    signed = next((f[len("version:"):] for f in fields if f.startswith("version:")), None)
    return key_id(raw), signed


def read_text(path) -> str:
    """A text file as UTF-8, whatever the machine's default (the build runs on Windows too)."""
    try:
        return pathlib.Path(path).read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as error:
        raise FeedError(f"cannot read {path}: {error}") from None


def old_pubkey() -> str:
    return read_text(OLD_PUBKEY_FILE).strip()


def old_key_id() -> str:
    return pubkey_id(old_pubkey(), "the old public key")


def load_conf(path) -> dict:
    """The parts of tauri.conf.json the feeds depend on."""
    try:
        conf = json.loads(read_text(path))
        updater = conf["plugins"]["updater"]
        return {"version": conf["version"], "pubkey": updater["pubkey"],
                "endpoints": updater.get("endpoints"),
                "require_signed_version": updater.get("requireSignedVersion")}
    except (ValueError, KeyError, TypeError) as error:
        raise FeedError(f"{path}: not a tauri.conf.json with an updater: {error}") from None


def rust_str_const(source: str, name: str) -> list:
    """The values of `const NAME: &str = "...";` in a Rust file."""
    pattern = re.compile(r"\bconst\s+" + name + r"\s*:\s*&(?:'static\s+)?str\s*=\s*\"([^\"\\]*)\"\s*;")
    return pattern.findall(source)


def load_launcher(path) -> dict:
    """What the launcher (installer/src-tauri/src/install.rs) trusts and reads."""
    source = read_text(path)
    keys = rust_str_const(source, "UPDATER_PUBKEY")
    if len(keys) != 1:
        raise FeedError(f"{path}: expected one `const UPDATER_PUBKEY: &str = \"...\";`, found {len(keys)}")
    urls = rust_str_const(source, "FEED_URL")
    if len(urls) > 1:
        raise FeedError(f"{path}: more than one FEED_URL")
    return {"pubkey": keys[0], "feed": urls[0] if urls else None, "api": f'"{LAUNCHER_API}"' in source}


def signer_for(version: tuple, conf: dict) -> tuple:
    """(what, key id) of the key a release of `version` must be signed with."""
    if version <= BRIDGE:
        return "the old key", old_key_id()
    return "the key in tauri.conf.json", pubkey_id(conf["pubkey"], "the public key in tauri.conf.json")


def check_conf(tag: str, conf: dict) -> tuple:
    """The app being released reads latest-v2.json, trusts the new key, and
    only installs an update whose signature binds the version announced."""
    version, numbers = version_of_tag(tag)
    if conf["version"] != version:
        raise FeedError(f"tauri.conf.json says version {conf['version']}, the tag says {tag}")
    if conf["endpoints"] != [V2_ENDPOINT]:
        raise FeedError(f"the app must read {V2_ENDPOINT} and nothing else, "
                        f"tauri.conf.json has {conf['endpoints']!r}")
    if pubkey_id(conf["pubkey"]) == old_key_id():
        raise FeedError("tauri.conf.json trusts the old (stolen) updater key")
    if conf.get("require_signed_version") is not True:
        raise FeedError("tauri.conf.json must set plugins.updater.requireSignedVersion to true, so a "
                        "feed cannot pair a newer version number with an older signed release")
    return version, numbers


def check_launcher(tag: str, numbers: tuple, conf: dict, launcher: dict) -> None:
    """The launcher of this release trusts the key, and reads the source, of what it installs.

    Up to v0.266.47 it installs a release signed with the old key, found through
    the GitHub API. After it, it installs releases signed with the key in
    tauri.conf.json, found through latest-v2.json.
    """
    got = pubkey_id(launcher["pubkey"], "the launcher's UPDATER_PUBKEY")
    if numbers <= BRIDGE:
        if got != old_key_id():
            raise FeedError(f"the launcher of {tag} trusts key {got}, but it installs {tag}, which is signed "
                            f"with the old key ({old_key_id()}): it would refuse every download")
        if launcher["feed"] is not None:
            raise FeedError(f"the launcher of {tag} reads {launcher['feed']}; up to {BRIDGE_TAG} it finds the "
                            f"installer through the GitHub API ({LAUNCHER_API})")
        if not launcher["api"]:
            raise FeedError(f"the launcher of {tag} does not ask {LAUNCHER_API} for the installer")
        return
    wanted = pubkey_id(conf["pubkey"], "the public key in tauri.conf.json")
    if got != wanted:
        raise FeedError(f"the launcher of {tag} trusts key {got}, but {tag} is signed with the key in "
                        f"tauri.conf.json ({wanted}): it would refuse every download")
    if launcher["feed"] != V2_ENDPOINT:
        raise FeedError(f"the launcher of {tag} must read {V2_ENDPOINT}, it reads {launcher['feed']!r}")


def plan(tag: str, conf: dict, launcher: dict) -> dict:
    _, numbers = check_conf(tag, conf)
    check_launcher(tag, numbers, conf, launcher)
    return {"frozen": numbers > BRIDGE}


def read_sigs(folder) -> dict:
    """The .sig files of a release, by file name."""
    folder = pathlib.Path(folder)
    if not folder.is_dir():
        raise FeedError(f"{folder}: no such folder")
    return {p.name: read_text(p) for p in folder.iterdir() if p.is_file() and p.name.endswith(".sig")}


def check_feed(raw: bytes, *, name: str, tag: str, version: str, signer: tuple, sigs: dict) -> dict:
    """A feed as the Tauri updater reads it, offering release `tag`.

    Every entry downloads from that release, carries exactly the .sig file
    published beside its file, is signed by `signer` (what, key id) and binds
    `version`, the version the feed announces.
    """
    try:
        feed = json.loads(raw)
    except ValueError:
        raise FeedError(f"{name} is not JSON") from None
    if not isinstance(feed, dict):
        raise FeedError(f"{name} is not a JSON object")
    if feed.get("version") != version:
        raise FeedError(f"{name} announces version {feed.get('version')!r}, not {version}")
    if not isinstance(feed.get("notes", ""), str):
        raise FeedError(f"{name}: notes is not text")
    if "pub_date" in feed and not (isinstance(feed["pub_date"], str) and RFC3339.match(feed["pub_date"])):
        raise FeedError(f"{name}: pub_date {feed['pub_date']!r} is not an RFC 3339 date")
    platforms = feed.get("platforms")
    if not isinstance(platforms, dict) or not platforms:
        raise FeedError(f"{name} lists no platforms")
    if "windows-x86_64" not in platforms:
        raise FeedError(f"{name} has no windows-x86_64 entry, the one every Windows DAW can use")
    what, wanted = signer
    base = DOWNLOAD + tag + "/"
    for target, entry in platforms.items():
        if not isinstance(entry, dict) or set(entry) != {"signature", "url"} \
                or not all(isinstance(v, str) for v in entry.values()):
            raise FeedError(f"{name}: {target} is not {{signature, url}}")
        url = entry["url"]
        if not url.startswith(base) or "/" in url[len(base):]:
            raise FeedError(f"{name}: {target} downloads {url}, not a file of release {tag}")
        file = urllib.parse.unquote(url[len(base):])
        published = sigs.get(file + ".sig")
        if published is None:
            raise FeedError(f"{name}: {target}: release {tag} has no {file}.sig")
        if published.strip() != entry["signature"].strip():
            raise FeedError(f"{name}: {target}: the signature is not the {file}.sig of release {tag}")
        signed_by, signed_version = signature_info(entry["signature"], f"{name}: {target} signature")
        if signed_by != wanted:
            raise FeedError(f"{name}: {target} is signed with key {signed_by}, not {what} ({wanted})")
        if signed_version != version:
            raise FeedError(f"{name}: {target} is signed for version {signed_version!r}, not {version}")
    return feed


def check_bridge_conf(bridge_conf: dict, conf: dict) -> None:
    """v0.266.47 reads latest-v2.json and trusts the key this release is signed with.

    Old installs are sent to v0.266.47; without both, they would stop there.
    """
    if bridge_conf["endpoints"] != [V2_ENDPOINT]:
        raise FeedError(f"{BRIDGE_TAG} does not read latest-v2.json ({bridge_conf['endpoints']!r}): "
                        f"installs sent there would never see a newer release")
    if bridge_conf["pubkey"] != conf["pubkey"]:
        raise FeedError(f"{BRIDGE_TAG} trusts key {pubkey_id(bridge_conf['pubkey'])}, this release "
                        f"trusts {pubkey_id(conf['pubkey'])}: installs sent there could not update further")


def frozen(feed_raw: bytes, sigs: dict, bridge_conf: dict, conf: dict, bridge_tag: str = BRIDGE_TAG) -> bytes:
    """The frozen latest.json: the bridge release's own latest.json, unchanged."""
    version, _ = version_of_tag(bridge_tag)
    check_bridge_conf(bridge_conf, conf)
    old = ("the old key", old_key_id())
    check_feed(feed_raw, name=f"latest.json of {bridge_tag}", tag=bridge_tag, version=version,
               signer=old, sigs=sigs)
    return feed_raw


def release_date(release: dict) -> str:
    """pub_date: when the release was published (created, for a draft), so a re-run writes the same feed."""
    date = release.get("publishedAt") or release.get("createdAt")
    if not isinstance(date, str) or not RFC3339.match(date):
        raise FeedError(f"the release has no publishedAt or createdAt date ({date!r}); "
                        f"read it with `gh release view --json assets,body,publishedAt,createdAt`")
    return date


def build_v2(release: dict, sigs: dict, version: str, pub_date: str) -> bytes:
    """A feed from the release's files, byte for byte in tauri-action's shape."""
    assets = {a["name"]: a["url"] for a in release.get("assets", [])}
    platforms = {}
    used = set()
    for target, suffix in UPDATER_FILES:
        files = sorted(n for n in assets if n.endswith(suffix))
        if len(files) != 1:
            raise FeedError(f"{target}: the release needs exactly one file ending {suffix}, it has {files}")
        sig = files[0] + ".sig"
        if sig not in sigs:
            raise FeedError(f"{target}: the release has no {sig}")
        platforms[target] = {"signature": sigs[sig], "url": assets[files[0]]}
        used.add(sig)
    unknown = sorted(n for n in assets if n.endswith(".sig") and n not in used
                     and not n.endswith(NOT_UPDATER_SIGS))
    if unknown:
        raise FeedError(f"updater files this feed does not know: {unknown}. Add their feed entry to "
                        f"UPDATER_FILES, or that platform gets no updates")
    feed = {"version": version, "notes": release.get("body") or "", "pub_date": pub_date,
            "platforms": platforms}
    return json.dumps(feed, indent=2, ensure_ascii=False).encode("utf-8")


def v2(tag: str, conf: dict, sigs: dict, release: dict) -> bytes:
    """This release's latest-v2.json, built from its files and checked against the rules for its version."""
    version, numbers = check_conf(tag, conf)
    raw = build_v2(release, sigs, version, release_date(release))
    check_feed(raw, name="latest-v2.json", tag=tag, version=version, signer=signer_for(numbers, conf), sigs=sigs)
    return raw


def check_latest_json(raw: bytes, name: str = "latest.json") -> None:
    """latest.json is read by every DAW up to v0.266.46: nothing in it may be signed with anything but the old key."""
    try:
        feed = json.loads(raw)
        platforms = feed["platforms"]
        entries = list(platforms.items())
    except (ValueError, KeyError, TypeError, AttributeError):
        raise FeedError(f"{name} is not a feed") from None
    old = old_key_id()
    for target, entry in entries:
        signed_by, _ = signature_info(entry.get("signature", "") if isinstance(entry, dict) else "",
                                      f"{name}: {target} signature")
        if signed_by != old:
            raise FeedError(f"{name}: {target} is signed with key {signed_by}, not the old key ({old}): "
                            f"DAWs up to v0.266.46 could never verify it")
    _, numbers = version_of_tag("v" + str(feed.get("version")))
    if numbers > BRIDGE:
        raise FeedError(f"{name} offers {feed.get('version')}: nothing after {BRIDGE_TAG} is signed with the old key")


def feeds(tag: str, conf: dict, sigs: dict, release: dict, *, bridge_feed: bytes = None,
          bridge_sigs: dict = None, bridge_conf: dict = None) -> dict:
    """{name: bytes} of both feeds this release publishes.

    Up to v0.266.47 both are the release's own feed, signed with the old key.
    After it, latest-v2.json is the release's own feed (key in tauri.conf.json)
    and latest.json is v0.266.47's, frozen.
    """
    version, numbers = check_conf(tag, conf)
    out = {"latest-v2.json": v2(tag, conf, sigs, release)}
    if numbers <= BRIDGE:
        out["latest.json"] = out["latest-v2.json"]
        check_feed(out["latest.json"], name="latest.json", tag=tag, version=version,
                   signer=("the old key", old_key_id()), sigs=sigs)
    else:
        if bridge_feed is None or bridge_sigs is None or bridge_conf is None:
            raise FeedError(f"after {BRIDGE_TAG}, latest.json is {BRIDGE_TAG}'s own: pass its latest.json, "
                            f".sig files and tauri.conf.json")
        out["latest.json"] = frozen(bridge_feed, bridge_sigs, bridge_conf, conf)
    check_latest_json(out["latest.json"])
    return out


def signing_key(tag: str, conf: dict, sig_text: str) -> str:
    """Refuses a signing secret that is not the key this version must carry."""
    _, numbers = check_conf(tag, conf)
    what, wanted = signer_for(numbers, conf)
    got, _ = signature_info(sig_text, "the test signature")
    if got == wanted:
        return f"the signing secret is {what} ({got}), as {tag} needs"
    if numbers <= BRIDGE:
        raise FeedError(f"the signing secret is key {got}, but {tag} must be signed with the old key "
                        f"({wanted}) so installed DAWs accept it. The secret was switched too early.")
    raise FeedError(f"the signing secret is key {got}, but {tag} must be signed with the key in "
                    f"tauri.conf.json ({wanted}). Run switch-secret.sh (HQ) once {BRIDGE_TAG} is out.")


def read_bytes(path) -> bytes:
    try:
        return pathlib.Path(path).read_bytes()
    except OSError as error:
        raise FeedError(f"cannot read {path}: {error}") from None


def read_json(path) -> dict:
    try:
        return json.loads(read_bytes(path))
    except ValueError:
        raise FeedError(f"{path} is not JSON") from None


def write(path, data: bytes) -> None:
    path = pathlib.Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    print(f"wrote {path} ({len(data)} bytes)")


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("plan")
    p.add_argument("--tag", required=True)
    p.add_argument("--conf", required=True)
    p.add_argument("--launcher", required=True)
    sub.add_parser("bridge-tag")
    p = sub.add_parser("signing-key")
    p.add_argument("--tag", required=True)
    p.add_argument("--conf", required=True)
    p.add_argument("--sig", required=True)
    p = sub.add_parser("frozen")
    p.add_argument("--conf", required=True)
    p.add_argument("--feed", required=True)
    p.add_argument("--sigs", required=True)
    p.add_argument("--bridge-conf", required=True)
    p.add_argument("--out", required=True)
    p = sub.add_parser("feeds")
    p.add_argument("--tag", required=True)
    p.add_argument("--conf", required=True)
    p.add_argument("--sigs", required=True)
    p.add_argument("--release", required=True)
    p.add_argument("--bridge-feed")
    p.add_argument("--bridge-sigs")
    p.add_argument("--bridge-conf")
    p.add_argument("--out", required=True)
    args = parser.parse_args(argv)

    try:
        if args.command == "plan":
            result = plan(args.tag, load_conf(args.conf), load_launcher(args.launcher))
            for key, value in result.items():
                print(f"{key}={'true' if value else 'false'}")
        elif args.command == "bridge-tag":
            print(BRIDGE_TAG)
        elif args.command == "signing-key":
            print(signing_key(args.tag, load_conf(args.conf), read_bytes(args.sig).decode("ascii", "replace")))
        elif args.command == "frozen":
            write(args.out, frozen(read_bytes(args.feed), read_sigs(args.sigs),
                                   load_conf(args.bridge_conf), load_conf(args.conf)))
        elif args.command == "feeds":
            out = feeds(args.tag, load_conf(args.conf), read_sigs(args.sigs), read_json(args.release),
                        bridge_feed=read_bytes(args.bridge_feed) if args.bridge_feed else None,
                        bridge_sigs=read_sigs(args.bridge_sigs) if args.bridge_sigs else None,
                        bridge_conf=load_conf(args.bridge_conf) if args.bridge_conf else None)
            # Written only once both passed every check.
            for name in FEED_NAMES:
                write(pathlib.Path(args.out) / name, out[name])
    except FeedError as error:
        print(f"::error::updater feeds: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
