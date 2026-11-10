#!/usr/bin/env bash
#
# Static checks for the production Compose stack and its gateway/static-server
# configuration. This is the acceptance seam for the deployment ticket: it
# reads the normalized `docker compose config` output and the checked-in
# configuration files, so it fails when the stack stops meeting the spec.
#
# It intentionally does not start containers or pull images.

set -euo pipefail

# shellcheck source=lib/deploy-check.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib/deploy-check.sh"
cd "$root"

# Assert that a jq expression evaluates true against the compose document.
expect_compose() {
    local label="$1"
    local expression="$2"
    if jq -e "$expression" >/dev/null <<<"$compose"; then
        pass "$label"
    else
        fail "$label"
    fi
}

# Assert that a file contains an extended regular expression.
expect_file() {
    local label="$1"
    local file="$2"
    local pattern="$3"
    if [[ ! -f "$file" ]]; then
        fail "$label (missing $file)"
    elif grep -Eq -- "$pattern" "$file"; then
        pass "$label"
    else
        fail "$label ($file does not match /$pattern/)"
    fi
}

expect_file_absent() {
    local label="$1"
    local file="$2"
    local pattern="$3"
    if [[ ! -f "$file" ]]; then
        fail "$label (missing $file)"
    elif grep -Eq -- "$pattern" "$file"; then
        fail "$label ($file unexpectedly matches /$pattern/)"
    else
        pass "$label"
    fi
}

if ! command -v jq >/dev/null 2>&1; then
    echo "error: jq is required" >&2
    exit 1
fi

compose="$(docker compose -f deploy/compose.yaml --profile local-db --profile tools config --format json)"

# --- Service topology -------------------------------------------------------
expect_compose \
    "stack defines gateway, web, api, db and migrate" \
    '.services | keys | sort == ["api", "db", "gateway", "migrate", "web"]'

expect_compose \
    "only the gateway publishes host ports" \
    '([.services | to_entries[] | select(.key != "gateway") | (.value.ports // [])[]] | length) == 0'

expect_compose \
    "gateway publishes exactly 80 and 443" \
    '(.services.gateway.ports | length) == 2 and ([.services.gateway.ports[].published] | sort) == ["443", "80"]'

expect_compose \
    "gateway maps 80 to the unprivileged HTTP port" \
    '(.services.gateway.ports[] | select(.published == "80") | .target) == 8080'

expect_compose \
    "gateway maps 443 to the unprivileged HTTPS port" \
    '(.services.gateway.ports[] | select(.published == "443") | .target) == 8443'

expect_compose \
    "api, web and db are never published" \
    '([.services.api, .services.web, .services.db] | map(.ports // []) | flatten | length) == 0'

expect_compose \
    "db is opt-in through the local-db profile" \
    '.services.db.profiles | index("local-db") != null'

expect_compose \
    "migrate is a one-shot task enabled for local-db and tools" \
    '(.services.migrate.profiles | index("tools") != null) and (.services.migrate.profiles | index("local-db") != null)'

expect_compose \
    "api waits for the local migration job before starting" \
    '.services.api.depends_on.migrate.condition == "service_completed_successfully"'

expect_compose \
    "api joins both the edge and data networks" \
    '[.services.api.networks | keys[]] | sort == ["data", "edge"]'

expect_compose \
    "db only joins the internal data network" \
    '(.services.db.networks | keys) == ["data"]'

expect_compose \
    "the edge network has a fixed subnet for trusted proxies" \
    '.networks.edge.ipam.config[0].subnet == "172.28.0.0/16"'

expect_compose \
    "the data network is internal" \
    '.networks.data.internal == true'

# --- Container hardening ----------------------------------------------------
for service in api web gateway db migrate; do
    expect_compose \
        "$service sets no-new-privileges" \
        "[.services.$service.security_opt[] | select(. == \"no-new-privileges:true\")] | length == 1"
done

expect_compose \
    "api runs as the non-root application user" \
    '.services.api.user == "10001:10001"'

expect_compose \
    "api has a read-only root filesystem" \
    '.services.api.read_only == true'

expect_compose \
    "api mounts a writable tmpfs for /tmp" \
    '[.services.api.tmpfs[] | select(startswith("/tmp"))] | length == 1'

expect_compose \
    "api healthcheck probes /health/ready" \
    '.services.api.healthcheck.test | tostring | contains("/health/ready")'

expect_compose \
    "api allows 25 seconds for graceful shutdown" \
    '.services.api.stop_grace_period == "25s"'

expect_compose \
    "api limits memory to 1 GiB" \
    '.services.api.deploy.resources.limits.memory == "1073741824" or .services.api.deploy.resources.limits.memory == "1g"'

expect_compose \
    "api pool is 10 connections per replica" \
    '.services.api.environment.DATABASE_MAX_CONNECTIONS == "10"'

expect_file \
    "API request timeout is 30 seconds" \
    "backend/crates/api/src/router.rs" \
    'pub const REQUEST_TIMEOUT: Duration = Duration::from_secs\(30\);'

