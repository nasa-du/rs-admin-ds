# CI/CD, release, rollback and migrations

This runbook covers the delivery path required by ADR-0008 §8-§9 and
AC-OPS-03/04:

```
pull request -> CI gates -> merge to main -> GHCR images (sha-<short>)
                                                   |
                          manual workflow_dispatch + production approval
                                                   v
                       host checkout -> pull -> migrate -> up -> ready
                                                   |
                                   failure -> previous tag -> up -> ready
```

The deployment host is the single-host Compose stack described in
[deploy/README.md](../../deploy/README.md). CI never contacts production
secrets or the deployment host.

## 1. Required checks (main branch protection)

Configure `main` branch protection with these required checks:

| Check | Workflow |
|---|---|
| `backend / fmt, clippy, build, test` | `.github/workflows/backend.yml` |
| `frontend / lint, typecheck, test, build` | `.github/workflows/frontend.yml` |

`publish-images` runs only after a push to `main` or a `v*` tag; it is not a
pull-request check. The environment approval for `production` is configured
under **Settings → Environments → production → Required reviewers**.

The backend check runs `cargo fmt`, clippy, build, `cargo sqlx prepare --check`,
OpenAPI generation plus `git diff --exit-code`, the forward-only migration
checks, the full test suite on the `postgres:18.6-alpine` service, the Compose
deployment checks, `docker compose config -q`, an API image build and the
database/pgBackRest image build.

The frontend check runs orval generation plus `git diff --exit-code`, lint,
format, typecheck, the i18n key check, browser-mode tests, the production build
and a web image build.

Both workflows run after any change to their contract artifacts, so OpenAPI or
generated-client drift fails the pull request. The backend check also validates
the acceptance package (`scripts/check-acceptance-docs.sh`) and the harness
behaviour tests (`scripts/tests/acceptance.sh`).

### Scheduled security scan

`.github/workflows/security.yml` runs weekly (Mondays 04:17 UTC) and on
demand. It runs `cargo audit`, `bun audit` and Trivy scans of the API, web and
database images. A CRITICAL finding opens or updates one `[security-scan]`
issue and the release is frozen until it is fixed or risk-accepted by the
security owner. HIGH/medium advisories are recorded in the run summary and
artifacts but do not block PRs; advisory-database churn must not fail a pull
request. Reproduce locally with `./scripts/security-scan.sh`.

### Full-stack E2E (pre-release)

The manual **Actions → e2e → Run workflow** job runs the nine journeys from
spec §12.5 on an ephemeral runner: `scripts/e2e.sh` builds the production
Compose stack, bootstraps a fresh super administrator, drives Chromium through
the gateway (real HTTPS edge, CORS, CSRF and `__Host-refresh` cookie) and
removes the stack. It is deliberately not a pull-request gate and never
contacts the deployment host. Run it before tagging a release, and attach the
workflow run (or the local Playwright report) to the release ticket. See
[e2e/README.md](../../e2e/README.md) for local options and the journey list.

## 2. Images and tags

`.github/workflows/publish-images.yml` publishes:

```
ghcr.io/nasa-du/rs-admin-ds/api:sha-<short>    # every main push
ghcr.io/nasa-du/rs-admin-ds/web:sha-<short>
ghcr.io/nasa-du/rs-admin-ds/db:sha-<short>     # PostgreSQL 18.6 + pgBackRest
ghcr.io/nasa-du/rs-admin-ds/api:vX.Y.Z         # v* tag pushes
ghcr.io/nasa-du/rs-admin-ds/web:vX.Y.Z
ghcr.io/nasa-du/rs-admin-ds/db:vX.Y.Z
```

`latest` is refused by both the workflow and `scripts/release.sh`. Production
deploys an explicit `sha-<short>` or `vX.Y.Z` tag; the workflow summary of a
publish run lists the tags it produced. Only tag a commit that already passed
the required checks on `main`; the release workflow refuses commits that are
not ancestors of `main`.

Set the repository variable `VITE_API_BASE_URL` to the production API origin
(origin only, no `/api/v1`) before publishing. The fallback
`https://api.example.com` is a placeholder and must not be used for a real
deployment. The web image embeds this value at build time.

Optional repository variables:

