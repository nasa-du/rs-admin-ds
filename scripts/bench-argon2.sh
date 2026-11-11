#!/usr/bin/env bash
#
# Calibrate Argon2id on this host (spec §8.1, §12.6).
#
# Builds the release `argon2-bench` binary, pins it to the CPU budget that the
# API container receives, measures the policy parameter set and — only when its
# p95 exceeds the 300 ms target — the documented fallback sequence. The exit
# status mirrors the binary:
#
#   0  the policy set meets the 100–300 ms target
#   3  a fallback is required; record it and update the constants before deploy
#   4  every candidate exceeds 1 s; the host/container CPU budget is too small
#
# Usage:
#   ./scripts/bench-argon2.sh                       # 2-CPU affinity budget
#   ./scripts/bench-argon2.sh --cpus 0-3            # 4-vCPU host baseline
#   ./scripts/bench-argon2.sh --container-cpus 2    # Docker cgroup quota (API-like)
#   ./scripts/bench-argon2.sh --out docs/acceptance/argon2.md
#   ./scripts/bench-argon2.sh --json                # machine-readable output
#
# The production host must be measured with the same CPU budget as the API
# container (`deploy.compose.yaml` limits `api` to 2 CPUs). `--container-cpus`
# runs the release binary in a `debian:bookworm-slim` container with a Docker
# CPU quota, which matches Compose's cgroup limit more closely than taskset.
# Re-run this script on the deployment host and archive the report; a
# measurement on a different host is only a methodology baseline.

set -euo pipefail

# shellcheck source=lib/deploy-check.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib/deploy-check.sh"
cd "$root"

iterations=30
cpus="${BENCH_CPUS:-0-1}"
container_cpus=""
build=1
json_output=0
out=""
candidate_args=()

while (($# > 0)); do
    case "$1" in
        --iterations)
            iterations="${2:?--iterations requires a value}"
            shift 2
            ;;
        --cpus)
            cpus="${2:?--cpus requires a taskset CPU list}"
            shift 2
            ;;
        --container-cpus)
            container_cpus="${2:?--container-cpus requires a CPU count}"
            shift 2
            ;;
        --candidate)
            candidate_args+=(--candidate "${2:?--candidate requires m,t,p}")
            shift 2
            ;;
        --out)
            out="${2:?--out requires a path}"
            shift 2
            ;;
        --no-build) build=0; shift ;;
        --json) json_output=1; shift ;;
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

if ! [[ "$iterations" =~ ^[1-9][0-9]*$ ]]; then
    echo "error: --iterations must be a positive integer" >&2
    exit 2
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo is required" >&2
    exit 1
fi
if ! command -v jq >/dev/null 2>&1; then
    echo "error: jq is required" >&2
    exit 1
fi

if ((build)); then
    echo "==> building the release benchmark binary" >&2
    cargo build --quiet --release -p rs-admin-security --bin argon2-bench
fi

binary="$root/target/release/argon2-bench"
if [[ ! -x "$binary" ]]; then
    echo "error: missing $binary; rerun without --no-build" >&2
    exit 1
fi

runner=()
pin_note=""
if [[ -n "$container_cpus" ]]; then
    if ! command -v docker >/dev/null 2>&1; then
        echo "error: docker is required for --container-cpus" >&2
        exit 1
    fi
    bench_image="${BENCH_IMAGE:-debian:bookworm-slim}"
    if ! docker run --rm --cpus "$container_cpus" \
        --volume "$binary:/bench:ro" "$bench_image" /bench --help \
        >/dev/null 2>&1; then
        echo "error: cannot run the benchmark in $bench_image" >&2
        exit 1
    fi
    runner=(docker run --rm --cpus "$container_cpus" \
        --volume "$binary:/bench:ro" "$bench_image" /bench)
    pin_note="container --cpus $container_cpus"
elif command -v taskset >/dev/null 2>&1; then
    # Fail early when the requested CPU list is not usable on this host.
    if taskset -c "$cpus" true 2>/dev/null; then
        runner=(taskset -c "$cpus" "$binary")
        pin_note="$cpus"
    else
        echo "warning: CPU list '$cpus' is not available; measuring unpinned" >&2
        runner=("$binary")
        pin_note="unpinned (requested $cpus)"
    fi
else
    echo "warning: taskset is not available; measuring unpinned" >&2
    runner=("$binary")
    pin_note="unpinned (taskset missing)"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
json_file="$tmp/report.json"

echo "==> measuring Argon2id (CPU budget ${pin_note:-all}, $iterations iterations)" >&2
set +e
"${runner[@]}" --json --iterations "$iterations" \
    ${candidate_args[@]+"${candidate_args[@]}"} >"$json_file"
status=$?
set -e

if ! jq -e '.candidates | length > 0' "$json_file" >/dev/null 2>&1; then
    echo "error: the benchmark did not produce a report" >&2
    cat "$json_file" >&2 || true
    exit 1
fi

if ((json_output)); then
    cat "$json_file"
    exit "$status"
fi

render_markdown() {
    jq -r --arg cpus "${pin_note:-all}" --arg status "$status" '
        def verdict:
            if .within_target then "ok"
            elif .within_hard_limit then "above target"
            else "unusable" end;
        "# Argon2id calibration",
        "",
        "Generated by `scripts/bench-argon2.sh`.",
        "",
        "| Field | Value |",
        "|---|---|",
        "| Host OS / arch | \(.environment.os) / \(.environment.arch) |",
        "| Visible logical CPUs | \(.environment.logical_cpus) |",
        "| CPU budget | \($cpus) |",
        "| Effective affinity | \(.environment.cpu_affinity) |",
        "| Iterations per candidate | \(.iterations) |",
        "| Target | p95 ≤ \(.target_p95_ms) ms |",
        "| Hard limit | p95 ≤ \(.hard_max_p95_ms) ms |",
        "",
        "| m (KiB) | t | p | min ms | p50 ms | p95 ms | max ms | mean ms | Verdict |",
        "|---|---|---|---|---|---|---|---|---|",
        (.candidates[] |
            "| \(.m_kib) | \(.t) | \(.p) | \(.min_ms) | \(.p50_ms) | \(.p95_ms) | \(.max_ms) | \(.mean_ms) | \(verdict) |"),
        "",
        (if .recommended then
            "**Recommended parameters:** `m=\(.recommended.m_kib) KiB, t=\(.recommended.t), p=\(.recommended.p)`."
        elif .within_hard_limit then
            "**No candidate reached 300 ms.** The OWASP floor is under 1 s; record a risk acceptance before deploying."
        else
            "**No candidate is under 1 s.** Resize the host or container CPU budget before deploying."
        end),
        "",
        "Apply a fallback by updating `ARGON2_MEMORY_KIB`, `ARGON2_TIME_COST` and",
        "`ARGON2_PARALLELISM` in `backend/crates/security/src/password.rs`, then",
        "re-run `scripts/bench-argon2.sh` and the full test suite.",
        "",
        "> This report is only valid for the measured CPU budget and host. Re-run",
        "> it on the production host before the first deployment and after any",
        "> host or container CPU-limit change."
    ' "$json_file"
}

if [[ -n "$out" ]]; then
    mkdir -p "$(dirname "$out")"
    render_markdown >"$out"
    echo "==> wrote $out" >&2
else
    render_markdown
fi

exit "$status"
