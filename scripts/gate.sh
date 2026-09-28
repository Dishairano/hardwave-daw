#!/usr/bin/env bash
set -euo pipefail

# Run the release gate once, and record that this exact tree passed it.
#
# The gate (fmt + clippy + cargo test --workspace + the frontend checks) is
# the same set CI's "Lint & Test (Linux)" job runs. It takes about 15 minutes
# on this box, and until now it ran twice per release: once by hand to catch
# failures before a tag exists, then again inside release.sh. The second run
# tests an identical tree with an identical toolchain, so it can only ever
# repeat the first answer.
#
# This script writes a stamp naming the tree fingerprint and the toolchain it
# proved green. release.sh skips its own gate only when that stamp matches
# what it is about to release. Edit any file, or change rustc, and the
# fingerprint changes and the gate runs in full.
#
# Usage:
#   ./scripts/gate.sh                 run the gate, stamp on success
#   ./scripts/gate.sh --stamp-path    print the stamp path for this tree
#
# The stamp lives in .git/, so it is never committed and never shared between
# machines: a green gate on one machine cannot vouch for another.

cd "$(git rev-parse --show-toplevel)"

# rustup installs here and non-login shells do not always have it on PATH.
export PATH="$HOME/.cargo/bin:$PATH"

# One gate at a time. Two runs share the same target directory, and the
# cache clean-up below can delete the incremental files the other run is
# writing, which surfaces as "failed to move dependency graph" and reads
# like a broken toolchain. A second gate waits rather than racing.
if [ -z "${HW_GATE_LOCKED:-}" ]; then
  export HW_GATE_LOCKED=1
  exec flock "$(git rev-parse --git-dir)/hw-gate.lock" "$0" "$@"
fi

# Builds die strangely when the target volume fills: rustc is killed and
# cargo reports "failed to parse process output", which reads like a
# compiler bug rather than a full disk. The incremental cache is the part
# that grows without bound and the part builds can rebuild, so it goes
# first, and only when the volume is nearly full.
free_target_gib() {
  local dir
  dir=$(cargo metadata --format-version 1 --no-deps 2>/dev/null \
    | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
  [ -n "$dir" ] || dir="target"
  mkdir -p "$dir" 2>/dev/null || true
  df -BG --output=avail "$dir" 2>/dev/null | tail -1 | tr -dc '0-9'
}
target_dir() {
  cargo metadata --format-version 1 --no-deps 2>/dev/null \
    | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p'
}
avail=$(free_target_gib)
if [ -n "$avail" ] && [ "$avail" -lt 12 ]; then
  echo "gate.sh: only ${avail}G free on the build volume; clearing the incremental cache"
  find "$(target_dir)" -maxdepth 2 -type d -name incremental -exec rm -rf {} + 2>/dev/null || true
  avail=$(free_target_gib)
  echo "gate.sh: ${avail}G free now"
fi
# The incremental cache is a few gigabytes; the compiled dependencies are
# tens. When dropping the cache is not enough, the debug build goes too. It
# costs one slow build and it is the difference between a gate that runs and
# a gate that dies with "No space left on device" halfway through linking.
if [ -n "$avail" ] && [ "$avail" -lt 8 ]; then
  echo "gate.sh: still ${avail}G free; clearing the debug build as well"
  rm -rf "$(target_dir)/debug" 2>/dev/null || true
  echo "gate.sh: $(free_target_gib)G free now, this build will be a slow one"
fi

# Fingerprint = the exact content of the working tree, as a git tree hash.
#
# Built in a throwaway index so the real one is never touched. The first
# version of this hashed `git status` output as well, which meant committing
# the very content that had just passed the gate changed the fingerprint and
# threw the stamp away: the gate then ran again on byte-identical files. A
# tree hash only changes when content changes, which is the question being
# asked.
tree_fingerprint() {
  local index tree
  index=$(mktemp)
  GIT_INDEX_FILE="$index" git read-tree HEAD 2>/dev/null || true
  GIT_INDEX_FILE="$index" git add -A 2>/dev/null
  tree=$(GIT_INDEX_FILE="$index" git write-tree)
  rm -f "$index"
  echo "$tree"
}

stamp_path() {
  local toolchain
  # No fallback string here on purpose: a stamp that does not name a real
  # rustc would match on any toolchain, which is the drift this guards.
  toolchain=$(rustc -V 2>/dev/null | tr ' /()' '____') || true
  if [ -z "$toolchain" ]; then
    echo "gate.sh: rustc not found on PATH, cannot identify the toolchain" >&2
    exit 2
  fi
  echo ".git/hw-gate-$(tree_fingerprint)-${toolchain}"
}

if [ "${1:-}" = "--stamp-path" ]; then
  stamp_path
  exit 0
fi

# Two parallel rustc jobs get OOM-killed on this 3.7 GB host while the
# production site is running. Override for a machine with real memory.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}"

# Pin the fingerprint before the first check runs. The stamp is only written
# if the tree is still identical at the end, so an edit (or a commit) landing
# mid-run can never stamp a tree that was not the one actually tested.
FINGERPRINT_AT_START=$(tree_fingerprint)

echo "Gate: frontend (typecheck, unit tests, build)..."
(cd packages/daw-ui && npm run typecheck && npm run test:unit && npm run build)

echo "Gate: cargo fmt --check..."
cargo fmt --all -- --check

echo "Gate: cargo clippy --workspace --all-targets -D warnings..."
cargo clippy --workspace --all-targets -- -D warnings

echo "Gate: cargo test --workspace..."
cargo test --workspace

if [ "$(tree_fingerprint)" != "$FINGERPRINT_AT_START" ]; then
  echo "gate.sh: the tree changed while the gate ran, so these results belong" >&2
  echo "gate.sh: to a tree that no longer exists. Nothing stamped; re-run." >&2
  exit 3
fi

STAMP=$(stamp_path)
: > "$STAMP"
echo "Gate GREEN. Stamped $STAMP"
echo "release.sh will skip its own gate for this tree; any edit invalidates it."
