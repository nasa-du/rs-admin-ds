#!/usr/bin/env bash
#
# Behavioural tests for scripts/upgrade-drill.sh. They use a stub `docker`
# binary, so no daemon, PostgreSQL image, backup repository or network is
# required.
#
#   ./scripts/tests/upgrade-drill.sh

set -euo pipefail

# shellcheck source=../lib/deploy-check.sh
. "$(dirname "${BASH_SOURCE[0]}")/../lib/deploy-check.sh"
cd "$root"

if ! command -v jq >/dev/null 2>&1; then
    echo "error: jq is required" >&2
    exit 1
fi

work="$(mktemp -d)"
cleanup() {
    rm -rf "$work"
}
trap cleanup EXIT

repo="$work/repo"
bin="$work/bin"
state="$work/state"
mkdir -p "$repo/scripts" "$repo/deploy/db/upgrade" "$bin" "$state"
cp "$root/scripts/upgrade-drill.sh" "$repo/scripts/upgrade-drill.sh"
chmod +x "$repo/scripts/upgrade-drill.sh"
printf 'services: {}\n' >"$repo/deploy/compose.yaml"
printf 'FROM scratch\n' >"$repo/deploy/db/upgrade/Dockerfile"
printf '.PHONY:\n' >"$repo/deploy/db/upgrade/Makefile"

cat >"$repo/deploy/.env" <<'EOF'
DB_IMAGE=ghcr.io/example/db:sha-abc1234
API_IMAGE=ghcr.io/example/api:sha-abc1234
RS_ADMIN_SECRETS_DIR=./secrets
BACKUP_S3_BUCKET=test-bucket
BACKUP_S3_ENDPOINT=https://s3.example.invalid
BACKUP_S3_REGION=us-east-1
BACKUP_S3_URI_STYLE=path
AUDIT_CHAIN_KEY_ID=test-key
POSTGRES_USER=rs_admin
POSTGRES_DB=rs_admin
EOF
mkdir -p "$repo/deploy/secrets"
printf 'access\n' >"$repo/deploy/secrets/backup_s3_access_key"
printf 'secret\n' >"$repo/deploy/secrets/backup_s3_secret_key"
printf 'cipher\n' >"$repo/deploy/secrets/backup_repo_cipher_pass"
printf '{"test-key":"MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY="}\n' \
    >"$repo/deploy/secrets/audit-chain-keys.json"

export STUB_ROOT="$work"
export STUB_STATE="$state"

cat >"$bin/docker" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail

printf 'docker' >>"$STUB_ROOT/calls.log"
printf ' %s' "$@" >>"$STUB_ROOT/calls.log"
printf '\n' >>"$STUB_ROOT/calls.log"

command="${1:-}"
if [[ "$command" == "build" || "$command" == "volume" || "$command" == "stop" ||
    "$command" == "rm" || "$command" == "network" ]]; then
    exit 0
fi

if [[ "$command" == "image" && "${2:-}" == "inspect" ]]; then
    [[ "${STUB_IMAGE_MISSING:-0}" == "1" ]] && exit 1
    exit 0
fi

if [[ "$command" == "inspect" ]]; then
    for argument in "$@"; do
        if [[ "$argument" == *"State.ExitCode"* ]]; then
            printf '0\n'
            exit 0
        fi
        if [[ "$argument" == *"State.Running"* ]]; then
            printf 'true\n'
            exit 0
        fi
    done
    printf '{}\n'
    exit 0
fi

if [[ "$command" == "compose" ]]; then
    # The script only uses compose to read the live migration version.
    printf '20261108000000\n'
    exit 0
fi

