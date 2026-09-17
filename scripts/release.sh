#!/usr/bin/env bash
set -euo pipefail

# Hardwave DAW release script — strict SemVer enforced per
# memory feedback_daw_versioning.md.
#
# Usage:
#   ./scripts/release.sh <patch|minor|major> "commit message"
#
#   patch  — bug fix, polish, refactor          (0.X.Y → 0.X.Y+1)
#   minor  — new user-visible feature           (0.X.Y → 0.X+1.0, patch resets)
#   major  — breaking change / major redesign   (X.Y.Z → X+1.0.0, both reset)
#
# No default. The script refuses to run without an explicit bump type
# so we cannot silently patch-bump a feature ever again.

if [ $# -lt 2 ]; then
  cat <<'USAGE' >&2
Usage: ./scripts/release.sh <patch|minor|major> "commit message"

  patch  bug fix, polish, refactor          (0.X.Y → 0.X.Y+1)
  minor  new user-visible feature           (0.X.Y → 0.X+1.0)
  major  breaking change / major redesign   (X.Y.Z → X+1.0.0)

Pick honestly:
  - new plug-in / new menu item / new panel  → minor
  - drag-drop fix / regression fix / polish  → patch
  - removed feature / changed API contract   → major

Write the version-bump line as `- internal: ...`, never `- chore: ...`.
"version bump for X" is not something anyone outside this repo can observe,
and chore bullets are published to users under Improvements.

Examples:
  ./scripts/release.sh patch "fix(daw): drag-drop on Windows"
  ./scripts/release.sh minor "feat(daw): KickSynth visual redesign"
  ./scripts/release.sh major "BREAKING: renamed project file format"
USAGE
  exit 1
fi

BUMP_TYPE="$1"
MSG="$2"

case "$BUMP_TYPE" in
  patch|minor|major) ;;
  *)
    echo "release.sh: bump type must be 'patch', 'minor', or 'major' — got '$BUMP_TYPE'" >&2
    echo "Run with no args for usage." >&2
    exit 1
    ;;
esac

# Enforce changelog bullet discipline.
#
# Per memory feedback_daw_commit_bullets.md: hardwave-daw release commits
# MUST use multi-line body with `-` bullets prefixed by feat: / fix: /
# improve: / perf: / refactor: / ui: / ux: / chore: — otherwise the
# auto-generated changelog degrades to a paragraph dump and Discord
# falls back to "Bug fixes and improvements". This check refuses the
# release before any tag lands.
# `internal:` counts as discipline too: a release whose content comes from
# the feature commits needs a bullet in its own message about as much as a
# version bump needs announcing, and writing `chore:` there just to satisfy
# this check is what published "version bump for X" to users.
MSG_BULLETS=$(printf '%s\n' "$MSG" | grep -cE '^- (feat|feature|add|new|fix|bug|bugfix|improve|perf|refactor|ui|ux|chore|internal):' || true)
if [ "$MSG_BULLETS" -lt 1 ]; then
  cat <<'BULLETERR' >&2
release.sh: commit message has no `- prefix:` bullets. Required format:

    fix(daw): one-line subject

    Optional context paragraph.

    - fix: short description of fix #1
    - fix: short description of fix #2
    - improve: side improvement

Bullet prefixes that route the changelog correctly:
  - feat: / feature: / add: / new:        → "New features"
  - fix: / bug: / bugfix:                   → "Bug fixes"
  - improve: / perf: / refactor: / ui:      → "Improvements"
  - ux: / chore:                            → "Improvements"

Refusing release until commit body has at least one prefixed bullet.
BULLETERR
  exit 1
fi

CONF="src-tauri/tauri.conf.json"
cd "$(git rev-parse --show-toplevel)"

# Typecheck + build frontend first. The explicit typecheck matters:
# Vite emits over TS errors, so `npm run build` alone can ship broken TS
# (CI now gates on this too — fail here, before anything is committed).
if [ "${HW_PREVIEW_CHANGELOG:-0}" = "1" ]; then
  echo "preview: skipping the frontend build and the gate; reading commits only"
else
echo "Typechecking + unit-testing + building frontend..."
cd packages/daw-ui && npm run typecheck && npm run test:unit && npm run build && cd ../..
fi

# Full test gate — the ENTIRE workspace, not a package subset. A partial
# local gate once let a broken release out (v0.204.x audio-reload); this
# check makes that structurally impossible: no green gate, no tag.
# HW_SKIP_GATE=1 is the emergency hatch for hotfixing a broken CI-only
# path; using it must be a deliberate, logged decision.
if [ "${HW_PREVIEW_CHANGELOG:-0}" = "1" ]; then
  : # preview only reads history; nothing to gate
