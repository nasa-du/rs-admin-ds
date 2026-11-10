# Single-host deployment

Docker Compose stack for one Linux host:

```
gateway (nginx, TLS, host routing, rate limit)   host :80/:443
  ├── web   (static SPA, internal :8080)
  └── api   (Axum, internal :8080, metrics :9100)
        └── db (PostgreSQL 18.6, internal :5432, profile local-db)
```

Only `gateway` publishes host ports. `api`, `web` and `db` are reachable only
over the fixed Compose networks (`edge` 172.28.0.0/16, internal `data`
172.29.0.0/16). The API trusts forwarding headers only from the `edge` range.

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

Run these steps from `deploy/` (or pass `--env-file deploy/.env -f
deploy/compose.yaml`).

1. Populate `/etc/rs-admin/secrets` with the files and modes listed in
   [secrets/README.md](./secrets/README.md).
2. Set `RS_ADMIN_SECRETS_DIR=/etc/rs-admin/secrets`, `APP_ENV=production`,
   the real `ADMIN_HOST`/`API_HOST`, origins, TLS certificate, image tags and
   `GIT_COMMIT` in `deploy/.env`.
3. `docker compose pull` (or build the images on a trusted builder).
4. `docker compose --profile local-db --profile tools run --rm migrate`.
   Non-zero exit aborts the release.
5. `docker compose --profile local-db up -d --remove-orphans`.
6. Wait up to 120 seconds for `docker compose ps` to report `api` healthy via
   its `/health/ready` healthcheck, then run the release smoke checks.

### External database

Remove the `local-db` profile (do not pass `--profile local-db`) and point
`database_url` / `database_url_migrator` at the external PostgreSQL instance.
Run `migrate` with only the `tools` profile:

```sh
docker compose --profile tools run --rm migrate
docker compose up -d --remove-orphans
```

The DSN must keep `sslmode=verify-full` and a reachable `sslrootcert`. The
external operator owns the four role definitions and their grants; run
`migrate` as `rs_admin_migrator` and ensure `deploy/db/grants.sql` was applied.

## Database roles

| Role | Purpose | Privileges |
|---|---|---|
| `rs_admin_migrator` | one-shot `migrate` job | database/schema owner, DDL |
| `rs_admin_runtime` | API | DML only; `audit_logs` INSERT/SELECT; `audit_chain_head` SELECT/UPDATE; `_sqlx_migrations` SELECT |
| `rs_admin_cleanup` | retention maintenance | DELETE on application tables including `audit_logs`; SELECT/UPDATE on `audit_chain_head` |
| `rs_admin_backup` | pgBackRest | LOGIN REPLICATION + `pg_read_all_data`, `pg_read_all_settings`, `pg_monitor` |

All roles authenticate with SCRAM over TLS. `pg_hba.conf` rejects md5 and
unencrypted host connections. The bootstrap superuser exists only for first
initialization and operator recovery.

`rs_admin_backup` uses the least-privilege pgBackRest setup (replication plus
read-only catalog roles). The backup job ticket finalises the tool version; if
that version still requires superuser for physical backup, its HBA exception is
scoped to the backup container address.

The role init script only runs when the `pgdata` volume is first initialized.
After restoring an existing cluster, provision these roles (or run
`deploy/db/init/00-rs-admin-roles.sh`) before running `migrate`. To recreate a
local development cluster from scratch, remove the volume with
`docker compose down --volumes` before starting again.

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
