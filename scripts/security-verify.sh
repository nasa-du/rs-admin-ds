#!/usr/bin/env bash
#
# Live security verification (spec §12.7, AC-API-04/05/06/11, AC-OPS-02).
#
# Boots the isolated acceptance stack and checks, against the real gateway and
# API:
#
#   - API security headers, problem-details shape and Bearer challenge
#   - exact-Origin CORS on preflight, errors and Bearer endpoints
#   - CSRF rejection when Origin / X-Requested-With / JSON is missing
#   - __Host-refresh cookie attributes
#   - account lockout thresholds and Retry-After
#   - log, audit and metric redaction of passwords, tokens, cookies and keys
#
# It then runs scripts/check-deploy-edge.sh (TLS, HSTS, CSP, SPA, edge rate
# limiting) and scripts/security-scan.sh (cargo audit, bun audit, Trivy).
#
# Usage:
#   ./scripts/security-verify.sh            # full run
#   ./scripts/security-verify.sh --no-scan  # skip dependency/image scans
#   ./scripts/security-verify.sh --keep     # leave the stack for debugging
#   ./scripts/security-verify.sh --no-build # reuse local images
#   ./scripts/security-verify.sh --out FILE # markdown report
#
# Never run against production: it destroys the isolated project's volumes and
# plants deliberate login failures.

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
out="$root/docs/acceptance/security.md"
keep=0
build_flag=""
run_scans=1

while (($# > 0)); do
    case "$1" in
        --keep) keep=1; shift ;;
        --no-build) build_flag="--no-build"; shift ;;
        --build) build_flag="--build"; shift ;;
        --no-scan) run_scans=0; shift ;;
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

echo "==> bootstrapping the acceptance administrator"
if ! acceptance_bootstrap "$admin_username" "$admin_email" "$admin_temp_password" >/dev/null; then
    echo "error: bootstrap failed" >&2
    acceptance_compose logs --tail=120 api migrate db >&2 || true
    exit 1
fi

port="$(acceptance_gateway_https_port)"
if [[ -z "$port" ]]; then
    echo "error: could not determine the gateway HTTPS port" >&2
    exit 1
fi

api_curl() {
    curl --noproxy '*' --silent --show-error \
        --resolve "api.example.com:${port}:127.0.0.1" \
        --cacert "$acceptance_secrets_dir/internal_ca.pem" "$@"
}
api_url="https://api.example.com:${port}"

record_pass() {
    pass "$1"
    printf 'pass\t%s\n' "$1" >>"$checks"
}
record_fail() {
    fail "$1"
    printf 'FAIL\t%s\n' "$1" >>"$checks"
}
check_true() {
    local label="$1"
    shift
    if "$@"; then record_pass "$label"; else record_fail "$label"; fi
}
header() {
    local file="$1" name="$2"
    awk -v name="$name" 'BEGIN { IGNORECASE = 1 }
        index($0, ":") == 0 { next }
        {
            key = substr($0, 1, index($0, ":") - 1)
            if (tolower(key) == tolower(name)) {
                value = substr($0, index($0, ":") + 1)
                sub(/^[ \t]+/, "", value)
                sub(/\r$/, "", value)
                print value
                exit
            }
        }' "$file"
}
header_all() {
    local file="$1" name="$2"
    awk -v name="$name" 'BEGIN { IGNORECASE = 1 }
        index($0, ":") == 0 { next }
        {
            key = substr($0, 1, index($0, ":") - 1)
            if (tolower(key) == tolower(name)) {
                value = substr($0, index($0, ":") + 1)
                sub(/^[ \t]+/, "", value)
                sub(/\r$/, "", value)
                print value
            }
        }' "$file"
}
header_present() {
    [[ -n "$(header "$1" "$2")" ]]
}
header_equals() {
    [[ "$(header "$1" "$2")" == "$3" ]]
}
header_matches() {
    [[ "$(header "$1" "$2")" =~ $3 ]]
}
body_code() {
    jq -r '.code // empty' "$1" 2>/dev/null
}