elif [ "${HW_GATE_ON_PC:-0}" = "1" ]; then
  # The gate runs on the founder's PC instead of here. Not a way around the
  # gate: gate.yml runs the same fmt, clippy, tests and frontend checks, on
  # the toolchain rust-toolchain.toml pins, and the release refuses to tag
  # until that run has passed on the commit being tagged (see below). It is
  # stricter than the local stamp, which could only vouch for the tree before
  # the version bump, and it keeps a 15-minute Rust build off a server with
  # 2 cores and no disk left.
  echo "release.sh: gating on the PC runner; the tag waits for it to pass"
elif [ "${HW_SKIP_GATE:-0}" = "1" ]; then
  echo "release.sh: WARNING — HW_SKIP_GATE=1, skipping cargo test --workspace" >&2
else
  # fmt + clippy mirror CI's Lint & Test job EXACTLY. Lesson of
  # v0.204.10–.17: unformatted test code passed the local test gate,
  # then every tag died at CI's `cargo fmt --check` — eight releases
  # never reached users. Anything CI gates on must fail HERE, before
  # the tag exists.
  # ./scripts/gate.sh stamps the fingerprint of a tree it proved green,
  # together with the rustc that proved it. When that stamp matches what we
  # are about to release, running the identical 15-minute gate again can only
  # repeat the same answer, so skip it. Any edited, added or deleted file
  # changes the fingerprint, and a different toolchain changes the stamp
  # name, so both fall through to the full gate below. The stamp lives in
  # .git/ and is never shared between machines.
  # A stamp we cannot compute (no rustc on PATH) means no skip, never a skip.
  GATE_STAMP=$(./scripts/gate.sh --stamp-path 2>/dev/null || true)
  if [ -n "$GATE_STAMP" ] && [ -f "$GATE_STAMP" ]; then
    echo "release.sh: gate.sh already proved this exact tree green on this toolchain"
    echo "release.sh: stamp $GATE_STAMP"
  else
  echo "cargo fmt --check..."
  if ! cargo fmt --all -- --check; then
    echo "release.sh: rustfmt differences — run 'cargo fmt --all' and re-release." >&2
    exit 1
  fi
  echo "cargo clippy (hard error, matching CI)..."
  if ! cargo clippy --workspace --all-targets -- -D warnings; then
    echo "release.sh: clippy warnings — fix them (see ./scripts/lint.sh) and re-release." >&2
    exit 1
  fi
  echo "Running full workspace test gate (cargo test --workspace)..."
  if ! cargo test --workspace; then
    echo "release.sh: workspace tests FAILED — refusing to release." >&2
    exit 1
  fi
  fi
fi

# Get current version
CURRENT=$(grep '"version"' "$CONF" | head -1 | sed 's/.*"\([0-9]*\.[0-9]*\.[0-9]*\)".*/\1/')
MAJOR=$(echo "$CURRENT" | cut -d. -f1)
MINOR=$(echo "$CURRENT" | cut -d. -f2)
PATCH=$(echo "$CURRENT" | cut -d. -f3)

# Compute new version per the chosen bump type. Major resets minor+patch;
# minor resets patch; patch increments only the last segment.
case "$BUMP_TYPE" in
  major)
    NEW_VERSION="$((MAJOR + 1)).0.0"
    ;;
  minor)
    NEW_VERSION="$MAJOR.$((MINOR + 1)).0"
    ;;
  patch)
    NEW_VERSION="$MAJOR.$MINOR.$((PATCH + 1))"
    ;;
esac

echo "Bump type: $BUMP_TYPE"
echo "Version: $CURRENT -> $NEW_VERSION"

# NOTE: the fe-v* 'frontend-only fast lane' was REMOVED (2026-07-06).
# The hot-swap updater it fed was retired in v0.200.0 (the app always
# loads the bundled UI and updates via the built-in Tauri updater), so
# every release is a full v* build. frontend-publish.yml is deleted.

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

