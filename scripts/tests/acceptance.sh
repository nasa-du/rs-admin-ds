#!/usr/bin/env bash
#
# Behavioural tests for the M6 acceptance harnesses. They use stub binaries
# and saved result documents, so no Docker daemon, database or network is
# required.
#
#   ./scripts/tests/acceptance.sh

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

# --- Acceptance documents ---------------------------------------------------
echo "==> acceptance-document checks"
if "$root/scripts/check-acceptance-docs.sh" >"$work/docs.log" 2>&1; then
    pass "the acceptance matrix covers all 62 IDs with decided statuses"
else
    fail "the acceptance matrix covers all 62 IDs with decided statuses"
    cat "$work/docs.log" >&2
fi

# --- Argon2 benchmark wrapper ----------------------------------------------
repo="$work/repo"
mkdir -p "$repo/scripts/lib" "$repo/target/release"
cp "$root/scripts/bench-argon2.sh" "$repo/scripts/bench-argon2.sh"
cp "$root/scripts/lib/deploy-check.sh" "$repo/scripts/lib/deploy-check.sh"
chmod +x "$repo/scripts/bench-argon2.sh"

cat >"$repo/target/release/argon2-bench" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
case "${STUB_ARGON2_MODE:-target}" in
    target)
        cat <<'JSON'
{
  "environment": {"os": "linux", "arch": "x86_64", "logical_cpus": 2, "cpu_affinity": "0-1"},
  "iterations": 1,
  "target_p95_ms": 300.0,
  "hard_max_p95_ms": 1000.0,
  "candidates": [
    {"m_kib": 65536, "t": 3, "p": 4, "iterations": 1, "min_ms": 120.0,
     "p50_ms": 130.0, "p95_ms": 140.0, "max_ms": 150.0, "mean_ms": 132.0,
     "within_target": true, "within_hard_limit": true}
  ],
  "recommended": {"m_kib": 65536, "t": 3, "p": 4},
  "within_target": true,
  "within_hard_limit": true
}
JSON
        exit 0
        ;;
    fallback)
        cat <<'JSON'
{
  "environment": {"os": "linux", "arch": "x86_64", "logical_cpus": 2, "cpu_affinity": "0-1"},
  "iterations": 1,
  "target_p95_ms": 300.0,
  "hard_max_p95_ms": 1000.0,
  "candidates": [
    {"m_kib": 65536, "t": 3, "p": 1, "iterations": 1, "min_ms": 280.0,
     "p50_ms": 300.0, "p95_ms": 320.0, "max_ms": 330.0, "mean_ms": 300.0,
     "within_target": false, "within_hard_limit": true}
  ],
  "recommended": null,
  "within_target": false,
  "within_hard_limit": true
}
JSON
        exit 3
        ;;
    unusable)
        cat <<'JSON'
{
  "environment": {"os": "linux", "arch": "x86_64", "logical_cpus": 1, "cpu_affinity": "0"},
  "iterations": 1,
  "target_p95_ms": 300.0,
  "hard_max_p95_ms": 1000.0,
  "candidates": [
    {"m_kib": 19456, "t": 2, "p": 1, "iterations": 1, "min_ms": 1500.0,
     "p50_ms": 1600.0, "p95_ms": 1700.0, "max_ms": 1800.0, "mean_ms": 1600.0,
     "within_target": false, "within_hard_limit": false}
  ],
  "recommended": null,
  "within_target": false,
  "within_hard_limit": false
}
JSON
        exit 4
        ;;
esac
STUB
chmod +x "$repo/target/release/argon2-bench"

run_bench() {
    local mode="$1" expected="$2"
    set +e
    STUB_ARGON2_MODE="$mode" "$repo/scripts/bench-argon2.sh" \
        --no-build --json >"$work/bench-$mode.json" 2>"$work/bench-$mode.err"
    local status=$?
    set -e
    if [[ "$status" == "$expected" ]] && jq -e '.candidates | length > 0' \
        "$work/bench-$mode.json" >/dev/null 2>&1; then
        pass "bench wrapper propagates the $mode exit status ($status)"
    else
        fail "bench wrapper propagates the $mode exit status (got $status, expected $expected)"
        cat "$work/bench-$mode.err" >&2
    fi
}

