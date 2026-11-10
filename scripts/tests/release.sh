#!/usr/bin/env bash
#
# Behavioural tests for scripts/release.sh. They use stub `docker` and `curl`
# binaries, so no daemon, database or network is required.
#
#   ./scripts/tests/release.sh

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
mkdir -p "$repo/scripts/lib" "$repo/deploy" "$repo/backend/migrations" "$bin" "$state"
cp "$root/scripts/release.sh" "$repo/scripts/release.sh"
cp "$root/scripts/check-migrations.sh" "$repo/scripts/check-migrations.sh"
cp "$root/scripts/lib/deploy-check.sh" "$repo/scripts/lib/deploy-check.sh"
chmod +x "$repo/scripts/release.sh" "$repo/scripts/check-migrations.sh"
printf 'services: {}\n' >"$repo/deploy/compose.yaml"

git -C "$repo" init -q
git -C "$repo" config user.email test@example.invalid
git -C "$repo" config user.name test
git -C "$repo" add .
git -C "$repo" commit -qm "initial release script"

export STUB_ROOT="$work"
export STUB_ENV_FILE="$repo/deploy/.env"
export STUB_NEW_TAG="sha-abc1234"

cat >"$bin/docker" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf 'docker %s\n' "$*" >>"$STUB_ROOT/calls.log"

if [[ "${1:-}" == "inspect" ]]; then
    container_id="${!#}"
    if [[ "$container_id" == "stub-db-id" ]]; then
        printf 'healthy\n'
        exit 0
    fi
    count_file="$STUB_ROOT/inspect_count"
    count=$(( $(cat "$count_file" 2>/dev/null || printf '0') + 1 ))
    printf '%s\n' "$count" >"$count_file"
    image="$(sed -n 's/^API_IMAGE=//p' "$STUB_ENV_FILE" | tail -n 1)"
    if [[ "$image" == *":$STUB_NEW_TAG" ]]; then
        case "${STUB_HEALTH_MODE:-healthy}" in
            healthy) printf 'healthy\n' ;;
            unhealthy) printf 'unhealthy\n' ;;
            timeout) printf 'starting\n' ;;
            after)
                if ((count > ${STUB_HEALTH_AFTER:-0})); then
                    printf 'healthy\n'
                else
                    printf 'starting\n'
                fi
                ;;
            *) printf 'starting\n' ;;
        esac
    else
        printf 'healthy\n'
    fi
    exit 0
fi

[[ "${1:-}" == "compose" ]] || exit 0
shift

while (($# > 0)); do
    case "$1" in
        -f | --env-file | --profile) shift 2 ;;
        *) break ;;
    esac
done

case "${1:-}" in
    pull)
        [[ "${STUB_FAIL_PULL:-0}" == "1" ]] && exit 1
        ;;
    run)
        [[ "${STUB_FAIL_MIGRATE:-0}" == "1" ]] && exit 1
        ;;
    up)
        [[ "${STUB_FAIL_UP:-0}" == "1" ]] && exit 1
        printf 'up\n' >>"$STUB_ROOT/up_count"
        ;;
    ps)
        if [[ "${3:-}" == "db" ]]; then
            printf 'stub-db-id\n'
        else
            printf 'stub-container-id\n'
        fi
        ;;
    logs)
        printf 'stub api logs\n'
        ;;
esac
exit 0
STUB
chmod +x "$bin/docker"

cat >"$bin/curl" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf 'curl %s\n' "$*" >>"$STUB_ROOT/calls.log"

out=""
headers=""
write=""
method="GET"
origin=""
url=""
while (($# > 0)); do
    case "$1" in
        -o) out="$2"; shift 2 ;;
        -D) headers="$2"; shift 2 ;;
        -w) write="$2"; shift 2 ;;
        -X) method="$2"; shift 2 ;;
        -H)
            header="$2"
            shift 2
            case "$header" in
                Origin:*) origin="${header#Origin: }" ;;
            esac
            ;;
        --max-time | --cacert)
            shift 2
            ;;
        -sS | -s | -S)
            shift
            ;;
        -*)
            shift
            ;;
        *)
            url="$1"
            shift
            ;;
    esac
done

