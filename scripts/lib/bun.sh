# Shared Bun resolution for scripts that need the pinned toolchain. Source
# this file; it must not be executed directly.
#
# The repository pins Bun 1.4.2 because frontend/ and e2e/ lockfiles use
# lockfile v2, which older releases cannot read. Resolution order:
#   1. BUN, when the caller set it explicitly;
#   2. a system Bun at least 1.4.0;
#   3. the pinned development binary used on hosts with an older system Bun;
#   4. any system Bun, so the caller can surface its own version error.

version_at_least() {
    [[ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n 1)" == "$2" ]]
}

resolve_bun() {
    if [[ -n "${BUN:-}" ]]; then
        printf '%s\n' "$BUN"
        return 0
    fi
    if command -v bun >/dev/null 2>&1 \
        && version_at_least "$(bun --version 2>/dev/null || echo 0)" "1.4.0"; then
        command -v bun
        return 0
    fi
    local_pinned="${HOME:-/root}/.local/bun-1.4.2/bun-linux-x64/bun"
    if [[ -x "$local_pinned" ]]; then
        printf '%s\n' "$local_pinned"
        return 0
    fi
    if command -v bun >/dev/null 2>&1; then
        command -v bun
        return 0
    fi
    return 1
}
