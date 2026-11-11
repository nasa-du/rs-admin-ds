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
    "stack defines gateway, web, api, db, migrate, maintenance and backup" \
    '.services | keys | sort == ["api", "backup", "db", "gateway", "maintenance", "migrate", "web"]'

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
    "db joins the internal data network and the outbound storage network" \
    '(.services.db.networks | keys | sort) == ["data", "storage"]'

expect_compose \
    "db never joins the public edge network" \
    '(.services.db.networks | has("edge")) | not'

expect_compose \
    "the edge network has a fixed subnet for trusted proxies" \
    '.networks.edge.ipam.config[0].subnet == "172.28.0.0/16"'

expect_compose \
    "the data network is internal" \
    '.networks.data.internal == true'

expect_compose \
    "the storage network has egress for S3 archiving" \
    '(.networks.storage.internal // false) == false and .networks.storage.ipam.config[0].subnet == "172.30.0.0/16"'

# --- Container hardening ----------------------------------------------------
for service in api web gateway db migrate maintenance backup; do
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
    "db uses the pinned PostgreSQL/pgBackRest image" \
    '.services.db.image == "rs-admin-db:local"'

expect_compose \
    "db preloads pg_stat_statements" \
    '.services.db.command | tostring | contains("shared_preload_libraries=pg_stat_statements")'

expect_compose \
    "db enables WAL archiving" \
    '.services.db.command | tostring | contains("archive_mode=on")'

expect_compose \
    "db archives every WAL segment through pgBackRest" \
    '.services.db.command | tostring | contains("archive_command=. /usr/local/bin/rs-admin-pgbackrest-env && pgbackrest --stanza=rs-admin archive-push %p")'

expect_compose \
    "db bounds the WAL archiving interval" \
    '.services.db.command | tostring | contains("archive_timeout=60s")'

expect_compose \
    "db mounts pgBackRest repository secrets" \
    '[.services.db.secrets[] | .source] | sort == ["backup_password", "backup_repo_cipher_pass", "backup_s3_access_key", "backup_s3_secret_key", "cleanup_password", "internal_ca", "migrator_password", "monitor_password", "postgres_admin_password", "postgres_server_cert", "postgres_server_key", "runtime_password"]'

expect_compose \
    "db shares the PostgreSQL socket with the backup job" \
    '[.services.db.volumes[] | select(.target == "/var/run/postgresql")] | length == 1'

# --- Scheduled backup and maintenance ---------------------------------------
expect_compose \
    "maintenance and backup are one-shot tools-profile jobs" \
    '(.services.maintenance.profiles | index("tools") != null) and (.services.backup.profiles | index("tools") != null)'

expect_compose \
    "maintenance uses the dedicated cleanup role DSN" \
    '.services.maintenance.environment.DATABASE_URL_FILE == "/run/secrets/database_url_cleanup"'

expect_compose \
    "maintenance reads the audit chain keyring and retention settings" \
    '.services.maintenance.environment.AUDIT_CHAIN_KEYS_FILE == "/run/secrets/audit_chain_keys" and .services.maintenance.environment.AUDIT_RETENTION_INFO_DAYS == "180" and .services.maintenance.environment.AUDIT_RETENTION_CRITICAL_DAYS == "365"'

expect_compose \
    "maintenance runs the cleanup binary as the non-root app user" \
    '.services.maintenance.entrypoint == ["/usr/local/bin/rs-admin-maintenance"] and .services.maintenance.user == "10001:10001" and .services.maintenance.read_only == true'

expect_compose \
    "maintenance publishes duration and row-count metrics" \
    '.services.maintenance.environment.MAINTENANCE_METRICS_FILE == "/var/lib/rs-admin/metrics/rs-admin-maintenance.prom" and ([.services.maintenance.volumes[] | select(.target == "/var/lib/rs-admin/metrics")] | length) == 1'

