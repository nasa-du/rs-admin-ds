# Single-host deployment

Docker Compose stack for one Linux host:

```
gateway (nginx, TLS, host routing, rate limit)   host :80/:443
  ├── web   (static SPA, internal :8080)
  └── api   (Axum, internal :8080, metrics :9100)
        └── db (PostgreSQL 18.6 + pgBackRest, internal :5432, local-db)

one-shot tools profile (systemd timers):
  maintenance (retention cleanup)     backup (daily full + WAL archive check)
```

Only `gateway` publishes host ports. `api`, `web` and `db` are reachable only
over the fixed Compose networks (`edge` 172.28.0.0/16, internal `data`
172.29.0.0/16). The API trusts forwarding headers only from the `edge` range.
`db` also joins the outbound `storage` network (172.30.0.0/16) so pgBackRest
can archive WAL to the S3-compatible repository; only `db` and the one-shot
`backup` job join it.

## Prerequisites

- Docker Engine 27+ with Compose v2.
- An SPA/API host pair in the same registrable domain (defaults:
  `admin.example.com` and `api.example.com`).
- TLS certificate and key for both hosts. The gateway terminates TLS; certificate
  issuance/renewal is outside this ticket.
- Key material in a secrets directory (see
  [secrets/README.md](./secrets/README.md)).

## Configuration

The stack reads non-secret settings from `deploy/.env`:

```sh
cp deploy/.env.example deploy/.env
```

Compose resolves `RS_ADMIN_SECRETS_DIR` relative to this directory. For a local
trial leave it at `./secrets-dev`; for production use
`/etc/rs-admin/secrets`.

`VITE_API_BASE_URL` is a **build argument** for the web image and the CSP
`connect-src` origin at runtime. Rebuild `web` when it changes. It must be an
origin only (`https://api.example.com`), with no `/api/v1` suffix.

## Local full stack

Generate development secrets and TLS material (valid for
`admin.example.com`, `api.example.com`, `db`, `localhost` and `127.0.0.1`):

```sh
./scripts/generate-dev-secrets.sh
cp deploy/.env.example deploy/.env
```

Point the two host names at the machine (for a browser trial):

```sh
sudo sh -c 'printf "127.0.0.1 admin.example.com api.example.com\n" >> /etc/hosts'
```

Start the stack:

```sh
cd deploy
docker compose --profile local-db up -d
```

The `local-db` profile also enables the one-shot `migrate` job. Compose runs it
to completion before starting `api`, so the same acceptance command brings up a
ready stack:

```sh
docker compose --profile local-db up
```

To run migrations explicitly (useful before a release, or with an external
database), use:

```sh
docker compose --profile local-db --profile tools run --rm migrate
```

The API never applies migrations on startup. Its `/health/ready` probe returns
503 until the migration version matches the embedded migrator.

Check the edge:

```sh
curl --cacert secrets-dev/internal_ca.pem https://api.example.com/health/ready   # 404: not public
curl --cacert secrets-dev/internal_ca.pem https://admin.example.com/              # 200: SPA
docker compose ps
```

Only the gateway should show published ports.

## Host-run backend against the Compose database

Publish PostgreSQL on loopback only through the development override (from the
repository root):

```sh
docker compose -f deploy/compose.yaml -f deploy/compose.dev.yaml \
  --profile local-db up -d db
```

`deploy/secrets-dev/host.env` contains the matching TLS host DSN. Merge it into
the repository `.env`, then run `cargo run -p rs-admin-api`. The database
requires TLS for every TCP connection, including loopback.

## Production deployment

Production releases run through the `release` GitHub Actions workflow. It
requires approval on the `production` environment, pulls immutable GHCR tags,
runs the one-shot migration, starts the stack, waits up to 120 seconds for
`/health/ready`, runs the edge smoke checks, and rolls back to the previous
tag on failure. The full procedure, required secrets/variables and the
dry-run flow are documented in
[docs/runbooks/ci-cd.md](../docs/runbooks/ci-cd.md).

The same flow is available on the deployment host:

```sh
# Rehearse: deploy, verify, then return to the previous release.
./scripts/release.sh --tag sha-abc1234 --dry-run

# Release an immutable tag produced by the publish workflow.
./scripts/release.sh --tag v1.0.0 --operator ops
```

Before the first release:

1. Populate `/etc/rs-admin/secrets` with the files and modes listed in
   [secrets/README.md](./secrets/README.md).
2. Set `RS_ADMIN_SECRETS_DIR=/etc/rs-admin/secrets`, `APP_ENV=production`,
   the real `ADMIN_HOST`/`API_HOST`, origins, TLS certificate, image tags and
   `GIT_COMMIT` in `deploy/.env`.