| Variable | Default | Purpose |
|---|---|---|
| `VITE_API_BASE_URL` | `https://api.example.com` | API origin baked into the SPA |
| `DEPLOY_PATH` | required | Repository checkout path on the deployment host |
| `RELEASE_STATE_DIR` | `/var/lib/rs-admin/releases` | Deployed state, lock and release records |
| `RELEASE_COMPOSE_PROFILES` | `local-db` | Compose profiles; `none` for an external database |
| `RELEASE_IMAGE_PREFIX` | `ghcr.io/nasa-du/rs-admin-ds` | Image prefix |
| `RELEASE_CA_CERT` | empty | CA bundle for the HTTPS smoke checks |

Required production environment secrets:

| Secret | Purpose |
|---|---|
| `DEPLOY_HOST` | Deployment host name or IP |
| `DEPLOY_USER` | SSH user allowed to run Docker Compose |
| `DEPLOY_SSH_KEY` | Private key for that user |
| `DEPLOY_KNOWN_HOSTS` | Pinned host key lines for `known_hosts` |

The host needs a read-only GHCR token (`docker login ghcr.io`) and a full
checkout of this repository at `DEPLOY_PATH`. It also needs `docker`, Docker
Compose v2, `git`, `curl` and a writable `RELEASE_STATE_DIR`. The release
workflow fetches and force-checks-out the release commit on the host; local
changes to tracked files are discarded, while the git-ignored `deploy/.env`
and secret mounts are preserved.

## 3. Release and dry-run

Start **Actions → release → Run workflow** with:

- `image_tag`: `sha-<short>` from a successful publish, or a `vX.Y.Z` tag.
- `dry_run`: leave enabled to rehearse.
- `allow_destructive_migrations`: only for a contract-phase release, after an
  on-demand backup. Without it, new migrations containing `DROP`, `TRUNCATE` or
  `ALTER ... TYPE` are refused.

The `resolve` job rejects `latest`, unknown tags, and commits that are not on
`main`. The `release` job then waits for the `production` environment approval.

### Dry-run

`dry_run` performs the full path — image pull, one-shot `migrate`, `up` of the
long-running services, 120-second `/health/ready` readiness, smoke checks — and
then restores the previous image tags, raises the stack again, waits for
readiness, and writes a record with `"mode": "dry-run"`. The deployed state
file is not changed.

The first release cannot be rehearsed this way because there is no previous
tag to return to; run the dry-run after the first successful release.

### Release

With `dry_run` disabled, the same steps run and on success the script writes
`deploy/.env` (`API_IMAGE`, `WEB_IMAGE`, `DB_IMAGE`, `GIT_COMMIT`) plus
`$RELEASE_STATE_DIR/deployed.env`. The JSON record is uploaded as the
`release-record` workflow artifact and appended to the run summary.

The script can also be run directly on the host:

```sh
./scripts/release.sh --tag sha-abc1234 --dry-run
./scripts/release.sh --tag v1.0.0 --operator ops
```

Direct runs bypass the GitHub approval gate; use them only for break-glass
recovery, and keep the record.

### Smoke checks

The release script checks, through the public edge:

1. unauthenticated `GET /api/v1/auth/me` returns `401`,
   `application/problem+json` and a problem `code`;
2. `OPTIONS /api/v1/auth/me` with the admin origin returns `204` and the exact
   `Access-Control-Allow-Origin`;
3. `GET /` on the admin origin returns `200`.

It never creates users or performs a real login. Pass `--skip-smoke` only when
the edge is intentionally unreachable from the host, and record why in the
release ticket.

## 4. Failure and rollback

| Failure | Behaviour |
|---|---|
| pull | Release aborts; the running stack and `.env` images are restored |
| migrate | Release aborts; the running stack is left untouched |
| `up`, readiness or smoke | Previous image tags are restored, the previous stack is raised (database first, then the long-running services), readiness is re-checked, and the record captures the rollback |
| rollback | If the previous release cannot be raised, the record reports `rollback.ready: false`; involve the on-call operator and use the backup/restore runbook |

Migrations are never rolled back. A failed release keeps the new schema; the
rollback only moves the code back. This is safe because every migration must
follow expand/contract, and `/health/ready` treats a newer schema as ready
(ADR-0008 2026-11-10 amendment). The rollback starts the long-running services
with `up -d --no-deps` so the previous image's migrator is never asked to
validate migrations it does not know about.

The contract phase (dropping a column/table/index) is only safe while the
previous release is not a rollback target. Ship it in a release whose
predecessor is already retired, or accept that rolling that release back
requires restoring a backup.

