#!/usr/bin/env bash
#
# Fault injection (spec §12.8, AC-AUTH-12, AC-API-07, AC-FE-02).
#
# Runs three destructive-but-isolated failure scenarios against the acceptance
# stack:
#
#   1. Database unavailable → authenticated requests fail closed with 503
#      `service_unavailable`; recovery restores 200 without losing the session.
#   2. Audit write failure (INSERT revoked for the runtime role) → a successful
#      login must return 503 and roll the session back; restoring the grant
#      makes the next login succeed.
#   3. Gateway 5xx during cold-start bootstrap → the SPA shows the retryable
#      "Can't restore your session" page, never a false sign-out; Retry works
#      after the API recovers (Playwright).
#
# Usage:
#   ./scripts/fault-injection.sh            # full run
#   ./scripts/fault-injection.sh --keep     # leave the stack for debugging
#   ./scripts/fault-injection.sh --no-build # reuse local images
#   ./scripts/fault-injection.sh --out FILE # markdown report
#
# Set BUN to the pinned Bun 1.4.2 binary when the system Bun is older than the
# e2e lockfile. Never run against production.

set -euo pipefail

# shellcheck source=lib/deploy-check.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib/deploy-check.sh"
# shellcheck source=lib/acceptance.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib/acceptance.sh"
cd "$root"

admin_username="${PERF_ADMIN_USERNAME:-ops.perf}"
admin_email="${PERF_ADMIN_EMAIL:-ops.perf@example.com}"
admin_temp_password="${PERF_ADMIN_TEMP_PASSWORD:-Perf-Bootstrap-Anchor-2026!}"
admin_origin="${ADMIN_ORIGIN:-https://admin.example.com}"
api_origin="${PUBLIC_API_ORIGIN:-https://api.example.com}"
out="$root/docs/acceptance/fault-injection.md"
keep=0
build_flag=""

while (($# > 0)); do
    case "$1" in
        --keep) keep=1; shift ;;
        --no-build) build_flag="--no-build"; shift ;;
        --build) build_flag="--build"; shift ;;
        --out) out="${2:?--out requires a path}"; shift 2 ;;
        --help | -h)
            awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0"
            exit 0
            ;;
        *)
            echo "error: unknown argument: $1" >&2
            exit 2
            ;;
    esac
done

acceptance_require_docker
acceptance_generate_secrets
if [[ -n "$build_flag" ]]; then
    acceptance_ensure_images "$build_flag"
else
    acceptance_ensure_images
fi

work="$(mktemp -d)"
checks="$work/checks.tsv"
: >"$checks"
teardown() {
    local status=$?
    rm -rf "$work"
    if ((keep == 0)); then
        acceptance_down
    else
        echo "==> stack kept; stop it with:" >&2
        acceptance_stop_hint
    fi
    exit "$status"
}
trap teardown EXIT

acceptance_up

record_pass() {
    pass "$1"
    printf 'pass\t%s\n' "$1" >>"$checks"
}
record_fail() {
    fail "$1"
    printf 'FAIL\t%s\n' "$1" >>"$checks"
}

echo "==> bootstrapping the acceptance administrator"
if ! acceptance_bootstrap "$admin_username" "$admin_email" "$admin_temp_password" >/dev/null; then
    echo "error: bootstrap failed" >&2
    acceptance_compose logs --tail=120 api migrate db >&2 || true
    exit 1
fi

port="$(acceptance_gateway_https_port)"
api_curl() {
    curl --noproxy '*' --silent --show-error \
        --resolve "api.example.com:${port}:127.0.0.1" \
        --cacert "$acceptance_secrets_dir/internal_ca.pem" "$@"
}
api_url="https://api.example.com:${port}"
status_of() {
    awk 'NR == 1 { print $2 }' "$1"
}
code_of() {
    jq -r '.code // empty' "$1" 2>/dev/null
}
login_admin() {
    local body="$1"
    # Keep the password out of curl's argv: jq writes the JSON to stdin and
    # curl reads it with --data @-.
    jq -n --arg identifier "$admin_username" --arg password "$admin_temp_password" \
        '{identifier: $identifier, password: $password}' |
        api_curl --dump-header "$work/login.headers" --output "$body" \
            -X POST -H "Content-Type: application/json" -H "Origin: $admin_origin" \
            -H "x-requested-with: XMLHttpRequest" --data @- \
            "$api_url/api/v1/auth/login"
    status_of "$work/login.headers"
}
session_count() {
    # psql does not interpolate -c strings; feed the query on stdin so the
    # username stays a bound variable.
    printf "SELECT count(*) FROM auth_sessions s
            JOIN users u ON u.id = s.user_id
            WHERE u.username = :'username';\n" |
        acceptance_psql -v username="$admin_username" -tA
}

# --- 1. Database unavailable ------------------------------------------------
echo "==> injecting database unavailability"
status="$(login_admin "$work/initial-login.body")"
token="$(jq -r '.access_token // empty' "$work/initial-login.body")"
if [[ "$status" == "200" && -n "$token" ]]; then
    record_pass "authenticated session established before the fault"
else
    record_fail "authenticated session established before the fault (status $status)"
    exit 1
fi

acceptance_compose stop db >/dev/null
db_down_status=""
for _ in $(seq 1 60); do
    api_curl --dump-header "$work/db-down.headers" --output "$work/db-down.body" \
        -H "Authorization: Bearer $token" "$api_url/api/v1/auth/me" || true
    db_down_status="$(status_of "$work/db-down.headers")"
    [[ "$db_down_status" == "503" ]] && break
    sleep 0.5
