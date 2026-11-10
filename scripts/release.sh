#!/usr/bin/env bash
#
# One-shot release / dry-run / rollback driver for the single-host Compose
# stack (ADR-0008 §9).
#
# The script runs on the deployment host, from the repository checkout whose
# `deploy/` directory holds the production `.env` and Compose file:
#
#   1. verify the requested tag is immutable (never `latest`);
#   2. refuse modified or deleted migrations since the previous release;
#   3. record the previous release, update API_IMAGE/WEB_IMAGE/GIT_COMMIT;
#   4. pull the images;
#   5. run the one-shot `migrate` container (non-zero aborts the release);
#   6. start the database (local-db profile), then `up -d --no-deps` for the
#      long-running services, so `migrate` never runs inside `up`;
#   7. wait up to 120 seconds for the API healthcheck (`/health/ready`);
#   8. run the post-release smoke checks;
#   9. on any failure, switch back to the previous images and rise the stack;
#  10. write a timestamped JSON release record and, on success, the deployed
#      state used by the next rollback.
#
# `--dry-run` performs steps 1-8 and then rolls back to the previous release,
# recording the exercise without changing the deployed state. It requires a
# previous state file; the first release cannot be rehearsed this way.
#
# Migrations are applied forward only. The script never attempts to reverse a
# migration; a failed release keeps the new schema and rolls the code back.

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"

log() {
    printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" >&2
}

die() {
    [[ -n "${error:-}" ]] || error="$*"
    log "error: $*"
    exit 1
}

append_error() {
    if [[ -n "$error" ]]; then
        error="$error; $1"
    else
        error="$1"
    fi
}

usage() {
    cat <<'EOF'
Usage: scripts/release.sh --tag TAG [options]

Required:
  --tag TAG                 Immutable image tag (sha-<short> or vX.Y.Z)

Options:
  --api-image IMAGE         Override the API image (default: <prefix>/api:<tag>)
  --web-image IMAGE         Override the web image (default: <prefix>/web:<tag>)
  --db-image IMAGE          Override the database image (default: <prefix>/db:<tag>)
  --image-prefix PREFIX     GHCR image prefix (default:
                            ghcr.io/nasa-du/rs-admin-ds, or $RELEASE_IMAGE_PREFIX)
  --commit SHA              Release commit recorded in the state/record
                            (default: HEAD)
  --env-file FILE           Compose env file (default: deploy/.env)
  --compose-file FILE       Compose file (default: deploy/compose.yaml)
  --profiles LIST           Space- or comma-separated Compose profiles
                            (default: local-db, or $RELEASE_COMPOSE_PROFILES;
                            use `none` for an external database)
  --state-dir DIR           Directory for deployed state, lock and records
                            (default: /var/lib/rs-admin/releases, or
                            $RELEASE_STATE_DIR)
  --timeout SECONDS         Readiness timeout (default: 120)
  --operator NAME           Operator recorded in the release record
  --ca-cert FILE            CA bundle for the HTTPS smoke checks
  --allow-destructive       Allow a new migration with destructive DDL after
                            taking an on-demand backup (contract phase)
  --dry-run                 Deploy, verify, then roll back to the previous tag
  --skip-smoke              Skip the HTTP smoke checks
  -h, --help                Show this help

Environment:
  RELEASE_IMAGE_PREFIX, RELEASE_COMPOSE_PROFILES, RELEASE_STATE_DIR,
  GITHUB_ACTOR

Exit status: 0 on a successful release or dry-run; non-zero on any failure,
including a successful deployment that had to be rolled back.
EOF
}

tag=""
api_image=""
web_image=""
db_image=""
image_prefix="${RELEASE_IMAGE_PREFIX:-ghcr.io/nasa-du/rs-admin-ds}"
commit=""
env_file="$repo_root/deploy/.env"
compose_file="$repo_root/deploy/compose.yaml"
profiles="${RELEASE_COMPOSE_PROFILES:-local-db}"
state_dir="${RELEASE_STATE_DIR:-/var/lib/rs-admin/releases}"
timeout=120
operator="${GITHUB_ACTOR:-$(id -un)}"
ca_cert=""
allow_destructive=0
dry_run=0
skip_smoke=0

