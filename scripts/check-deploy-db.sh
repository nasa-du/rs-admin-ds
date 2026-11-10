#!/usr/bin/env bash
#
# Integration check for the production PostgreSQL setup: TLS-only SCRAM
# connections, least-privilege roles, real SQLx migrations and the table grants
# applied by the migrate job.
#
# Requires Docker plus the Rust migrate binary (built here with cargo). The
# development secrets must already exist; run scripts/generate-dev-secrets.sh
# first.
#
#   ./scripts/check-deploy-db.sh

set -euo pipefail

# shellcheck source=lib/deploy-check.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib/deploy-check.sh"
cd "$root"

secret_dir="${RS_ADMIN_SECRETS_DIR:-$root/deploy/secrets-dev}"
postgres_image="${POSTGRES_TEST_IMAGE:-postgres:18.6-alpine}"
container="rs-admin-db-check-$$"
dsn_file="$(mktemp)"

cleanup() {
    docker rm -f "$container" >/dev/null 2>&1 || true
    rm -f "$dsn_file"
}
trap cleanup EXIT

if [[ ! -r "$secret_dir/postgres_admin_password" ]]; then
    echo "error: missing development secrets; run ./scripts/generate-dev-secrets.sh" >&2
    exit 1
fi

if ! cargo build --quiet --package rs-admin-api --bin migrate; then
    echo "error: could not build the migrate binary" >&2
    exit 1
fi

# The host-side test connection reaches the published port, so use the
# loopback/bridge development HBA. The production HBA (data network only) is
# asserted by check-deploy.sh.
docker run --detach --rm --name "$container" \
    --publish 127.0.0.1::5432 \
    --env POSTGRES_DB=rs_admin \
    --env POSTGRES_USER=rs_admin \
    --env POSTGRES_PASSWORD_FILE=/run/secrets/postgres_admin_password \
    --volume "$secret_dir:/run/secrets:ro" \
    --volume "$root/deploy/db/entrypoint.sh:/usr/local/bin/rs-admin-db-entrypoint.sh:ro" \
    --volume "$root/deploy/db/init:/docker-entrypoint-initdb.d:ro" \
    --volume "$root/deploy/db/pg_hba.dev.conf:/etc/rs-admin/pg_hba.conf:ro" \
    --entrypoint /usr/local/bin/rs-admin-db-entrypoint.sh \
    "$postgres_image" \
    postgres \
    -c shared_preload_libraries=pg_stat_statements \
    -c password_encryption=scram-sha-256 \
    -c ssl=on \
    -c ssl_cert_file=/run/rs-admin-secrets/postgres_server_cert \
    -c ssl_key_file=/run/rs-admin-secrets/postgres_server_key \
    -c ssl_ca_file=/run/rs-admin-secrets/internal_ca \
    -c ssl_min_protocol_version=TLSv1.2 \
    -c hba_file=/etc/rs-admin/pg_hba.conf \
    >/dev/null

ready=0
for _ in $(seq 1 90); do
    if docker exec "$container" psql -U rs_admin -d rs_admin -tAc \
        "SELECT 1 FROM pg_roles WHERE rolname = 'rs_admin_runtime'" \
        2>/dev/null | grep -q 1; then
        ready=1
        break
    fi
    sleep 1
done

if [[ "$ready" -ne 1 ]]; then
    echo "error: PostgreSQL did not finish initialization" >&2
    docker logs "$container" 2>&1 | tail -40 >&2
    exit 1
fi

host_port="$(docker port "$container" 5432/tcp | head -n 1 | sed 's/.*://')"
if [[ -z "$host_port" ]]; then
    echo "error: could not determine the published PostgreSQL port" >&2
    exit 1
fi

# The generated migrator DSN targets the Compose service name `db`; rewrite it
# for the loopback-published test container and host-side CA path.
sed \
    -e "s#@db:5432#@127.0.0.1:${host_port}#" \
    -e "s#sslrootcert=/run/secrets/internal_ca#sslrootcert=${secret_dir}/internal_ca.pem#" \
    "$secret_dir/database_url_migrator" >"$dsn_file"

if APP_ENV=production DATABASE_URL_FILE="$dsn_file" \
    "$root/target/debug/migrate" >/dev/null; then
    pass "migrations apply over verified TLS as rs_admin_migrator"
else
    fail "migrations apply over verified TLS as rs_admin_migrator"
fi

# Apply the table grants through the same migrator role and TLS path. The
# container-side DSN keeps the CA copy made by the database entrypoint.
if docker exec -i "$container" sh -c '
    url=$(sed \
        -e "s#@db:5432#@127.0.0.1:5432#" \
        -e "s#sslrootcert=/run/secrets/internal_ca#sslrootcert=/run/rs-admin-secrets/internal_ca#" \
        /run/secrets/database_url_migrator)
    psql "$url" --no-psqlrc --set=ON_ERROR_STOP=1 --file=-
' <deploy/db/grants.sql >/dev/null; then
    pass "role grants apply as rs_admin_migrator over TLS"
else
    fail "role grants apply as rs_admin_migrator over TLS"
fi

super() {
    docker exec "$container" psql -U rs_admin -d rs_admin -tAc "$1" | tr -d '[:space:]'
}

