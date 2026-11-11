#!/usr/bin/env bash
#
# Static checks for the M6 acceptance package (spec §14, AC-DOC-01–04).
#
#   - the spec defines exactly 62 acceptance IDs
#   - the matrix covers every ID exactly once and no unknown ID
#   - every matrix row is Pass or Risk accepted
#   - the handover documents contain no unresolved placeholders
#
# Usage: ./scripts/check-acceptance-docs.sh

set -euo pipefail

# shellcheck source=lib/deploy-check.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib/deploy-check.sh"
cd "$root"

spec="docs/specs/login-system.md"
matrix="docs/acceptance/matrix.md"

if [[ ! -f "$spec" ]]; then
    echo "error: missing $spec" >&2
    exit 1
fi
if [[ ! -f "$matrix" ]]; then
    echo "error: missing $matrix" >&2
    exit 1
fi

ids() {
    grep -oE 'AC-(AUTH|RBAC|API|FE|OPS|DOC)-[0-9]{2}' "$1" | sort -u
}

spec_ids="$(ids "$spec")"
matrix_ids="$(grep -E '^\| AC-' "$matrix" | grep -oE 'AC-(AUTH|RBAC|API|FE|OPS|DOC)-[0-9]{2}' | sort -u)"

spec_count="$(grep -c . <<<"$spec_ids")"
matrix_count="$(grep -c . <<<"$matrix_ids")"

if [[ "$spec_count" == "62" ]]; then
    pass "spec §14 contains 62 acceptance IDs"
else
    fail "spec §14 contains 62 acceptance IDs (found $spec_count)"
fi

if [[ "$matrix_count" == "62" ]]; then
    pass "matrix has 62 rows"
else
    fail "matrix has 62 rows (found $matrix_count)"
fi

missing="$(comm -23 <(printf '%s\n' "$spec_ids") <(printf '%s\n' "$matrix_ids") || true)"
extra="$(comm -13 <(printf '%s\n' "$spec_ids") <(printf '%s\n' "$matrix_ids") || true)"
if [[ -z "$missing" ]]; then
    pass "matrix covers every spec acceptance ID"
else
    fail "matrix is missing: $(tr '\n' ' ' <<<"$missing")"
fi
if [[ -z "$extra" ]]; then
    pass "matrix contains no unknown acceptance IDs"
else
    fail "matrix contains unknown IDs: $(tr '\n' ' ' <<<"$extra")"
fi

unresolved=0
while IFS= read -r row; do
    status="$(awk -F'|' '{ value = $(NF - 1); gsub(/^[ \t]+|[ \t]+$/, "", value); print value }' <<<"$row")"
    if [[ "$status" != Pass* && ! "${status,,}" =~ risk\ accepted ]]; then
        unresolved=$((unresolved + 1))
        echo "FAIL matrix row without a decided status: $row" >&2
    fi
done < <(grep -E '^\| AC-' "$matrix")
if ((unresolved == 0)); then
    pass "every matrix row is Pass or Risk accepted"
else
    fail "$unresolved matrix row(s) lack a decided status"
fi

placeholders="$(
    grep -RnE '\b(TBD|TODO|FIXME)\b|待定|待补' \
        README.md CONTRIBUTING.md deploy/README.md docs/adr docs/runbooks docs/acceptance \
        2>/dev/null || true
)"
if [[ -z "$placeholders" ]]; then
    pass "handover documents contain no unresolved placeholders"
else
    fail "handover documents contain unresolved placeholders:
$placeholders"
fi

if ((failures != 0)); then
    printf '\n%d acceptance-document check(s) failed\n' "$failures" >&2
    exit 1
fi
printf '\nall acceptance-document checks passed\n'