expect_file \
    "API drains in-flight requests for 15 seconds" \
    "backend/crates/api/src/main.rs" \
    'const SHUTDOWN_GRACE: Duration = Duration::from_secs\(15\);'

expect_file \
    "connection pool default is 10 connections" \
    "backend/crates/config/src/lib.rs" \
    '"DATABASE_MAX_CONNECTIONS", 10'

expect_compose \
    "trusted proxy CIDR covers the Compose edge network" \
    '.services.api.environment.TRUSTED_PROXY_CIDRS == "172.28.0.0/16"'

# --- Secrets ----------------------------------------------------------------
expect_compose \
    "api reads the database DSN from a mounted file" \
    '.services.api.environment.DATABASE_URL_FILE == "/run/secrets/database_url"'

expect_compose \
    "api reads signing material from mounted files" \
    '.services.api.environment.JWT_EDDSA_PRIVATE_KEY_FILE == "/run/secrets/jwt_ed25519_private" and .services.api.environment.REFRESH_DERIVATION_KEY_FILE == "/run/secrets/refresh_derivation_key" and .services.api.environment.THROTTLE_KEY_FILE == "/run/secrets/throttle_key" and .services.api.environment.AUDIT_CHAIN_KEYS_FILE == "/run/secrets/audit_chain_keys"'

expect_compose \
    "api mounts the internal CA" \
    '.services.api.secrets | tostring | contains("internal_ca")'

expect_compose \
    "migrate uses the migrator role DSN from a mounted file" \
    '.services.migrate.environment.DATABASE_URL_FILE == "/run/secrets/database_url_migrator"'

expect_compose \
    "db reads its bootstrap password from a mounted file" \
    '.services.db.environment.POSTGRES_PASSWORD_FILE == "/run/secrets/postgres_admin_password"'

expect_compose \
    "db is pinned to PostgreSQL 18.6 alpine" \
    '.services.db.image == "postgres:18.6-alpine"'

expect_compose \
    "db preloads pg_stat_statements" \
    '.services.db.command | tostring | contains("shared_preload_libraries=pg_stat_statements")'

expect_compose \
    "db leaves WAL archiving off until pgBackRest is configured" \
    '.services.db.command | tostring | contains("archive_mode=off")'

expect_compose \
    "all services cap the Docker log buffer" \
    '[.services[] | .logging.options["max-size"] == "10m" and .logging.options["max-file"] == "5"] | all'

# --- Gateway configuration --------------------------------------------------
gateway="deploy/gateway/templates/default.conf.template"
expect_file "gateway terminates TLS on the unprivileged port" "$gateway" 'listen 8443 ssl'
expect_file "gateway enables TLS 1.2 and 1.3" "$gateway" 'ssl_protocols TLSv1\.2 TLSv1\.3'
expect_file "gateway redirects HTTP to HTTPS with 301" "$gateway" 'return 301 https://'
expect_file "gateway emits HSTS" "$gateway" 'Strict-Transport-Security.*max-age=31536000.*includeSubDomains'
expect_file "gateway applies the auth rate limit" "$gateway" 'limit_req_zone \$binary_remote_addr zone=auth:10m rate=60r/m;'
expect_file "gateway uses a burst of 10 without queuing" "$gateway" 'limit_req zone=auth burst=10 nodelay;'
expect_file "gateway returns 429 on rate limit" "$gateway" 'limit_req_status 429;'
expect_file "gateway returns Retry-After on 429" "$gateway" 'Retry-After'
proxy_snippet="deploy/gateway/snippets/proxy-headers.conf"
expect_file "gateway uses a 30 second proxy timeout" "$proxy_snippet" 'proxy_(connect|read|send)_timeout 30s'
expect_file "gateway overwrites X-Forwarded-For" "$proxy_snippet" 'proxy_set_header X-Forwarded-For \$remote_addr;'
expect_file "gateway ignores client Forwarded headers" "$proxy_snippet" 'proxy_set_header Forwarded "";'
expect_file "gateway shares one proxy header snippet" "$gateway" 'include /etc/nginx/snippets/proxy-headers\.conf;'
expect_file "gateway caps auth request bodies at 1 KiB" "$gateway" 'client_max_body_size 1k'
expect_file "gateway resolves upstreams through Docker DNS" "$gateway" 'resolver 127\.0\.0\.11'
expect_file "gateway routes the admin host" "$gateway" 'server_name \$\{ADMIN_HOST\}'
expect_file "gateway routes the API host" "$gateway" 'server_name \$\{API_HOST\}'
expect_file "gateway blocks metrics" "$gateway" 'metrics'
expect_file "gateway blocks health endpoints" "$gateway" 'health'
expect_file "gateway blocks API docs" "$gateway" 'api-docs'

