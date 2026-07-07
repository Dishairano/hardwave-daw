#!/usr/bin/env bash
# One-command pre-push gate: everything CI checks, locally, in order of
# speed. Run this before pushing to avoid discovering failures at tag
# time (the v0.204.8 lesson: partial local checks let a broken release
# out; release.sh now runs the full test gate, and this script is the
# cheaper "am I clean?" loop for everyday development).
#
# Usage: ./scripts/lint.sh [--fix]
#   --fix  apply rustfmt instead of just checking
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"

FAILED=0
step() {
  echo
  echo "── $1"
  shift
  if "$@"; then
    echo "   OK"
  else
    echo "   FAILED: $*"
    FAILED=1
  fi
}

if [ "${1:-}" = "--fix" ]; then
  step "cargo fmt (applying)" cargo fmt --all
else
  step "cargo fmt --check" cargo fmt --all -- --check
fi

step "frontend typecheck (tsc -b --noEmit)" \
  npm run --prefix packages/daw-ui typecheck

step "cargo clippy (warnings are errors)" \
  cargo clippy --workspace --all-targets -- -D warnings

if [ "$FAILED" -ne 0 ]; then
  echo
  echo "lint.sh: one or more checks failed — fix before pushing."
  exit 1
fi
echo
echo "lint.sh: all checks green."
