#!/usr/bin/env bash
#
# PostgreSQL 18 -> 19 upgrade drill (ADR-0008 §7.4 and §11.4, AC-OPS-09).
#
# Restores the newest pgBackRest backup into a throwaway PostgreSQL 18
# container, verifies the restored schema, key rows and audit chain, confirms
# the cluster is cleanly shut down, runs `pg_upgrade --check`, performs the
# upgrade with `--copy` (the old cluster is retained for rollback), starts the
# upgraded PostgreSQL 19 cluster, runs ANALYZE, and verifies the migration
# version, key row counts, extensions and audit chain again.
#
# The measured phases (restore, check, upgrade, verification) are written to a
# JSON record under $UPGRADE_DRILL_STATE_DIR. The production data directory is
# never touched: the drill uses its own volume and network namespace.
#
#   ./scripts/upgrade-drill.sh
#   ./scripts/upgrade-drill.sh --expected-migration-version 20261108000000
#
# The maintenance image is built from deploy/db/upgrade/Dockerfile with the
# production database image as the old-binary source and a pinned PostgreSQL 19
# image as the target. Pass --upgrade-image to use a prebuilt image instead.

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"

log() {
    printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" >&2
}

die() {
    failure_reason="$*"
    log "error: $*"
    exit 1
}

json_escape() {
    local value="$1"
    value="${value//\\/\\\\}"
    value="${value//\"/\\\"}"
    value="${value//$'\n'/\\n}"
    value="${value//$'\r'/\\r}"
    value="${value//$'\t'/\\t}"
    printf '%s' "$value"
}

# Render "name|version" lines (psql --no-align --field-separator '|') as a JSON
# array of objects.
json_extension_array() {
    local input="$1"
    local first=1
    local name version

    printf '['
    while IFS='|' read -r name version; do
        [[ -z "$name" ]] && continue
        if [[ "$first" -eq 1 ]]; then
            first=0
        else
            printf ','
        fi
        printf '{"name":"%s","version":"%s"}' \
            "$(json_escape "$name")" "$(json_escape "$version")"
    done <<<"$input"
    printf ']'
}

usage() {
    cat <<'EOF'
Usage: scripts/upgrade-drill.sh [options]

Options:
  --env-file FILE                   Compose env file (default: deploy/.env)
  --state-dir DIR                   Drill record directory
                                    (default: $UPGRADE_DRILL_STATE_DIR, the
                                    env file's value, or
                                    /var/lib/rs-admin/upgrade-drills)
  --expected-migration-version N    Fail unless the restored and upgraded
                                    schemas are at N
  --upgrade-image IMAGE             Use a prebuilt maintenance image instead of
                                    building rs-admin-pg-upgrade:drill
  --old-image IMAGE                 PostgreSQL 18 source image for the build
                                    (default: DB_IMAGE)
  --new-image IMAGE                 PostgreSQL 19 target image for the build
                                    (default: $PG_UPGRADE_NEW_IMAGE or
                                    postgres:19rc1-alpine)
  --operator NAME                   Operator recorded in the drill record
  --timeout SECONDS                 Wait for restored and upgraded servers
                                    (default: 1800)
  --keep                            Keep containers and the drill volume
  -h, --help                        Show this help

Environment (from the env file): DB_IMAGE, API_IMAGE, RS_ADMIN_SECRETS_DIR,
BACKUP_S3_BUCKET, BACKUP_S3_ENDPOINT, BACKUP_S3_REGION, BACKUP_S3_URI_STYLE,
AUDIT_CHAIN_KEY_ID, POSTGRES_USER, POSTGRES_DB, PG_UPGRADE_NEW_IMAGE.

Exit status: 0 when every check passes; non-zero otherwise.
EOF
}

env_file="$repo_root/deploy/.env"
state_dir=""
expected_migration_version=""
operator="${OPERATOR:-$(id -un)}"
timeout=1800
keep=0
upgrade_image=""
upgrade_image_provided=0
old_image=""
new_image=""