if [[ "$command" == "exec" ]]; then
    container=""
    query=""
    is_pg_isready=0
    is_pgbackrest=0
    for argument in "$@"; do
        if [[ "$argument" == rs-admin-upgrade-* ]]; then
            container="$argument"
        fi
        if [[ "$argument" == "pg_isready" ]]; then
            is_pg_isready=1
        fi
        if [[ "$argument" == *"pgbackrest"* ]]; then
            is_pgbackrest=1
        fi
        if [[ "$argument" == --command=* ]]; then
            query="${argument#--command=}"
        fi
    done

    if [[ "$is_pg_isready" -eq 1 ]]; then
        exit 0
    fi
    if [[ "$is_pgbackrest" -eq 1 ]]; then
        printf '[]\n'
        exit 0
    fi

    case "$query" in
        *"SHOW server_version"*)
            case "$container" in
                *restore*) printf '18.6\n' ;;
                *pg19*) printf '19rc1\n' ;;
                *) printf '18.6\n' ;;
            esac
            ;;
        *"SHOW server_encoding"*)
            printf 'UTF8\n'
            ;;
        *"datcollate"*)
            printf 'en_US.utf8\n'
            ;;
        *"datctype"*)
            printf 'en_US.utf8\n'
            ;;
        *"FROM pg_extension"*)
            if [[ "$container" == *pg19* && "${STUB_MISSING_EXTENSION:-0}" == "1" ]]; then
                printf 'plpgsql|1.0\npg_stat_statements|1.11\n'
            else
                printf 'plpgsql|1.0\npg_stat_statements|1.11\npg_trgm|1.6\n'
            fi
            ;;
        *"_sqlx_migrations"*)
            printf '20261108000000\n'
            ;;
        *"auth_sessions"*)
            if [[ "$container" == *pg19* && "${STUB_COUNT_MISMATCH:-0}" == "1" ]]; then
                printf '1|3|0|3\n'
            else
                printf '1|3|0|2\n'
            fi
            ;;
        *"ANALYZE"*)
            ;;
        *) ;;
    esac
    exit 0
fi

if [[ "$command" == "run" ]]; then
    entrypoint=""
    for argument in "$@"; do
        if [[ "${previous:-}" == "--entrypoint" ]]; then
            entrypoint="$argument"
        fi
        previous="$argument"
    done

    case "$entrypoint" in
        /usr/local/bin/rs-admin-restore-drill-start)
            printf 'stub-restore\n'
            ;;
        /usr/local/bin/rs-admin-upgrade-start)
            printf 'stub-pg19\n'
            ;;
        pgbackrest)
            printf 'pgBackRest 2.58.0\n'
            ;;
        /opt/pg18/bin/pg_controldata)
            printf 'Database cluster state:               shut down\n'
            ;;
        /usr/local/bin/rs-admin-verify-audit-chain)
            count_file="$STUB_ROOT/verify_count"
            count=$(( $(cat "$count_file" 2>/dev/null || printf '0') + 1 ))
            printf '%s\n' "$count" >"$count_file"
            printf '{"migration_version":20261108000000,"migrations_ok":true,"row_counts":{"users":1,"roles":3,"sessions":0,"audit_logs":2},"audit_chain":{"verified_rows":2,"head_seq":2}}\n'
            ;;
        /usr/local/bin/pg_upgrade)
            if [[ "$*" == *"--check"* ]]; then
                [[ "${STUB_FAIL_CHECK:-0}" == "1" ]] && exit 1
            else
                [[ "${STUB_FAIL_UPGRADE:-0}" == "1" ]] && exit 1
            fi
            ;;
        /bin/sh)
            ;;
        *) ;;
    esac
    exit 0
fi

exit 0
STUB
chmod +x "$bin/docker"

reset_case() {
    rm -rf "$state"
    mkdir -p "$state"
    rm -f "$work/calls.log" "$work/verify_count"
}

run_drill() {
    set +e
    PATH="$bin:$PATH" "$repo/scripts/upgrade-drill.sh" \
        --env-file "$repo/deploy/.env" \
        --state-dir "$state" \
        --timeout 5 \
        "$@" >"$work/output.log" 2>&1
    local rc=$?
    set -e
    printf '%s' "$rc"
}

latest_record() {
    find "$state" -maxdepth 1 -name '*.json' -printf '%T@ %p\n' |
        sort -nr | head -n 1 | cut -d' ' -f2-
}

assert_record() {
    local label="$1"
    local expression="$2"
    local record
    record="$(latest_record)"
    if [[ -n "$record" ]] && jq -e "$expression" "$record" >/dev/null 2>&1; then
        pass "$label"
    else
        fail "$label"
        [[ -n "$record" ]] && cat "$record" >&2
        cat "$work/output.log" >&2
    fi
}

assert_output() {
    local label="$1"
    local pattern="$2"
    if grep -Eq -- "$pattern" "$work/output.log"; then
        pass "$label"
    else
        fail "$label"
        cat "$work/output.log" >&2
    fi
}