3. Log the host in to GHCR with a read-only token (`docker login ghcr.io`).
4. Check out the release commit and run the release workflow (or
   `scripts/release.sh`). The script pulls the images, runs
   `docker compose --profile local-db --profile tools run --rm migrate`,
   starts the database and long-running services with `up -d --no-deps`, and
   confirms readiness. A non-zero migration exit aborts before the new
   containers start; the one-shot migration is not executed a second time.

### External database

Remove the `local-db` profile (do not pass `--profile local-db`) and point
`database_url` / `database_url_migrator` at the external PostgreSQL instance.
Run `migrate` with only the `tools` profile:

```sh
docker compose --profile tools run --rm migrate
docker compose up -d --remove-orphans
```

With `scripts/release.sh` (and the release workflow), set
`RELEASE_COMPOSE_PROFILES=none`; the script still adds the `tools` profile for
the one-shot migration and keeps the 120-second readiness/rollback behaviour.

The DSN must keep `sslmode=verify-full` and a reachable `sslrootcert`. The
external operator owns the four role definitions and their grants; run
`migrate` as `rs_admin_migrator` and ensure `deploy/db/grants.sql` was applied.

## Database roles

| Role | Purpose | Privileges |
|---|---|---|
| `rs_admin_migrator` | one-shot `migrate` job | database/schema owner, DDL |
| `rs_admin_runtime` | API | DML only; `audit_logs` INSERT/SELECT; `audit_chain_head` SELECT/UPDATE; `_sqlx_migrations` SELECT |
| `rs_admin_cleanup` | retention maintenance | DELETE on application tables including `audit_logs`; SELECT/UPDATE on `audit_chain_head`; SELECT on `_sqlx_migrations` |
| `rs_admin_backup` | pgBackRest | LOGIN REPLICATION + `pg_read_all_data`, `pg_read_all_settings`, `pg_monitor` |

All roles authenticate with SCRAM over TLS. `pg_hba.conf` rejects md5 and
unencrypted host connections. The bootstrap superuser exists only for first
initialization and operator recovery.

`rs_admin_backup` uses the least-privilege pgBackRest setup (replication plus
read-only catalog roles). The backup job runs as the `postgres` OS user in a
one-shot container that shares the `pgdata` and `pgsocket` volumes read-only
and connects through the local Unix socket, so no superuser exception or
additional HBA entry is needed. PostgreSQL 18's non-superuser backup functions
are granted to the role by the first-initialization script.

The role init script only runs when the `pgdata` volume is first initialized.
After restoring an existing cluster, provision these roles (or run
`deploy/db/init/00-rs-admin-roles.sh`) before running `migrate`. To recreate a
local development cluster from scratch, remove the volume with
`docker compose down --volumes` before starting again.

## Backups, WAL archiving and restore drills

The `db` service runs `postgres:18.6-alpine` plus
`pgbackrest=2.58.0-r0` (built from [db/Dockerfile](./db/Dockerfile)). The
`[rs-admin]` stanza archives every WAL segment through
`pgbackrest archive-push` and the `backup` tools-profile job runs a daily full
backup. A time-based retention keeps 30 days of full backups and the WAL
needed to restore them.

Non-secret repository settings live in `deploy/.env`:

```sh
BACKUP_S3_BUCKET=rs-admin-backups
BACKUP_S3_ENDPOINT=https://s3.example.com
BACKUP_S3_REGION=us-east-1
BACKUP_S3_URI_STYLE=path
BACKUP_RETENTION_DAYS=30
RS_ADMIN_METRICS_DIR=/var/lib/rs-admin/metrics
```

The S3 access key, secret key and repository cipher pass live in the secrets
directory (`backup_s3_access_key`, `backup_s3_secret_key`,
`backup_repo_cipher_pass`). Keep an offline copy of the cipher pass: without it
the repository cannot be decrypted.

The backup job writes a Prometheus textfile at
`$RS_ADMIN_METRICS_DIR/rs-admin-backup.prom`:

```
pgbackrest_backup_last_success_timestamp
pgbackrest_backup_last_attempt_success
pgbackrest_backup_last_attempt_timestamp
```

The maintenance job writes
`$RS_ADMIN_MAINTENANCE_METRICS_DIR/rs-admin-maintenance.prom` with
`maintenance_last_success_timestamp`, `maintenance_last_attempt_success`,
`maintenance_duration_seconds` and `maintenance_rows_deleted{kind}` so cleanup
latency and volume are observable.