while (($# > 0)); do
    case "$1" in
        --env-file)
            [[ $# -ge 2 ]] || { echo "error: --env-file needs a value" >&2; exit 2; }
            env_file="$2"
            shift 2
            ;;
        --state-dir)
            [[ $# -ge 2 ]] || { echo "error: --state-dir needs a value" >&2; exit 2; }
            state_dir="$2"
            shift 2
            ;;
        --expected-migration-version)
            [[ $# -ge 2 ]] || { echo "error: --expected-migration-version needs a value" >&2; exit 2; }
            expected_migration_version="$2"
            shift 2
            ;;
        --upgrade-image)
            [[ $# -ge 2 ]] || { echo "error: --upgrade-image needs a value" >&2; exit 2; }
            upgrade_image="$2"
            upgrade_image_provided=1
            shift 2
            ;;
        --old-image)
            [[ $# -ge 2 ]] || { echo "error: --old-image needs a value" >&2; exit 2; }
            old_image="$2"
            shift 2
            ;;
        --new-image)
            [[ $# -ge 2 ]] || { echo "error: --new-image needs a value" >&2; exit 2; }
            new_image="$2"
            shift 2
            ;;
        --operator)
            [[ $# -ge 2 ]] || { echo "error: --operator needs a value" >&2; exit 2; }
            operator="$2"
            shift 2
            ;;
        --timeout)
            [[ $# -ge 2 ]] || { echo "error: --timeout needs a value" >&2; exit 2; }
            timeout="$2"
            shift 2
            ;;
        --keep)
            keep=1
            shift
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            echo "error: unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

[[ -f "$env_file" ]] || die "missing env file: $env_file"
[[ "$timeout" =~ ^[0-9]+$ ]] && ((timeout > 0)) || die "--timeout must be a positive integer"
[[ -z "$expected_migration_version" || "$expected_migration_version" =~ ^[0-9]+$ ]] ||
    die "--expected-migration-version must be an integer"
command -v docker >/dev/null 2>&1 || die "docker is required"

env_get() {
    local key="$1"
    local value
    value="$(sed -n -E "s/^[[:space:]]*(export[[:space:]]+)?${key}=//p" "$env_file" | tail -n 1)"
    value="${value%\"}"
    value="${value#\"}"
    value="${value%\'}"
    value="${value#\'}"
    printf '%s' "$value"
}

if [[ -z "$state_dir" ]]; then
    state_dir="${UPGRADE_DRILL_STATE_DIR:-$(env_get UPGRADE_DRILL_STATE_DIR)}"
    state_dir="${state_dir:-/var/lib/rs-admin/upgrade-drills}"
fi

db_image="$(env_get DB_IMAGE)"
api_image="$(env_get API_IMAGE)"
secrets_dir="$(env_get RS_ADMIN_SECRETS_DIR)"
[[ -n "$db_image" ]] || die "DB_IMAGE is missing from $env_file"
[[ -n "$api_image" ]] || die "API_IMAGE is missing from $env_file"
[[ -n "$secrets_dir" ]] || die "RS_ADMIN_SECRETS_DIR is missing from $env_file"

# Compose resolves a relative secrets directory against deploy/; do the same.
if [[ "$secrets_dir" != /* ]]; then
    secrets_dir="$(cd "$(dirname "$env_file")" && pwd)/$secrets_dir"
fi
[[ -r "$secrets_dir/backup_s3_access_key" ]] ||
    die "missing backup S3 credentials in $secrets_dir"
[[ -r "$secrets_dir/backup_repo_cipher_pass" ]] ||
    die "missing the pgBackRest repository cipher pass in $secrets_dir"
[[ -r "$secrets_dir/audit-chain-keys.json" ]] ||
    die "missing the audit chain keyring in $secrets_dir"

bucket="$(env_get BACKUP_S3_BUCKET)"
endpoint="$(env_get BACKUP_S3_ENDPOINT)"
region="$(env_get BACKUP_S3_REGION)"
uri_style="$(env_get BACKUP_S3_URI_STYLE)"
storage_verify_tls="$(env_get PGBACKREST_REPO1_STORAGE_VERIFY_TLS)"
if [[ "$endpoint" == http://* ]]; then
    # Local S3-compatible test endpoints use plain HTTP; production endpoints
    # stay on the pgBackRest default (verify TLS).
    storage_verify_tls="n"
fi
audit_key_id="$(env_get AUDIT_CHAIN_KEY_ID)"
postgres_user="$(env_get POSTGRES_USER)"
postgres_user="${postgres_user:-rs_admin}"
postgres_db="$(env_get POSTGRES_DB)"
postgres_db="${postgres_db:-rs_admin}"
[[ -n "$bucket" && -n "$endpoint" && -n "$region" ]] ||
    die "BACKUP_S3_BUCKET/BACKUP_S3_ENDPOINT/BACKUP_S3_REGION are required"

old_image="${old_image:-$db_image}"
new_image="${new_image:-${PG_UPGRADE_NEW_IMAGE:-$(env_get PG_UPGRADE_NEW_IMAGE)}}"
new_image="${new_image:-postgres:19rc1-alpine}"
if [[ "$upgrade_image_provided" -eq 0 ]]; then
    upgrade_image="rs-admin-pg-upgrade:drill"
fi

# Reuse the live database's migration version as the default expectation, so a
# stale backup cannot pass silently. Without a running database the verifier
# still records the version, and the drill compares restore with upgrade.
if [[ -z "$expected_migration_version" ]]; then
    source_version="$(docker compose -f "$repo_root/deploy/compose.yaml" \
        --env-file "$env_file" --profile local-db exec -T db \
        psql -U "$postgres_user" -d "$postgres_db" --no-psqlrc \
        --tuples-only --no-align \
        --command="SELECT MAX(version) FROM _sqlx_migrations" 2>/dev/null |
        tr -d '[:space:]' || true)"
    if [[ "$source_version" =~ ^[0-9]+$ ]]; then
        expected_migration_version="$source_version"
        log "comparing against the live database migration version $expected_migration_version"
    else
        log "warning: could not read the live database migration version; recording it only"
    fi
fi

mkdir -p "$state_dir"
work="$(mktemp -d)"
volume="rs-admin-upgrade-drill-$$"
restore_container="rs-admin-upgrade-restore-$$"
server_container="rs-admin-upgrade-pg19-$$"
started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
timestamp="${started_at//:/-}"
start_epoch="$(date +%s)"
result=""
failure_reason=""
restore_verification_file="$work/restore-verification.json"
upgrade_verification_file="$work/upgrade-verification.json"
backup_info_file="$work/backup-info.json"
check_log="$state_dir/$timestamp-upgrade-drill-$$-pg_upgrade-check.log"
upgrade_log="$state_dir/$timestamp-upgrade-drill-$$-pg_upgrade.log"
record_path=""

source_version=""
destination_version=""
pgbackrest_version=""
source_encoding=""
source_collate=""
source_ctype=""
source_extensions=""
destination_extensions=""
source_counts=""
destination_counts=""
source_migration_version=""
destination_migration_version=""
extensions_match="false"
extension_updates_applied="false"
restore_seconds=0
initdb_seconds=0
check_seconds=0
upgrade_seconds=0
analyze_seconds=0
verification_seconds=0

if command -v flock >/dev/null 2>&1; then
    exec 9>"$state_dir/upgrade-drill.lock"
    flock -n 9 || die "another upgrade drill is already running"
fi

old_psql() {
    docker exec -u postgres "$restore_container" \
        psql -h /upgrade/socket -U "$postgres_user" -d "$postgres_db" \
        --no-psqlrc --tuples-only --no-align --command="$1"
}

new_psql() {
    docker exec -u postgres "$server_container" \
        psql -h 127.0.0.1 -U "$postgres_user" -d "$postgres_db" \
        --no-psqlrc --tuples-only --no-align --command="$1"
}

wait_for_restore() {
    local deadline=$((SECONDS + timeout))
    until docker exec "$restore_container" \
        pg_isready -h 127.0.0.1 -p 5432 -U "$postgres_user" -d "$postgres_db" \
        >/dev/null 2>&1; do
        if ! docker inspect --format '{{.State.Running}}' "$restore_container" 2>/dev/null |
            grep -q true; then
            docker logs --tail 80 "$restore_container" >&2 || true
            die "the restore container exited before PostgreSQL became ready"
        fi
        if ((SECONDS >= deadline)); then
            docker logs --tail 80 "$restore_container" >&2 || true
            die "the restored database did not become ready within ${timeout}s"
        fi
        sleep 2
    done
}

wait_for_upgraded() {
    local deadline=$((SECONDS + timeout))
    until docker exec "$server_container" \
        pg_isready -h 127.0.0.1 -p 5432 -U "$postgres_user" -d "$postgres_db" \
        >/dev/null 2>&1; do
        if ! docker inspect --format '{{.State.Running}}' "$server_container" 2>/dev/null |
            grep -q true; then
            docker logs --tail 80 "$server_container" >&2 || true
            die "the upgraded PostgreSQL 19 container exited before becoming ready"
        fi
        if ((SECONDS >= deadline)); then
            docker logs --tail 80 "$server_container" >&2 || true
            die "the upgraded database did not become ready within ${timeout}s"
        fi
        sleep 2
    done
}

# pg_upgrade requires an already-initialized, empty target cluster. Initialize
# it with the source cluster's encoding and locale so the compatibility check
# does not fail on a default-locale mismatch.
prepare_new_cluster() {
    local label="$1"
    local dir="$2"
    local initdb_log="$work/initdb-$label.log"
    local locale_args=()

    [[ -n "$source_encoding" ]] && locale_args+=(--encoding="$source_encoding")
    [[ -n "$source_collate" ]] && locale_args+=(--locale="$source_collate")
    [[ -n "$source_ctype" ]] && locale_args+=(--lc-ctype="$source_ctype")

    if ! docker run --rm --user 0:0 --volume "$volume:/upgrade" \
        --entrypoint /bin/sh "$upgrade_image" -c \
        "rm -rf '$dir' /upgrade/work
         mkdir -p '$dir' /upgrade/work
         chown 70:70 '$dir' /upgrade/work" >/dev/null; then
        die "could not prepare the $label data directory"
    fi

    local initdb_started
    initdb_started="$(date +%s)"
    if ! docker run --rm --user 70:70 --volume "$volume:/upgrade" \
        --entrypoint /usr/local/bin/initdb "$upgrade_image" \
        -D "$dir" --username="$postgres_user" "${locale_args[@]}" --no-sync \
        >"$initdb_log" 2>&1; then
        tail -n 40 "$initdb_log" >&2 || true
        die "could not initialize the $label PostgreSQL 19 cluster"
    fi
    initdb_seconds=$(( initdb_seconds + $(date +%s) - initdb_started ))
}

finish() {
    local code=$?
    trap - EXIT
    local finished_at total_seconds
    finished_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    total_seconds=$(( $(date +%s) - start_epoch ))

    if [[ -z "$result" ]]; then
        result="failed"
        [[ -n "$failure_reason" ]] || failure_reason="drill aborted"
    fi

    if [[ "$keep" -eq 1 ]]; then
        log "keeping containers $restore_container/$server_container and volume $volume for inspection"
    else
        docker rm -f "$restore_container" >/dev/null 2>&1 || true
        docker rm -f "$server_container" >/dev/null 2>&1 || true
        docker volume rm "$volume" >/dev/null 2>&1 || true
    fi

    local restore_json="null"
    local upgrade_json="null"
    local backup_json="null"
    if [[ -s "$restore_verification_file" ]]; then
        restore_json="$(cat "$restore_verification_file")"
    fi
    if [[ -s "$upgrade_verification_file" ]]; then
        upgrade_json="$(cat "$upgrade_verification_file")"
    fi
    if [[ -s "$backup_info_file" ]]; then
        backup_json="$(cat "$backup_info_file")"
    fi

    record_path="$state_dir/${timestamp}-upgrade-drill-$$.json"
    {
        printf '{\n'
        printf '  "schema_version": 1,\n'
        printf '  "mode": "upgrade-drill",\n'
        printf '  "result": "%s",\n' "$result"
        if [[ -n "$failure_reason" ]]; then
            printf '  "failure_reason": "%s",\n' "$(json_escape "$failure_reason")"
        else
            printf '  "failure_reason": null,\n'
        fi
        printf '  "started_at": "%s",\n' "$started_at"
        printf '  "finished_at": "%s",\n' "$finished_at"
        printf '  "total_seconds": %s,\n' "$total_seconds"
        printf '  "restore_seconds": %s,\n' "$restore_seconds"
        printf '  "initdb_seconds": %s,\n' "$initdb_seconds"
        printf '  "check_seconds": %s,\n' "$check_seconds"
        printf '  "upgrade_seconds": %s,\n' "$upgrade_seconds"
        printf '  "analyze_seconds": %s,\n' "$analyze_seconds"
        printf '  "verification_seconds": %s,\n' "$verification_seconds"
        printf '  "operator": "%s",\n' "$(json_escape "$operator")"
        printf '  "copy_mode": "copy",\n'
        printf '  "old_image": "%s",\n' "$(json_escape "$old_image")"
        printf '  "new_image": "%s",\n' "$(json_escape "$new_image")"
        printf '  "upgrade_image": "%s",\n' "$(json_escape "$upgrade_image")"
        printf '  "api_image": "%s",\n' "$(json_escape "$api_image")"
        printf '  "expected_migration_version": %s,\n' \
            "${expected_migration_version:-null}"
        printf '  "source_version": "%s",\n' "$(json_escape "$source_version")"
        printf '  "destination_version": "%s",\n' "$(json_escape "$destination_version")"
        printf '  "pgbackrest_version": "%s",\n' "$(json_escape "$pgbackrest_version")"
        printf '  "source_migration_version": %s,\n' "${source_migration_version:-null}"
        printf '  "destination_migration_version": %s,\n' "${destination_migration_version:-null}"
        printf '  "source_encoding": "%s",\n' "$(json_escape "$source_encoding")"
        printf '  "source_collate": "%s",\n' "$(json_escape "$source_collate")"
        printf '  "source_ctype": "%s",\n' "$(json_escape "$source_ctype")"
        printf '  "source_extensions": %s,\n' "$(json_extension_array "$source_extensions")"
        printf '  "destination_extensions": %s,\n' "$(json_extension_array "$destination_extensions")"
        printf '  "extensions_match": %s,\n' "$extensions_match"
        printf '  "extension_updates_applied": %s,\n' "$extension_updates_applied"
        printf '  "restore_verification": %s,\n' "$restore_json"
        printf '  "upgrade_verification": %s,\n' "$upgrade_json"
        printf '  "backup_info": %s,\n' "$backup_json"
        if [[ -f "$check_log" ]]; then
            printf '  "pg_upgrade_check_log": "%s",\n' "$(json_escape "$check_log")"
        else
            printf '  "pg_upgrade_check_log": null,\n'
        fi
        if [[ -f "$upgrade_log" ]]; then
            printf '  "pg_upgrade_log": "%s"\n' "$(json_escape "$upgrade_log")"
        else
            printf '  "pg_upgrade_log": null\n'
        fi
        printf '}\n'
    } >"$record_path"

    rm -rf "$work"
    printf 'UPGRADE_DRILL_RESULT=%s\n' "$result"
    printf 'UPGRADE_DRILL_RECORD=%s\n' "$record_path"
    exit "$code"
}
trap finish EXIT
trap 'die "interrupted"' TERM INT

log "starting PostgreSQL upgrade drill at $started_at"
log "repository: s3://$bucket (region $region)"
log "source image: $old_image"
log "target image: $new_image"

# --- Maintenance image ------------------------------------------------------
if [[ "$upgrade_image_provided" -eq 1 ]]; then
    if ! docker image inspect "$upgrade_image" >/dev/null 2>&1; then
        die "the prebuilt maintenance image $upgrade_image is not available"
    fi
else
    log "building the maintenance image $upgrade_image"
    if ! docker build --quiet \
        --build-arg "PG_OLD_IMAGE=$old_image" \
        --build-arg "PG_NEW_IMAGE=$new_image" \
        --file "$repo_root/deploy/db/upgrade/Dockerfile" \
        --tag "$upgrade_image" \
        "$repo_root/deploy/db" >&2; then
        die "could not build the maintenance image"
    fi
fi

if ! docker run --rm --entrypoint /bin/sh "$upgrade_image" -c \
    'test -x /usr/local/bin/pg_upgrade && test -x /opt/pg18/bin/pg_controldata' \
    >/dev/null 2>&1; then
    die "the maintenance image must provide /usr/local/bin/pg_upgrade and /opt/pg18/bin/pg_controldata"
fi

pgbackrest_version="$(docker run --rm --entrypoint pgbackrest "$upgrade_image" version \
    2>/dev/null | head -n 1 | tr -d '\r' || true)"
[[ -n "$pgbackrest_version" ]] || log "warning: the maintenance image does not report a pgBackRest version"

# --- Restore the newest backup into a throwaway PostgreSQL 18 cluster -------
docker volume create "$volume" >/dev/null

restore_started="$(date +%s)"
if ! docker run --detach --name "$restore_container" \
    --network bridge \
    --add-host "host.docker.internal:host-gateway" \
    --read-only \
    --security-opt no-new-privileges:true \
    --tmpfs "/tmp:size=256m,mode=1777" \
    --tmpfs "/run/rs-admin-secrets:size=1m,mode=0700" \
    --volume "$volume:/upgrade" \
    --volume "$secrets_dir:/run/secrets:ro" \
    --env RS_ADMIN_SECRETS_DIR=/run/secrets \
    --env PGBACKREST_CONFIG=/etc/pgbackrest/pgbackrest.conf \
    --env "PGBACKREST_REPO1_S3_BUCKET=$bucket" \
    --env "PGBACKREST_REPO1_S3_ENDPOINT=$endpoint" \
    --env "PGBACKREST_REPO1_S3_REGION=$region" \
    --env "PGBACKREST_REPO1_S3_URI_STYLE=${uri_style:-path}" \
    --env "PGBACKREST_REPO1_STORAGE_VERIFY_TLS=${storage_verify_tls:-y}" \
    --env RESTORE_ROOT=/upgrade \
    --env RESTORE_PGDATA=/upgrade/pgdata \
    --entrypoint /usr/local/bin/rs-admin-restore-drill-start \
    "$db_image" >/dev/null; then
    die "could not start the restore container"
fi

wait_for_restore
log "restored database is accepting connections; verifying the backup"

# Capture repository information before the container is removed. Failure to
# read it is not fatal; the record then omits it.
docker exec -e RS_ADMIN_SECRETS_DIR=/run/rs-admin-secrets "$restore_container" sh -c \
    '. /usr/local/bin/rs-admin-pgbackrest-env; pgbackrest --stanza=rs-admin info --output=json' \
    >"$backup_info_file" 2>/dev/null || rm -f "$backup_info_file"

source_version="$(old_psql 'SHOW server_version' | tr -d '[:space:]')"
source_encoding="$(old_psql 'SHOW server_encoding' | tr -d '[:space:]')"
source_collate="$(old_psql \
    'SELECT datcollate FROM pg_database WHERE datname = current_database()' |
    tr -d '[:space:]')"
source_ctype="$(old_psql \
    'SELECT datctype FROM pg_database WHERE datname = current_database()' |
    tr -d '[:space:]')"
source_extensions="$(old_psql 'SELECT extname, extversion FROM pg_extension ORDER BY extname' |
    tr -d '\r')"
source_counts="$(old_psql 'SELECT
    (SELECT COUNT(*) FROM users),
    (SELECT COUNT(*) FROM roles),
    (SELECT COUNT(*) FROM auth_sessions),
    (SELECT COUNT(*) FROM audit_logs)' | tr -d '[:space:]')"
source_migration_version="$(old_psql 'SELECT MAX(version) FROM _sqlx_migrations' |
    tr -d '[:space:]')"

verify_args=()
if [[ -n "$expected_migration_version" ]]; then
    verify_args+=(--expected-migration-version "$expected_migration_version")
fi

if ! docker run --rm \
    --network "container:$restore_container" \
    --read-only \
    --security-opt no-new-privileges:true \
    --volume "$secrets_dir/audit-chain-keys.json:/run/secrets/audit_chain_keys:ro" \
    --env APP_ENV=test \
    --env "DATABASE_URL=postgres://$postgres_user@127.0.0.1:5432/$postgres_db?sslmode=require" \
    --env "AUDIT_CHAIN_KEY_ID=$audit_key_id" \
    --env AUDIT_CHAIN_KEYS_FILE=/run/secrets/audit_chain_keys \
    --entrypoint /usr/local/bin/rs-admin-verify-audit-chain \
    "$api_image" "${verify_args[@]}" >"$restore_verification_file"; then
    die "the pre-upgrade restore verification failed"
fi

restore_seconds=$(( $(date +%s) - restore_started ))
log "pre-upgrade restore verified in ${restore_seconds}s: $(tr -d '\n' <"$restore_verification_file")"

# A clean shutdown is required by pg_upgrade. Keep the volume; drop the
# container so the upgraded server can reuse the same network-free volume.
if ! docker stop --time 120 "$restore_container" >/dev/null; then
    die "the restored PostgreSQL 18 server did not stop cleanly"
fi
if [[ "$(docker inspect --format '{{.State.ExitCode}}' "$restore_container" 2>/dev/null)" != "0" ]]; then
    docker logs --tail 80 "$restore_container" >&2 || true
    die "the restored PostgreSQL 18 server exited non-zero"
fi
docker rm -f "$restore_container" >/dev/null 2>&1 || true

if ! docker run --rm --user 70:70 --volume "$volume:/upgrade" \
    --entrypoint /opt/pg18/bin/pg_controldata "$upgrade_image" \
    -D /upgrade/pgdata >"$work/pg_controldata.txt" 2>&1; then
    die "could not read the restored cluster control file"
fi
if ! grep -Eq 'Database cluster state:[[:space:]]+shut down' "$work/pg_controldata.txt"; then
    cat "$work/pg_controldata.txt" >&2
    die "the restored PostgreSQL 18 cluster is not cleanly shut down"
fi

# --- pg_upgrade --check -----------------------------------------------------
prepare_new_cluster "check" /upgrade/pg19-check

check_started="$(date +%s)"
if ! docker run --rm --name "rs-admin-upgrade-check-$$" \
    --user 70:70 \
    --security-opt no-new-privileges:true \
    --cap-drop ALL \
    --tmpfs "/tmp:size=256m,mode=1777" \
    --volume "$volume:/upgrade" \
    --workdir /upgrade/work \
    --entrypoint /usr/local/bin/pg_upgrade \
    "$upgrade_image" \
    --check --copy \
    --username="$postgres_user" \
    --old-datadir=/upgrade/pgdata \
    --new-datadir=/upgrade/pg19-check \
    --old-bindir=/opt/pg18/bin \
    --new-bindir=/usr/local/bin \
    >"$check_log" 2>&1; then
    tail -n 80 "$check_log" >&2 || true
    die "pg_upgrade --check failed; see $check_log"
fi
check_seconds=$(( $(date +%s) - check_started ))
log "pg_upgrade --check passed in ${check_seconds}s"

# --- pg_upgrade --copy ------------------------------------------------------
prepare_new_cluster "upgrade" /upgrade/pg19

upgrade_started="$(date +%s)"
if ! docker run --rm --name "rs-admin-upgrade-apply-$$" \
    --user 70:70 \
    --security-opt no-new-privileges:true \
    --cap-drop ALL \
    --tmpfs "/tmp:size=256m,mode=1777" \
    --volume "$volume:/upgrade" \
    --workdir /upgrade/work \
    --entrypoint /usr/local/bin/pg_upgrade \
    "$upgrade_image" \
    --copy \
    --username="$postgres_user" \
    --old-datadir=/upgrade/pgdata \
    --new-datadir=/upgrade/pg19 \
    --old-bindir=/opt/pg18/bin \
    --new-bindir=/usr/local/bin \
    >"$upgrade_log" 2>&1; then
    tail -n 80 "$upgrade_log" >&2 || true
    die "pg_upgrade --copy failed; see $upgrade_log"
fi
upgrade_seconds=$(( $(date +%s) - upgrade_started ))
log "pg_upgrade --copy completed in ${upgrade_seconds}s"

# --- Start and verify PostgreSQL 19 ----------------------------------------
if ! docker run --detach --name "$server_container" \
    --network bridge \
    --user 70:70 \
    --security-opt no-new-privileges:true \
    --cap-drop ALL \
    --tmpfs "/tmp:size=64m,mode=1777" \
    --volume "$volume:/upgrade" \
    --env PG_UPGRADE_DATA_DIR=/upgrade/pg19 \
    --entrypoint /usr/local/bin/rs-admin-upgrade-start \
    "$upgrade_image" >/dev/null; then
    die "could not start the upgraded PostgreSQL 19 server"
fi

wait_for_upgraded

destination_version="$(new_psql 'SHOW server_version' | tr -d '[:space:]')"

analyze_started="$(date +%s)"
if ! new_psql 'ANALYZE' >/dev/null; then
    die "ANALYZE failed on the upgraded cluster"
fi
analyze_seconds=$(( $(date +%s) - analyze_started ))

# pg_upgrade writes update_extensions.sql when an installed extension has a
# newer default version in the target server. Run it before record/verify so
# the drill exercises the same post-upgrade step as production.
if docker exec -u postgres "$server_container" test -s /upgrade/work/update_extensions.sql \
    >/dev/null 2>&1; then
    if ! docker exec -u postgres "$server_container" \
        psql -h 127.0.0.1 -U "$postgres_user" -d postgres \
        --no-psqlrc -v ON_ERROR_STOP=1 \
        -f /upgrade/work/update_extensions.sql >"$work/update_extensions.log" 2>&1; then
        tail -n 40 "$work/update_extensions.log" >&2 || true
        die "the pg_upgrade extension update script failed"
    fi
    extension_updates_applied="true"
fi

destination_extensions="$(new_psql 'SELECT extname, extversion FROM pg_extension ORDER BY extname' |
    tr -d '\r')"
destination_counts="$(new_psql 'SELECT
    (SELECT COUNT(*) FROM users),
    (SELECT COUNT(*) FROM roles),
    (SELECT COUNT(*) FROM auth_sessions),
    (SELECT COUNT(*) FROM audit_logs)' | tr -d '[:space:]')"
destination_migration_version="$(new_psql 'SELECT MAX(version) FROM _sqlx_migrations' |
    tr -d '[:space:]')"

if [[ "$source_counts" != "$destination_counts" ]]; then
    die "key row counts changed during the upgrade (before: $source_counts, after: $destination_counts)"
fi
if [[ "$source_migration_version" != "$destination_migration_version" ]]; then
    die "migration version changed during the upgrade (before: $source_migration_version, after: $destination_migration_version)"
fi

if [[ -z "$source_extensions" || -z "$destination_extensions" ]]; then
    die "the source or upgraded cluster reports no installed extensions"
fi

extensions_match="true"
missing_extensions=()
while IFS='|' read -r name _version; do
    [[ -z "$name" ]] && continue
    if ! awk -F'|' -v name="$name" '$1 == name { found = 1 } END { exit !found }' \
        <<<"$destination_extensions"; then
        missing_extensions+=("$name")
        extensions_match="false"
    fi
done <<<"$source_extensions"
if [[ "${#missing_extensions[@]}" -ne 0 ]]; then
    die "extensions missing after the upgrade: ${missing_extensions[*]}"
fi

verify_started="$(date +%s)"
if ! docker run --rm \
    --network "container:$server_container" \
    --read-only \
    --security-opt no-new-privileges:true \
    --volume "$secrets_dir/audit-chain-keys.json:/run/secrets/audit_chain_keys:ro" \
    --env APP_ENV=test \
    --env "DATABASE_URL=postgres://$postgres_user@127.0.0.1:5432/$postgres_db?sslmode=disable" \
    --env "AUDIT_CHAIN_KEY_ID=$audit_key_id" \
    --env AUDIT_CHAIN_KEYS_FILE=/run/secrets/audit_chain_keys \
    --entrypoint /usr/local/bin/rs-admin-verify-audit-chain \
    "$api_image" "${verify_args[@]}" >"$upgrade_verification_file"; then
    die "the post-upgrade verification failed"
fi
verification_seconds=$(( $(date +%s) - verify_started ))

log "post-upgrade verification: $(tr -d '\n' <"$upgrade_verification_file")"
log "upgrade drill succeeded: PostgreSQL $source_version -> $destination_version"
result="success"
exit 0