expect_super() {
    local label="$1"
    local query="$2"
    local expected="$3"
    local actual
    actual="$(super "$query")"
    if [[ "$actual" == "$expected" ]]; then
        pass "$label"
    else
        fail "$label (expected $expected, got $actual)"
    fi
}

expect_super \
    "PostgreSQL 18.6 is running" \
    "SELECT current_setting('server_version') LIKE '18.6%'" \
    "t"
expect_super "TLS is enabled" "SHOW ssl" "on"

if docker top "$container" -o pid,user,comm \
    | awk 'NR > 1 { print $2 }' \
    | grep -qxE '(0|root)'; then
    fail "database server processes run as a non-root user"
else
    pass "database server processes run as a non-root user"
fi
expect_super \
    "all four application roles use SCRAM" \
    "SELECT count(*) FROM pg_authid WHERE rolname LIKE 'rs_admin_%' AND rolpassword LIKE 'SCRAM-SHA-256\$%'" \
    "4"

runtime_dsn="$(sed \
    -e "s#@db:5432#@127.0.0.1:5432#" \
    -e "s#sslrootcert=/run/secrets/internal_ca#sslrootcert=/run/rs-admin-secrets/internal_ca#" \
    "$secret_dir/database_url")"

if docker exec -e RUNTIME_DSN="$runtime_dsn" "$container" sh -c \
    'psql "$RUNTIME_DSN" --no-psqlrc --set=ON_ERROR_STOP=1 --tuples-only --command="SELECT MAX(version) FROM _sqlx_migrations"' \
    >/dev/null 2>&1; then
    pass "runtime role reads migration state over verified TLS"
else
    fail "runtime role reads migration state over verified TLS"
fi

insecure_dsn="${runtime_dsn/sslmode=verify-full/sslmode=disable}"
if docker exec -e RUNTIME_DSN="$insecure_dsn" "$container" sh -c \
    'psql "$RUNTIME_DSN" --no-psqlrc --command="SELECT 1"' >/dev/null 2>&1; then
    fail "unencrypted runtime connections are rejected"
else
    pass "unencrypted runtime connections are rejected"
fi

if docker exec -e RUNTIME_DSN="$runtime_dsn" "$container" sh -c \
    'psql "$RUNTIME_DSN" --no-psqlrc --set=ON_ERROR_STOP=1 --command="CREATE TABLE should_fail (id integer)"' \
    >/dev/null 2>&1; then
    fail "runtime role cannot execute DDL"
else
    pass "runtime role cannot execute DDL"
fi

expect_super \
    "runtime may read and append audit rows" \
    "SELECT has_table_privilege('rs_admin_runtime', 'audit_logs', 'SELECT'), has_table_privilege('rs_admin_runtime', 'audit_logs', 'INSERT')" \
    "t|t"
expect_super \
    "runtime cannot update or delete audit rows" \
    "SELECT has_table_privilege('rs_admin_runtime', 'audit_logs', 'UPDATE'), has_table_privilege('rs_admin_runtime', 'audit_logs', 'DELETE')" \
    "f|f"
expect_super \
    "runtime may read and advance the audit chain head" \
    "SELECT has_table_privilege('rs_admin_runtime', 'audit_chain_head', 'SELECT'), has_table_privilege('rs_admin_runtime', 'audit_chain_head', 'UPDATE')" \
    "t|t"
expect_super \
    "runtime has no DML on the migration table" \
    "SELECT has_table_privilege('rs_admin_runtime', '_sqlx_migrations', 'INSERT')" \
    "f"
expect_super \
    "cleanup may delete audit rows" \
    "SELECT has_table_privilege('rs_admin_cleanup', 'audit_logs', 'DELETE')" \
    "t"
expect_super \
    "cleanup may delete expired sessions" \
    "SELECT has_table_privilege('rs_admin_cleanup', 'auth_sessions', 'DELETE'), has_table_privilege('rs_admin_cleanup', 'auth_refresh_tokens', 'DELETE')" \
    "t|t"
expect_super \
    "cleanup may read the migration version" \
    "SELECT has_table_privilege('rs_admin_cleanup', '_sqlx_migrations', 'SELECT')" \
    "t"
expect_super \
    "cleanup has no access to user or role data" \
    "SELECT has_table_privilege('rs_admin_cleanup', 'users', 'SELECT'), has_table_privilege('rs_admin_cleanup', 'roles', 'SELECT')" \
    "f|f"
expect_super \
    "backup role can run the non-superuser backup protocol" \
    "SELECT has_function_privilege('rs_admin_backup', 'pg_backup_start(text,boolean)', 'EXECUTE'), has_function_privilege('rs_admin_backup', 'pg_backup_stop(boolean)', 'EXECUTE'), has_function_privilege('rs_admin_backup', 'pg_create_restore_point(text)', 'EXECUTE'), has_function_privilege('rs_admin_backup', 'pg_switch_wal()', 'EXECUTE')" \
    "t|t|t|t"

if [[ "$failures" -ne 0 ]]; then
    printf '\n%d database deployment check(s) failed\n' "$failures" >&2
    exit 1
fi

printf '\nall database deployment checks passed\n'