# --- Security headers and problem details ----------------------------------
echo "==> checking API security headers and problem details"
set +e
api_curl --dump-header "$work/me.headers" --output "$work/me.body" \
    "$api_url/api/v1/auth/me"
set -e
status="$(awk 'NR == 1 { print $2 }' "$work/me.headers")"
check_true "unauthenticated /auth/me returns 401" test "$status" = "401"
check_true "problem content type" \
    header_equals "$work/me.headers" "content-type" "application/problem+json"
check_true "Cache-Control: no-store" \
    header_equals "$work/me.headers" "cache-control" "no-store"
check_true "X-Content-Type-Options: nosniff" \
    header_equals "$work/me.headers" "x-content-type-options" "nosniff"
check_true "Referrer-Policy: no-referrer" \
    header_equals "$work/me.headers" "referrer-policy" "no-referrer"
check_true "X-Frame-Options: DENY" \
    header_equals "$work/me.headers" "x-frame-options" "DENY"
check_true "CSP denies everything by default" \
    header_matches "$work/me.headers" "content-security-policy" \
    "default-src 'none'; frame-ancestors 'none'; base-uri 'none'"
check_true "Permissions-Policy is restrictive" \
    header_matches "$work/me.headers" "permissions-policy" \
    "camera=\(\), microphone=\(\), geolocation=\(\)"
check_true "401 carries a Bearer challenge" \
    header_matches "$work/me.headers" "www-authenticate" "Bearer"
check_true "X-Request-Id is a UUIDv7" \
    header_matches "$work/me.headers" "x-request-id" \
    "^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"
if jq -e '(.type | startswith("urn:rs-admin:problem:"))
        and (.code == "access_token_invalid")
        and (.title | type == "string") and (.status == 401)
        and (.detail | type == "string") and (.instance == "/api/v1/auth/me")
        and (.request_id | type == "string")' "$work/me.body" >/dev/null; then
    record_pass "problem body has the closed field set"
else
    record_fail "problem body has the closed field set"
fi
check_true "problem body matches the X-Request-Id header" \
    test "$(jq -r '.request_id' "$work/me.body")" = "$(header "$work/me.headers" x-request-id)"
if grep -Eqi 'sqlx|panicked|stack backtrace|select .* from' "$work/me.body"; then
    record_fail "problem body does not leak SQL, stack traces or internals"
else
    record_pass "problem body does not leak SQL, stack traces or internals"
fi

# --- CORS -------------------------------------------------------------------
echo "==> checking CORS"
preflight() {
    local origin="$1" path="$2" method="$3"
    api_curl --dump-header "$work/preflight.headers" --output /dev/null \
        -X OPTIONS \
        -H "Origin: $origin" \
        -H "Access-Control-Request-Method: $method" \
        -H "Access-Control-Request-Headers: authorization,content-type,x-requested-with" \
        "$api_url$path"
    awk 'NR == 1 { print $2 }' "$work/preflight.headers"
}

status="$(preflight "$admin_origin" /api/v1/auth/login POST)"
check_true "allowed-origin preflight returns 204" test "$status" = "204"
check_true "allowed origin is echoed exactly" \
    header_equals "$work/preflight.headers" "access-control-allow-origin" "$admin_origin"
check_true "cookie endpoint preflight allows credentials" \
    header_equals "$work/preflight.headers" "access-control-allow-credentials" "true"
check_true "preflight has Vary: Origin" \
    bash -c 'grep -qi Origin <<<"$1"' _ "$(header_all "$work/preflight.headers" vary)"

preflight "$admin_origin" /api/v1/auth/me GET >/dev/null
check_true "Bearer endpoint preflight does not allow credentials" \
    test -z "$(header "$work/preflight.headers" access-control-allow-credentials)"

for origin in "https://evil.example.com" "null" "*"; do
    preflight "$origin" /api/v1/auth/login POST >/dev/null
    check_true "origin '$origin' gets no allow-origin" \
        test -z "$(header "$work/preflight.headers" access-control-allow-origin)"
    check_true "origin '$origin' gets no credentials" \
        test -z "$(header "$work/preflight.headers" access-control-allow-credentials)"
