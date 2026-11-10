#!/usr/bin/env bash
#
# Quarterly restore drill (ADR-0008 §5.6, AC-OPS-05).
#
# Restores the newest pgBackRest backup from the off-site S3 repository into a
# throwaway PostgreSQL 18 container, verifies the migration version, key table
# row counts and the audit HMAC chain, records the measured RTO and removes
# the container and volume.
#
#   ./scripts/restore-drill.sh
#   ./scripts/restore-drill.sh --expected-migration-version 20261108000000
#
# Runs on the deployment host with the production `deploy/.env` and secrets.
# The temporary server listens on loopback inside its own network namespace;
# no host port is published and no production data is modified.

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

usage() {
    cat <<'EOF'
Usage: scripts/restore-drill.sh [options]

Options:
  --env-file FILE                   Compose env file (default: deploy/.env)
  --state-dir DIR                   Drill record directory
                                    (default: $RESTORE_DRILL_STATE_DIR, the
                                    env file's value, or
                                    /var/lib/rs-admin/restore-drills)
  --expected-migration-version N    Fail unless the restored schema is at N
  --operator NAME                   Operator recorded in the drill record
  --timeout SECONDS                 Wait for the restored server (default: 1800)
  --keep                            Keep the container and volume for debugging
  -h, --help                        Show this help

Environment (from the env file): DB_IMAGE, API_IMAGE, RS_ADMIN_SECRETS_DIR,
BACKUP_S3_BUCKET, BACKUP_S3_ENDPOINT, BACKUP_S3_REGION, BACKUP_S3_URI_STYLE,
AUDIT_CHAIN_KEY_ID.

Exit status: 0 when every check passes; non-zero otherwise.
EOF
}

env_file="$repo_root/deploy/.env"
state_dir=""
expected_migration_version=""
operator="${OPERATOR:-$(id -un)}"
timeout=1800
keep=0

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
    state_dir="${RESTORE_DRILL_STATE_DIR:-$(env_get RESTORE_DRILL_STATE_DIR)}"
    state_dir="${state_dir:-/var/lib/rs-admin/restore-drills}"
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
[[ -r "$secrets_dir/internal_ca.pem" ]] ||
    die "missing the internal CA in $secrets_dir"

bucket="$(env_get BACKUP_S3_BUCKET)"
endpoint="$(env_get BACKUP_S3_ENDPOINT)"
region="$(env_get BACKUP_S3_REGION)"
uri_style="$(env_get BACKUP_S3_URI_STYLE)"
audit_key_id="$(env_get AUDIT_CHAIN_KEY_ID)"
postgres_user="$(env_get POSTGRES_USER)"
postgres_user="${postgres_user:-rs_admin}"
postgres_db="$(env_get POSTGRES_DB)"
postgres_db="${postgres_db:-rs_admin}"
[[ -n "$bucket" && -n "$endpoint" && -n "$region" ]] ||
    die "BACKUP_S3_BUCKET/BACKUP_S3_ENDPOINT/BACKUP_S3_REGION are required"

# The scheduled drill compares the restored schema with the live database so a
# stale backup cannot pass silently. An explicit --expected-migration-version
# wins; without a running local database the drill records the version only.
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
container="rs-admin-restore-drill-$$"
volume="rs-admin-restore-drill-$$"
started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
start_epoch="$(date +%s)"
result=""
failure_reason=""
verification_file="$work/verification.json"
backup_info_file="$work/backup-info.json"
record_path=""

if command -v flock >/dev/null 2>&1; then
    exec 9>"$state_dir/restore-drill.lock"
    flock -n 9 || die "another restore drill is already running"
fi

