# Shared helpers for the deployment check scripts. Source this file from
# scripts/check-deploy*.sh; it must not be executed directly.

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
failures=0

pass() {
    printf 'ok   %s\n' "$1"
}

fail() {
    printf 'FAIL %s\n' "$1"
    failures=$((failures + 1))
}