code=200
content_type="text/html"
body="<html></html>"
if [[ "$url" == */api/v1/auth/me ]]; then
    if [[ "$method" == "OPTIONS" ]]; then
        code=204
        content_type="text/plain"
        body=""
    else
        code=401
        content_type="application/problem+json"
        body='{"code":"unauthenticated"}'
    fi
    if [[ "${STUB_FAIL_SMOKE:-0}" == "1" && "$method" != "OPTIONS" ]]; then
        code=500
        body='stub failure'
    fi
fi

if [[ -n "$out" ]]; then
    printf '%s' "$body" >"$out"
fi
if [[ -n "$headers" ]]; then
    {
        printf 'HTTP/1.1 %s\r\n' "$code"
        printf 'Content-Type: %s\r\n' "$content_type"
        if [[ "$method" == "OPTIONS" && -n "$origin" ]]; then
            printf 'Access-Control-Allow-Origin: %s\r\n' "$origin"
        fi
    } >"$headers"
fi
if [[ -n "$write" ]]; then
    printf '%s' "$write" | sed "s/%{http_code}/$code/"
else
    [[ -n "$out" ]] || printf '%s' "$body"
fi
STUB
chmod +x "$bin/curl"

reset_case() {
    rm -rf "$state"
    mkdir -p "$state"
    cat >"$repo/deploy/.env" <<'EOF'
API_IMAGE=ghcr.io/example/api:sha-def5678
WEB_IMAGE=ghcr.io/example/web:sha-def5678
DB_IMAGE=ghcr.io/example/db:sha-def5678
GIT_COMMIT=unknown
PUBLIC_API_ORIGIN=https://api.example.com
ADMIN_ORIGIN=https://admin.example.com
EOF
    cat >"$state/deployed.env" <<'EOF'
TAG=sha-def5678
API_IMAGE=ghcr.io/example/api:sha-def5678
WEB_IMAGE=ghcr.io/example/web:sha-def5678
DB_IMAGE=ghcr.io/example/db:sha-def5678
COMMIT=unknown
DEPLOYED_AT=2026-01-01T00:00:00Z
EOF
    rm -f "$work/calls.log" "$work/inspect_count" "$work/up_count"
}

run_release() {
    local tag="$1"
    shift
    set +e
    PATH="$bin:$PATH" "$repo/scripts/release.sh" \
        --tag "$tag" \
        --image-prefix ghcr.io/example \
        --state-dir "$state" \
        --env-file "$repo/deploy/.env" \
        --compose-file "$repo/deploy/compose.yaml" \
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
    fi
}

assert_env() {
    local label="$1"
    local expected="$2"
    if grep -qx "$expected" "$repo/deploy/.env"; then
        pass "$label"
    else
        fail "$label"
        cat "$repo/deploy/.env" >&2
    fi
}

assert_state() {
    local label="$1"
    local expected="$2"
    if grep -qx "$expected" "$state/deployed.env"; then
        pass "$label"
    else
        fail "$label"
        cat "$state/deployed.env" >&2
    fi
}

assert_migrate_runs() {
    local label="$1"
    local expected="$2"
    local actual
    actual="$(grep -c 'run --rm migrate' "$work/calls.log" 2>/dev/null || true)"
    actual="${actual:-0}"
    if [[ "$actual" == "$expected" ]]; then
        pass "$label"
    else
        fail "$label (got $actual)"
    fi
}

# --- Successful release -----------------------------------------------------

reset_case
rc="$(run_release sha-abc1234 --skip-smoke)"
if [[ "$rc" == "0" ]]; then pass "successful release exits 0"; else fail "successful release exits 0"; fi
assert_record "successful release record" '
    .mode == "release" and
    .result == "success" and
    .failure_stage == null and
    .rollback.performed == false and
    .smoke == "skipped"
'
assert_env "release updates the API image" "API_IMAGE=ghcr.io/example/api:sha-abc1234"
assert_env "release updates the web image" "WEB_IMAGE=ghcr.io/example/web:sha-abc1234"
assert_env "release updates the database image" "DB_IMAGE=ghcr.io/example/db:sha-abc1234"
assert_state "release records the deployed tag" "TAG=sha-abc1234"
assert_state "release records the database image" "DB_IMAGE=ghcr.io/example/db:sha-abc1234"
if [[ "$(wc -l <"$work/up_count" 2>/dev/null || printf '0')" == "2" ]]; then
    pass "successful release starts the database and the long-running services"
