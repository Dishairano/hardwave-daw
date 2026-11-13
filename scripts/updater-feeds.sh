#!/usr/bin/env bash
# The downloads and uploads around scripts/updater_feeds.py, which holds the
# rules and the checks (read its header for why there are two feeds).
#
#   scripts/updater-feeds.sh check        before the build: the app and the
#                                         launcher follow the rules for this
#                                         version, and from v0.266.48 on, the
#                                         frozen latest.json can be made
#   scripts/updater-feeds.sh signing-key  signs a throwaway file with the
#                                         signing secret (TAURI_SIGNING_*) and
#                                         refuses a key this version must not
#                                         be signed with
#   scripts/updater-feeds.sh publish      after the build: writes latest-v2.json
#                                         and latest.json and attaches them
#
# Needs gh with GH_TOKEN, GITHUB_REPOSITORY and GITHUB_REF_NAME (the tag), as
# the release workflow has them (signing-key needs only the tag and the Tauri
# CLI). Every check runs before anything is uploaded, and any failure stops
# with an error: nothing falls back.
#
# publish is safe to run again (re-run the failed job): both feeds are built
# from the release's own files and its publication date, so a second run
# writes the same bytes. A feed already on the release with exactly those
# bytes is left alone; any other copy is replaced. Each feed is downloaded
# again after the upload and must match what was checked.
set -euo pipefail

mode="${1:-}"
case "$mode" in check|signing-key|publish) ;; *) echo "usage: $0 check|signing-key|publish" >&2; exit 2 ;; esac
: "${GITHUB_REF_NAME:?the release tag}"
tag="$GITHUB_REF_NAME"

cd "$(git rev-parse --show-toplevel)"
conf=src-tauri/tauri.conf.json
launcher=installer/src-tauri/src/install.rs
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# On a Windows runner `python3` may be missing or the Microsoft Store stub.
py=python3
"$py" -c '' >/dev/null 2>&1 || py=python
feeds() { "$py" -I scripts/updater_feeds.py "$@"; }
fail() { echo "::error::updater feeds: $*" >&2; exit 1; }

if [ "$mode" = signing-key ]; then
  : "${TAURI_SIGNING_PRIVATE_KEY:?the signing secret is not set for this job}"
  printf 'Hardwave DAW updater key check\n' > "$work/key-check.txt"
  tauri signer sign "$work/key-check.txt" > /dev/null || fail "tauri signer sign failed"
  feeds signing-key --tag "$tag" --conf "$conf" --sig "$work/key-check.txt.sig"
  exit 0
fi

: "${GITHUB_REPOSITORY:?owner/repo}"
repo="$GITHUB_REPOSITORY"

plan="$(feeds plan --tag "$tag" --conf "$conf" --launcher "$launcher")"
frozen="$(printf '%s\n' "$plan" | sed -n 's/^frozen=//p')"
case "$frozen" in true|false) ;; *) fail "the plan for $tag says nothing about latest.json: $plan" ;; esac
echo "$tag: latest.json is frozen on the bridge release: $frozen"

# The frozen latest.json: the bridge release's own feed, its .sig files and
# its tauri.conf.json, checked, then copied unchanged.
bridge_args=()
if [ "$frozen" = true ]; then
  bridge="$(feeds bridge-tag)"
  mkdir -p "$work/bridge"
  gh release download "$bridge" --repo "$repo" --dir "$work/bridge" \
    --pattern latest.json --pattern '*.sig' \
    || fail "could not download latest.json and the .sig files of $bridge"
  gh api -H "Accept: application/vnd.github.raw" \
    "repos/$repo/contents/src-tauri/tauri.conf.json?ref=$bridge" > "$work/bridge-conf.json" \
    || fail "could not read src-tauri/tauri.conf.json at $bridge"
  feeds frozen --conf "$conf" --feed "$work/bridge/latest.json" --sigs "$work/bridge" \
    --bridge-conf "$work/bridge-conf.json" --out "$work/frozen-check.json"
  bridge_args=(--bridge-feed "$work/bridge/latest.json" --bridge-sigs "$work/bridge"
               --bridge-conf "$work/bridge-conf.json")
fi

[ "$mode" = publish ] || exit 0

mkdir -p "$work/this"
gh release download "$tag" --repo "$repo" --dir "$work/this" --pattern '*.sig' \
  || fail "could not download the .sig files of $tag"
gh release view "$tag" --repo "$repo" --json assets,body,publishedAt,createdAt > "$work/release.json" \
  || fail "could not read the files of $tag"
feeds feeds --tag "$tag" --conf "$conf" --sigs "$work/this" --release "$work/release.json" \
  ${bridge_args[@]+"${bridge_args[@]}"} --out "$work/out"

# latest.json first: it is what every DAW up to v0.266.46 asks.
for name in latest.json latest-v2.json; do
  mkdir -p "$work/live-$name"
  if gh release download "$tag" --repo "$repo" --dir "$work/live-$name" --pattern "$name" >/dev/null 2>&1 \
     && cmp -s "$work/out/$name" "$work/live-$name/$name"; then
    echo "$name: already on $tag with these exact bytes"
  else
    gh release upload "$tag" --repo "$repo" --clobber "$work/out/$name" || fail "could not upload $name to $tag"
    echo "$name: uploaded to $tag"
  fi
  mkdir -p "$work/check-$name"
  gh release download "$tag" --repo "$repo" --dir "$work/check-$name" --pattern "$name" \
    || fail "$name is not on $tag after the upload"
  cmp -s "$work/out/$name" "$work/check-$name/$name" \
    || fail "$name on $tag is not the feed that was checked"
done
ls -l "$work/out"