done

set +e
api_curl --dump-header "$work/cors-error.headers" --output /dev/null \
    -H "Origin: $admin_origin" "$api_url/api/v1/auth/me"
set -e
check_true "error responses keep the CORS grant" \
    header_equals "$work/cors-error.headers" "access-control-allow-origin" "$admin_origin"
check_true "error responses on a Bearer endpoint are not credentialed" \
    test -z "$(header "$work/cors-error.headers" access-control-allow-credentials)"

# --- CSRF -------------------------------------------------------------------
echo "==> checking CSRF"
login_payload() {
    jq -n --arg identifier "$1" --arg password "$2" \
        '{identifier: $identifier, password: $password}'
}
post_login() {
    local body_file="$1"
    shift
    # Keep the body out of curl's argv.
    login_payload csrf.probe wrong-password-value |
        api_curl --dump-header "$work/login.headers" --output "$body_file" \
            -X POST -H "Content-Type: application/json" "$@" --data @- \
            "$api_url/api/v1/auth/login"
    awk 'NR == 1 { print $2 }' "$work/login.headers"
}

status="$(post_login "$work/csrf-missing-header.body" -H "Origin: $admin_origin")"
check_true "login without X-Requested-With is refused" test "$status" = "403"
check_true "missing CSRF header is csrf_failed" \
    test "$(body_code "$work/csrf-missing-header.body")" = "csrf_failed"

status="$(post_login "$work/csrf-missing-origin.body" -H "x-requested-with: XMLHttpRequest")"
check_true "login without Origin is refused" test "$status" = "403"
check_true "missing Origin is csrf_failed" \
    test "$(body_code "$work/csrf-missing-origin.body")" = "csrf_failed"

status="$(printf '{"identifier":"csrf.probe","password":"wrong-password-value"}' |
    api_curl --dump-header "$work/login.headers" --output "$work/csrf-text.body" \
        -X POST -H "Content-Type: text/plain" -H "Origin: $admin_origin" \
        -H "x-requested-with: XMLHttpRequest" --data @- \
        "$api_url/api/v1/auth/login" >/dev/null
    awk 'NR == 1 { print $2 }' "$work/login.headers")"
check_true "non-JSON login is refused" test "$status" = "403"
check_true "non-JSON body is csrf_failed" \
    test "$(body_code "$work/csrf-text.body")" = "csrf_failed"

status="$(post_login "$work/csrf-pass.body" -H "Origin: $admin_origin" \
    -H "x-requested-with: XMLHttpRequest")"
check_true "complete CSRF guard reaches credential verification" test "$status" = "401"
check_true "wrong password is invalid_credentials" \
    test "$(body_code "$work/csrf-pass.body")" = "invalid_credentials"

# --- Cookie attributes ------------------------------------------------------
echo "==> checking __Host-refresh attributes"
status="$(login_payload "$admin_username" "$admin_temp_password" |
    api_curl --dump-header "$work/session.headers" --output "$work/session.body" \
        -X POST -H "Content-Type: application/json" -H "Origin: $admin_origin" \
        -H "x-requested-with: XMLHttpRequest" --data @- \
        "$api_url/api/v1/auth/login" >/dev/null
    awk 'NR == 1 { print $2 }' "$work/session.headers")"
check_true "bootstrap login succeeds" test "$status" = "200"
set_cookie="$(header "$work/session.headers" set-cookie)"
check_true "login sets __Host-refresh" grep -q '__Host-refresh=' <<<"$set_cookie"
check_true "refresh cookie is HttpOnly" grep -qi 'HttpOnly' <<<"$set_cookie"
check_true "refresh cookie is Secure" grep -qi 'Secure' <<<"$set_cookie"
check_true "refresh cookie is SameSite=Strict" grep -qi 'SameSite=Strict' <<<"$set_cookie"
check_true "refresh cookie is Path=/" grep -qi 'Path=/' <<<"$set_cookie"
check_true "refresh cookie has Max-Age=43200" grep -qi 'Max-Age=43200' <<<"$set_cookie"
check_true "refresh cookie has no Domain" bash -c '! grep -qi "Domain=" <<<"$1"' _ "$set_cookie"