else
    fail "successful release starts the database and the long-running services"
fi
assert_migrate_runs "successful release runs the migration exactly once" "1"

# --- Immutable tag validation ----------------------------------------------

reset_case
rc="$(run_release v1.2.3 --skip-smoke)"
if [[ "$rc" == "0" ]]; then pass "semver release tag is accepted"; else fail "semver release tag is accepted"; fi

reset_case
rc="$(run_release v1-latest --skip-smoke)"
if [[ "$rc" != "0" ]]; then pass "floating tag-shaped release is refused"; else fail "floating tag-shaped release is refused"; fi
if [[ ! -f "$work/up_count" ]]; then
    pass "invalid image tag does not raise the stack"
else
    fail "invalid image tag does not raise the stack"
fi

# --- Successful dry-run -----------------------------------------------------

reset_case
rc="$(run_release sha-abc1234 --dry-run --skip-smoke)"
if [[ "$rc" == "0" ]]; then pass "dry-run exits 0"; else fail "dry-run exits 0"; fi
assert_record "dry-run record" '
    .mode == "dry-run" and
    .result == "success" and
    .rollback.performed == true and
    .rollback.ready == true
'
assert_env "dry-run restores the API image" "API_IMAGE=ghcr.io/example/api:sha-def5678"
assert_env "dry-run restores the database image" "DB_IMAGE=ghcr.io/example/db:sha-def5678"
assert_state "dry-run leaves the deployed tag unchanged" "TAG=sha-def5678"
if [[ "$(wc -l <"$work/up_count" 2>/dev/null || printf '0')" == "4" ]]; then
    pass "dry-run raises the new and previous stacks"
else
    fail "dry-run raises the new and previous stacks"
fi
assert_migrate_runs "dry-run migrates once and does not re-migrate on rollback" "1"

# --- Readiness failure rolls back ------------------------------------------

reset_case
export STUB_HEALTH_MODE="timeout"
rc="$(run_release sha-abc1234 --skip-smoke --timeout 3)"
unset STUB_HEALTH_MODE
if [[ "$rc" != "0" ]]; then pass "readiness failure exits non-zero"; else fail "readiness failure exits non-zero"; fi
assert_record "readiness failure record" '
    .result == "failed" and
    .failure_stage == "ready" and
    .rollback.performed == true and
    .rollback.ready == true
'
assert_env "readiness failure restores the API image" "API_IMAGE=ghcr.io/example/api:sha-def5678"
assert_state "readiness failure leaves the deployed tag unchanged" "TAG=sha-def5678"
assert_migrate_runs "rollback does not run the previous migrator" "1"

# --- Migration failure leaves the running release in place -----------------

reset_case
export STUB_FAIL_MIGRATE="1"
rc="$(run_release sha-abc1234 --skip-smoke)"
unset STUB_FAIL_MIGRATE
if [[ "$rc" != "0" ]]; then pass "migration failure exits non-zero"; else fail "migration failure exits non-zero"; fi
assert_record "migration failure record" '
    .result == "failed" and
    .failure_stage == "migrate" and
    .rollback.performed == false
'
assert_env "migration failure restores the API image" "API_IMAGE=ghcr.io/example/api:sha-def5678"
if [[ ! -f "$work/up_count" ]]; then
    pass "migration failure does not raise the stack"
else
    fail "migration failure does not raise the stack"
fi

# --- Pull failure aborts before migrations ---------------------------------

reset_case
export STUB_FAIL_PULL="1"
rc="$(run_release sha-abc1234 --skip-smoke)"
unset STUB_FAIL_PULL
if [[ "$rc" != "0" ]]; then pass "pull failure exits non-zero"; else fail "pull failure exits non-zero"; fi
assert_record "pull failure record" '
    .result == "failed" and
    .failure_stage == "pull" and
    .rollback.performed == false
