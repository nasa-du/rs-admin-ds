#!/usr/bin/env bash
#
# Migration policy checks for CI and release.
#
# Static checks (always):
#   - migration files use a UTC timestamp prefix and a unique version;
#   - migrations that already exist in the base ref are never modified,
#     renamed or deleted (forward-only);
#   - new migrations sort after every base-ref migration;
#   - destructive DDL in new migrations is reported as a warning so the
#     expand/contract order is reviewed.
#
# Schema checks (when --database-url is given):
#   - apply all migrations to an empty database;
#   - apply the base-ref migrations, then the new migrations, to a second
#     database, proving the release is compatible with the previous schema.
#
# The database checks require psql and a role that can CREATE DATABASE. CI
# passes the same PG18 service DSN as the integration tests.
#
#   ./scripts/check-migrations.sh --base-ref origin/main
#   ./scripts/check-migrations.sh --base-ref "$BASE_SHA" --database-url "$DATABASE_URL"

set -euo pipefail

# shellcheck source=lib/deploy-check.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib/deploy-check.sh"
cd "$root"

migrations_dir="$root/backend/migrations"
base_ref="HEAD"
database_url=""
fail_on_destructive=0

usage() {
    cat <<'EOF'
Usage: scripts/check-migrations.sh [options]

  --base-ref REF        Git ref that already exists in the previous release
                        (default: HEAD; CI passes the PR base or previous push)
  --database-url URL    PostgreSQL DSN used for the empty-database and
                        previous-schema checks
  --fail-on-destructive Fail when a new migration contains destructive DDL;
                        the release flow uses this unless the operator
                        explicitly acknowledges the contract phase
  -h, --help            Show this help
EOF
}

