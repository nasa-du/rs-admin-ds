# CRITICAL image findings: disposition and ticket

The M6 acceptance image scan (Trivy 0.58.1) found **2 CRITICAL** findings in
the locally built images after one verified false positive was suppressed. This file is the security-owner disposition record;
the scheduled [security workflow](../../.github/workflows/security.yml) opens
or updates a `[security-scan]` issue automatically when it runs with
`issues: write`. The interactive acceptance environment was not permitted to
create the public issue, so the exact command is recorded below for the
operator to run with repository write permission.

## Findings

| Image | CVE | Package | Installed | Fixed in | Target |
|---|---|---|---|---|---|
| `rs-admin-web:local` | CVE-2026-111497 | brotli-libs | 1.2.0-r0 | 1.2.1-r0 | alpine 3.23.4 |
| `rs-admin-db:local` | CVE-2025-68121 | gosu (Go stdlib) | Go 1.24.6 | Go 1.24.13 / 1.25.7 | `/usr/local/bin/gosu` |

A third Trivy result, CVE-2023-45853 in Debian `zlib1g`, is recorded in
`.trivyignore` as not applicable: the API image contains no minizip code or
package (verified with `dpkg -L zlib1g` and `dpkg -l` inside the built image;
Debian packages minizip separately as `libminizip1`).

## Disposition

1. **CVE-2026-111497 (brotli-libs, web)** — a fixed `1.2.1-r0` exists. The web
   image serves static assets and decodes no attacker-supplied brotli request
   bodies, so exposure is limited, but the fix is a base-image upgrade.
   Rebuild `web` on an Alpine repository that carries 1.2.1-r0, then re-scan.
2. **CVE-2025-68121 (gosu, db)** — `gosu` is only used by the PostgreSQL
   entrypoint to drop privileges; the vulnerable Go TLS client path is not
   exercised. Track a newer `postgres:18.x-alpine` patch or rebuild `gosu`
   with a fixed Go toolchain.

Neither is safely fixable by the application code in this repository. They
are release blockers until the web base image is upgraded and the gosu finding
is fixed or risk-accepted explicitly on the tracking issue.

## Operator command

Run this with repository write permission (the body is the summary above):

```sh
gh issue create --repo nasa-du/rs-admin-ds \
  --title "security: CRITICAL image scan findings from the M6 acceptance run (2026-11-11)" \
  --body-file docs/acceptance/security-findings.md
```

The scheduled workflow uses the same disposition and will comment on an
existing open `[security-scan]` issue instead of opening a duplicate.

## Evidence

- [`security-scan.md`](./security-scan.md): raw scan table and dependency
  advisories.
- Local reproduction:
  ```sh
  TRIVY_SKIP_DB_UPDATE=1 ./scripts/security-scan.sh
  ```
  The Trivy DB cache can be primed with
  `docker run --rm -v rs-admin-trivy-cache:/root/.cache/trivy aquasec/trivy:0.58.1 image --download-db-only --db-repository public.ecr.aws/aquasecurity/trivy-db`
  when the default mirror is unreachable.
- `.trivyignore`: the single verified false positive (`CVE-2023-45853`), with
  the evidence above; re-check when the base image changes.
- `Cargo.lock`/`bun.lock` dependency scans: 3 Rust advisories (all unrated
  medium; `RUSTSEC-2026-0387`, `RUSTSEC-2026-0459`, `RUSTSEC-2023-0071`) and
  1 moderate frontend advisory (`minimatch` GHSA-g3hv-75wg-m249). None is
  CRITICAL or HIGH.