access_token="$(jq -r '.access_token // empty' "$work/session.body")"
refresh_token="$(sed -n 's/.*__Host-refresh=\([^;]*\).*/\1/p' <<<"$set_cookie" | head -n 1)"
check_true "login returned an access token" test -n "$access_token"
check_true "login returned a refresh token" test -n "$refresh_token"

api_curl --dump-header "$work/refresh.headers" --output "$work/refresh.body" \
    -X POST -H "Content-Type: application/json" -H "Origin: $admin_origin" \
    -H "x-requested-with: XMLHttpRequest" \
    -H "Cookie: __Host-refresh=$refresh_token" --data '{}' \
    "$api_url/api/v1/auth/refresh" >/dev/null
status="$(awk 'NR == 1 { print $2 }' "$work/refresh.headers")"
check_true "refresh succeeds" test "$status" = "200"
rotated_cookie="$(header "$work/refresh.headers" set-cookie)"
check_true "refresh rotates __Host-refresh" grep -q '__Host-refresh=' <<<"$rotated_cookie"
check_true "rotated cookie keeps HttpOnly; Secure; SameSite=Strict; Path=/" \
    bash -c 'grep -qi HttpOnly <<<"$1" && grep -qi Secure <<<"$1" && grep -qi "SameSite=Strict" <<<"$1" && grep -qi "Path=/" <<<"$1"' _ "$rotated_cookie"
rotated_refresh="$(sed -n 's/.*__Host-refresh=\([^;]*\).*/\1/p' <<<"$rotated_cookie" | head -n 1)"
rotated_access="$(jq -r '.access_token // empty' "$work/refresh.body")"

# --- Account lockout --------------------------------------------------------
echo "==> checking account lockout thresholds"
ghost="ghost.$(date +%s)"
attempt() {
    login_payload "$ghost" wrong-password-value |
        api_curl --dump-header "$work/attempt.headers" --output "$work/attempt.body" \
            -X POST -H "Content-Type: application/json" -H "Origin: $admin_origin" \
            -H "x-requested-with: XMLHttpRequest" --data @- \
            "$api_url/api/v1/auth/login" >/dev/null
    awk 'NR == 1 { print $2 }' "$work/attempt.headers"
}
for _ in 1 2 3 4; do
    status="$(attempt)"
    check_true "unknown-identifier failure returns 401" test "$status" = "401"
done
status="$(attempt)"
check_true "fifth failure returns 429 login_locked" test "$status" = "429"
check_true "lockout code is login_locked" test "$(body_code "$work/attempt.body")" = "login_locked"
check_true "lockout carries Retry-After" \
    header_matches "$work/attempt.headers" "retry-after" "^[0-9]+$"

# --- Log, audit and metric redaction ---------------------------------------
echo "==> checking log and audit redaction"
log_secret() {
    local label="$1" secret="$2" source="$3"
    if [[ -z "$secret" ]]; then
        record_fail "$label (secret value was empty; cannot scan)"
        return
    fi
    if grep -Fq -- "$secret" <<<"$source"; then
        record_fail "$label"
    else
        record_pass "$label"
    fi
}

api_logs="$(acceptance_compose logs --no-color --tail=2000 api 2>/dev/null || true)"
gateway_logs="$(acceptance_compose logs --no-color --tail=2000 gateway 2>/dev/null || true)"
db_logs="$(acceptance_compose logs --no-color --tail=2000 db 2>/dev/null || true)"
for secret in "$admin_temp_password" "$access_token" "$refresh_token" \
    "$rotated_access" "$rotated_refresh"; do
    log_secret "API log does not contain a credential value" "$secret" "$api_logs"
    log_secret "gateway log does not contain a credential value" "$secret" "$gateway_logs"
    log_secret "PostgreSQL log does not contain a credential value" "$secret" "$db_logs"
