#!/usr/bin/env bash
#
# Dependency and image vulnerability scan (spec §10.8, §12.7).
#
#   - cargo audit   (Rust advisories, from Cargo.lock)
#   - bun audit     (frontend advisories, from bun.lock)
#   - Trivy         (OS and language packages in the built images)
#
# CRITICAL findings make the script exit non-zero and print the exact
# `gh issue create` command; the scheduled security workflow opens the issue
# automatically. PRs are not blocked by advisory-database churn because this
# script runs on a schedule, not as a required check (ADR-0008 §8.6).
#
# Usage:
#   ./scripts/security-scan.sh                       # all three scans
#   ./scripts/security-scan.sh --allow-missing       # skip absent tools
#   ./scripts/security-scan.sh --images a:tag,b:tag  # explicit image list
#   ./scripts/security-scan.sh --out docs/acceptance/security-scan.md
#
# The first Trivy run downloads the vulnerability database. The report records
# the database version so a stale scan is visible.

set -euo pipefail

# shellcheck source=lib/deploy-check.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib/deploy-check.sh"
cd "$root"

# shellcheck source=lib/bun.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib/bun.sh"

allow_missing=0
images=""
out="$root/docs/acceptance/security-scan.md"
trivy_image="${TRIVY_IMAGE:-aquasec/trivy:0.58.1}"
trivy_db_repository="${TRIVY_DB_REPOSITORY:-ghcr.io/aquasecurity/trivy-db}"
trivy_cache_volume="${TRIVY_CACHE_VOLUME:-rs-admin-trivy-cache}"
trivy_skip_db_update="${TRIVY_SKIP_DB_UPDATE:-0}"
trivy_ignore_file="${TRIVY_IGNORE_FILE:-$root/.trivyignore}"

while (($# > 0)); do
    case "$1" in
        --allow-missing) allow_missing=1; shift ;;
        --images)
            images="${2:?--images requires a comma-separated list}"
            shift 2
            ;;
        --out)
            out="${2:?--out requires a path}"
            shift 2
            ;;
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

missing_tool() {
    if ((allow_missing)); then
        echo "skip: $1 is not available (--allow-missing)" >&2
        return 0
    fi
    echo "error: $1 is required (install it or pass --allow-missing)" >&2
    return 1
}

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

critical=0
high=0
moderate=0
summary="$work/summary.tsv"
: >"$summary"
record() {
    printf '%s\t%s\t%s\n' "$1" "$2" "$3" >>"$summary"
}