echo "==> Argon2 benchmark wrapper"
run_bench target 0
run_bench fallback 3
run_bench unusable 4

# --- Performance gate -------------------------------------------------------
mkdir -p "$repo/scripts/lib"
cp "$root/scripts/perf.sh" "$repo/scripts/perf.sh"
cp "$root/scripts/lib/acceptance.sh" "$repo/scripts/lib/acceptance.sh"
cp "$root/scripts/lib/bun.sh" "$repo/scripts/lib/bun.sh"
chmod +x "$repo/scripts/perf.sh"

cat >"$work/perf-ok.json" <<'JSON'
{
  "ok": true,
  "thresholds": {"login_p95_ms": 1000, "api_p95_ms": 300, "refresh_p95_ms": 200},
  "concurrency": {"login": 20, "api": 50, "refresh": 20},
  "setup": {"users": 50, "duration_ms": 1000},
  "login": {"samples": 80, "p50_ms": 300, "p95_ms": 700, "max_ms": 900, "mean_ms": 350, "errors": 0},
  "api": {"samples": 500, "p50_ms": 40, "p95_ms": 120, "max_ms": 250, "mean_ms": 50, "errors": 0},
  "refresh": {"samples": 60, "p50_ms": 8, "p95_ms": 20, "max_ms": 30, "mean_ms": 9, "errors": 0, "cookie_mismatch_batches": 0, "reuse_delta": 0},
  "pool_last": {"active": 2, "idle": 8, "max": 10},
  "duration_ms": 10000
}
JSON
cat >"$work/perf-failed.json" <<'JSON'
{
  "ok": false,
  "thresholds": {"login_p95_ms": 1000, "api_p95_ms": 300, "refresh_p95_ms": 200},
  "concurrency": {"login": 20, "api": 50, "refresh": 20},
  "setup": {"users": 50, "duration_ms": 1000},
  "login": {"samples": 80, "p50_ms": 900, "p95_ms": 1500, "max_ms": 2000, "mean_ms": 1000, "errors": 0},
  "api": {"samples": 500, "p50_ms": 40, "p95_ms": 120, "max_ms": 250, "mean_ms": 50, "errors": 0},
  "refresh": {"samples": 60, "p50_ms": 8, "p95_ms": 20, "max_ms": 30, "mean_ms": 9, "errors": 0, "cookie_mismatch_batches": 0, "reuse_delta": 0},
  "pool_last": {"active": 2, "idle": 8, "max": 10},
  "duration_ms": 10000
}
JSON

run_perf() {
    local label="$1" file="$2" peak="$3" expected="$4"
    set +e
    PERF_POOL_PEAK="$peak" PERF_POOL_SAMPLES=10 \
        "$repo/scripts/perf.sh" --evaluate "$file" >"$work/perf-$label.md" 2>&1
    local status=$?
    set -e
    if [[ "$status" == "$expected" ]]; then
        pass "perf gate $label (exit $status)"
    else
        fail "perf gate $label (got $status, expected $expected)"
        cat "$work/perf-$label.md" >&2
    fi
}

echo "==> performance gate"
run_perf healthy "$work/perf-ok.json" 5 0
run_perf saturated "$work/perf-ok.json" 9 1
run_perf slow "$work/perf-failed.json" 1 1

if grep -q 'Database pool peak: 5/10' "$work/perf-healthy.md" \
    && grep -q 'Pass' "$work/perf-healthy.md"; then
    pass "perf report renders the measured pool peak and verdict"
else
    fail "perf report renders the measured pool peak and verdict"
    cat "$work/perf-healthy.md" >&2
fi

if ((failures != 0)); then
    printf '\n%d acceptance harness check(s) failed\n' "$failures" >&2
    exit 1
fi
printf '\nall acceptance harness checks passed\n'