while (($# > 0)); do
    case "$1" in
        --base-ref)
            [[ $# -ge 2 ]] || { echo "error: --base-ref needs a value" >&2; exit 2; }
            base_ref="$2"
            shift 2
            ;;
        --database-url)
            [[ $# -ge 2 ]] || { echo "error: --database-url needs a value" >&2; exit 2; }
            database_url="$2"
            shift 2
            ;;
        --fail-on-destructive)
            fail_on_destructive=1
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

if [[ ! -d "$migrations_dir" ]]; then
    echo "error: missing migration directory: $migrations_dir" >&2
    exit 1
fi

mapfile -t current_files < <(
    find "$migrations_dir" -maxdepth 1 -type f -name '*.sql' -printf '%f\n' | LC_ALL=C sort
)

if ((${#current_files[@]} == 0)); then
    fail "at least one migration exists"
fi

declare -A seen_versions=()
for file in "${current_files[@]}"; do
    if [[ ! "$file" =~ ^([0-9]{14})_[a-z0-9_]+\.sql$ ]]; then
        fail "migration name uses a UTC timestamp prefix: $file"
        continue
    fi
    version="${BASH_REMATCH[1]}"
    if [[ -n "${seen_versions[$version]:-}" ]]; then
        fail "migration version $version is unique (also used by ${seen_versions[$version]})"
    else
        seen_versions[$version]="$file"
        pass "migration name is valid: $file"
    fi
done

base_commit=""
declare -A base_files=()
declare -a base_versions=()
if git rev-parse --verify --quiet "${base_ref}^{commit}" >/dev/null; then
    base_commit="$(git rev-parse "${base_ref}^{commit}")"
    mapfile -t base_entries < <(
        git ls-tree -r --name-only "$base_commit" -- backend/migrations |
            sed 's#^backend/migrations/##' |
            grep -E '\.sql$' |
            LC_ALL=C sort || true
    )
    for file in "${base_entries[@]}"; do
        [[ -n "$file" ]] || continue
        base_files["$file"]=1
        if [[ "$file" =~ ^([0-9]{14})_ ]]; then
            base_versions+=("${BASH_REMATCH[1]}")
        fi
    done
else
    echo "warning: base ref '$base_ref' is not available; skipping immutability and previous-schema checks" >&2
fi

base_max_version=""
if ((${#base_versions[@]} > 0)); then
    base_max_version="$(printf '%s\n' "${base_versions[@]}" | LC_ALL=C sort | tail -n 1)"
fi

new_files=()
if [[ -n "$base_commit" ]]; then
    for file in "${current_files[@]}"; do
        if [[ -n "${base_files[$file]:-}" ]]; then
            # Tracked in the base ref: any working-tree change is a violation.
            if ! git diff --quiet "$base_commit" -- "backend/migrations/$file"; then
                fail "existing migration is immutable: $file"
            else
                pass "existing migration is unchanged: $file"
            fi
        else
            new_files+=("$file")
        fi
    done

    for file in "${base_entries[@]}"; do
        [[ -n "$file" ]] || continue
        if [[ ! -f "$migrations_dir/$file" ]]; then
            fail "existing migration was deleted or renamed: $file"
        fi
    done

    if [[ -n "$base_max_version" ]]; then
        for file in "${new_files[@]}"; do
            version="${file%%_*}"
            if [[ "$version" > "$base_max_version" ]]; then
                pass "new migration sorts after the previous release: $file"
            else
                fail "new migration must sort after $base_max_version: $file"
            fi
        done
    fi
fi

for file in "${new_files[@]}"; do
    if grep -Eiq '\b(DROP[[:space:]]+(TABLE|COLUMN|INDEX|CONSTRAINT)|TRUNCATE|ALTER[[:space:]]+COLUMN[^;]*TYPE)\b' \
        "$migrations_dir/$file"; then
        if [[ "$fail_on_destructive" -eq 1 ]]; then
            fail "new migration contains destructive DDL: $file (take an on-demand backup and approve the contract phase explicitly)"
        else
            echo "warning: $file contains destructive DDL; confirm the expand/contract order and take an on-demand backup before release" >&2
        fi
    fi
done

# --- Schema checks ---------------------------------------------------------

database_name_from_url() {
    local url="$1"
    if [[ "$url" =~ ^([^?]*/)([^/?]+)(\?.*)?$ && "${BASH_REMATCH[1]}" == *://* ]]; then
        printf '%s' "${BASH_REMATCH[2]}"
        return 0
    fi
    return 1
}

url_with_database() {
    local url="$1"
    local name="$2"
    if [[ "$url" =~ ^([^?]*/)([^/?]+)(\?.*)?$ && "${BASH_REMATCH[1]}" == *://* ]]; then
        printf '%s%s%s' "${BASH_REMATCH[1]}" "$name" "${BASH_REMATCH[3]:-}"
        return 0
    fi
    return 1
}

apply_migration_files() {
    local url="$1"
    shift
    local file
    for file in "$@"; do
        if ! psql "$url" --no-psqlrc --set=ON_ERROR_STOP=1 --quiet --file="$file" >/dev/null; then
            echo "error: migration failed on $url: $file" >&2
            return 1
        fi
    done
}

if [[ -n "$database_url" ]]; then
    if ! command -v psql >/dev/null 2>&1; then
        echo "error: psql is required for --database-url" >&2
        exit 1
    fi
    if ! database_name_from_url "$database_url" >/dev/null; then
        echo "error: DATABASE_URL must include a database name" >&2
        exit 1
    fi

    created_databases=()
    cleanup_databases() {
        local db
        for db in "${created_databases[@]:-}"; do
            [[ -n "$db" ]] || continue
            psql "$database_url" --no-psqlrc --quiet --set=ON_ERROR_STOP=1 \
                --command="DROP DATABASE IF EXISTS \"$db\" WITH (FORCE)" >/dev/null 2>&1 || true
        done
    }
    trap cleanup_databases EXIT

    suffix="$$_${RANDOM}"

    fresh_db="rs_admin_migcheck_fresh_$suffix"
    if ! psql "$database_url" --no-psqlrc --quiet --set=ON_ERROR_STOP=1 \
        --command="CREATE DATABASE \"$fresh_db\"" >/dev/null; then
        echo "error: could not create the empty-schema check database" >&2
        exit 1
    fi
    created_databases+=("$fresh_db")
    fresh_url="$(url_with_database "$database_url" "$fresh_db")"
    mapfile -t current_paths < <(printf '%s\n' "${current_files[@]}" | sed "s#^#$migrations_dir/#")
    if apply_migration_files "$fresh_url" "${current_paths[@]}"; then
        pass "all migrations apply to an empty database"
    else
        fail "all migrations apply to an empty database"
    fi

    if [[ -n "$base_commit" ]]; then
        previous_db="rs_admin_migcheck_previous_$suffix"
        if ! psql "$database_url" --no-psqlrc --quiet --set=ON_ERROR_STOP=1 \
            --command="CREATE DATABASE \"$previous_db\"" >/dev/null; then
            echo "error: could not create the previous-schema check database" >&2
            exit 1
        fi
        created_databases+=("$previous_db")
        previous_url="$(url_with_database "$database_url" "$previous_db")"

        base_work="$(mktemp -d)"
        trap 'rm -rf "$base_work"; cleanup_databases' EXIT
        previous_files=()
        for file in "${base_entries[@]}"; do
            [[ -n "$file" ]] || continue
            git show "$base_commit:backend/migrations/$file" >"$base_work/$file"
            previous_files+=("$base_work/$file")
        done

        if apply_migration_files "$previous_url" "${previous_files[@]}"; then
            new_paths=()
            for file in "${new_files[@]}"; do
                new_paths+=("$migrations_dir/$file")
            done
            if apply_migration_files "$previous_url" "${new_paths[@]}"; then
                pass "new migrations apply to the previous release schema"
            else
                fail "new migrations apply to the previous release schema"
            fi
        else
            fail "previous release migrations apply to an empty database"
        fi
    fi
fi

if [[ "$failures" -ne 0 ]]; then
    printf '\n%d migration check(s) failed\n' "$failures" >&2
    exit 1
fi

printf '\nall migration checks passed\n'
