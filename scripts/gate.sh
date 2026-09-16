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
