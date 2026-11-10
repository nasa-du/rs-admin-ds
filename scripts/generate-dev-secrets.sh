#!/usr/bin/env bash
#
# Generate development-only key material for the Compose stack under
# deploy/secrets-dev/. Never use these keys in production.
#
# Requires openssl (1.1.1+ for Ed25519).
#
# Override the destination with RS_ADMIN_SECRETS_TARGET (used by tests), and
# the certificate names with ADMIN_HOST / API_HOST / DB_HOST.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target="${RS_ADMIN_SECRETS_TARGET:-$root/deploy/secrets-dev}"
admin_host="${ADMIN_HOST:-admin.example.com}"
api_host="${API_HOST:-api.example.com}"
db_host="${DB_HOST:-db}"
database_name="${POSTGRES_DB:-rs_admin}"
database_port="${POSTGRES_PORT:-5432}"
key_id="${AUDIT_CHAIN_KEY_ID:-dev-2026-11}"

if ! command -v openssl >/dev/null 2>&1; then
    echo "error: openssl is required to generate development secrets" >&2
    exit 1
fi

if [ -d "$target" ] && [ -n "$(ls -A "$target" 2>/dev/null)" ]; then
    echo "error: $target already contains secrets; remove it first to regenerate" >&2
    exit 1
fi

umask 077
mkdir -p "$target"

write_secret() {
    local name="$1"
    local value="$2"
    printf '%s\n' "$value" >"$target/$name"
}

random_password() {
    openssl rand -hex 24
}

build_dsn() {
    local user="$1"
    local password="$2"
    local host="$3"
    local port="$4"
    local root_cert="$5"

    printf 'postgres://%s:%s@%s:%s/%s?sslmode=verify-full&sslrootcert=%s' \
        "$user" "$password" "$host" "$port" "$database_name" "$root_cert"
}

issue_server_certificate() {
    local certificate="$1"
    local key="$2"
    local common_name="$3"
    local subject_alt_names="$4"
    local serial_file="$target/internal_ca.srl"
    local serial_args=()
    local csr
    local extensions

    if [ -s "$serial_file" ]; then
        serial_args=(-CAserial "$serial_file")
    else
        serial_args=(-CAcreateserial)
    fi

    csr="$(mktemp "$target/.csr.XXXXXX")"
    extensions="$(mktemp "$target/.ext.XXXXXX")"

    openssl req -newkey rsa:2048 -sha256 -nodes \
        -subj "/CN=$common_name" \
        -keyout "$key" \
        -out "$csr"

    {
        printf 'subjectAltName=%s\n' "$subject_alt_names"
        printf 'extendedKeyUsage=serverAuth\n'
        printf 'keyUsage=critical,digitalSignature,keyEncipherment\n'
    } >"$extensions"

    openssl x509 -req -sha256 -days 3650 \
        -in "$csr" \
        -CA "$target/internal_ca.pem" \
        -CAkey "$target/internal-ca.key" \
        "${serial_args[@]}" \
        -extfile "$extensions" \
        -out "$certificate"

    rm -f "$csr" "$extensions"
}

# Identity and application secrets.
openssl genpkey -algorithm ED25519 -out "$target/jwt-ed25519-private.pem"
openssl rand -base64 32 >"$target/refresh-derivation.key"
openssl rand -base64 32 >"$target/throttle.key"

audit_key="$(openssl rand -base64 32)"
printf '{"%s":"%s"}\n' "$key_id" "$audit_key" >"$target/audit-chain-keys.json"

postgres_admin_password="$(random_password)"
migrator_password="$(random_password)"
runtime_password="$(random_password)"
cleanup_password="$(random_password)"
backup_password="$(random_password)"

write_secret postgres_admin_password "$postgres_admin_password"
write_secret migrator_password "$migrator_password"
write_secret runtime_password "$runtime_password"
write_secret cleanup_password "$cleanup_password"
write_secret backup_password "$backup_password"

# pgBackRest repository credentials. Development uses placeholder S3 values;
# the backup job is only meaningful with a reachable S3-compatible repository.
write_secret backup_s3_access_key "$(random_password)"
write_secret backup_s3_secret_key "$(random_password)"
write_secret backup_repo_cipher_pass "$(openssl rand -hex 32)"

# Internal CA and server certificates.
openssl req -x509 -newkey rsa:2048 -sha256 -nodes -days 3650 \
    -subj "/CN=rs-admin-dev-ca" \
    -keyout "$target/internal-ca.key" \
    -out "$target/internal_ca.pem"

issue_server_certificate \
    "$target/postgres_server_cert" \
    "$target/postgres_server_key" \
    "$db_host" \
    "DNS:$db_host,DNS:localhost,IP:127.0.0.1"

issue_server_certificate \
    "$target/tls_certificate" \
    "$target/tls_key" \
    "$admin_host" \
    "DNS:$admin_host,DNS:$api_host,DNS:localhost,IP:127.0.0.1"

rm -f "$target/internal_ca.srl" "$target/internal-ca.srl"

# Container DSNs: the API and migrate containers verify the mounted CA.
write_secret database_url "$(build_dsn \
    rs_admin_runtime "$runtime_password" "$db_host" 5432 /run/secrets/internal_ca)"
write_secret database_url_migrator "$(build_dsn \
    rs_admin_migrator "$migrator_password" "$db_host" 5432 /run/secrets/internal_ca)"
write_secret database_url_cleanup "$(build_dsn \
    rs_admin_cleanup "$cleanup_password" "$db_host" 5432 /run/secrets/internal_ca)"

# Host DSN for a cargo-run API against the loopback-published database.
write_secret host.env "DATABASE_URL=$(build_dsn \
    rs_admin_runtime "$runtime_password" 127.0.0.1 "$database_port" "$target/internal_ca.pem")"

# The database entrypoint runs as root and copies its key material; the
# gateway (uid 101) and API (uid 10001) read their secrets as non-root, so the
# development files are world-readable and the directory is traversable.
# Production files are group-readable.
chmod 0711 "$target"
chmod 0600 "$target/internal-ca.key" "$target/postgres_server_key" "$target/host.env"
chmod 0444 \
    "$target/jwt-ed25519-private.pem" \
    "$target/refresh-derivation.key" \
    "$target/throttle.key" \
    "$target/audit-chain-keys.json" \
    "$target/postgres_admin_password" \
    "$target/migrator_password" \
    "$target/runtime_password" \
    "$target/cleanup_password" \
    "$target/backup_password" \
    "$target/backup_s3_access_key" \
    "$target/backup_s3_secret_key" \
    "$target/backup_repo_cipher_pass" \
    "$target/internal_ca.pem" \
    "$target/postgres_server_cert" \
    "$target/tls_certificate" \
    "$target/tls_key" \
    "$target/database_url" \
    "$target/database_url_migrator" \
    "$target/database_url_cleanup"

cat <<EOF
Development secrets written to $target.

Full stack (from the repository root):
  cd deploy
  cp .env.example .env
  docker compose --profile local-db --profile tools run --rm migrate
  docker compose --profile local-db up -d

Host development database:
  docker compose -f deploy/compose.yaml -f deploy/compose.dev.yaml \\
    --profile local-db up -d db
  Add the DATABASE_URL from $target/host.env to the repository .env.

AUDIT_CHAIN_KEY_ID=$key_id
EOF
