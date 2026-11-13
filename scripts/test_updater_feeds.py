"""Tests for updater_feeds.py: the two update feeds and their rules.

  python3 -m unittest discover -s scripts -p 'test_updater_feeds.py'

The signatures here are made up: the script reads key ids and signed
versions, it does not verify Ed25519. The RealRelease tests run the same code
on a published release: v0.266.46's latest.json, its .sig files and
`release.json` (`gh release view <tag> --json assets,body,publishedAt,createdAt`),
kept in scripts/fixtures/v0.266.46 so they always run. HW_FEED_FIXTURES points
them at another release's files instead.
"""
import base64
import json
import os
import pathlib
import tempfile
import unittest

import updater_feeds as feeds

NEW_PUBKEY = ("dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEFBQzc5ODA1MzM1Q0UzNjYKUldSbTQxd3pCWmpI"
              "cXVPeFE1NjcxUkcrUTBSTVRZK1U2dS8vOXdJRkRMcW52UXl3eHU5R0pHQ2EK")
OLD_ID = "0F1BD86D2A5B9A48"
NEW_ID = "AAC79805335CE366"
OTHER_ID = "1122334455667788"
PUBLISHED = "2026-11-13T09:00:00Z"
ROOT = pathlib.Path(__file__).resolve().parent.parent
FIXTURES = pathlib.Path(__file__).resolve().parent / "fixtures" / "v0.266.46"


def b64(data: bytes) -> str:
    return base64.b64encode(data).decode()


def sig(key: str, version: str, file: str = "x") -> str:
    raw = b"ED" + bytes.fromhex(key)[::-1] + bytes(64)
    text = (f"untrusted comment: signature from tauri secret key\n{b64(raw)}\n"
            f"trusted comment: timestamp:1794494548\tfile:{file}\tversion:{version}\n{b64(bytes(64))}")
    return b64(text.encode())


def conf(version: str, *, endpoint: str = feeds.V2_ENDPOINT, pubkey: str = NEW_PUBKEY,
         require_signed_version=True) -> dict:
    return {"version": version, "pubkey": pubkey, "endpoints": [endpoint],
            "require_signed_version": require_signed_version}


def launcher(era: str, **changes) -> dict:
    """The launcher as load_launcher reads it, for the bridge release or the ones after it."""
    found = {"bridge": {"pubkey": feeds.old_pubkey(), "feed": None, "api": True},
             "after": {"pubkey": NEW_PUBKEY, "feed": feeds.V2_ENDPOINT, "api": True}}[era]
    found.update(changes)
    return found


def files(version: str) -> tuple:
    return (f"Hardwave.DAW_{version}_x64-setup.nsis.zip", f"Hardwave.DAW_{version}_x64-setup.exe")


def release(version: str, key: str, *, extra=(), published=PUBLISHED) -> tuple:
    """(release.json, sigs) the way gh shows a Windows release."""
    tag = f"v{version}"
    names = list(files(version)) + list(extra) + ["HardwaveDAW-Setup.exe"]
    sigs = {}
    for name in list(files(version)) + list(extra):
        sigs[name + ".sig"] = sig(key, version, name)
    assets = [{"name": n, "url": f"{feeds.DOWNLOAD}{tag}/{n}"} for n in names + list(sigs)]
    return {"assets": assets, "body": "- fix: something", "publishedAt": published,
            "createdAt": "2026-11-13T08:00:00Z"}, sigs


def tauri_feed(version: str, key: str, *, tag: str = None) -> tuple:
    """(latest.json bytes, sigs) in tauri-action's shape."""
    rel, sigs = release(version, key)
    raw = feeds.build_v2(rel, sigs, version, "2026-11-13T09:00:00.000Z")
    if tag:
        raw = raw.replace(f"/v{version}/".encode(), f"/{tag}/".encode())
    return raw, sigs