while (($# > 0)); do
    case "$1" in
        --tag)
            [[ $# -ge 2 ]] || { echo "error: --tag needs a value" >&2; exit 2; }
            tag="$2"
            shift 2
            ;;
        --api-image)
            [[ $# -ge 2 ]] || { echo "error: --api-image needs a value" >&2; exit 2; }
            api_image="$2"
            shift 2
            ;;
        --web-image)
            [[ $# -ge 2 ]] || { echo "error: --web-image needs a value" >&2; exit 2; }
            web_image="$2"
            shift 2
            ;;
        --db-image)
            [[ $# -ge 2 ]] || { echo "error: --db-image needs a value" >&2; exit 2; }
            db_image="$2"
            shift 2
            ;;
        --image-prefix)
            [[ $# -ge 2 ]] || { echo "error: --image-prefix needs a value" >&2; exit 2; }
            image_prefix="$2"
            shift 2
            ;;
        --commit)
            [[ $# -ge 2 ]] || { echo "error: --commit needs a value" >&2; exit 2; }
            commit="$2"
            shift 2
            ;;
        --env-file)
            [[ $# -ge 2 ]] || { echo "error: --env-file needs a value" >&2; exit 2; }
            env_file="$2"
            shift 2
            ;;
        --compose-file)
            [[ $# -ge 2 ]] || { echo "error: --compose-file needs a value" >&2; exit 2; }
            compose_file="$2"
            shift 2
            ;;
        --profiles)
            [[ $# -ge 2 ]] || { echo "error: --profiles needs a value" >&2; exit 2; }
            profiles="$2"
            shift 2
            ;;
        --state-dir)
            [[ $# -ge 2 ]] || { echo "error: --state-dir needs a value" >&2; exit 2; }
            state_dir="$2"
            shift 2
            ;;
        --timeout)
            [[ $# -ge 2 ]] || { echo "error: --timeout needs a value" >&2; exit 2; }
            timeout="$2"
            shift 2
            ;;
        --operator)
            [[ $# -ge 2 ]] || { echo "error: --operator needs a value" >&2; exit 2; }
            operator="$2"
            shift 2
            ;;
        --ca-cert)
            [[ $# -ge 2 ]] || { echo "error: --ca-cert needs a value" >&2; exit 2; }
            ca_cert="$2"
            shift 2
            ;;
        --allow-destructive)
            allow_destructive=1
            shift
            ;;
        --dry-run)
            dry_run=1
            shift
            ;;
        --skip-smoke)
            skip_smoke=1
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

[[ -n "$tag" ]] || { echo "error: --tag is required" >&2; exit 2; }
[[ "$tag" != "latest" ]] || die "refusing the floating 'latest' tag"
[[ "$tag" =~ ^[A-Za-z0-9_][A-Za-z0-9_.-]*$ ]] || die "invalid tag: $tag"
[[ "$timeout" =~ ^[0-9]+$ ]] || die "--timeout must be an integer number of seconds"
[[ "$timeout" -gt 0 ]] || die "--timeout must be greater than zero"
[[ -f "$env_file" ]] || die "missing env file: $env_file"
[[ -f "$compose_file" ]] || die "missing compose file: $compose_file"
command -v docker >/dev/null 2>&1 || die "docker is required"
command -v curl >/dev/null 2>&1 || die "curl is required for the smoke checks"

case "$profiles" in
    none) profiles="" ;;
esac
profiles="${profiles//,/ }"

# Accept either a digest or one of the two immutable tag shapes produced by
# the publish workflow. `latest` (and anything else floating) is refused.
is_immutable_image_ref() {
    local image="$1"
    local ref
    if [[ "$image" == *@sha256:* ]]; then
        return 0
    fi
    [[ "$image" == *:* ]] || return 1
    ref="${image##*:}"
    [[ "$ref" =~ ^(sha-[0-9a-f]{7,40}|v[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?)$ ]]
}

image_prefix="${image_prefix%/}"
api_image="${api_image:-$image_prefix/api:$tag}"
web_image="${web_image:-$image_prefix/web:$tag}"
db_image="${db_image:-$image_prefix/db:$tag}"
is_immutable_image_ref "$api_image" || die "API image does not use an immutable tag: $api_image"
is_immutable_image_ref "$web_image" || die "web image does not use an immutable tag: $web_image"
is_immutable_image_ref "$db_image" || die "database image does not use an immutable tag: $db_image"

commit="${commit:-$(git -C "$repo_root" rev-parse HEAD 2>/dev/null || printf 'unknown')}"

mkdir -p "$state_dir"
state_file="$state_dir/deployed.env"
lock_file="$state_dir/release.lock"
started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
mode="release"
[[ "$dry_run" -eq 1 ]] && mode="dry-run"

result=""
failure_stage=""
error=""
ready_seconds=""
smoke_result="not-run"
rollback_performed="false"
rollback_ready_json="null"
rollback_seconds=""
record_path=""
smoke_work=""
migrations=()

# Serialise releases. `flock` is part of util-linux on the supported hosts;
# if it is missing the operator still gets the state checks below.
if command -v flock >/dev/null 2>&1; then
    exec 9>"$lock_file"
    flock -n 9 || die "another release is already running (lock: $lock_file)"
fi

state_get() {
    local key="$1"
    [[ -f "$state_file" ]] || return 0
    sed -n "s/^${key}=//p" "$state_file" | tail -n 1
}

previous_tag="$(state_get TAG)"
previous_api_image="$(state_get API_IMAGE)"
previous_web_image="$(state_get WEB_IMAGE)"
previous_db_image="$(state_get DB_IMAGE)"
previous_commit="$(state_get COMMIT)"

json_escape() {
    local value="$1"
    value="${value//\\/\\\\}"
    value="${value//\"/\\\"}"
    value="${value//$'\n'/\\n}"
    value="${value//$'\r'/\\r}"
    value="${value//$'\t'/\\t}"
    printf '%s' "$value"
}

json_string_or_null() {
    if [[ -n "$1" ]]; then
        printf '"%s"' "$(json_escape "$1")"
    else
        printf 'null'
    fi
}

json_number_or_null() {
    if [[ -n "$1" ]]; then
        printf '%s' "$1"
    else
        printf 'null'
    fi
}

write_record() {
    local exit_code="$1"
    local finished_at
    finished_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    record_path="$state_dir/${started_at//:/-}-${mode}-${tag}-$$.json"

    local migrations_json=""
    if ((${#migrations[@]} > 0)); then
        local migration first=1
        for migration in "${migrations[@]}"; do
            [[ -n "$migration" ]] || continue
            if [[ "$first" -eq 1 ]]; then first=0; else migrations_json+=", "; fi
            migrations_json+="\"$(json_escape "$migration")\""
        done
    fi

    local rollback_performed_json="false"
    [[ "$rollback_performed" == "true" ]] && rollback_performed_json="true"
    local destructive_ack_json="false"
    [[ "$allow_destructive" -eq 1 ]] && destructive_ack_json="true"

    {
        printf '{\n'
        printf '  "schema_version": 1,\n'
        printf '  "mode": "%s",\n' "$(json_escape "$mode")"
        printf '  "result": "%s",\n' "$(json_escape "${result:-failed}")"
        printf '  "failure_stage": %s,\n' "$(json_string_or_null "$failure_stage")"
        printf '  "error": %s,\n' "$(json_string_or_null "$error")"
        printf '  "exit_code": %s,\n' "$exit_code"
        printf '  "requested_tag": "%s",\n' "$(json_escape "$tag")"
        printf '  "api_image": "%s",\n' "$(json_escape "$api_image")"
        printf '  "web_image": "%s",\n' "$(json_escape "$web_image")"
        printf '  "db_image": "%s",\n' "$(json_escape "$db_image")"
        printf '  "commit": "%s",\n' "$(json_escape "$commit")"
        printf '  "previous_tag": %s,\n' "$(json_string_or_null "$previous_tag")"
        printf '  "previous_api_image": %s,\n' "$(json_string_or_null "$previous_api_image")"
        printf '  "previous_web_image": %s,\n' "$(json_string_or_null "$previous_web_image")"
        printf '  "previous_db_image": %s,\n' "$(json_string_or_null "$previous_db_image")"
        printf '  "operator": "%s",\n' "$(json_escape "$operator")"
        printf '  "started_at": "%s",\n' "$started_at"
        printf '  "finished_at": "%s",\n' "$finished_at"
        printf '  "ready_seconds": %s,\n' "$(json_number_or_null "$ready_seconds")"
        printf '  "smoke": "%s",\n' "$(json_escape "$smoke_result")"
        printf '  "rollback": {"performed": %s, "ready": %s, "tag": %s, "seconds": %s},\n' \
            "$rollback_performed_json" "$rollback_ready_json" \
            "$(json_string_or_null "$previous_tag")" "$(json_number_or_null "$rollback_seconds")"
        printf '  "destructive_migrations_acknowledged": %s,\n' "$destructive_ack_json"
        printf '  "migrations": [%s]\n' "$migrations_json"
        printf '}\n'
    } >"$record_path"
}

finish() {
    local code=$?
    trap - EXIT
    if [[ -z "$result" ]]; then
        result="failed"
        [[ -n "$error" ]] || error="release aborted"
    fi
    write_record "$code" || log "warning: could not write the release record"
    if [[ -n "$smoke_work" ]]; then
        rm -rf "$smoke_work"
    fi
    printf 'RELEASE_RESULT=%s\n' "$result"
    printf 'RELEASE_RECORD=%s\n' "$record_path"
    exit "$code"
}
trap finish EXIT

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

set_env_var() {
    local key="$1"
    local value="$2"
    local tmp mode
    tmp="$(mktemp "${env_file}.XXXXXX")"
    awk -v k="$key" -v v="$value" '
        BEGIN { found = 0 }
        index($0, k "=") == 1 { print k "=" v; found = 1; next }
        { print }
        END { if (!found) print k "=" v }
    ' "$env_file" >"$tmp"
    mode="$(stat -c '%a' "$env_file" 2>/dev/null || printf '600')"
    chmod "$mode" "$tmp"
    mv "$tmp" "$env_file"
}

compose_args=(-f "$compose_file" --env-file "$env_file")
for profile in $profiles; do
    compose_args+=(--profile "$profile")
done

compose() {
    docker compose "${compose_args[@]}" "$@"
}

compose_migrate() {
    # This explicit run is the release gate. The later `compose_up` deliberately
    # uses `--no-deps` so the one-shot service is not executed a second time
    # (and so a rollback does not run the previous image's migrator against a
    # newer schema, which SQLx rejects as VersionMissing).
    local args=("${compose_args[@]}")
    local has_tools=0 profile
    for profile in $profiles; do
        [[ "$profile" == "tools" ]] && has_tools=1
    done
    if [[ "$has_tools" -eq 0 ]]; then
        args+=(--profile tools)
    fi
    docker compose "${args[@]}" run --rm migrate
}

compose_up() {
    # Start the database first when it is part of this stack, then raise only
    # the long-running services. `--no-deps` keeps the one-shot migration out
    # of `up`; it has already run explicitly (ADR-0008 §9.2).
    if [[ " $profiles " == *" local-db "* ]]; then
        if ! compose up -d db; then
            return 1
        fi
        if ! wait_for_service db; then
            log "error: the database did not become ready"
            return 1
        fi
    fi
    compose up -d --remove-orphans --no-deps gateway web api
}

wait_for_service() {
    local service="$1"
    local deadline=$((SECONDS + timeout))
    local container_id status
    while ((SECONDS < deadline)); do
        container_id="$(compose ps -q "$service" 2>/dev/null || true)"
        if [[ -n "$container_id" ]]; then
            status="$(docker inspect \
                --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' \
                "$container_id" 2>/dev/null || true)"
            if [[ "$status" == "healthy" ]]; then
                return 0
            fi
        fi
        sleep 1
    done
    return 1
}

restore_previous_env() {
    if [[ -z "$previous_api_image" || -z "$previous_web_image" ]]; then
        return 1
    fi
    set_env_var API_IMAGE "$previous_api_image"
    set_env_var WEB_IMAGE "$previous_web_image"
    # Releases before the database/backup image existed have no DB_IMAGE in
    # their state. Keep the current database image: it is the same PostgreSQL
    # 18.6 major version and the schema is forward-only.
    if [[ -n "$previous_db_image" ]]; then
        set_env_var DB_IMAGE "$previous_db_image"
    fi
    if [[ -n "$previous_commit" ]]; then
        set_env_var GIT_COMMIT "$previous_commit"
    fi
}

rollback_to_previous() {
    log "rolling back to ${previous_tag:-the previous release}"
    if ! restore_previous_env; then
        log "error: no previous release is recorded; rollback is not possible"
        return 1
    fi
    rollback_performed="true"
    if ! compose pull; then
        log "error: could not pull the previous images"
        return 1
    fi
    if ! compose_up; then
        log "error: could not start the previous release"
        return 1
    fi
    local rollback_start=$SECONDS
    if ! wait_for_service api; then
        log "error: the previous release did not become ready"
        return 1
    fi
    rollback_ready_json="true"
    rollback_seconds=$((SECONDS - rollback_start))
    return 0
}

smoke_checks() {
    if [[ "$skip_smoke" -eq 1 ]]; then
        smoke_result="skipped"
        return 0
    fi

    local api_origin admin_origin
    api_origin="$(env_get PUBLIC_API_ORIGIN)"
    admin_origin="$(env_get ADMIN_ORIGIN)"
    if [[ -z "$api_origin" || -z "$admin_origin" ]]; then
        smoke_result="failed"
        error="PUBLIC_API_ORIGIN or ADMIN_ORIGIN is missing from $env_file"
        return 1
    fi

    smoke_work="$(mktemp -d)"
    local work="$smoke_work"
    local curl_opts=(-sS --max-time 15)
    if [[ -n "$ca_cert" ]]; then
        curl_opts+=(--cacert "$ca_cert")
    fi

    local code
    code="$(curl "${curl_opts[@]}" -o "$work/me.json" -D "$work/me.headers" \
        -w '%{http_code}' "$api_origin/api/v1/auth/me" || true)"
    if [[ "$code" != "401" ]]; then
        smoke_result="failed"
        error="unauthenticated GET /api/v1/auth/me returned ${code:-no response}, expected 401"
        return 1
    fi
    if ! grep -qi '^content-type: application/problem+json' "$work/me.headers"; then
        smoke_result="failed"
        error="unauthenticated GET /api/v1/auth/me did not return application/problem+json"
        return 1
    fi
    if ! grep -q '"code"' "$work/me.json"; then
        smoke_result="failed"
        error="unauthenticated GET /api/v1/auth/me did not return a problem code"
        return 1
    fi

    code="$(curl "${curl_opts[@]}" -o /dev/null -D "$work/preflight.headers" \
        -w '%{http_code}' -X OPTIONS \
        -H "Origin: $admin_origin" \
        -H "Access-Control-Request-Method: GET" \
        "$api_origin/api/v1/auth/me" || true)"
    if [[ "$code" != "204" ]]; then
        smoke_result="failed"
        error="CORS preflight returned ${code:-no response}, expected 204"
        return 1
    fi
    local allow_origin
    allow_origin="$(tr -d '\r' <"$work/preflight.headers" | awk '
        {
            colon = index($0, ":")
            if (colon > 0 && tolower(substr($0, 1, colon - 1)) == "access-control-allow-origin") {
                value = substr($0, colon + 1)
                sub(/^[[:space:]]+/, "", value)
                found = value
            }
        }
        END { print found }
    ')"
    if [[ "$allow_origin" != "$admin_origin" ]]; then
        smoke_result="failed"
        error="CORS preflight allowed '${allow_origin:-no origin}', expected '$admin_origin'"
        return 1
    fi

    code="$(curl "${curl_opts[@]}" -o "$work/index.html" -w '%{http_code}' "$admin_origin/" || true)"
    if [[ "$code" != "200" ]]; then
        smoke_result="failed"
        error="frontend GET / returned ${code:-no response}, expected 200"
        return 1
    fi

    smoke_result="passed"
    return 0
}

# --- Forward-only migration guard ------------------------------------------

if [[ -n "$previous_commit" && "$previous_commit" != "unknown" && "$previous_commit" != "$commit" ]]; then
    if git -C "$repo_root" cat-file -e "${previous_commit}^{commit}" 2>/dev/null; then
        # The host checkout is at the release commit, so the shared checker
        # validates the target tree against the previously deployed schema.
        checker_args=(--base-ref "$previous_commit")
        if [[ "$allow_destructive" -eq 0 ]]; then
            checker_args+=(--fail-on-destructive)
        fi
        if ! "$repo_root/scripts/check-migrations.sh" "${checker_args[@]}" >&2; then
            failure_stage="migration-policy"
            die "refusing release: migration policy check failed"
        fi
        mapfile -t migrations < <(
            git -C "$repo_root" diff --no-renames --name-only "$previous_commit" "$commit" \
                -- backend/migrations 2>/dev/null | sed 's#^backend/migrations/##'
        )
    else
        log "warning: previous release commit $previous_commit is not available locally; skipping the forward-only migration guard"
    fi
elif [[ -z "$previous_commit" || "$previous_commit" == "unknown" ]]; then
    mapfile -t migrations < <(
        git -C "$repo_root" ls-files backend/migrations 2>/dev/null | sed 's#^backend/migrations/##'
    )
fi

if [[ "$dry_run" -eq 1 && ( -z "$previous_api_image" || -z "$previous_web_image" ) ]]; then
    failure_stage="dry-run-unavailable"
    die "dry-run requires a previous release in $state_file"
fi

log "release $mode: tag=$tag commit=$commit operator=$operator"
log "state directory: $state_dir"
log "previous release: ${previous_tag:-none}"

set_env_var API_IMAGE "$api_image"
set_env_var WEB_IMAGE "$web_image"
set_env_var DB_IMAGE "$db_image"
set_env_var GIT_COMMIT "$commit"

if ! compose pull; then
    failure_stage="pull"
    if [[ -n "$previous_api_image" ]]; then
        restore_previous_env || true
    fi
    die "image pull failed; release was not started"
fi

if ! compose_migrate; then
    failure_stage="migrate"
    if [[ -n "$previous_api_image" ]]; then
        restore_previous_env || true
    fi
    die "migration failed; the running release was left untouched"
fi

if ! compose_up; then
    failure_stage="up"
    rollback_to_previous || append_error "rollback to the previous release failed"
    die "docker compose up failed"
fi

ready_start=$SECONDS
if ! wait_for_service api; then
    failure_stage="ready"
    compose logs --tail=100 api >&2 || true
    rollback_to_previous || append_error "rollback to the previous release failed"
    die "API did not become ready within ${timeout}s"
fi
ready_seconds=$((SECONDS - ready_start))
log "API became ready after ${ready_seconds}s"

if ! smoke_checks; then
    failure_stage="smoke"
    rollback_to_previous || append_error "rollback to the previous release failed"
    die "post-release smoke checks failed: $error"
fi

if [[ "$dry_run" -eq 1 ]]; then
    if ! rollback_to_previous; then
        failure_stage="dry-run-rollback"
        die "dry-run deployed and verified, but the rollback to ${previous_tag:-the previous release} failed"
    fi
    log "dry-run verified ${tag} and rolled back to ${previous_tag:-the previous release}"
    result="success"
    exit 0
fi

{
    printf 'TAG=%s\n' "$tag"
    printf 'API_IMAGE=%s\n' "$api_image"
    printf 'WEB_IMAGE=%s\n' "$web_image"
    printf 'DB_IMAGE=%s\n' "$db_image"
    printf 'COMMIT=%s\n' "$commit"
    printf 'DEPLOYED_AT=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} >"$state_file.tmp"
mv "$state_file.tmp" "$state_file"

log "release $tag is live"
result="success"
exit 0