expect_compose \
    "backup uses the database image and reads the live cluster" \
    '.services.backup.image == "rs-admin-db:local" and ([.services.backup.volumes[] | select(.target == "/var/lib/postgresql" and .read_only == true)] | length) == 1 and ([.services.backup.volumes[] | select(.target == "/var/run/postgresql")] | length) == 1'

expect_compose \
    "backup runs as postgres, is read-only and only joins the storage network" \
    '.services.backup.user == "70:70" and .services.backup.read_only == true and (.services.backup.networks | keys) == ["storage"]'

expect_compose \
    "backup publishes the success metric textfile" \
    '.services.backup.environment.BACKUP_METRICS_FILE == "/var/lib/rs-admin/metrics/rs-admin-backup.prom"'

expect_compose \
    "backup keeps 30 days of full backups" \
    '.services.backup.environment.PGBACKREST_REPO1_RETENTION_FULL == "30"'

expect_file \
    "database image is based on the pinned PostgreSQL 18.6 alpine tag" \
    "deploy/db/Dockerfile" \
    '^FROM postgres:18\.6-alpine$'

expect_file \
    "database image pins the pgBackRest package version" \
    "deploy/db/Dockerfile" \
    'ARG PGBACKREST_VERSION=2\.58\.0-r0'

expect_file \
    "pgBackRest stanza is named rs-admin" \
    "deploy/db/pgbackrest.conf" \
    '^\[rs-admin\]$'

expect_file \
    "pgBackRest archives to an S3-compatible repository" \
    "deploy/db/pgbackrest.conf" \
    '^repo1-type=s3$'

expect_file \
    "pgBackRest encrypts the repository" \
    "deploy/db/pgbackrest.conf" \
    '^repo1-cipher-type=aes-256-cbc$'

expect_file \
    "pgBackRest keeps a 30-day time-based full retention" \
    "deploy/db/pgbackrest.conf" \
    '^repo1-retention-full-type=time$'

expect_file \
    "pgBackRest retention is 30 days" \
    "deploy/db/pgbackrest.conf" \
    '^repo1-retention-full=30$'

expect_file \
    "the daily backup writes the success metric" \
    "deploy/db/backup.sh" \
    'pgbackrest_backup_last_success_timestamp'

expect_file \
    "a failed backup withholds the success timestamp and fails the unit" \
    "deploy/db/backup.sh" \
    'exit 1'

expect_file \
    "backup container secrets are copied for the postgres user" \
    "deploy/db/entrypoint.sh" \
    'backup_s3_access_key'

expect_file \
    "the monitoring password is copied for the postgres user" \
    "deploy/db/entrypoint.sh" \
    'monitor_password'

expect_file \
    "the cleanup role can read the migration version" \
    "deploy/db/grants.sql" \
    'GRANT SELECT ON TABLE _sqlx_migrations TO rs_admin_runtime, rs_admin_cleanup'

expect_file \
    "the API image carries the maintenance and verify binaries" \
    "backend/Dockerfile" \
    'rs-admin-maintenance'

expect_file \
    "the API image carries the audit-chain verifier" \
    "backend/Dockerfile" \
    'rs-admin-verify-audit-chain'

expect_file \
    "the restore drill restores from the repository" \
    "deploy/db/restore-drill-start.sh" \
    '--type=immediate --target-action=promote restore'

expect_file \
    "the restore drill verifies the audit chain" \
    "scripts/restore-drill.sh" \
    'rs-admin-verify-audit-chain'

expect_file \
    "the restore drill records the measured RTO" \
    "scripts/restore-drill.sh" \
    '"rto_seconds"'

# --- systemd timers ---------------------------------------------------------
expect_file \
    "backup runs daily at 03:00 UTC" \
    "deploy/systemd/rs-admin-backup.timer" \
    '^OnCalendar=\*-\*-\* 03:00:00$'

expect_file \
    "maintenance runs daily at 04:00 UTC" \
    "deploy/systemd/rs-admin-maintenance.timer" \
    '^OnCalendar=\*-\*-\* 04:00:00$'

