#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/release.sh "commit message"
# Bumps patch version in tauri.conf.json, commits, pushes, and tags.
# Generates a changelog from git commits since the last tag.

CONF="src-tauri/tauri.conf.json"
cd "$(git rev-parse --show-toplevel)"

# Build frontend first
echo "Building frontend..."
cd packages/daw-ui && npm run build && cd ../..

# Get current version
CURRENT=$(grep '"version"' "$CONF" | head -1 | sed 's/.*"\([0-9]*\.[0-9]*\.[0-9]*\)".*/\1/')
MAJOR=$(echo "$CURRENT" | cut -d. -f1)
MINOR=$(echo "$CURRENT" | cut -d. -f2)
PATCH=$(echo "$CURRENT" | cut -d. -f3)
NEW_PATCH=$((PATCH + 1))
NEW_VERSION="$MAJOR.$MINOR.$NEW_PATCH"

echo "Version: $CURRENT -> $NEW_VERSION"

# Generate changelog from commits since last tag, categorized into 3 sections.
# Bullet convention in commit bodies:
#   - feat: ...       → New features
#   - fix: ...        → Bug fixes
#   - improve: ...    → Improvements (also: perf:, refactor:, ui:, ux:)
# Bullets without a recognized prefix fall into Improvements.
LAST_TAG=$(git describe --tags --abbrev=0 2>/dev/null || echo "")
FEATURES=""
FIXES=""
IMPROVEMENTS=""

classify() {
  local line="$1"
  # Strip leading "- " and whitespace
  local stripped="${line#- }"
  stripped="${stripped#* }"
  case "$line" in
    -\ feat:*|-\ feature:*|-\ add:*|-\ new:*)
      FEATURES="${FEATURES}- ${line#*: }"$'\n' ;;
    -\ fix:*|-\ bug:*|-\ bugfix:*)
      FIXES="${FIXES}- ${line#*: }"$'\n' ;;
    -\ improve:*|-\ perf:*|-\ refactor:*|-\ ui:*|-\ ux:*|-\ chore:*)
      IMPROVEMENTS="${IMPROVEMENTS}- ${line#*: }"$'\n' ;;
    *)
      IMPROVEMENTS="${IMPROVEMENTS}${line}"$'\n' ;;
  esac
}

while IFS= read -r hash || [[ -n "$hash" ]]; do
  [ -z "$hash" ] && continue
  SUBJECT=$(git log -1 --format="%s" "$hash")
  BODY=$(git log -1 --format="%b" "$hash")

  # Skip internal commits (version bumps, CI fixes, formatting, refactors)
  [[ "$SUBJECT" =~ ^(Release|v[0-9]|Fix\ rust|Fix\ clippy|Fix\ fmt|Merge) ]] && continue

  BULLETS=$(echo "$BODY" | grep '^\s*[-*]' | sed 's/^\s*//; s/^\*/-/' || true)
  if [ -n "$BULLETS" ]; then
    BULLETS=$(echo "$BULLETS" | grep -iv \
      -e 'rustfmt\|clippy\|sccache\|RUSTC_WRAPPER\|tformat\|trailing newline' \
      -e 'continue-on-error\|GITHUB_OUTPUT\|read loop\|non-zero' \
      -e 'cache\|fallback\|frontend\|exposes\|state now' \
      -e '^- Backend:\|^- Root cause:\|^- Fix ' || true)
  fi

  if [ -n "$BULLETS" ]; then
    while IFS= read -r line; do
      [ -z "$line" ] && continue
      classify "$line"
    done <<< "$BULLETS"
  elif ! echo "$SUBJECT" | grep -qiE 'fmt|clippy|sccache|ci|rustfmt|changelog|fallback|cache'; then
    classify "- ${SUBJECT}"
  fi
done < <(
  if [ -n "$LAST_TAG" ]; then
    git log "${LAST_TAG}..HEAD" --pretty=tformat:"%H" --no-merges
  else
    git log --pretty=tformat:"%H" --no-merges -10
  fi
)

# Deduplicate each section
FEATURES=$(echo "$FEATURES" | awk '!seen[$0]++' | sed '/^$/d')
FIXES=$(echo "$FIXES" | awk '!seen[$0]++' | sed '/^$/d')
IMPROVEMENTS=$(echo "$IMPROVEMENTS" | awk '!seen[$0]++' | sed '/^$/d')

CHANGELOG_FILE="RELEASE_CHANGELOG.md"
{
  HAS_ANY=0
  if [ -n "$FEATURES" ]; then
    echo "### New features"
    echo "$FEATURES"
    echo
    HAS_ANY=1
  fi
  if [ -n "$FIXES" ]; then
    echo "### Bug fixes"
    echo "$FIXES"
    echo
    HAS_ANY=1
  fi
  if [ -n "$IMPROVEMENTS" ]; then
    echo "### Improvements"
    echo "$IMPROVEMENTS"
    HAS_ANY=1
  fi
  if [ "$HAS_ANY" -eq 0 ]; then
    echo "### Improvements"
    echo "- Internal maintenance and stability updates"
  fi
} > "$CHANGELOG_FILE"

# Bump version in BOTH tauri.conf.json AND the workspace Cargo.toml so the
# Rust binary's `env!("CARGO_PKG_VERSION")` (read by frontend_updater.rs as
# API_VERSION) stays in lockstep with the bundle version. Drift here would
# disable hot-swap for every running binary the moment a fresh manifest is
# published — see frontend-publish.yml's `assert workspace version` step.
sed -i "s/\"version\": \"$CURRENT\"/\"version\": \"$NEW_VERSION\"/" "$CONF"

CARGO_TOML="Cargo.toml"
# Match the `version = "..."` line that lives directly under the
# `[workspace.package]` section header. awk is more robust than a single
# sed against a comment-laden, multi-section Cargo.toml.
awk -v new="$NEW_VERSION" '
  BEGIN { in_section = 0; bumped = 0 }
  /^\[workspace\.package\]/ { in_section = 1; print; next }
  /^\[/ && in_section { in_section = 0 }
  in_section && !bumped && /^version[[:space:]]*=[[:space:]]*"[^"]+"/ {
    sub(/"[^"]+"/, "\"" new "\"")
    bumped = 1
  }
  { print }
  END {
    if (!bumped) {
      print "release.sh: failed to bump [workspace.package] version in Cargo.toml" > "/dev/stderr"
      exit 1
    }
  }
' "$CARGO_TOML" > "$CARGO_TOML.tmp"
mv "$CARGO_TOML.tmp" "$CARGO_TOML"

# Cargo.lock mirrors workspace versions; refresh it without rebuilding.
# `--offline` keeps the bump fast on flaky networks. Failure here is not
# fatal — CI's normal build step will refresh the lockfile if needed.
if command -v cargo >/dev/null 2>&1; then
  cargo update --workspace --offline >/dev/null 2>&1 \
    || cargo update --workspace >/dev/null 2>&1 \
    || echo "release.sh: warning — cargo update failed, Cargo.lock may need a rebuild in CI"
fi

# Commit message from arg or default
MSG="${1:-Release v$NEW_VERSION}"

# Stage all changes, commit, push, tag
git add -A
git commit -m "$MSG"
git push origin master
git tag -a "v$NEW_VERSION" -m "$(cat "$CHANGELOG_FILE")"
git push origin "v$NEW_VERSION"

# Clean up changelog file
rm -f "$CHANGELOG_FILE"

echo "Released v$NEW_VERSION — CI building at https://github.com/Dishairano/hardwave-daw/actions"