pg_upgrade_calls() {
    grep 'entrypoint /usr/local/bin/pg_upgrade' "$work/calls.log" 2>/dev/null || true
}

# --- Successful drill -------------------------------------------------------
reset_case
rc="$(run_drill)"
if [[ "$rc" == "0" ]]; then pass "successful drill exits 0"; else fail "successful drill exits 0"; fi
assert_record "successful drill record" '
    .mode == "upgrade-drill" and
    .result == "success" and
    .failure_reason == null and
    .copy_mode == "copy" and
    .source_version == "18.6" and
    .destination_version == "19rc1" and
    .pgbackrest_version == "pgBackRest 2.58.0" and
    .source_encoding == "UTF8" and
    .source_collate == "en_US.utf8" and
    .source_ctype == "en_US.utf8" and
    .extensions_match == true and
    .extension_updates_applied == true and
    .source_migration_version == 20261108000000 and
    .destination_migration_version == 20261108000000 and
    .restore_verification.row_counts.users == 1 and
    .upgrade_verification.audit_chain.verified_rows == 2 and
    (.initdb_seconds | type) == "number" and
    (.check_seconds | type) == "number" and
    (.upgrade_seconds | type) == "number" and
    (.total_seconds | type) == "number"
'
assert_output "the drill logs the successful upgrade" 'upgrade drill succeeded'
if [[ "$(pg_upgrade_calls | grep -c -- '--check --copy')" == "1" ]]; then
    pass "the check phase uses --check --copy"
else
    fail "the check phase uses --check --copy"
fi
if [[ "$(pg_upgrade_calls | grep -vc -- '--check')" -ge 1 ]]; then
    pass "the actual upgrade runs without --check"
else
    fail "the actual upgrade runs without --check"
fi
if ! grep -q -- '--link' "$work/calls.log"; then
    pass "the drill never invokes link mode"
else
    fail "the drill never invokes link mode"
fi

# --- Failed check aborts before the upgrade ---------------------------------
reset_case
export STUB_FAIL_CHECK=1
rc="$(run_drill)"
unset STUB_FAIL_CHECK
if [[ "$rc" != "0" ]]; then pass "failed pg_upgrade --check exits non-zero"; else fail "failed pg_upgrade --check exits non-zero"; fi
assert_record "failed check record" '
    .result == "failed" and
    (.failure_reason | contains("pg_upgrade --check failed")) and
    .copy_mode == "copy"
'
if [[ "$(pg_upgrade_calls | grep -vc -- '--check')" == "0" ]]; then
    pass "a failed check does not start the actual upgrade"
else
    fail "a failed check does not start the actual upgrade"
fi

# --- Missing extension fails verification -----------------------------------
reset_case
export STUB_MISSING_EXTENSION=1
rc="$(run_drill)"
unset STUB_MISSING_EXTENSION
if [[ "$rc" != "0" ]]; then pass "a missing extension fails the drill"; else fail "a missing extension fails the drill"; fi
assert_record "missing extension record" '
    .result == "failed" and
    (.failure_reason | contains("extensions missing")) and
    .extensions_match == false
'

# --- Changed key rows fail verification -------------------------------------
reset_case
export STUB_COUNT_MISMATCH=1
rc="$(run_drill)"
unset STUB_COUNT_MISMATCH
if [[ "$rc" != "0" ]]; then pass "changed key row counts fail the drill"; else fail "changed key row counts fail the drill"; fi
assert_record "count mismatch record" '
    .result == "failed" and
    (.failure_reason | contains("key row counts changed"))
'

# --- Missing prebuilt image is refused --------------------------------------
reset_case
export STUB_IMAGE_MISSING=1
rc="$(run_drill --upgrade-image missing:tag)"
unset STUB_IMAGE_MISSING
if [[ "$rc" != "0" ]]; then pass "a missing prebuilt maintenance image is refused"; else fail "a missing prebuilt maintenance image is refused"; fi
assert_record "missing image record" '
    .result == "failed" and
    (.failure_reason | contains("not available"))
'

# --- Result -----------------------------------------------------------------
if [[ "$failures" -ne 0 ]]; then
    printf '\n%d upgrade drill test(s) failed\n' "$failures" >&2
    exit 1
fi

printf '\nall upgrade drill tests passed\n'