# Sort a bullet into a section, KEEPING its prefix.
#
# The prefix used to be stripped here, which quietly broke the Discord
# announcement: that step groups bullets into New features / Bug fixes /
# Improvements by reading the same `feat:` / `fix:` prefix, so prefix-less
# bullets all fell into Improvements and every post became one flat list.
# v0.204.19 read as three sections because its bullets still carried them.
# The `### heading` lines below are for the GitHub release page; Discord
# ignores them and classifies per bullet.
classify() {
  local line="$1"
  case "$line" in
    -\ feat:*|-\ feature:*|-\ add:*|-\ new:*)
      FEATURES="${FEATURES}${line}"$'\n' ;;
    -\ fix:*|-\ bug:*|-\ bugfix:*)
      FIXES="${FIXES}${line}"$'\n' ;;
    -\ improve:*|-\ perf:*|-\ refactor:*|-\ ui:*|-\ ux:*|-\ chore:*)
      IMPROVEMENTS="${IMPROVEMENTS}${line}"$'\n' ;;
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

  # A commit that touched nothing but build plumbing is not a product change.
  # The keyword blacklist below leaks, because it can only catch words someone
  # thought of: v0.210.0 announced "the PC gate can download rustup", "the PC
  # gate can install its Rust toolchain" and "formatting only" to users as bug
  # fixes, burying the one change that mattered to them. What a commit TOUCHED
  # is a fact rather than a guess, so infrastructure-only commits are dropped
  # whatever their wording.
  CHANGED=$(git show --pretty=format: --name-only "$hash" | sed '/^$/d')
  if [ -n "$CHANGED" ] && ! printf '%s\n' "$CHANGED" | grep -qvE \
      '^(\.github/|scripts/|docs/|README|\.gitignore|rust-toolchain|.*/tests/|.*/__tests__/|.*\.test\.[tj]sx?$|.*\.spec\.[tj]sx?$)'; then
    continue
  fi

  BULLETS=$(echo "$BODY" | grep '^\s*[-*]' | sed 's/^\s*//; s/^\*/-/' || true)
  HAD_BULLETS=0
  [ -n "$BULLETS" ] && HAD_BULLETS=1
  # `- internal:` is the explicit opt-out for a change inside a product commit
  # that users have no way to observe.
  BULLETS=$(printf '%s\n' "$BULLETS" | grep -v '^- internal:' || true)
  # A commit whose every bullet was internal has said its piece, so it must
  # not fall through to the subject-line fallback below. v0.214.1 announced
  # "fix(engine): the tauri::command attribute belonged to the function, not
  # the struct" to users in exactly that way.
  if [ "$HAD_BULLETS" = "1" ] && [ -z "$BULLETS" ]; then
    continue
  fi
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

# The release commit's own bullets count too. It is written last, after the
# loop above has already walked history, so without this the one place meant
# for a summary contributed nothing: v0.204.19 carried its "fixes users have
# not received yet" recap there. `- internal:` is dropped the same as
# anywhere else.
while IFS= read -r line; do
  [ -z "$line" ] && continue
  case "$line" in
    -\ internal:*) continue ;;
  esac
  classify "$line"
done < <(printf '%s\n' "$MSG" | grep '^\s*[-*] ' | sed 's/^\s*//; s/^\*/-/' || true)

# Deduplicate each section
FEATURES=$(echo "$FEATURES" | awk '!seen[$0]++' | sed '/^$/d')
FIXES=$(echo "$FIXES" | awk '!seen[$0]++' | sed '/^$/d')
IMPROVEMENTS=$(echo "$IMPROVEMENTS" | awk '!seen[$0]++' | sed '/^$/d')

CHANGELOG_FILE="RELEASE_CHANGELOG.md"

# An announcement is a deliberate message to everyone, written between
# `--- announcement ---` and `--- end announcement ---` in the release commit.
# It is carried through to the tag annotation with its markers intact, because
# the Discord step looks for them: an announcement posts as its own embed
# above the change list AND pings @everyone. Nothing generates one
# automatically, so no ordinary release can ping a whole server by accident.
ANNOUNCEMENT_BLOCK=$(printf '%s\n' "$MSG" \
  | sed -n '/^--- announcement ---$/,/^--- end announcement ---$/p' || true)