# --- Static frontend --------------------------------------------------------
web="frontend/nginx/default.conf.template"
expect_file "web falls back to index.html for SPA routes" "$web" 'try_files \$uri /index\.html'
expect_file "web caches hashed assets immutably" "$web" 'public, max-age=31536000, immutable'
expect_file "web does not cache index.html" "$web" 'no-cache'
expect_file "web emits a strict CSP" "$web" "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'"
expect_file "web CSP restricts connect-src to the API origin" "$web" "connect-src 'self' \\\$\{API_ORIGIN\}"
expect_file "web CSP denies framing" "$web" "frame-ancestors 'none'"

# --- Images and database roles ---------------------------------------------
expect_file "api image runs as the non-root app user" "backend/Dockerfile" '^USER 10001:10001$'
expect_file "api image is a multi-stage build without the toolchain" "backend/Dockerfile" '^FROM debian:bookworm-slim$'
expect_file "web builds with the pinned bun image" "frontend/Dockerfile" '^FROM oven/bun:1\.4\.2 AS build$'
expect_file "web runs on the pinned unprivileged nginx image" "frontend/Dockerfile" '^FROM nginxinc/nginx-unprivileged:1\.29\.8-alpine$'

expect_file "db requires TLS for host connections" "deploy/db/pg_hba.conf" '^hostssl .* scram-sha-256$'
expect_file_absent "db never accepts md5 authentication" "deploy/db/pg_hba.conf" '^[^#].*md5'
expect_file_absent "db rejects unencrypted host connections" "deploy/db/pg_hba.conf" '^host .* (trust|password|scram-sha-256)'
expect_file "production db only accepts the data subnet" "deploy/db/pg_hba.conf" '^hostssl .* 172\.29\.0\.0/16 +scram-sha-256$'
expect_file_absent "production db never accepts the edge subnet" "deploy/db/pg_hba.conf" '^hostssl .*172\.28\.0\.0/16'
expect_file_absent "production db has no host-development ranges" "deploy/db/pg_hba.conf" '172\.16\.0\.0/12'
expect_file "db creates the migration role" "deploy/db/init/00-rs-admin-roles.sh" 'CREATE ROLE rs_admin_migrator'
expect_file "db creates the runtime role" "deploy/db/init/00-rs-admin-roles.sh" 'CREATE ROLE rs_admin_runtime'
expect_file "db creates the cleanup role" "deploy/db/init/00-rs-admin-roles.sh" 'CREATE ROLE rs_admin_cleanup'
expect_file "db creates the backup role" "deploy/db/init/00-rs-admin-roles.sh" 'CREATE ROLE rs_admin_backup'
expect_file "runtime may only read and append audit rows" "deploy/db/grants.sql" 'GRANT SELECT, INSERT ON TABLE audit_logs TO rs_admin_runtime'
expect_file "runtime may not mutate audit rows" "deploy/db/grants.sql" 'REVOKE ALL ON TABLE audit_logs FROM rs_admin_runtime'
expect_file "runtime may read and advance the chain head" "deploy/db/grants.sql" 'GRANT SELECT, UPDATE ON TABLE audit_chain_head TO rs_admin_runtime'
expect_file "runtime may read migration state for readiness" "deploy/db/grants.sql" 'GRANT SELECT ON TABLE _sqlx_migrations TO rs_admin_runtime'

# --- Generated development secrets -----------------------------------------
if command -v openssl >/dev/null 2>&1; then
    secret_dir="$(mktemp -d)"
    trap 'rm -rf "$secret_dir"' EXIT

    if RS_ADMIN_SECRETS_TARGET="$secret_dir" ./scripts/generate-dev-secrets.sh >/dev/null 2>&1; then
        pass "development secret generator runs"
    else
        fail "development secret generator runs"
    fi

    expect_file \
        "generated runtime DSN verifies PostgreSQL TLS" \
        "$secret_dir/database_url" \
        'sslmode=verify-full'
    expect_file \
        "generated runtime DSN trusts the mounted internal CA" \
        "$secret_dir/database_url" \
        'sslrootcert=/run/secrets/internal_ca'
    expect_file \
        "generated database server certificate is valid PEM" \
        "$secret_dir/postgres_server_cert" \
        '^-----BEGIN CERTIFICATE-----$'
    if openssl x509 -in "$secret_dir/postgres_server_cert" -noout -text 2>/dev/null \
        | grep -q 'DNS:db'; then
        pass "generated database certificate covers the db host"
    else
        fail "generated database certificate covers the db host"
    fi

    expect_file \
        "generated gateway certificate is valid PEM" \
        "$secret_dir/tls_certificate" \
        '^-----BEGIN CERTIFICATE-----$'
    if openssl x509 -in "$secret_dir/tls_certificate" -noout -text 2>/dev/null \
        | grep -q 'DNS:admin.example.com'; then
        pass "generated gateway certificate covers the admin host"
    else
        fail "generated gateway certificate covers the admin host"
    fi
else
    echo "skip development secret generator checks (openssl not installed)"
fi

if [[ "$failures" -ne 0 ]]; then
    printf '\n%d deployment check(s) failed\n' "$failures" >&2
    exit 1
fi

printf '\nall deployment checks passed\n'