done
if [[ "$db_down_status" == "503" && "$(code_of "$work/db-down.body")" == "service_unavailable" ]]; then
    record_pass "DB unavailable: authenticated request fails closed with 503 service_unavailable"
else
    record_fail "DB unavailable: authenticated request fails closed with 503 (got ${db_down_status:-no response})"
fi

api_curl --dump-header "$work/login-down.headers" --output "$work/login-down.body" \
    -X POST -H "Content-Type: application/json" -H "Origin: $admin_origin" \
    -H "x-requested-with: XMLHttpRequest" \
    --data "{\"identifier\":\"$admin_username\",\"password\":\"$admin_temp_password\"}" \
    "$api_url/api/v1/auth/login" || true
if [[ "$(status_of "$work/login-down.headers")" == "503" ]]; then
    record_pass "DB unavailable: login fails closed with 503"
else
    record_fail "DB unavailable: login fails closed with 503 (got $(status_of "$work/login-down.headers"))"
fi

acceptance_compose up --detach --wait --wait-timeout 120 db >/dev/null
recovered=0
for _ in $(seq 1 120); do
    api_curl --dump-header "$work/recovered.headers" --output "$work/recovered.body" \
        -H "Authorization: Bearer $token" "$api_url/api/v1/auth/me" || true
    if [[ "$(status_of "$work/recovered.headers")" == "200" ]]; then
        recovered=1
        break
    fi
    sleep 0.5
done
if ((recovered == 1)); then
    record_pass "DB recovery: the existing session works again without re-login"
else
    record_fail "DB recovery: the existing session works again without re-login"
fi

# --- 2. Audit write failure -------------------------------------------------
echo "==> injecting audit write failure"
sessions_before="$(session_count)"
acceptance_psql -c \
    "REVOKE INSERT ON audit_logs FROM rs_admin_runtime" >/dev/null
status="$(login_admin "$work/audit-fail.body")"
sessions_after="$(session_count)"
acceptance_psql -c \
    "GRANT INSERT ON audit_logs TO rs_admin_runtime" >/dev/null

if [[ "$status" == "503" && "$(code_of "$work/audit-fail.body")" == "service_unavailable" ]]; then
    record_pass "audit write failure: successful login fails closed with 503"
else
    record_fail "audit write failure: successful login fails closed with 503 (got $status)"
fi
if [[ "$sessions_before" == "$sessions_after" ]]; then
    record_pass "audit write failure: the business transaction rolled back (session count $sessions_before)"
else
    record_fail "audit write failure: business rollback (sessions $sessions_before → $sessions_after)"
fi
# The rollback is only meaningful if the handler actually reached the audit
# write; the writer logs this exact message before failing closed.
if acceptance_compose logs --no-color --tail=500 api 2>/dev/null \
    | grep -q 'audit write failed; failing closed'; then
    record_pass "audit write failure: the audit writer failed the request closed"
else
    record_fail "audit write failure: the audit writer failed the request closed"
fi

status="$(login_admin "$work/audit-recovered.body")"
sessions_recovered="$(session_count)"
if [[ "$status" == "200" && "$sessions_recovered" -gt "$sessions_after" ]]; then
    record_pass "audit write recovery: the next login succeeds and creates a session"
else
    record_fail "audit write recovery: the next login succeeds (status $status, sessions $sessions_after → $sessions_recovered)"
fi

# --- 3. Gateway 5xx → frontend error page -----------------------------------
echo "==> running the browser bootstrap-failure journey"
bun_bin="$(acceptance_bun)"
(
    cd "$root/e2e"
    "$bun_bin" install --frozen-lockfile >/dev/null
    "$bun_bin" x playwright install chromium >/dev/null
)
if (
    cd "$root/e2e"
    E2E_FAULT_INJECTION=1 \
        E2E_ADMIN_ORIGIN="$admin_origin" \
        E2E_API_ORIGIN="$api_origin" \
        FAULT_COMPOSE_PROJECT="$acceptance_project" \
        FAULT_COMPOSE_ENV_FILE="$acceptance_env_file" \
        FAULT_COMPOSE_FILES="$acceptance_compose_file:$acceptance_overlay_file" \
        FAULT_SECRETS_DIR="./secrets-acceptance" \
        "$bun_bin" x playwright test --config playwright.config.ts \
        fault-injection.spec.ts
) >"$work/browser.log" 2>&1; then
    record_pass "gateway 5xx: SPA shows the retryable session page; Retry recovers"
else
    record_fail "gateway 5xx: SPA shows the retryable session page; Retry recovers"
    tail -n 60 "$work/browser.log" >&2 || true
fi

# --- Report -----------------------------------------------------------------
mkdir -p "$(dirname "$out")"
{
    echo "# Fault injection"
    echo
    echo "Generated by \`scripts/fault-injection.sh\` at $(date -u +%Y-%m-%dT%H:%M:%SZ) UTC."
    echo
    echo "| Scenario | Result |"
    echo "|---|---|"
    while IFS=$'\t' read -r result label; do
        echo "| $label | $result |"
    done <"$checks"
    echo
    if ((failures == 0)); then
        echo "**Pass.** Every injected failure failed closed and recovered correctly."
    else
        echo "**Fail.** $failures scenario(s) did not behave as required."
    fi
} >"$out"
echo "==> wrote $out"

if ((failures > 0)); then
    echo "error: $failures fault-injection check(s) failed" >&2
    exit 1
fi
