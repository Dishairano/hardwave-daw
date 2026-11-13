"""Tests for updater_feeds.py: the two update feeds and their rules.

  python3 -m unittest discover -s scripts -p 'test_updater_feeds.py'

The signatures here are made up: the script reads key ids and signed
versions, it does not verify Ed25519. With HW_FEED_FIXTURES set to a folder
holding a real release's latest.json, its .sig files and `release.json`
(`gh release view <tag> --json assets,body`), the last test runs the frozen
feed and the feed builder on that release as well.
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


def b64(data: bytes) -> str:
    return base64.b64encode(data).decode()


def sig(key: str, version: str, file: str = "x") -> str:
    raw = b"ED" + bytes.fromhex(key)[::-1] + bytes(64)
    text = (f"untrusted comment: signature from tauri secret key\n{b64(raw)}\n"
            f"trusted comment: timestamp:1794494548\tfile:{file}\tversion:{version}\n{b64(bytes(64))}")
    return b64(text.encode())


def conf(version: str, *, endpoint: str = feeds.V2_ENDPOINT, pubkey: str = NEW_PUBKEY) -> dict:
    return {"version": version, "pubkey": pubkey, "endpoints": [endpoint]}


def files(version: str) -> tuple:
    return (f"Hardwave.DAW_{version}_x64-setup.nsis.zip", f"Hardwave.DAW_{version}_x64-setup.exe")


def release(version: str, key: str, *, extra=()) -> tuple:
    """(release.json, sigs) the way gh shows a Windows release."""
    tag = f"v{version}"
    names = list(files(version)) + list(extra) + ["HardwaveDAW-Setup.exe"]
    sigs = {}
    for name in list(files(version)) + list(extra):
        sigs[name + ".sig"] = sig(key, version, name)
    assets = [{"name": n, "url": f"{feeds.DOWNLOAD}{tag}/{n}"} for n in names + list(sigs)]
    return {"assets": assets, "body": "- fix: something"}, sigs


def tauri_feed(version: str, key: str, *, tag: str = None) -> tuple:
    """(latest.json bytes, sigs) as tauri-action writes them."""
    rel, sigs = release(version, key)
    raw = feeds.build_v2(rel, sigs, version, "2026-11-13T09:00:00.000Z")
    if tag:
        raw = raw.replace(f"/v{version}/".encode(), f"/{tag}/".encode())
    return raw, sigs


class Plan(unittest.TestCase):
    def test_the_bridge_keeps_tauris_feed_later_releases_freeze_it(self):
        self.assertEqual(feeds.plan("v0.266.47", conf("0.266.47")), {"tauri-feed": True, "frozen": False})
        self.assertEqual(feeds.plan("v0.266.48", conf("0.266.48")), {"tauri-feed": False, "frozen": True})
        self.assertEqual(feeds.plan("v0.267.0-rc1", conf("0.267.0-rc1")), {"tauri-feed": False, "frozen": True})

    def test_the_app_must_read_the_v2_feed_and_trust_the_new_key(self):
        with self.assertRaisesRegex(feeds.FeedError, "latest-v2.json and nothing else"):
            feeds.plan("v0.266.48", conf("0.266.48", endpoint=feeds.V2_ENDPOINT.replace("-v2", "")))
        with self.assertRaisesRegex(feeds.FeedError, "stolen"):
            feeds.plan("v0.266.47", conf("0.266.47", pubkey=feeds.old_pubkey()))
        with self.assertRaisesRegex(feeds.FeedError, "the tag says"):
            feeds.plan("v0.266.48", conf("0.266.47"))
        with self.assertRaisesRegex(feeds.FeedError, "not a release tag"):
            feeds.plan("fast-v1", conf("0.266.48"))


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


class V2Feed(unittest.TestCase):
    def test_the_bridge_publishes_tauris_feed_unchanged(self):
        raw, sigs = tauri_feed("0.266.47", OLD_ID)
        self.assertEqual(feeds.v2("v0.266.47", conf("0.266.47"), sigs, tauri_feed=raw), raw)

    def test_the_bridge_feed_must_carry_the_old_key(self):
        raw, sigs = tauri_feed("0.266.47", NEW_ID)
        with self.assertRaisesRegex(feeds.FeedError, f"signed with key {NEW_ID}, not the old key"):
            feeds.v2("v0.266.47", conf("0.266.47"), sigs, tauri_feed=raw)

    def test_later_feeds_are_built_from_the_release_in_tauris_shape(self):
        rel, sigs = release("0.266.48", NEW_ID, extra=["hardwave-daw-0.266.48-secure-code-check.json"])
        raw = feeds.v2("v0.266.48", conf("0.266.48"), sigs, release=rel, pub_date="2026-11-14T10:00:00.000Z")
        feed = json.loads(raw)
        zip_name, exe_name = files("0.266.48")
        self.assertEqual(list(feed), ["version", "notes", "pub_date", "platforms"])
        self.assertEqual(feed["version"], "0.266.48")
        self.assertEqual(feed["notes"], "- fix: something")
        self.assertEqual(feed["platforms"], {
            "windows-x86_64": {"signature": sigs[zip_name + ".sig"],
                               "url": f"{feeds.DOWNLOAD}v0.266.48/{zip_name}"},
            "windows-x86_64-nsis": {"signature": sigs[exe_name + ".sig"],
                                    "url": f"{feeds.DOWNLOAD}v0.266.48/{exe_name}"},
        })

    def test_later_feeds_refuse_the_old_key(self):
        rel, sigs = release("0.266.48", OLD_ID)
        with self.assertRaisesRegex(feeds.FeedError, f"signed with key {OLD_ID}, not the key in tauri.conf.json"):
            feeds.v2("v0.266.48", conf("0.266.48"), sigs, release=rel)

    def test_a_signature_for_another_version_is_refused(self):
        rel, sigs = release("0.266.48", NEW_ID)
        name = files("0.266.48")[0] + ".sig"
        sigs[name] = sig(NEW_ID, "0.266.40")
        with self.assertRaisesRegex(feeds.FeedError, "signed for version '0.266.40'"):
            feeds.v2("v0.266.48", conf("0.266.48"), sigs, release=rel)

    def test_an_updater_file_the_feed_does_not_know_stops_it(self):
        rel, sigs = release("0.266.48", NEW_ID, extra=["Hardwave.DAW_aarch64.app.tar.gz"])
        with self.assertRaisesRegex(feeds.FeedError, "does not know"):
            feeds.v2("v0.266.48", conf("0.266.48"), sigs, release=rel)

    def test_a_missing_signature_stops_it(self):
        rel, sigs = release("0.266.48", NEW_ID)
        del sigs[files("0.266.48")[1] + ".sig"]
        with self.assertRaisesRegex(feeds.FeedError, "has no"):
            feeds.v2("v0.266.48", conf("0.266.48"), sigs, release=rel)

    def test_each_era_needs_its_own_source(self):
        with self.assertRaisesRegex(feeds.FeedError, "copy of the latest.json Tauri wrote"):
            feeds.v2("v0.266.47", conf("0.266.47"), {}, release={"assets": []})
        with self.assertRaisesRegex(feeds.FeedError, "built from the release"):
            feeds.v2("v0.266.48", conf("0.266.48"), {}, tauri_feed=b"{}")


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


class Commands(unittest.TestCase):
    def test_a_refusal_fails_the_command_and_writes_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            raw, sigs = tauri_feed("0.266.46", OLD_ID)
            (tmp / "sigs").mkdir()
            for name, text in sigs.items():
                (tmp / "sigs" / name).write_text(text)
            (tmp / "latest.json").write_bytes(raw)
            for name, version in (("conf.json", "0.266.48"), ("bridge.json", "0.266.47")):
                (tmp / name).write_text(json.dumps({"version": version, "plugins": {"updater": {
                    "pubkey": NEW_PUBKEY, "endpoints": [feeds.V2_ENDPOINT]}}}))
            code = feeds.main(["frozen", "--conf", str(tmp / "conf.json"), "--feed", str(tmp / "latest.json"),
                               "--sigs", str(tmp / "sigs"), "--bridge-conf", str(tmp / "bridge.json"),
                               "--out", str(tmp / "out" / "latest.json")])
            self.assertEqual(code, 1)
            self.assertFalse((tmp / "out" / "latest.json").exists())


@unittest.skipUnless(os.environ.get("HW_FEED_FIXTURES"), "HW_FEED_FIXTURES is not set")
class RealRelease(unittest.TestCase):
    """A published release (v0.266.46 when this was written) through the same code."""

    def setUp(self):
        folder = pathlib.Path(os.environ["HW_FEED_FIXTURES"])
        self.raw = (folder / "latest.json").read_bytes()
        self.feed = json.loads(self.raw)
        self.version = self.feed["version"]
        self.tag = "v" + self.version
        self.sigs = feeds.read_sigs(folder)
        self.release = json.loads((folder / "release.json").read_text())

    def test_its_feed_frozen_is_byte_for_byte_the_published_one(self):
        bridge_conf = conf(self.version)
        out = feeds.frozen(self.raw, self.sigs, bridge_conf, conf("0.266.48"), bridge_tag=self.tag)
        self.assertEqual(out, self.raw)
        print(f"\n  frozen {self.tag}: {sorted(json.loads(out)['platforms'])}, signed with the old key")

    def test_it_is_not_taken_for_the_bridge(self):
        with self.assertRaisesRegex(feeds.FeedError, f"announces version '{self.version}', not 0.266.47"):
            feeds.frozen(self.raw, self.sigs, conf("0.266.47"), conf("0.266.48"))

    def test_the_builder_writes_what_tauri_action_wrote(self):
        built = feeds.build_v2(self.release, self.sigs, self.version, self.feed["pub_date"])
        self.assertEqual(built, self.raw)


if __name__ == "__main__":
    unittest.main()