finish() {
    local code=$?
    trap - EXIT
    local finished_at rto_seconds
    finished_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    rto_seconds=$(( $(date +%s) - start_epoch ))

    if [[ -z "$result" ]]; then
        result="failed"
        [[ -n "$failure_reason" ]] || failure_reason="drill aborted"
    fi

    if [[ "$keep" -eq 1 ]]; then
        log "keeping container $container and volume $volume for inspection"
    else
        docker rm -f "$container" >/dev/null 2>&1 || true
        docker volume rm "$volume" >/dev/null 2>&1 || true
    fi

    local verification_json="null"
    local backup_info_json="null"
    if [[ -s "$verification_file" ]]; then
        verification_json="$(cat "$verification_file")"
    fi
    if [[ -s "$backup_info_file" ]]; then
        backup_info_json="$(cat "$backup_info_file")"
    fi

    record_path="$state_dir/${started_at//:/-}-restore-drill-$$.json"
    {
        printf '{\n'
        printf '  "schema_version": 1,\n'
        printf '  "mode": "restore-drill",\n'
        printf '  "result": "%s",\n' "$result"
        if [[ -n "$failure_reason" ]]; then
            printf '  "failure_reason": "%s",\n' "$(json_escape "$failure_reason")"
        else
            printf '  "failure_reason": null,\n'
        fi
        printf '  "started_at": "%s",\n' "$started_at"
        printf '  "finished_at": "%s",\n' "$finished_at"
        printf '  "rto_seconds": %s,\n' "$rto_seconds"
        printf '  "operator": "%s",\n' "$(json_escape "$operator")"
        printf '  "database_image": "%s",\n' "$(json_escape "$db_image")"
        printf '  "api_image": "%s",\n' "$(json_escape "$api_image")"
        printf '  "verification": %s,\n' "$verification_json"
        printf '  "backup_info": %s\n' "$backup_info_json"
        printf '}\n'
    } >"$record_path"

    rm -rf "$work"
    printf 'RESTORE_DRILL_RESULT=%s\n' "$result"
    printf 'RESTORE_DRILL_RECORD=%s\n' "$record_path"
    exit "$code"
}
trap finish EXIT
trap 'die "interrupted"' TERM INT

log "starting restore drill at $started_at"
log "repository: s3://$bucket (region $region)"
log "database image: $db_image"

docker volume create "$volume" >/dev/null

if ! docker run --detach --name "$container" \
    --network bridge \
    --add-host "host.docker.internal:host-gateway" \
    --read-only \
    --security-opt no-new-privileges:true \
    --tmpfs "/tmp:size=256m,mode=1777" \
    --tmpfs "/run/rs-admin-secrets:size=1m,mode=0700" \
    --volume "$volume:/restore" \
    --volume "$secrets_dir:/run/secrets:ro" \
    --env RS_ADMIN_SECRETS_DIR=/run/secrets \
    --env PGBACKREST_CONFIG=/etc/pgbackrest/pgbackrest.conf \
    --env "PGBACKREST_REPO1_S3_BUCKET=$bucket" \
    --env "PGBACKREST_REPO1_S3_ENDPOINT=$endpoint" \
    --env "PGBACKREST_REPO1_S3_REGION=$region" \
    --env "PGBACKREST_REPO1_S3_URI_STYLE=${uri_style:-path}" \
    --entrypoint /usr/local/bin/rs-admin-restore-drill-start \
    "$db_image" >/dev/null; then
    die "could not start the restore container"
fi

deadline=$((SECONDS + timeout))
until docker exec -e PGSSLMODE=require "$container" \
    pg_isready -h 127.0.0.1 -p 5432 -U "$postgres_user" -d "$postgres_db" >/dev/null 2>&1; do
    if ! docker inspect --format '{{.State.Running}}' "$container" 2>/dev/null | grep -q true; then
        docker logs --tail 80 "$container" >&2 || true
        die "the restore container exited before PostgreSQL became ready"
    fi
    if ((SECONDS >= deadline)); then
        docker logs --tail 80 "$container" >&2 || true
        die "the restored database did not become ready within ${timeout}s"
    fi
    sleep 2
done

log "restored database is accepting connections; running checks"

# Capture repository information before the container is removed. Failure to
# read it is not fatal; the drill record then omits it.
docker exec -e RS_ADMIN_SECRETS_DIR=/run/rs-admin-secrets "$container" sh -c \
    '. /usr/local/bin/rs-admin-pgbackrest-env; pgbackrest --stanza=rs-admin info --output=json' \
    >"$backup_info_file" 2>/dev/null || rm -f "$backup_info_file"

verify_args=()
if [[ -n "$expected_migration_version" ]]; then
    verify_args+=(--expected-migration-version "$expected_migration_version")
fi

if ! docker run --rm \
    --network "container:$container" \
    --read-only \
    --security-opt no-new-privileges:true \
    --volume "$secrets_dir/audit-chain-keys.json:/run/secrets/audit_chain_keys:ro" \
    --volume "$secrets_dir/internal_ca.pem:/run/secrets/internal_ca:ro" \
    --env APP_ENV=test \
    --env "DATABASE_URL=postgres://$postgres_user@127.0.0.1:5432/$postgres_db?sslmode=verify-full&sslrootcert=/run/secrets/internal_ca" \
    --env "AUDIT_CHAIN_KEY_ID=$audit_key_id" \
    --env AUDIT_CHAIN_KEYS_FILE=/run/secrets/audit_chain_keys \
    --entrypoint /usr/local/bin/rs-admin-verify-audit-chain \
    "$api_image" "${verify_args[@]}" >"$verification_file"; then
    die "audit chain or schema verification failed"
fi

log "audit chain and schema verified: $(tr -d '\n' <"$verification_file")"
result="success"
exit 0
