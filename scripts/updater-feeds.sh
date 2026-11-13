#!/usr/bin/env bash
# The downloads and uploads around scripts/updater_feeds.py, which holds the
# rules and the checks (read its header for why there are two feeds).
#
#   scripts/updater-feeds.sh check     before the build: the app's config
#                                      follows the rules, and from v0.266.48
#                                      on, the frozen latest.json can be made
#   scripts/updater-feeds.sh publish   after the build: writes latest-v2.json
#                                      (and from v0.266.48 on the frozen
#                                      latest.json) and attaches them
#
# Needs gh with GH_TOKEN, GITHUB_REPOSITORY and GITHUB_REF_NAME (the tag), as
# the release workflow has them. Every check runs before anything is uploaded,
# and any failure stops with an error: nothing falls back.
set -euo pipefail

mode="${1:-}"
case "$mode" in check|publish) ;; *) echo "usage: $0 check|publish" >&2; exit 2 ;; esac
: "${GITHUB_REF_NAME:?the release tag}" "${GITHUB_REPOSITORY:?owner/repo}"
tag="$GITHUB_REF_NAME"
repo="$GITHUB_REPOSITORY"

cd "$(git rev-parse --show-toplevel)"
conf=src-tauri/tauri.conf.json
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

feeds() { python3 -I scripts/updater_feeds.py "$@"; }
fail() { echo "::error::updater feeds: $*" >&2; exit 1; }

plan="$(feeds plan --tag "$tag" --conf "$conf")"
tauri_feed="$(printf '%s\n' "$plan" | sed -n 's/^tauri-feed=//p')"
frozen="$(printf '%s\n' "$plan" | sed -n 's/^frozen=//p')"
echo "$tag: Tauri writes latest.json: $tauri_feed; latest.json frozen on the bridge release: $frozen"

mkdir -p "$work/out"

# The frozen latest.json: the bridge release's own feed, its .sig files and
# its tauri.conf.json, checked, then copied unchanged.
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
    --bridge-conf "$work/bridge-conf.json" --out "$work/out/latest.json"
fi

[ "$mode" = publish ] || exit 0

mkdir -p "$work/this"
gh release download "$tag" --repo "$repo" --dir "$work/this" --pattern '*.sig' \
  || fail "could not download the .sig files of $tag"
if [ "$tauri_feed" = true ]; then
  gh release download "$tag" --repo "$repo" --dir "$work/this" --pattern latest.json \
    || fail "$tag has no latest.json from the build"
  feeds v2 --tag "$tag" --conf "$conf" --sigs "$work/this" \
    --tauri-feed "$work/this/latest.json" --out "$work/out/latest-v2.json"
else
  gh release view "$tag" --repo "$repo" --json assets,body > "$work/release.json" \
    || fail "could not read the files of $tag"
  feeds v2 --tag "$tag" --conf "$conf" --sigs "$work/this" \
    --release "$work/release.json" --out "$work/out/latest-v2.json"
fi

gh release upload "$tag" --repo "$repo" --clobber "$work"/out/*.json
ls -l "$work/out"