def bridge_args(**changes) -> dict:
    """v0.266.47 as a later release finds it: its latest.json, .sig files and tauri.conf.json."""
    raw, sigs = tauri_feed("0.266.47", OLD_ID)
    return {"bridge_feed": raw, "bridge_sigs": sigs, "bridge_conf": conf("0.266.47", **changes)}


class Plan(unittest.TestCase):
    def test_only_releases_after_the_bridge_freeze_latest_json(self):
        self.assertEqual(feeds.plan("v0.266.47", conf("0.266.47"), launcher("bridge")), {"frozen": False})
        self.assertEqual(feeds.plan("v0.266.47-rc1", conf("0.266.47-rc1"), launcher("bridge")), {"frozen": False})
        self.assertEqual(feeds.plan("v0.266.48", conf("0.266.48"), launcher("after")), {"frozen": True})
        self.assertEqual(feeds.plan("v0.267.0-rc1", conf("0.267.0-rc1"), launcher("after")), {"frozen": True})

    def test_the_app_must_read_the_v2_feed_and_trust_the_new_key(self):
        with self.assertRaisesRegex(feeds.FeedError, "latest-v2.json and nothing else"):
            feeds.plan("v0.266.48", conf("0.266.48", endpoint=feeds.V2_ENDPOINT.replace("-v2", "")),
                       launcher("after"))
        with self.assertRaisesRegex(feeds.FeedError, "stolen"):
            feeds.plan("v0.266.47", conf("0.266.47", pubkey=feeds.old_pubkey()), launcher("bridge"))
        with self.assertRaisesRegex(feeds.FeedError, "the tag says"):
            feeds.plan("v0.266.48", conf("0.266.47"), launcher("after"))
        with self.assertRaisesRegex(feeds.FeedError, "not a release tag"):
            feeds.plan("fast-v1", conf("0.266.48"), launcher("after"))

    def test_the_app_must_require_the_signed_version(self):
        for value in (None, False, "true"):
            with self.assertRaisesRegex(feeds.FeedError, "requireSignedVersion"):
                feeds.plan("v0.266.48", conf("0.266.48", require_signed_version=value), launcher("after"))

    def test_the_bridge_launcher_trusts_the_old_key_and_asks_the_github_api(self):
        with self.assertRaisesRegex(feeds.FeedError, "would refuse every download"):
            feeds.plan("v0.266.47", conf("0.266.47"), launcher("bridge", pubkey=NEW_PUBKEY))
        with self.assertRaisesRegex(feeds.FeedError, "through the GitHub API"):
            feeds.plan("v0.266.47", conf("0.266.47"), launcher("bridge", feed=feeds.V2_ENDPOINT))
        with self.assertRaisesRegex(feeds.FeedError, "does not ask"):
            feeds.plan("v0.266.47", conf("0.266.47"), launcher("bridge", api=False))

    def test_later_launchers_trust_the_apps_key_and_read_latest_v2(self):
        with self.assertRaisesRegex(feeds.FeedError, "would refuse every download"):
            feeds.plan("v0.266.48", conf("0.266.48"), launcher("after", pubkey=feeds.old_pubkey()))
        with self.assertRaisesRegex(feeds.FeedError, "must read .*latest-v2.json"):
            feeds.plan("v0.266.48", conf("0.266.48"), launcher("after", feed=None))
        with self.assertRaisesRegex(feeds.FeedError, "must read .*latest-v2.json"):
            feeds.plan("v0.266.48", conf("0.266.48"),
                       launcher("after", feed=feeds.V2_ENDPOINT.replace("-v2", "")))

    def test_the_launcher_is_read_from_its_source(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "install.rs"
            path.write_text(f'const UPDATER_PUBKEY: &str = "{NEW_PUBKEY}";\n'
                            f'const FEED_URL: &str =\n    "{feeds.V2_ENDPOINT}";\n'
                            f'fn f() {{ get("{feeds.LAUNCHER_API}") }}\n')
            self.assertEqual(feeds.load_launcher(path), launcher("after"))
            path.write_text(f'const UPDATER_PUBKEY: &str = "{NEW_PUBKEY}";\n')
            self.assertEqual(feeds.load_launcher(path), {"pubkey": NEW_PUBKEY, "feed": None, "api": False})
            path.write_text("fn main() {}\n")
            with self.assertRaisesRegex(feeds.FeedError, "UPDATER_PUBKEY"):
                feeds.load_launcher(path)

    def test_this_checkout_follows_the_rules_of_one_side_of_the_bridge(self):
        app = feeds.load_conf(ROOT / "src-tauri" / "tauri.conf.json")
        found = feeds.load_launcher(ROOT / "installer" / "src-tauri" / "src" / "install.rs")
        passed = []
        for tag in ("v0.266.47", "v0.266.48"):
            try:
                feeds.plan(tag, dict(app, version=tag[1:]), found)
                passed.append(tag)
            except feeds.FeedError:
                pass
        self.assertEqual(len(passed), 1, f"the app and the launcher pass the rules for {passed}")


class SigningKey(unittest.TestCase):
    def test_the_bridge_is_signed_with_the_old_key(self):
        self.assertIn(OLD_ID, feeds.signing_key("v0.266.47", conf("0.266.47"), sig(OLD_ID, "")))
        with self.assertRaisesRegex(feeds.FeedError, "switched too early"):
            feeds.signing_key("v0.266.47", conf("0.266.47"), sig(NEW_ID, ""))

    def test_later_releases_are_signed_with_the_key_in_the_app(self):
        self.assertIn(NEW_ID, feeds.signing_key("v0.266.48", conf("0.266.48"), sig(NEW_ID, "")))
        with self.assertRaisesRegex(feeds.FeedError, "switch-secret.sh"):
            feeds.signing_key("v0.266.48", conf("0.266.48"), sig(OLD_ID, ""))
        with self.assertRaises(feeds.FeedError):
            feeds.signing_key("v0.266.48", conf("0.266.48"), "not a signature")


class BridgeFeeds(unittest.TestCase):
    def test_the_bridge_publishes_one_feed_under_both_names(self):
        rel, sigs = release("0.266.47", OLD_ID)
        out = feeds.feeds("v0.266.47", conf("0.266.47"), sigs, rel)
        self.assertEqual(out["latest.json"], out["latest-v2.json"])
        self.assertEqual(out["latest-v2.json"], feeds.build_v2(rel, sigs, "0.266.47", PUBLISHED))

    def test_the_bridge_feed_must_carry_the_old_key(self):
        rel, sigs = release("0.266.47", NEW_ID)
        with self.assertRaisesRegex(feeds.FeedError, f"signed with key {NEW_ID}, not the old key"):
            feeds.feeds("v0.266.47", conf("0.266.47"), sigs, rel)


class LaterFeeds(unittest.TestCase):
    def test_latest_v2_is_built_from_the_release_in_tauris_shape(self):
        rel, sigs = release("0.266.48", NEW_ID, extra=["hardwave-daw-0.266.48-secure-code-check.json"])
        out = feeds.feeds("v0.266.48", conf("0.266.48"), sigs, rel, **bridge_args())
        feed = json.loads(out["latest-v2.json"])
        zip_name, exe_name = files("0.266.48")
        self.assertEqual(list(feed), ["version", "notes", "pub_date", "platforms"])
        self.assertEqual(feed["version"], "0.266.48")
        self.assertEqual(feed["notes"], "- fix: something")
        self.assertEqual(feed["pub_date"], PUBLISHED)
        self.assertEqual(feed["platforms"], {
            "windows-x86_64": {"signature": sigs[zip_name + ".sig"],
                               "url": f"{feeds.DOWNLOAD}v0.266.48/{zip_name}"},
            "windows-x86_64-nsis": {"signature": sigs[exe_name + ".sig"],
                                    "url": f"{feeds.DOWNLOAD}v0.266.48/{exe_name}"},
        })

    def test_latest_json_is_the_bridges_byte_for_byte(self):
        rel, sigs = release("0.266.48", NEW_ID)
        args = bridge_args()
        out = feeds.feeds("v0.266.48", conf("0.266.48"), sigs, rel, **args)
        self.assertEqual(out["latest.json"], args["bridge_feed"])

    def test_latest_json_needs_the_bridge(self):
        rel, sigs = release("0.266.48", NEW_ID)
        with self.assertRaisesRegex(feeds.FeedError, "latest.json is v0.266.47's own"):
            feeds.feeds("v0.266.48", conf("0.266.48"), sigs, rel)

    def test_a_rerun_writes_the_same_bytes(self):
        rel, sigs = release("0.266.48", NEW_ID)
        first = feeds.feeds("v0.266.48", conf("0.266.48"), sigs, rel, **bridge_args())
        second = feeds.feeds("v0.266.48", conf("0.266.48"), sigs, json.loads(json.dumps(rel)), **bridge_args())
        self.assertEqual(first, second)

    def test_pub_date_is_when_the_release_was_published(self):
        rel, sigs = release("0.266.48", NEW_ID)
        self.assertEqual(feeds.release_date(rel), PUBLISHED)
        rel["publishedAt"] = None
        self.assertEqual(feeds.release_date(rel), "2026-11-13T08:00:00Z")
        rel["createdAt"] = ""
        with self.assertRaisesRegex(feeds.FeedError, "publishedAt"):
            feeds.feeds("v0.266.48", conf("0.266.48"), sigs, rel, **bridge_args())

    def test_later_feeds_refuse_the_old_key(self):
        rel, sigs = release("0.266.48", OLD_ID)
        with self.assertRaisesRegex(feeds.FeedError, f"signed with key {OLD_ID}, not the key in tauri.conf.json"):
            feeds.feeds("v0.266.48", conf("0.266.48"), sigs, rel, **bridge_args())

    def test_a_signature_for_another_version_is_refused(self):
        rel, sigs = release("0.266.48", NEW_ID)
        name = files("0.266.48")[0] + ".sig"
        sigs[name] = sig(NEW_ID, "0.266.40")
        with self.assertRaisesRegex(feeds.FeedError, "signed for version '0.266.40'"):
            feeds.feeds("v0.266.48", conf("0.266.48"), sigs, rel, **bridge_args())

    def test_an_updater_file_the_feed_does_not_know_stops_it(self):
        rel, sigs = release("0.266.48", NEW_ID, extra=["Hardwave.DAW_aarch64.app.tar.gz"])
        with self.assertRaisesRegex(feeds.FeedError, "does not know"):
            feeds.feeds("v0.266.48", conf("0.266.48"), sigs, rel, **bridge_args())

    def test_a_missing_signature_stops_it(self):
        rel, sigs = release("0.266.48", NEW_ID)
        del sigs[files("0.266.48")[1] + ".sig"]
        with self.assertRaisesRegex(feeds.FeedError, "has no"):
            feeds.feeds("v0.266.48", conf("0.266.48"), sigs, rel, **bridge_args())


class LatestJsonGuard(unittest.TestCase):
    def test_new_key_content_is_never_latest_json(self):
        raw, _ = tauri_feed("0.266.47", NEW_ID)
        with self.assertRaisesRegex(feeds.FeedError, "could never verify it"):
            feeds.check_latest_json(raw)

    def test_nothing_after_the_bridge_is_latest_json(self):
        raw, _ = tauri_feed("0.266.48", OLD_ID)
        with self.assertRaisesRegex(feeds.FeedError, "nothing after v0.266.47"):
            feeds.check_latest_json(raw)

    def test_the_bridge_feed_is_fine(self):
        raw, _ = tauri_feed("0.266.47", OLD_ID)
        feeds.check_latest_json(raw)


class FrozenFeed(unittest.TestCase):
    def bridge(self, **changes):
        raw, sigs = tauri_feed("0.266.47", OLD_ID)
        return raw, sigs, conf("0.266.47", **changes)

    def test_old_installs_get_v0_266_47_byte_for_byte(self):
        raw, sigs, bridge_conf = self.bridge()
        self.assertEqual(feeds.frozen(raw, sigs, bridge_conf, conf("0.266.49")), raw)

    def test_a_feed_for_another_version_is_refused(self):
        raw, sigs = tauri_feed("0.266.48", OLD_ID, tag="v0.266.47")
        with self.assertRaisesRegex(feeds.FeedError, "announces version '0.266.48', not 0.266.47"):
            feeds.frozen(raw, sigs, conf("0.266.47"), conf("0.266.49"))

    def test_new_key_content_is_never_frozen_into_latest_json(self):
        raw, sigs = tauri_feed("0.266.47", NEW_ID)
        with self.assertRaisesRegex(feeds.FeedError, "not the old key"):
            feeds.frozen(raw, sigs, conf("0.266.47"), conf("0.266.49"))

    def test_files_of_another_release_are_refused(self):
        raw, sigs = tauri_feed("0.266.47", OLD_ID, tag="v0.266.48")
        with self.assertRaisesRegex(feeds.FeedError, "not a file of release v0.266.47"):
            feeds.frozen(raw, sigs, conf("0.266.47"), conf("0.266.49"))

    def test_a_signature_that_is_not_the_published_one_is_refused(self):
        raw, sigs, bridge_conf = self.bridge()
        name = files("0.266.47")[1] + ".sig"
        sigs[name] = sig(OLD_ID, "0.266.47", "other")
        with self.assertRaisesRegex(feeds.FeedError, "is not the .* of release v0.266.47"):
            feeds.frozen(raw, sigs, bridge_conf, conf("0.266.49"))

    def test_the_bridge_must_lead_on_to_the_new_feed_and_key(self):
        raw, sigs, bridge_conf = self.bridge(endpoint=feeds.V2_ENDPOINT.replace("-v2", ""))
        with self.assertRaisesRegex(feeds.FeedError, "never see a newer release"):
            feeds.frozen(raw, sigs, bridge_conf, conf("0.266.49"))
        raw, sigs, bridge_conf = self.bridge(pubkey=feeds.old_pubkey())
        with self.assertRaisesRegex(feeds.FeedError, "could not update further"):
            feeds.frozen(raw, sigs, bridge_conf, conf("0.266.49"))

    def test_damaged_feeds_are_refused(self):
        _, sigs, bridge_conf = self.bridge()
        for raw in (b"", b"not json", b"[]", b'{"version": "0.266.47", "platforms": {}}'):
            with self.assertRaises(feeds.FeedError):
                feeds.frozen(raw, sigs, bridge_conf, conf("0.266.49"))


def write_conf(path: pathlib.Path, version: str) -> None:
    path.write_text(json.dumps({"version": version, "plugins": {"updater": {
        "pubkey": NEW_PUBKEY, "endpoints": [feeds.V2_ENDPOINT], "requireSignedVersion": True}}}))


def write_sigs(folder: pathlib.Path, sigs: dict) -> None:
    folder.mkdir()
    for name, text in sigs.items():
        (folder / name).write_text(text)


class Commands(unittest.TestCase):
    def test_a_refusal_fails_the_command_and_writes_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            raw, sigs = tauri_feed("0.266.46", OLD_ID)
            write_sigs(tmp / "sigs", sigs)
            (tmp / "latest.json").write_bytes(raw)
            write_conf(tmp / "conf.json", "0.266.48")
            write_conf(tmp / "bridge.json", "0.266.47")
            code = feeds.main(["frozen", "--conf", str(tmp / "conf.json"), "--feed", str(tmp / "latest.json"),
                               "--sigs", str(tmp / "sigs"), "--bridge-conf", str(tmp / "bridge.json"),
                               "--out", str(tmp / "out" / "latest.json")])
            self.assertEqual(code, 1)
            self.assertFalse((tmp / "out" / "latest.json").exists())

    def test_the_feeds_command_writes_both_or_neither(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            rel, sigs = release("0.266.48", NEW_ID)
            write_sigs(tmp / "sigs", sigs)
            (tmp / "release.json").write_text(json.dumps(rel))
            write_conf(tmp / "conf.json", "0.266.48")
            args = bridge_args()
            write_sigs(tmp / "bridge-sigs", args["bridge_sigs"])
            (tmp / "bridge.json").write_bytes(args["bridge_feed"])
            write_conf(tmp / "bridge-conf.json", "0.266.47")
            command = ["feeds", "--tag", "v0.266.48", "--conf", str(tmp / "conf.json"),
                       "--sigs", str(tmp / "sigs"), "--release", str(tmp / "release.json"),
                       "--bridge-feed", str(tmp / "bridge.json"), "--bridge-sigs", str(tmp / "bridge-sigs"),
                       "--bridge-conf", str(tmp / "bridge-conf.json")]
            self.assertEqual(feeds.main(command + ["--out", str(tmp / "out")]), 0)
            self.assertEqual((tmp / "out" / "latest.json").read_bytes(), args["bridge_feed"])
            self.assertEqual(json.loads((tmp / "out" / "latest-v2.json").read_bytes())["version"], "0.266.48")

            (tmp / "sigs" / (files("0.266.48")[0] + ".sig")).write_text(sig(OLD_ID, "0.266.48"))
            self.assertEqual(feeds.main(command + ["--out", str(tmp / "refused")]), 1)
            self.assertFalse((tmp / "refused").exists())


class RealRelease(unittest.TestCase):
    """A published release (v0.266.46, kept in scripts/fixtures) through the same code."""

    def setUp(self):
        folder = pathlib.Path(os.environ.get("HW_FEED_FIXTURES") or FIXTURES)
        self.raw = (folder / "latest.json").read_bytes()
        self.feed = json.loads(self.raw)
        self.version = self.feed["version"]
        self.tag = "v" + self.version
        self.sigs = feeds.read_sigs(folder)
        self.release = json.loads((folder / "release.json").read_text(encoding="utf-8"))

    def test_its_feed_frozen_is_byte_for_byte_the_published_one(self):
        bridge_conf = conf(self.version)
        out = feeds.frozen(self.raw, self.sigs, bridge_conf, conf("0.266.48"), bridge_tag=self.tag)
        self.assertEqual(out, self.raw)
        feeds.check_latest_json(out)

    def test_it_is_not_taken_for_the_bridge(self):
        with self.assertRaisesRegex(feeds.FeedError, f"announces version '{self.version}', not 0.266.47"):
            feeds.frozen(self.raw, self.sigs, conf("0.266.47"), conf("0.266.48"))

    def test_the_builder_writes_what_tauri_action_wrote(self):
        built = feeds.build_v2(self.release, self.sigs, self.version, self.feed["pub_date"])
        self.assertEqual(built, self.raw)

    def test_both_feeds_of_a_release_up_to_the_bridge_are_its_own_dated_when_published(self):
        out = feeds.feeds(self.tag, conf(self.version), self.sigs, self.release)
        self.assertEqual(out["latest.json"], out["latest-v2.json"])
        self.assertEqual(json.loads(out["latest.json"]), dict(self.feed, pub_date=self.release["publishedAt"]))
        self.assertEqual(out, feeds.feeds(self.tag, conf(self.version), self.sigs, self.release))


if __name__ == "__main__":
    unittest.main()