Mount both directories into node_exporter's textfile collector; the
observability ticket (#51) wires the collector and the Prometheus rule.
`BackupStale` (`time() - last_success > 93600`, ADR-0008 §6.6) is the primary
alert; a failed job also exits non-zero and triggers `rs-admin-alert@.service`
when a webhook is configured.

Install the host timers (host timezone is UTC). The units assume the
repository checkout is `/opt/rs-admin`; adjust the paths if it lives
elsewhere.

```sh
sudo install -d -m 0750 /etc/rs-admin
sudo install -m 0640 deploy/systemd/rs-admin-ops.env.example /etc/rs-admin/ops.env
sudo install -m 0644 deploy/systemd/rs-admin-*.service \
    deploy/systemd/rs-admin-*.timer /etc/systemd/system/
sudo install -D -m 0755 deploy/systemd/notify-failure.sh \
    /usr/local/lib/rs-admin/notify-failure.sh
sudo systemctl daemon-reload
sudo systemctl enable --now rs-admin-backup.timer \
    rs-admin-maintenance.timer rs-admin-restore-drill.timer
```

| Timer | UTC schedule | Action |
|---|---|---|
| `rs-admin-backup.timer` | daily 03:00 | `stanza-create`, `check`, full backup, metric |
| `rs-admin-maintenance.timer` | daily 04:00 | retention cleanup |
| `rs-admin-restore-drill.timer` | quarterly, 1st at 05:30 | restore + verification |

The maintenance job takes a fixed `pg_try_advisory_lock`; a second concurrent
run prints a notice and exits 0. It removes refresh tokens, then sessions, then
unlocked login-failure/throttle rows older than 30 days, then an expired audit
prefix. Audit rows follow the 180/365-day severity tiers; the purge event and
deleted rows commit in one transaction, and `last_purged_hash` anchors the
surviving chain.

Run a restore drill by hand or let the quarterly timer do it:

```sh
./scripts/restore-drill.sh
./scripts/restore-drill.sh --expected-migration-version 20261108000000
```

The drill restores the newest repository backup into a throwaway container,
checks the migration version and key table row counts, recomputes the audit
HMAC chain, records the measured RTO under `$RESTORE_DRILL_STATE_DIR`, and
removes the container and volume. A failed drill exits non-zero, keeps the
record and notifies through `rs-admin-alert@.service`.

With an external database, remove the `local-db` profile: the external
operator owns backup, WAL archiving, restore drills and the `database_url_cleanup`
DSN. The maintenance timer can still run with `--profile tools`.

## Edge behavior

- HTTP redirects to HTTPS with `301`; ACME HTTP-01 challenges are served from
  the `certbot-webroot` volume.
- HSTS is `max-age=31536000; includeSubDomains` (preload is added only after the
  real domain is confirmed).
- `/api/v1/auth/*` is limited to 60 requests/minute per client IP with a
  `burst=10`; over-limit responses are `429` with `Retry-After`.
- `/health/*`, `/metrics`, `/api-docs`, Swagger UI and `/openapi.json` return
  `404` on both public hosts. Use the internal network for monitoring.
- Proxy connect/read/send timeouts are 30 seconds; auth bodies are capped at
  1 KiB and other API bodies at 64 KiB.
- The SPA is served with the strict CSP from ADR-0005 §7.3, SPA fallback,
  `no-cache` for `index.html`, and one-year immutable caching for
  `/assets/` (content-hashed build output). gzip is enabled; brotli needs a
  custom nginx build (the pinned stock image does not ship the module) and is
  deferred.
- The web image no longer loads Google Fonts; the CSP only permits self-hosted
  fonts, so the UI uses the system font stack.

## Operational limits

| Setting | Value |
|---|---|
| API request timeout | 30 s |
| Graceful shutdown | 15 s app, 25 s Compose grace |
| DB pool per API replica | 10 (min 1, acquire 3 s, idle 10 min, lifetime 30 min) |
| API memory/CPU cap | 1 GiB / 2 CPUs |
| PostgreSQL `max_connections` | 100 |
| Docker log buffer | 10 MB × 5 per container |

Capacity follows ADR-0008 §10: one API replica and 10 pool connections leave
ample room for migration, maintenance, backup and exporter connections under
`max_connections=100`.

## Verification

Static acceptance checks (also run in CI):

```sh
./scripts/check-deploy.sh
```

They assert the service topology, published ports, non-root/read-only/no-new-
privileges hardening, healthcheck, secret wiring, gateway TLS/rate-limit/deny
rules, SPA caching/CSP, pinned images, and database role grants.