'
assert_env "pull failure restores the API image" "API_IMAGE=ghcr.io/example/api:sha-def5678"
if ! grep -q 'run --rm migrate' "$work/calls.log"; then
    pass "pull failure skips migrations"
else
    fail "pull failure skips migrations"
fi

# --- Smoke failure rolls back ----------------------------------------------

reset_case
export STUB_FAIL_SMOKE="1"
rc="$(run_release sha-abc1234)"
unset STUB_FAIL_SMOKE
if [[ "$rc" != "0" ]]; then pass "smoke failure exits non-zero"; else fail "smoke failure exits non-zero"; fi
assert_record "smoke failure record" '
    .result == "failed" and
    .failure_stage == "smoke" and
    .rollback.performed == true and
    .rollback.ready == true
'
assert_env "smoke failure restores the web image" "WEB_IMAGE=ghcr.io/example/web:sha-def5678"

# --- Dry-run needs a previous release --------------------------------------

reset_case
rm -f "$state/deployed.env"
rc="$(run_release sha-abc1234 --dry-run --skip-smoke)"
if [[ "$rc" != "0" ]]; then pass "first-release dry-run is refused"; else fail "first-release dry-run is refused"; fi
assert_record "first-release dry-run record" '
    .result == "failed" and
    .failure_stage == "dry-run-unavailable"
'

# --- Modified migrations abort before any deployment ----------------------

reset_case
printf 'SELECT 1;\n' >"$repo/backend/migrations/20990101000000_base.sql"
git -C "$repo" add backend/migrations
git -C "$repo" commit -qm "add base migration"
previous_commit="$(git -C "$repo" rev-parse HEAD)"
printf 'SELECT 2;\n' >>"$repo/backend/migrations/20990101000000_base.sql"
git -C "$repo" add backend/migrations
git -C "$repo" commit -qm "modify base migration"
new_commit="$(git -C "$repo" rev-parse HEAD)"
sed -i "s/^COMMIT=.*/COMMIT=$previous_commit/" "$state/deployed.env"
rc="$(run_release sha-abc1234 --skip-smoke --commit "$new_commit")"
if [[ "$rc" != "0" ]]; then pass "modified migration aborts the release"; else fail "modified migration aborts the release"; fi
assert_record "modified migration record" '
    .result == "failed" and
    .failure_stage == "migration-policy"
'
assert_env "modified migration leaves the API image unchanged" "API_IMAGE=ghcr.io/example/api:sha-def5678"
if [[ ! -f "$work/up_count" ]]; then
    pass "modified migration does not raise the stack"
else
    fail "modified migration does not raise the stack"
fi

# --- Destructive migrations need explicit approval -------------------------

git -C "$repo" checkout "$previous_commit" -- backend/migrations/20990101000000_base.sql
printf 'DROP TABLE temp_table;\n' >"$repo/backend/migrations/20990201000000_drop_temp.sql"
git -C "$repo" add backend/migrations
git -C "$repo" commit -qm "add destructive migration"
destructive_commit="$(git -C "$repo" rev-parse HEAD)"

reset_case
sed -i "s/^COMMIT=.*/COMMIT=$previous_commit/" "$state/deployed.env"
rc="$(run_release sha-abc1234 --skip-smoke --commit "$destructive_commit")"
if [[ "$rc" != "0" ]]; then pass "destructive migration is refused by default"; else fail "destructive migration is refused by default"; fi
assert_record "destructive migration refusal record" '
    .result == "failed" and
    .failure_stage == "migration-policy" and
    .destructive_migrations_acknowledged == false
'

reset_case
sed -i "s/^COMMIT=.*/COMMIT=$previous_commit/" "$state/deployed.env"
rc="$(run_release sha-abc1234 --skip-smoke --commit "$destructive_commit" --allow-destructive)"
if [[ "$rc" == "0" ]]; then pass "destructive migration proceeds after approval"; else fail "destructive migration proceeds after approval"; fi
assert_record "destructive migration approval record" '
    .result == "success" and
    .destructive_migrations_acknowledged == true
'

# --- Result -----------------------------------------------------------------

if [[ "$failures" -ne 0 ]]; then
    printf '\n%d release test(s) failed\n' "$failures" >&2
    exit 1
fi

printf '\nall release tests passed\n'