{
  if [ -n "$ANNOUNCEMENT_BLOCK" ]; then
    printf '%s\n\n' "$ANNOUNCEMENT_BLOCK"
  fi
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

# HW_PREVIEW_CHANGELOG=1 ./scripts/release.sh patch "msg" shows exactly what
# the announcement will say, without touching the version, the tree or the
# tag. Worth running before every release: a changelog is read by everyone
# and fixed by nobody.
if [ "${HW_PREVIEW_CHANGELOG:-0}" = "1" ]; then
  echo
  echo "===== changelog preview ($LAST_TAG..HEAD) ====="
  cat "$CHANGELOG_FILE"
  echo "===== end preview ====="
  rm -f "$CHANGELOG_FILE"
  exit 0
fi

# Bump version in BOTH tauri.conf.json AND the workspace Cargo.toml so the
# Rust binary's `env!("CARGO_PKG_VERSION")` stays in lockstep with the
# bundle version reported to the Tauri updater feed.
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

# Verification step — confirm `cargo metadata` reports the new version for
# the `hardwave-daw` package. This is the same interrogation Cargo itself
# does at compile time to populate `env!("CARGO_PKG_VERSION")`, so a green
# light here means the binary will report `API_VERSION` matching the new
# tauri.conf.json. A mismatch would silently DOS hot-swap the moment a
# manifest with a min_installer floor publishes — better to fail the
# release script before the commit lands than discover it post-tag.
# Skipped if cargo or jq is unavailable (release.sh must still work in
# minimal containers); CI's matching `assert workspace version` step in
# frontend-publish.yml is the load-bearing check.
if command -v cargo >/dev/null 2>&1 && command -v jq >/dev/null 2>&1; then
  META_VERSION=$(cargo metadata --format-version 1 --no-deps 2>/dev/null \
    | jq -r '.packages[] | select(.name=="hardwave-daw") | .version' \
    || echo "")
  if [ -z "$META_VERSION" ]; then
    echo "release.sh: warning — cargo metadata could not resolve hardwave-daw version; CI's frontend-publish guard will catch any drift"
  elif [ "$META_VERSION" != "$NEW_VERSION" ]; then
    echo "release.sh: ERROR — cargo metadata reports $META_VERSION but expected $NEW_VERSION." >&2
    echo "  workspace Cargo.toml or src-tauri/Cargo.toml is out of sync with the bumped version." >&2
    echo "  Aborting release before the bad commit lands." >&2
    exit 1
  else
    echo "release.sh: cargo metadata version $META_VERSION matches tauri.conf.json — OK"
  fi
fi

# Stage all changes, commit, push, tag.
# Pushes whichever branch is currently checked out — we live on
# `redesign-port-from-master` for the duration of the redesign work, so a
# hard-coded `master` aborts the release half-way through and leaves the
# tree committed but unpushed.
CURRENT_BRANCH=$(git rev-parse --abbrev-ref HEAD)
git add -A
git commit -m "$MSG"
# The commit this release is, captured now. The tag below names it by SHA
# rather than following HEAD: with HW_GATE_ON_PC the gate wait is minutes
# long, and anything committed in that window would otherwise ride into the
# release without the gate ever having seen it.
RELEASE_COMMIT=$(git rev-parse HEAD)
git push origin "$CURRENT_BRANCH"

if [ "${HW_GATE_ON_PC:-0}" = "1" ]; then
  RELEASE_SHA="$RELEASE_COMMIT"
  echo "release.sh: waiting for the Gate run on ${RELEASE_SHA:0:7} (the commit about to be tagged)..."
  GATE_OK=0
  # gate.yml starts from the push above; give the runner time to pick it up.
  for _ in $(seq 1 80); do
    sleep 20
    STATUS=$(gh run list --workflow Gate --limit 10 \
      --json headSha,status,conclusion \
      --jq "[.[] | select(.headSha == \"$RELEASE_SHA\")] | first | \"\(.status) \(.conclusion // \"pending\")\"" 2>/dev/null || echo "")
    case "$STATUS" in
      "completed success")
        GATE_OK=1
        break
        ;;
      "completed "*)
        echo "release.sh: the PC gate FAILED on this commit ($STATUS)." >&2
        echo "release.sh: nothing was tagged. Fix it and release again." >&2
        exit 1
        ;;
      *)
        printf '.'
        ;;
    esac
  done
  echo
  if [ "$GATE_OK" != "1" ]; then
    echo "release.sh: the PC gate did not finish in time (is the PC on?)." >&2
    echo "release.sh: nothing was tagged. Re-run, or gate locally without HW_GATE_ON_PC." >&2
    exit 1
  fi
  echo "release.sh: PC gate green on ${RELEASE_SHA:0:7}"
fi

# Tag the release — every release is a full v* build (fires release.yml
# across Windows / Mac / Linux and advances the auto-updater feed).
TAG_NAME="v$NEW_VERSION"
# --cleanup=verbatim: git treats a line starting with '#' as a comment and
# deletes it, which silently ate the "### New features" headings out of every
# tag annotation and therefore out of the GitHub release body.
git tag -a "$TAG_NAME" --cleanup=verbatim -F "$CHANGELOG_FILE" "$RELEASE_COMMIT"
git push origin "$TAG_NAME"

# Clean up changelog file

echo "Released $TAG_NAME ($BUMP_TYPE, full build) — CI at https://github.com/Dishairano/hardwave-daw/actions"
echo "Users update via Help → Check for updates… once Windows / Mac / Linux artifacts are ready."