expect_file \
    "the restore drill runs quarterly" \
    "deploy/systemd/rs-admin-restore-drill.timer" \
    '^OnCalendar=\*-01,04,07,10-01 05:30:00$'

expect_file \
    "backup is triggered by a persistent timer" \
    "deploy/systemd/rs-admin-backup.timer" \
    '^Persistent=true$'

expect_file \
    "maintenance is triggered by a persistent timer" \
    "deploy/systemd/rs-admin-maintenance.timer" \
    '^Persistent=true$'

expect_file \
    "failed scheduled jobs notify the alert hook" \
    "deploy/systemd/rs-admin-backup.service" \
    '^OnFailure=rs-admin-alert@%n\.service$'

expect_file \
    "systemd units use the documented absolute install path" \
    "deploy/systemd/rs-admin-backup.service" \
    '^WorkingDirectory=/opt/rs-admin/deploy$'

expect_file \
    "the restore drill unit uses an absolute script path" \
    "deploy/systemd/rs-admin-restore-drill.service" \
    '^ExecStart=/opt/rs-admin/scripts/restore-drill.sh$'

expect_compose \
    "all services cap the Docker log buffer" \
    '[.services[] | .logging.options["max-size"] == "10m" and .logging.options["max-file"] == "5"] | all'

# --- Gateway configuration --------------------------------------------------
gateway="deploy/gateway/templates/default.conf.template"
expect_file "gateway terminates TLS on the unprivileged port" "$gateway" 'listen 8443 ssl'
tls_snippet="deploy/gateway/snippets/tls.conf"
expect_file "gateway enables TLS 1.2 and 1.3" "$tls_snippet" 'ssl_protocols TLSv1\.2 TLSv1\.3'
expect_file "gateway loads the certificate from a mounted secret" "$tls_snippet" 'ssl_certificate /run/secrets/tls_certificate;'
expect_file "gateway shares one TLS snippet" "$gateway" 'include /etc/nginx/snippets/tls\.conf;'
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
expect_file "db creates the monitoring role" "deploy/db/init/00-rs-admin-roles.sh" 'CREATE ROLE rs_admin_monitor'
expect_file "db grants pg_monitor to the monitoring role" "deploy/db/init/00-rs-admin-roles.sh" 'GRANT pg_monitor TO rs_admin_monitor'
expect_file "production db accepts the monitoring role on the data subnet" "deploy/db/pg_hba.conf" '^hostssl .* rs_admin_monitor +172\.29\.0\.0/16 +scram-sha-256$'
expect_file "development db accepts the monitoring role on loopback" "deploy/db/pg_hba.dev.conf" '^hostssl .* rs_admin_monitor +127\.0\.0\.1/32 +scram-sha-256$'
expect_file "runtime may only read and append audit rows" "deploy/db/grants.sql" 'GRANT SELECT, INSERT ON TABLE audit_logs TO rs_admin_runtime'
expect_file "runtime may not mutate audit rows" "deploy/db/grants.sql" 'REVOKE ALL ON TABLE audit_logs FROM rs_admin_runtime'
expect_file "runtime may read and advance the chain head" "deploy/db/grants.sql" 'GRANT SELECT, UPDATE ON TABLE audit_chain_head TO rs_admin_runtime'
expect_file "runtime may read migration state for readiness" "deploy/db/grants.sql" 'GRANT SELECT ON TABLE _sqlx_migrations TO rs_admin_runtime'

# --- Runbooks, drills and upgrade path -------------------------------------
runbooks=(
    docs/runbooks/README.md
    docs/runbooks/first-deploy.md
    docs/runbooks/ci-cd.md
    docs/runbooks/secret-rotation.md
    docs/runbooks/backup-restore.md
    docs/runbooks/postgres-upgrade.md
    docs/runbooks/alerting.md
    docs/runbooks/break-glass.md
    docs/runbooks/certificate-renewal.md
)
for runbook in "${runbooks[@]}"; do
    if [[ -f "$runbook" ]]; then
        pass "runbook exists: $runbook"
    else
        fail "runbook exists: $runbook"
    fi
    if [[ "$runbook" == *README.md ]]; then
        continue
    fi
    expect_file \
        "runbook has a verification section: $runbook" \
        "$runbook" \
        '^#{2,3} .*[Vv]erification'