The state file is the source of truth for rollback:

```sh
cat /var/lib/rs-admin/releases/deployed.env
```

To roll back explicitly after a successful release, run the release script with
the previous tag (a normal release attempt). The script records the new
attempt; if it fails, it attempts to return to the tag it found at start.

## 5. Migration policy

Migrations are embedded by `sqlx::migrate!` and are append-only:

- Files use a UTC timestamp prefix (`YYYYMMDDHHMMSS_name.sql`) and a unique
  version.
- A migration that exists in the base ref may never be modified, renamed or
  deleted. SQLx also records a checksum in `_sqlx_migrations`.
- New migrations must sort after every existing migration.
- `scripts/check-migrations.sh` applies all migrations to an empty database
  and applies the base-ref migrations followed by the new migrations to a
  second database. CI runs it against the PG18 service on every change.
- Destructive DDL in a new migration is refused by the release flow unless the
  operator passes `allow_destructive_migrations` / `--allow-destructive`; CI
  still reports it as a warning. The contract phase (dropping a
  column/table/index) belongs to a later release, after every running version
  stops using it, and requires an on-demand backup first.

Expand/contract procedure:

1. **Expand**: add nullable columns, new tables or new indexes; code tolerates
   both the old and new shape.
2. **Migrate**: backfill and switch code in a later release.
3. **Contract**: drop the old object only after no supported code version reads
   it; take an on-demand backup first and record the migration in the release
   ticket. A rollback to the immediately previous tag must still be safe; if it
   is not, the release note must say that a failed release requires a backup
   restore.

The release script re-checks migration immutability against the previous
deployed commit before pulling images. If the host checkout is too shallow to
contain that commit, it warns and skips the local check; CI remains the gate.

## 6. Release records

Every run writes a JSON record under `$RELEASE_STATE_DIR`:

```json
{
  "mode": "release",
  "result": "success",
  "failure_stage": null,
  "requested_tag": "sha-abc1234",
  "api_image": "ghcr.io/nasa-du/rs-admin-ds/api:sha-abc1234",
  "web_image": "ghcr.io/nasa-du/rs-admin-ds/web:sha-abc1234",
  "db_image": "ghcr.io/nasa-du/rs-admin-ds/db:sha-abc1234",
  "previous_tag": "sha-def5678",
  "commit": "…",
  "ready_seconds": 11,
  "smoke": "passed",
  "rollback": {"performed": false, "ready": null, "tag": "sha-def5678", "seconds": null},
  "destructive_migrations_acknowledged": false,
  "migrations": ["20261110000000_add_example.sql"]
}
```

The record contains the operator, timestamps, image references, failure stage,
rollback outcome and the migration files between the previous and released
commits. Keep the last 10 release records and image tags for rollback
(ADR-0008 §9.6). The GitHub Actions run, its `release-record` artifact and the
step summary are the deployment log: link the run from the release ticket.

## 7. Verification

After every release or rollback:

```sh
# 1. The record exists and says what the workflow summary says.
latest="$(find "${RELEASE_STATE_DIR:-/var/lib/rs-admin/releases}" \
  -maxdepth 1 -name '*.json' | sort | tail -n 1)"
jq '{mode, result, failure_stage, requested_tag, previous_tag, ready_seconds,
     smoke, rollback}' "$latest"

# 2. The deployed state points at the intended tag and images.
cat "${RELEASE_STATE_DIR:-/var/lib/rs-admin/releases}/deployed.env"

# 3. Readiness and the public edge.
docker compose -f deploy/compose.yaml exec api \
  curl -fsS http://127.0.0.1:8080/health/ready
curl --cacert <CA> https://admin.example.com/                  # 200
curl --cacert <CA> https://api.example.com/api/v1/auth/me       # 401 problem+json

# 4. The database is at the migration version the release expects.
docker compose exec db psql -U rs_admin -d rs_admin -tAc \
  "SELECT MAX(version), COUNT(*) FILTER (WHERE NOT success) FROM _sqlx_migrations"
```

For a rollback, additionally confirm `deployed.env` still names the previous
tag, the API image in `deploy/.env` matches it, and `/health/ready` is `200`.
`./scripts/tests/release.sh` is the offline regression seam for the release,
dry-run and rollback state machine; it must pass in CI.
