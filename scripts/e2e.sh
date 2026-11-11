#!/usr/bin/env bash
#
# Full-stack E2E journeys (spec §12.5).
#
# Boots an isolated Compose project (gateway/web/api/db), applies the embedded
# migrations, bootstraps a fresh super administrator and drives the nine
# journeys in a real Chromium through the production gateway. The stack is
# reset first, so journey 1 always starts from an empty database.
#
# Usage:
#   ./scripts/e2e.sh                 # reset, build, run, tear down
#   ./scripts/e2e.sh --keep          # leave the stack running for debugging
#   ./scripts/e2e.sh --no-build      # reuse the local images
#   ./scripts/e2e.sh --headed        # watch the browser
#   ./scripts/e2e.sh -- --grep "..." # extra Playwright arguments
#
# Never run this against production: it destroys the project's volumes and
# creates test users.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

# Deliberately not overridable: the script destroys this project's volumes,
# and a typo or an inherited variable pointing at the production project would
# be unrecoverable. Parallel runs are not supported because the gateway binds
# host ports 80/443.
project="rs-admin-e2e"
env_file="$root/deploy/.env.example"
compose_file="$root/deploy/compose.yaml"
e2e_compose_file="$root/deploy/compose.e2e.yaml"
secrets_dir="$root/deploy/secrets-e2e"
# Keep the E2E key material separate from a developer's secrets-dev directory,
# which may intentionally contain only a subset of the files.
export RS_ADMIN_SECRETS_DIR=./secrets-e2e
# The suite makes credential calls faster than a human. This only changes the
# isolated E2E gateway; production keeps the 60r/m + burst 10 defaults, and the
# edge limit itself is verified by scripts/check-deploy-edge.sh.
export AUTH_RATE_LIMIT="${AUTH_RATE_LIMIT:-600r/m}"
export AUTH_RATE_BURST="${AUTH_RATE_BURST:-100}"

bootstrap_username="${E2E_BOOTSTRAP_USERNAME:-ops.e2e}"
bootstrap_email="${E2E_BOOTSTRAP_EMAIL:-ops.e2e@example.com}"
bootstrap_password="${E2E_BOOTSTRAP_PASSWORD:-E2E-Bootstrap-Anchor-2026!}"
admin_password="${E2E_ADMIN_PASSWORD:-E2E-Admin-Harbor-2026!}"
admin_origin="${E2E_ADMIN_ORIGIN:-https://admin.example.com}"
api_origin="${E2E_API_ORIGIN:-https://api.example.com}"

build=1
keep=0
playwright_args=()

while (($# > 0)); do
    case "$1" in
        --keep) keep=1 ;;
        --no-build) build=0 ;;
        --headed) playwright_args+=(--headed) ;;
        --help | -h)
            awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0"
            exit 0
            ;;
        --)
            shift
            playwright_args+=("$@")
            break
            ;;
        *)
            echo "error: unknown argument: $1" >&2
            exit 2
            ;;
    esac
    shift
done

for command in docker bun; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "error: $command is required" >&2
        exit 1
    fi
done

compose() {
    docker compose \
        --project-name "$project" \
        --env-file "$env_file" \
        --file "$compose_file" \
        --file "$e2e_compose_file" \
        "$@"
}

stop_hint() {
    echo "    admin: $admin_origin (Chromium resolves the host to 127.0.0.1)" >&2
    echo "    stop:  RS_ADMIN_SECRETS_DIR=./secrets-e2e docker compose --project-name $project --env-file deploy/.env.example -f deploy/compose.yaml -f deploy/compose.e2e.yaml --profile local-db down --volumes" >&2
}

if [[ ! -f "$secrets_dir/database_url" ]]; then
    echo "==> generating E2E secrets in $secrets_dir"
    RS_ADMIN_SECRETS_TARGET="$secrets_dir" \
        "$root/scripts/generate-dev-secrets.sh"
fi

# Prepare the runner before touching Docker: a toolchain failure must not
# leave a stack running with well-known test credentials.
echo "==> installing E2E dependencies and Chromium"
(cd "$root/e2e" && bun install --frozen-lockfile)
(cd "$root/e2e" && bunx playwright install chromium)
(cd "$root/e2e" && bun run typecheck)

echo "==> resetting Compose project $project (volumes included)"
compose --profile local-db down --volumes --remove-orphans

if ((build)); then
    echo "==> building api, web and db images"
    compose --profile local-db build api web db
fi

echo "==> starting db, migrate, api, web and gateway"
if ! compose --profile local-db up --detach --wait --wait-timeout 300 \
    db migrate api web gateway; then
    echo "error: the stack did not become healthy" >&2
    compose --profile local-db ps >&2 || true
    compose --profile local-db logs --tail=120 api migrate db >&2 || true
    stop_hint
    exit 1
fi

echo "==> bootstrapping the super administrator"
if ! bootstrap_output="$(
    printf '%s\n' "$bootstrap_password" |
        compose --profile local-db run --rm --no-deps -T \
            --entrypoint /usr/local/bin/rs-admin-bootstrap-super-admin \
            api --email "$bootstrap_email" --username "$bootstrap_username" \
            --password-stdin 2>&1
)"; then
    printf '%s\n' "$bootstrap_output" >&2
    echo "error: bootstrap failed; is the database fresh?" >&2
    compose --profile local-db logs --tail=120 api migrate db >&2 || true
    stop_hint
    exit 1
fi
if ! grep -q "Super administrator created\." <<<"$bootstrap_output"; then
    printf '%s\n' "$bootstrap_output" >&2
    echo "error: bootstrap did not report a created super administrator" >&2
    compose --profile local-db logs --tail=120 api migrate db >&2 || true
    stop_hint
    exit 1
fi

status=0
echo "==> running the nine journeys against $admin_origin"
(
    cd "$root/e2e"
    E2E_ADMIN_ORIGIN="$admin_origin" \
        E2E_API_ORIGIN="$api_origin" \
        E2E_BOOTSTRAP_USERNAME="$bootstrap_username" \
        E2E_BOOTSTRAP_PASSWORD="$bootstrap_password" \
        E2E_ADMIN_PASSWORD="$admin_password" \
        bunx playwright test ${playwright_args[@]+"${playwright_args[@]}"}
) || status=$?

if ((status != 0)); then
    echo "==> journeys failed; keeping the stack for debugging" >&2
    compose --profile local-db ps >&2 || true
    compose --profile local-db logs --tail=120 api gateway >&2 || true
    stop_hint
    exit "$status"
fi

if ((keep)); then
    echo "==> journeys passed; stack kept at $admin_origin"
    stop_hint
else
    echo "==> journeys passed; removing the stack"
    compose --profile local-db down --volumes --remove-orphans
fi