done

expect_file \
    "the runbook index links first deployment" \
    docs/runbooks/README.md \
    'first-deploy\.md'
expect_file \
    "the runbook index links release and rollback" \
    docs/runbooks/README.md \
    'ci-cd\.md'
expect_file \
    "the runbook index links secret rotation" \
    docs/runbooks/README.md \
    'secret-rotation\.md'
expect_file \
    "the runbook index links backup and restore" \
    docs/runbooks/README.md \
    'backup-restore\.md'
expect_file \
    "the runbook index links the PostgreSQL upgrade" \
    docs/runbooks/README.md \
    'postgres-upgrade\.md'
expect_file \
    "the runbook index links alert troubleshooting" \
    docs/runbooks/README.md \
    'alerting\.md'
expect_file \
    "the runbook index links break-glass" \
    docs/runbooks/README.md \
    'break-glass\.md'
expect_file \
    "the runbook index links certificate renewal" \
    docs/runbooks/README.md \
    'certificate-renewal\.md'

expect_file \
    "the production image stays pinned to PostgreSQL 18.6" \
    deploy/db/Dockerfile \
    '^FROM postgres:18\.6-alpine$'
expect_file \
    "the upgrade maintenance image defaults to the pinned PG18 source" \
    deploy/db/upgrade/Dockerfile \
    '^ARG PG_OLD_IMAGE=postgres:18\.6-alpine$'
expect_file \
    "the upgrade maintenance image defaults to the pinned PG19 target" \
    deploy/db/upgrade/Dockerfile \
    '^ARG PG_NEW_IMAGE=postgres:19rc1-alpine$'
expect_file \
    "the upgrade maintenance image pins pgBackRest" \
    deploy/db/upgrade/Dockerfile \
    '^ARG PGBACKREST_VERSION=2\.58\.0-r0$'
expect_file \
    "the upgrade maintenance image installs pgBackRest" \
    deploy/db/upgrade/Dockerfile \
    'apk add --no-cache.*pgbackrest'
expect_file \
    "the upgrade pg_config wrapper relocates the old library directory" \
    deploy/db/upgrade/pg_config \
    '--pkglibdir.*/opt/pg18/lib/postgresql'
expect_file \
    "the upgrade drill runs pg_upgrade --check with copy mode" \
    scripts/upgrade-drill.sh \
    '--check --copy'
expect_file \
    "the upgrade drill performs the copy-mode upgrade" \
    scripts/upgrade-drill.sh \
    '^    --copy \\$'
expect_file_absent \
    "the upgrade drill never uses link mode" \
    scripts/upgrade-drill.sh \
    '--link'
if [[ -x scripts/upgrade-drill.sh ]]; then
    pass "the upgrade drill is executable"
else
    fail "the upgrade drill is executable"
fi
expect_file \
    "the upgrade drill records the measured initdb phase" \
    scripts/upgrade-drill.sh \
    '"initdb_seconds"'
expect_file \
    "the upgrade drill records the measured check phase" \
    scripts/upgrade-drill.sh \
    '"check_seconds"'
expect_file \
    "the upgrade drill records the measured upgrade phase" \
    scripts/upgrade-drill.sh \
    '"upgrade_seconds"'
expect_file \
    "the upgrade drill initializes the target cluster with initdb" \
    scripts/upgrade-drill.sh \
    'initdb'
expect_file \
    "the upgrade drill uses the application bootstrap role" \
    scripts/upgrade-drill.sh \
    '--username="\$postgres_user"'
expect_file \
    "the upgrade drill verifies the audit chain before and after" \
    scripts/upgrade-drill.sh \
    'rs-admin-verify-audit-chain'
