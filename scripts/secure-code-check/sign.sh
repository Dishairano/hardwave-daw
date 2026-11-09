#!/usr/bin/env bash
# Secure Code Check: sign the release report with our own key.
#
#   sign.sh check          the key is there and is the one whose public
#                          half is in security/secure-code-check/
#   sign.sh sign FILE...   writes FILE.sig for each, then verifies it
#                          against allowed_signers, as a buyer would
#
# The private key comes from the SECURE_CODE_CHECK_SIGNING_KEY secret (an
# OpenSSH Ed25519 private key, no passphrase). It is written to a 0600
# file in the runner's temp folder, used, and removed; it is never echoed.
# Signing uses ssh-keygen, which every runner and every buyer's machine
# already has, so checking a signature needs no extra tool.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
keys="$here/../../security/secure-code-check"
namespace="hardwave-secure-code-check"
identity="secure-code-check@hardwavestudios.com"

if [ -z "${SECURE_CODE_CHECK_SIGNING_KEY:-}" ]; then
  echo "::error::The SECURE_CODE_CHECK_SIGNING_KEY secret is not set. A release needs a signed Secure Code Check report, so nothing is built until it is added."
  exit 1
fi

# mktemp creates the file 0600.
key_file="$(mktemp "${RUNNER_TEMP:-/tmp}/scc-key.XXXXXX")"
trap 'rm -f "$key_file"' EXIT
printf '%s\n' "$SECURE_CODE_CHECK_SIGNING_KEY" > "$key_file"

if ! derived="$(ssh-keygen -y -f "$key_file" 2>/dev/null)"; then
  echo "::error::SECURE_CODE_CHECK_SIGNING_KEY is not a readable OpenSSH private key without a passphrase."
  exit 1
fi
expected="$(awk '{print $1" "$2}' "$keys/signing_key.pub")"
if [ "$(echo "$derived" | awk '{print $1" "$2}')" != "$expected" ]; then
  echo "::error::SECURE_CODE_CHECK_SIGNING_KEY does not match security/secure-code-check/signing_key.pub."
  exit 1
fi

case "${1:-}" in
  check)
    echo "Signing key present and matches $(ssh-keygen -lf "$keys/signing_key.pub" | awk '{print $2}')"
    ;;
  sign)
    shift
    [ "$#" -gt 0 ] || { echo "sign.sh sign: no files given" >&2; exit 2; }
    for file in "$@"; do
      rm -f "$file.sig"
      ssh-keygen -q -Y sign -f "$key_file" -n "$namespace" "$file"
      ssh-keygen -Y verify -f "$keys/allowed_signers" -I "$identity" -n "$namespace" \
        -s "$file.sig" < "$file"
    done
    ;;
  *)
    echo "usage: sign.sh check | sign FILE..." >&2
    exit 2
    ;;
esac
