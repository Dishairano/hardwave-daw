#!/usr/bin/env bash
# packages/daw-ui/vendor/picomatch is picomatch 4.0.7 with our fix for
# GHSA-fc5q-39xc-vjpf (see its README). npm audit and Dependabot never look
# at a local package, so this script does, for the "picomatch fork" job in
# .github/workflows/checks.yml:
#
#   tests     the fork's own tests, a check that only lib/parse.js differs
#             from 4.0.7, and picomatch's own test suite at the 4.0.7 tag run
#             against the fork (on node:test, with the pinned
#             scripts/picomatch-fork-check/package-lock.json)
#   upstream  fails when picomatch has published a release after 4.0.7, when
#             a picomatch advisory exists that is not in REVIEWED below, or
#             when GHSA-fc5q-39xc-vjpf is not among them (then the lookup
#             itself is broken); any of these means the fork needs a look
#
# Usage: scripts/picomatch-fork-check.sh tests|upstream|all
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
fork="$root/packages/daw-ui/vendor/picomatch"
BASE_VERSION=4.0.7
BASE_COMMIT=6bb40679c218cefba5d4e9662408c5fdf699a8bb # the 4.0.7 tag
# Fixed before 4.0.7 (3v7f, c2c7), or fixed in the fork (fc5q).
REVIEWED="GHSA-3v7f-55p6-f55p GHSA-c2c7-rcm5-vvqj GHSA-fc5q-39xc-vjpf"

work=""
trap '[ -z "$work" ] || rm -rf "$work"' EXIT

run_tests() {
  node --test "$fork/test/"

  work=$(mktemp -d)
  git init -q "$work/upstream"
  git -C "$work/upstream" fetch -q --depth 1 https://github.com/micromatch/picomatch.git "$BASE_COMMIT"
  git -C "$work/upstream" checkout -q FETCH_HEAD

  local changed="" f
  for f in index.js posix.js LICENSE $(cd "$work/upstream" && ls lib/*.js); do
    cmp -s "$fork/$f" "$work/upstream/$f" || changed="$changed $f"
  done
  if [ "$changed" != " lib/parse.js" ]; then
    echo "::error::vendor/picomatch should differ from picomatch $BASE_VERSION in lib/parse.js only; differs in:$changed"
    return 1
  fi

  # The fork in place of the 4.0.7 sources, then picomatch's own suite.
  cp "$fork/lib/parse.js" "$work/upstream/lib/parse.js"
  cp "$root/scripts/picomatch-fork-check/package.json" "$root/scripts/picomatch-fork-check/package-lock.json" "$work/"
  (cd "$work" && npm ci --ignore-scripts --no-audit --no-fund >/dev/null)
  (cd "$work/upstream" && node --require "$root/scripts/picomatch-fork-check/shim.js" \
    --test --test-reporter=tap test/*.js | grep -E '^# (tests|pass|fail)')
}

check_upstream() {
  local status=0 latest ids id
  latest=$(npm view picomatch version)
  if [ "$latest" != "$BASE_VERSION" ]; then
    echo "::error::picomatch $latest is out (the fork is $BASE_VERSION). If it fixes GHSA-fc5q-39xc-vjpf, drop vendor/picomatch; if not, rebase the fork on it."
    status=1
  fi
  ids=$(gh api -X GET /advisories -f ecosystem=npm -f affects=picomatch -f per_page=100 --jq '.[].ghsa_id')
  case " $(echo $ids) " in
    *" GHSA-fc5q-39xc-vjpf "*) ;;
    *)
      echo "::error::GHSA-fc5q-39xc-vjpf is not among picomatch's advisories (got: ${ids:-none}). Either the lookup failed, or the advisory was withdrawn: check, and drop vendor/picomatch if it was."
      status=1
      ;;
  esac
  for id in $ids; do
    case " $REVIEWED " in
      *" $id "*) ;;
      *)
        echo "::error::New picomatch advisory $id (https://github.com/advisories/$id): check whether vendor/picomatch is affected, then add it to REVIEWED in $0."
        status=1
        ;;
    esac
  done
  [ "$status" -ne 0 ] || echo "picomatch: latest $latest, advisories all reviewed:" $ids
  return "$status"
}

case "${1:-all}" in
  tests) run_tests ;;
  upstream) check_upstream ;;
  all) run_tests && check_upstream ;;
  *) echo "usage: $0 tests|upstream|all" >&2; exit 2 ;;
esac