expect_file \
    "the upgrade runbook documents the check-then-copy procedure" \
    docs/runbooks/postgres-upgrade.md \
    'pg_upgrade --check --copy'
expect_file \
    "the upgrade runbook documents the copy-mode production command" \
    docs/runbooks/postgres-upgrade.md \
    '--check --copy --username=rs_admin'
expect_file \
    "the upgrade runbook documents the rollback path" \
    docs/runbooks/postgres-upgrade.md \
    '^## 5\. Rollback path$'
expect_file \
    "the upgrade runbook initializes the target with the bootstrap role" \
    docs/runbooks/postgres-upgrade.md \
    '--username=rs_admin'
expect_file \
    "the maintenance image and runbook agree on the PG19 target" \
    docs/runbooks/postgres-upgrade.md \
    'postgres:19rc1-alpine'
expect_file \
    "the API image carries the bootstrap CLI" \
    backend/Dockerfile \
    'rs-admin-bootstrap-super-admin'
expect_file \
    "the API image carries the break-glass CLI" \
    backend/Dockerfile \
    'rs-admin-recover-super-admin'
expect_file \
    "the API image carries the key-rotation audit CLI" \
    backend/Dockerfile \
    'rs-admin-record-key-rotation'
expect_file \
    "the refresh rotation override mounts the previous derivation key" \
    deploy/compose.rotation-refresh.yaml \
    'refresh_derivation_previous'
expect_file \
    "the JWT rotation override mounts the previous public key" \
    deploy/compose.rotation-jwt.yaml \
    'jwt_ed25519_previous_public'
expect_file \
    "the refresh rotation runbook documents the override" \
    docs/runbooks/secret-rotation.md \
    'compose.rotation-refresh.yaml'
expect_file \
    "the JWT rotation runbook documents the override" \
    docs/runbooks/secret-rotation.md \
    'compose.rotation-jwt.yaml'
expect_file \
    "the environment example documents the previous JWT key id" \
    deploy/.env.example \
    'JWT_EDDSA_PREVIOUS_KID'
expect_file \
    "the secret rotation runbook documents the rotation audit command" \
    docs/runbooks/secret-rotation.md \
    'rs-admin-record-key-rotation'
expect_file \
    "the environment example records the upgrade drill state directory" \
    deploy/.env.example \
    '^UPGRADE_DRILL_STATE_DIR='
expect_file \
    "the environment example pins the upgrade target image" \
    deploy/.env.example \
    '^PG_UPGRADE_NEW_IMAGE=postgres:19rc1-alpine$'
expect_file \
    "the deployment README links the runbook index" \
    deploy/README.md \
    'docs/runbooks/README\.md'
expect_file \
    "the alerting runbook links the backup runbook" \
    docs/runbooks/alerting.md \
    'backup-restore\.md'
expect_file \
    "the alerting runbook links the certificate runbook" \
    docs/runbooks/alerting.md \
    'certificate-renewal\.md'

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
        "generated maintenance DSN uses the cleanup role" \
        "$secret_dir/database_url_cleanup" \
        'postgres://rs_admin_cleanup:'
    expect_file \
        "generated S3 access key exists" \
        "$secret_dir/backup_s3_access_key" \
        '^[0-9a-f]{48}$'
    expect_file \
        "generated S3 secret key exists" \
        "$secret_dir/backup_s3_secret_key" \
        '^[0-9a-f]{48}$'
    expect_file \
        "generated repository cipher pass exists" \
        "$secret_dir/backup_repo_cipher_pass" \
        '^[0-9a-f]{64}$'
    expect_file \
        "generated monitor password exists" \
        "$secret_dir/monitor_password" \
        '^[0-9a-f]{48}$'
    expect_file \
        "generated Grafana admin password exists" \
        "$secret_dir/grafana_admin_password" \
        '^[0-9a-f]{48}$'
    expect_file \
        "generated alert webhook file exists and is empty by default" \
        "$secret_dir/alert_webhook_url" \
        '^$'
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