# --- cargo audit ------------------------------------------------------------
if command -v cargo >/dev/null 2>&1 && cargo audit --version >/dev/null 2>&1; then
    echo "==> cargo audit"
    set +e
    cargo audit --json >"$work/cargo-audit.json" 2>"$work/cargo-audit.err"
    cargo_status=$?
    set -e
    if ! jq -e '.vulnerabilities' "$work/cargo-audit.json" >/dev/null 2>&1; then
        echo "error: cargo audit did not produce JSON; see $work/cargo-audit.err" >&2
        cat "$work/cargo-audit.err" >&2 || true
        exit 1
    fi
    cargo_found="$(jq -r '.vulnerabilities.count // 0' "$work/cargo-audit.json")"
    # RustSec advisories usually carry no severity string; use CVSS when the
    # advisory has one, exactly as the scheduled workflow does.
    cargo_critical="$(jq -r '[.vulnerabilities.list[]?
        | select(((.advisory.severity // "") | ascii_downcase) == "critical"
            or ((.advisory.cvss | if type == "object" then (.cvss3.base_score // 0) else 0 end) >= 9))] | length' "$work/cargo-audit.json")"
    cargo_high="$(jq -r '[.vulnerabilities.list[]?
        | select(((.advisory.severity // "") | ascii_downcase) == "high"
            or ((.advisory.cvss | if type == "object" then (.cvss3.base_score // 0) else 0 end) >= 7))] | length' "$work/cargo-audit.json")"
    critical=$((critical + cargo_critical))
    high=$((high + cargo_high))
    record "cargo audit" "$cargo_found vulnerabilities" "critical=$cargo_critical high=$cargo_high"
    echo "    $cargo_found advisories ($cargo_critical critical, $cargo_high high)"
    jq -r '.vulnerabilities.list[]? |
        "- `\(.advisory.id)` \(.package.name) \(.package.version) — \(.advisory.title) (severity: \(.advisory.severity // "unrated"))"' \
        "$work/cargo-audit.json" >"$work/cargo-advisories.md" 2>/dev/null || true
    if ((cargo_status != 0 && cargo_found == 0)); then
        echo "warning: cargo audit exited $cargo_status without a JSON vulnerability list" >&2
    fi
else
    missing_tool "cargo audit" || exit 1
    record "cargo audit" "skipped" "tool missing"
fi

# --- bun audit --------------------------------------------------------------
# This repository pins Bun 1.4.2 (bun.lock uses lockfile v2); lib/bun.sh
# resolves BUN, a new enough system Bun, or the pinned development binary.
bun_bin="$(resolve_bun || true)"
if [[ -n "$bun_bin" ]]; then
    echo "==> bun audit ($("$bun_bin" --version))"
    set +e
    (cd "$root/frontend" && "$bun_bin" audit --json >"$work/bun-audit.json" 2>"$work/bun-audit.err")
    bun_status=$?
    set -e
    if [[ -s "$work/bun-audit.json" ]] && jq -e 'type == "object"' "$work/bun-audit.json" >/dev/null 2>&1; then
        bun_total="$(jq -r '[to_entries[].value[]?] | length' "$work/bun-audit.json")"
        bun_critical="$(jq -r '[to_entries[].value[]? | select((.severity // "") | ascii_downcase == "critical")] | length' "$work/bun-audit.json")"
        bun_high="$(jq -r '[to_entries[].value[]? | select((.severity // "") | ascii_downcase == "high")] | length' "$work/bun-audit.json")"
        critical=$((critical + bun_critical))
        high=$((high + bun_high))
        record "bun audit" "$bun_total advisories" "critical=$bun_critical high=$bun_high"
        echo "    $bun_total advisories ($bun_critical critical, $bun_high high)"
        jq -r 'to_entries[] | .key as $package | .value[] |
            "- `\(.id)` \($package) — \(.title) (severity: \(.severity // "unrated"))"' \
            "$work/bun-audit.json" >"$work/bun-advisories.md" 2>/dev/null || true
    else
        echo "error: bun audit did not produce JSON; see $work/bun-audit.err" >&2
        cat "$work/bun-audit.err" >&2 || true
        exit 1
    fi
    if ((bun_status != 0)); then
        echo "warning: bun audit exited $bun_status" >&2
    fi
else
    missing_tool "bun 1.4.2" || exit 1
    record "bun audit" "skipped" "tool missing"
fi

# --- Trivy image scan -------------------------------------------------------
if [[ -z "$images" ]]; then
    images=""
    for candidate in rs-admin-api:local rs-admin-web:local rs-admin-db:local; do
        if docker image inspect "$candidate" >/dev/null 2>&1; then
            images+="${images:+,}$candidate"
        fi
    done
fi

if [[ -n "$images" ]]; then
    if ! command -v docker >/dev/null 2>&1; then
        missing_tool "docker (for Trivy)" || exit 1
    else
        echo "==> Trivy image scan ($images)"
        IFS=',' read -r -a image_list <<<"$images"
        for image in "${image_list[@]}"; do
            if ! docker image inspect "$image" >/dev/null 2>&1; then
                echo "warning: skipping missing image $image" >&2
                record "trivy $image" "skipped" "image missing"
                continue
            fi
            docker volume create "$trivy_cache_volume" >/dev/null
            trivy_args=(--skip-java-db-update)
            if [[ "$trivy_skip_db_update" == "1" ]]; then
                trivy_args+=(--skip-db-update)
            else
                trivy_args+=(--db-repository "$trivy_db_repository")
            fi
            ignore_run_args=()
            ignore_trivy_args=()
            if [[ -f "$trivy_ignore_file" ]]; then
                ignore_run_args=(--volume "$trivy_ignore_file:/tmp/.trivyignore:ro")
                ignore_trivy_args=(--ignorefile /tmp/.trivyignore)
            fi
            if ! docker run --rm \
                --volume /var/run/docker.sock:/var/run/docker.sock \
                --volume "$trivy_cache_volume:/root/.cache/trivy" \
                "${ignore_run_args[@]}" \
                "$trivy_image" image --quiet --format json --timeout 20m \
                "${trivy_args[@]}" "${ignore_trivy_args[@]}" \
                --severity CRITICAL,HIGH "$image" >"$work/trivy-${image//[:\/]/_}.json" 2>"$work/trivy.err"; then
                echo "error: Trivy failed for $image" >&2
                cat "$work/trivy.err" >&2 || true
                exit 1
            fi
            report="$work/trivy-${image//[:\/]/_}.json"
            image_critical="$(jq -r '[.Results[]?.Vulnerabilities[]? | select(.Severity == "CRITICAL")] | length' "$report")"
            image_high="$(jq -r '[.Results[]?.Vulnerabilities[]? | select(.Severity == "HIGH")] | length' "$report")"
            critical=$((critical + image_critical))
            high=$((high + image_high))
            record "trivy $image" "critical=$image_critical high=$image_high" "db=$(
                jq -r '.Metadata.DBVersion.UpdatedAt // "unknown"' "$report"
            )"
            echo "    $image: $image_critical critical, $image_high high"
            jq -r '.Results[]? | .Target as $target |
                .Vulnerabilities[]? | select(.Severity == "CRITICAL") |
                "- `\(.VulnerabilityID)` \(.PkgName) \(.InstalledVersion) in \($target) (fixed: \(.FixedVersion // "none"))"' \
                "$report" >>"$work/trivy-advisories.md" 2>/dev/null || true
        done
    fi
else
    echo "==> Trivy image scan skipped: no local rs-admin images and no --images"
    record "trivy images" "skipped" "no images"
fi

# --- Report -----------------------------------------------------------------
mkdir -p "$(dirname "$out")"
{
    echo "# Dependency and image scan"
    echo
    echo "Generated by \`scripts/security-scan.sh\` at $(date -u +%Y-%m-%dT%H:%M:%SZ) UTC."
    echo
    echo "| Scan | Result | Detail |"
    echo "|---|---|---|"
    while IFS=$'\t' read -r scan result detail; do
        echo "| $scan | $result | $detail |"
    done <"$summary"
    echo
    echo "Critical findings: **$critical**. High findings: **$high**."
    echo
    if ((critical > 0)); then
        echo "A CRITICAL advisory is a release blocker until fixed or risk-accepted by the"
        echo "security owner. Open a ticket:"
        echo
        echo '```sh'
        echo 'gh issue create --title "security: CRITICAL finding in the weekly scan" \'
        echo '  --body-file docs/acceptance/security-findings.md'
        echo '```'
        echo
        echo "See docs/acceptance/security-findings.md for the current disposition."
    else
        echo "No CRITICAL advisories were found. The scheduled workflow re-runs this scan"
        echo "weekly; advisory database changes can add findings without a code change."
    fi
    echo
    echo "## Advisories"
    echo
    if [[ -s "$work/cargo-advisories.md" ]]; then
        echo "### Rust (cargo audit)"
        echo
        cat "$work/cargo-advisories.md"
        echo
    fi
    if [[ -s "$work/bun-advisories.md" ]]; then
        echo "### Frontend (bun audit)"
        echo
        cat "$work/bun-advisories.md"
        echo
    fi
    if [[ -s "$work/trivy-advisories.md" ]]; then
        echo "### Images (Trivy, CRITICAL)"
        echo
        cat "$work/trivy-advisories.md"
        echo
    fi
} >"$out"
# Keep the generated report clean for git diff --check.
sed -i '${/^$/d;}' "$out"

echo "==> wrote $out"
if ((critical > 0)); then
    echo "error: $critical CRITICAL finding(s); open a ticket before deploying" >&2
    exit 1
fi
