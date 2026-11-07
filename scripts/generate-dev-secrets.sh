#!/usr/bin/env bash
#
# Generate development-only key material under deploy/secrets-dev/.
# Never use these keys in production.
#
# Requires openssl (1.1.1+ for Ed25519).

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target="$root/deploy/secrets-dev"
key_id="dev-2026-11"

if ! command -v openssl >/dev/null 2>&1; then
    echo "error: openssl is required to generate development secrets" >&2
    exit 1
fi

if [ -e "$target/jwt-ed25519-private.pem" ]; then
    echo "error: $target already contains secrets; remove it first to regenerate" >&2
    exit 1
fi

umask 077
mkdir -p "$target"

openssl genpkey -algorithm ED25519 -out "$target/jwt-ed25519-private.pem"
openssl rand -base64 32 > "$target/refresh-derivation.key"
openssl rand -base64 32 > "$target/throttle.key"

audit_key="$(openssl rand -base64 32)"
printf '{"%s":"%s"}\n' "$key_id" "$audit_key" > "$target/audit-chain-keys.json"

chmod 600 "$target"/*

cat <<EOF
Development secrets written to $target (mode 0600).

Load them with the defaults in .env.example:
  cp .env.example .env

AUDIT_CHAIN_KEY_ID=$key_id
EOF