done
check_true "API log does not contain an Authorization header" \
    bash -c '! grep -qi "authorization: bearer" <<<"$1"' _ "$api_logs"
check_true "API log does not contain a cookie header" \
    bash -c '! grep -qi "__Host-refresh=" <<<"$1"' _ "$api_logs"
check_true "API log does not contain a password hash" \
    bash -c '! grep -q "\$argon2id\$" <<<"$1"' _ "$api_logs"
check_true "API log does not contain the admin username" \
    bash -c '! grep -Fq -- "$2" <<<"$1"' _ "$api_logs" "$admin_username"

audit_dump="$(acceptance_psql -tAc "SELECT row_to_json(a)::text FROM audit_logs a" 2>/dev/null || true)"
if [[ -z "$audit_dump" ]]; then
    record_fail "audit rows are readable for the redaction scan"
else
    record_pass "audit rows are readable for the redaction scan"
fi
for secret in "$admin_temp_password" "$access_token" "$refresh_token" \
    "$rotated_access" "$rotated_refresh"; do
    log_secret "audit details do not contain a credential value" "$secret" "$audit_dump"
done
check_true "audit rows do not contain a password hash" \
    bash -c '! grep -q "\$argon2id\$" <<<"$1"' _ "$audit_dump"

metrics="$(acceptance_compose exec -T api curl -fsS http://127.0.0.1:9100/metrics 2>/dev/null || true)"
if [[ -z "$metrics" ]]; then
    record_fail "metrics endpoint is readable inside the edge network"
else
    record_pass "metrics endpoint is readable inside the edge network"
fi
for secret in "$access_token" "$refresh_token" "$rotated_access"; do
    log_secret "metrics do not contain a credential value" "$secret" "$metrics"
done
check_true "metrics do not contain the admin username" \
    bash -c '! grep -Fq -- "$2" <<<"$1"' _ "$metrics" "$admin_username"
check_true "metrics do not contain the admin email" \
    bash -c '! grep -Fq -- "$2" <<<"$1"' _ "$metrics" "$admin_email"

# --- Edge and dependency scans ---------------------------------------------
edge_status=0
echo "==> running scripts/check-deploy-edge.sh"
if "$root/scripts/check-deploy-edge.sh" >"$work/edge.log" 2>&1; then
    record_pass "gateway TLS/HSTS/CSP/SPA/edge-rate-limit checks"
else
    edge_status=1
    record_fail "gateway TLS/HSTS/CSP/SPA/edge-rate-limit checks"
    tail -n 40 "$work/edge.log" >&2 || true
fi

scan_status=0
if ((run_scans)); then
    echo "==> running scripts/security-scan.sh"
    if "$root/scripts/security-scan.sh" --out "$root/docs/acceptance/security-scan.md"; then
        record_pass "dependency and image scans (no CRITICAL)"
    else
        scan_status=1
        record_fail "dependency and image scans (CRITICAL finding)"
    fi
fi

# --- Report -----------------------------------------------------------------
mkdir -p "$(dirname "$out")"
{
    echo "# Live security verification"
    echo
    echo "Generated by \`scripts/security-verify.sh\` at $(date -u +%Y-%m-%dT%H:%M:%SZ) UTC."
    echo
    echo "| Check | Result |"
    echo "|---|---|"
    while IFS=$'\t' read -r result label; do
        echo "| $label | $result |"
    done <"$checks"
    echo
    echo "Dependency and image scan results: [security-scan.md](./security-scan.md)."
    echo
    echo "The gateway edge checks (TLS, HSTS, CSP, SPA fallback, production"
    echo "60r/m + burst 10 rate limit) are run by \`scripts/check-deploy-edge.sh\`."
    echo
    if ((failures == 0)); then
        echo "**Pass.** Every live security check succeeded."
    else
        echo "**Fail.** $failures check(s) failed; do not record this run as accepted."
    fi
} >"$out"
echo "==> wrote $out"

if ((failures > 0)); then
    echo "error: $failures security check(s) failed" >&2
    exit 1
fi
if ((edge_status != 0 || scan_status != 0)); then
    exit 1
fi
