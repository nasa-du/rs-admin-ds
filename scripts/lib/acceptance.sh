# Shared helpers for the acceptance harnesses (spec §12.6–§12.8). Source this
# file from scripts/perf.sh, scripts/security-verify.sh and
# scripts/fault-injection.sh; it must not be executed directly.

# shellcheck source=bun.sh
. "$(dirname "${BASH_SOURCE[0]}")/bun.sh"
#
# The harness owns an isolated Compose project with its own network ranges and
# secrets directory. It is destructive by design: every entry point resets the
# project volumes so measurements always start from an empty database.

acceptance_project="${ACCEPTANCE_PROJECT:-rs-admin-accept}"
acceptance_env_file="$root/deploy/.env.example"
acceptance_secrets_dir="$root/deploy/secrets-acceptance"
acceptance_compose_file="$root/deploy/compose.yaml"
acceptance_overlay_file="$root/deploy/compose.acceptance.yaml"

acceptance_compose() {
    # The harness makes auth calls faster than a human; the overlay raises the
    # edge ceiling. The production limiter is verified separately by
    # scripts/check-deploy-edge.sh with the 60r/m defaults.
    RS_ADMIN_SECRETS_DIR=./secrets-acceptance \
        AUTH_RATE_LIMIT="${AUTH_RATE_LIMIT:-600r/m}" \
        AUTH_RATE_BURST="${AUTH_RATE_BURST:-100}" \
        docker compose \
        --project-name "$acceptance_project" \
        --env-file "$acceptance_env_file" \
        --file "$acceptance_compose_file" \
        --file "$acceptance_overlay_file" \
        --profile local-db \
        "$@"
}

# Locate Bun. Resolve order and the version rule live in lib/bun.sh.
acceptance_bun() {
    resolve_bun
}

acceptance_require_docker() {
    if ! command -v docker >/dev/null 2>&1; then
        echo "error: docker is required" >&2
        exit 1
    fi
    if ! command -v jq >/dev/null 2>&1; then
        echo "error: jq is required" >&2
        exit 1
    fi
}

acceptance_generate_secrets() {
    if [[ -f "$acceptance_secrets_dir/database_url" ]]; then
        return 0
    fi
    # Reuse the existing development material when it is present. The
    # acceptance stack never leaves the host and these keys are explicitly
    # development-only; copying still keeps this project isolated from writes
    # to secrets-dev. On a fresh checkout fall back to the generator.
    if [[ -f "$root/deploy/secrets-dev/database_url" ]]; then
        echo "==> copying development secrets to $acceptance_secrets_dir"
        cp -a "$root/deploy/secrets-dev" "$acceptance_secrets_dir"
        return 0
    fi
    echo "==> generating acceptance secrets in $acceptance_secrets_dir"
    RS_ADMIN_SECRETS_TARGET="$acceptance_secrets_dir" \
        "$root/scripts/generate-dev-secrets.sh"
}

# Build the images only when they are absent; --build forces a rebuild and
# --no-build refuses to build at all.
acceptance_ensure_images() {
    local build=1 force=0
    case "${1:-}" in
        --no-build) build=0 ;;
        --build) force=1 ;;
    esac

    local api_image web_image db_image
    api_image="$(sed -n 's/^API_IMAGE=//p' "$acceptance_env_file" | tail -n 1)"
    web_image="$(sed -n 's/^WEB_IMAGE=//p' "$acceptance_env_file" | tail -n 1)"
    db_image="$(sed -n 's/^DB_IMAGE=//p' "$acceptance_env_file" | tail -n 1)"

    if ((build == 0)); then
        return 0
    fi
    if ((force == 0)) &&
        docker image inspect "$api_image" "$web_image" "$db_image" >/dev/null 2>&1; then
        echo "==> reusing local images ($api_image, $web_image, $db_image)"
        return 0
    fi

    echo "==> building api, web and db images"
    acceptance_compose build api web db
}

acceptance_stop_hint() {
    echo "    stop:  RS_ADMIN_SECRETS_DIR=./secrets-acceptance docker compose \\" >&2
    echo "             --project-name $acceptance_project --env-file deploy/.env.example \\" >&2
    echo "             -f deploy/compose.yaml -f deploy/compose.acceptance.yaml \\" >&2
    echo "             --profile local-db down --volumes" >&2
}

# Reset the project (including volumes) and start db → migrate → api → web →
# gateway, waiting for readiness.
acceptance_up() {
    echo "==> resetting Compose project $acceptance_project (volumes included)"
    acceptance_compose down --volumes --remove-orphans >/dev/null 2>&1 || true

    echo "==> starting db, migrate, api, web and gateway"
    if ! acceptance_compose up --detach --wait --wait-timeout 300 \
        db migrate api web gateway; then
        echo "error: the acceptance stack did not become healthy" >&2
        acceptance_compose ps >&2 || true
        acceptance_compose logs --tail=120 api migrate db >&2 || true
        acceptance_stop_hint
        exit 1
    fi
}

acceptance_down() {
    [[ "${1:-}" == "--keep" ]] && return 0
    echo "==> removing the acceptance stack"
    acceptance_compose down --volumes --remove-orphans >/dev/null 2>&1 || true
}

# Bootstrap the acceptance super administrator. Prints the CLI output on
# success; returns non-zero (with output on stderr) on failure.
acceptance_bootstrap() {
    local username="$1" email="$2" password="$3"
    local output
    if ! output="$(
        printf '%s\n' "$password" |
            acceptance_compose run --rm --no-deps -T \
                --entrypoint /usr/local/bin/rs-admin-bootstrap-super-admin \
                api --email "$email" --username "$username" \
                --password-stdin 2>&1
    )"; then
        printf '%s\n' "$output" >&2
        return 1
    fi
    if ! grep -q "Super administrator created\." <<<"$output"; then
        printf '%s\n' "$output" >&2
        return 1
    fi
    printf '%s\n' "$output"
}

acceptance_psql() {
    acceptance_compose exec -T db psql \
        -U "${POSTGRES_USER:-rs_admin}" \
        -d "${POSTGRES_DB:-rs_admin}" \
        -v ON_ERROR_STOP=1 "$@"
}

acceptance_gateway_https_port() {
    acceptance_compose port gateway 8443 | head -n 1 | sed 's/.*://'
}
