# rs-admin-ds

Production-ready login, session and RBAC system for a single-host admin
deployment: a Rust/Axum API, a trimmed `satnaing/shadcn-admin` SPA, PostgreSQL
18, a TLS gateway and a pinned observability stack.

The implementation follows the accepted specification in
[docs/specs/login-system.md](docs/specs/login-system.md) and the eight ADRs in
[docs/adr/](docs/adr/). Acceptance evidence for the 62 criteria in spec §14 is
recorded in [docs/acceptance/](docs/acceptance/README.md).

## Architecture

```
browser ── TLS ── gateway (nginx: TLS, host routing, edge rate limit)
                     ├── web   (static SPA, internal :8080)
                     └── api   (Axum, internal :8080, metrics :9100)
                           └── db (PostgreSQL 18.6, internal :5432)

one-shot: migrate (SQLx, before api) · maintenance / backup (systemd timers)
separate: Prometheus · Alertmanager · Grafana · Loki · Promtail · exporters
```

- **Authentication**: username or email + password; Argon2id; EdDSA access JWT
  (15 min, memory only); `__Host-refresh` rotating cookie with a 30-second
  grace window, replay detection and at most five sessions per user.
- **Authorization**: 13 permissions, built-in `user`/`admin`/`super_admin`
  plus custom roles, grant-subset and management-scope rules, and the
  last-super-admin invariant.
- **Audit**: append-only `audit_logs` in the same transaction as the business
  action, chained with HMAC-SHA256 and a keyed JCS canonical form.
- **Operations**: single-host Compose, one-shot migrations, pgBackRest
  backups + WAL archiving, Prometheus alerting and runbooks for every
  operational path.

## Repository layout

| Path | Contents |
|---|---|
| `backend/crates/api` | Axum router, handlers, middleware, binaries (`rs-admin-api`, `openapi`, `migrate`, `alert-webhook`) |
| `backend/crates/security` | Argon2, JWT, refresh derivation, throttle keys, trusted proxies |
| `backend/crates/domain` | Permission catalog, RBAC and lifecycle policies (pure functions) |
| `backend/crates/store` | SQLx queries, models, readiness, maintenance |
| `backend/crates/audit` | Audit event catalog, JCS canonicalization, HMAC chain writer/verifier |
| `backend/crates/config` | Environment and secret-file validation |
| `backend/crates/cli` | Bootstrap, break-glass recovery, maintenance, key-rotation audit CLI |
| `backend/migrations` | Forward-only SQL migrations |
| `frontend` | Trimmed shadcn-admin SPA (React 19, TanStack Router/Query, i18n) |
| `e2e` | Playwright full-stack journeys and the fault-injection spec |
| `deploy` | Compose stack, gateway/static config, database image, observability |
| `scripts` | Release, drills, E2E, performance/security/fault-injection acceptance harnesses |
| `docs` | Specification, ADRs, research, runbooks, acceptance records |

## Prerequisites

- Docker Engine 27+ with Compose v2 and about 4 GiB of free memory for the
  full stack.
- Rust 1.99.1 (pinned by `rust-toolchain.toml`) and `cargo`.
- Bun 1.4.2 (pinned by `frontend/package.json` and `e2e/package.json`).
  `frontend/bun.lock` uses lockfile v2, which older Bun releases cannot read.
- `openssl` (development secrets), `jq` (acceptance harnesses), and `curl`.

## Quick start

### Development database and API

```sh
./scripts/generate-dev-secrets.sh          # deploy/secrets-dev, development only
docker compose -f deploy/compose.yaml -f deploy/compose.dev.yaml \
  --profile local-db --profile tools up -d db
docker compose -f deploy/compose.yaml -f deploy/compose.dev.yaml \
  --profile local-db --profile tools run --rm migrate

# The host DSN in host.env uses the loopback-published database and the host
# CA path; the four key-file variables point at the same development material.
export DATABASE_URL="$(sed -n 's/^DATABASE_URL=//p' deploy/secrets-dev/host.env)"
export JWT_EDDSA_PRIVATE_KEY_FILE="$PWD/deploy/secrets-dev/jwt-ed25519-private.pem"
export REFRESH_DERIVATION_KEY_FILE="$PWD/deploy/secrets-dev/refresh-derivation.key"
export THROTTLE_KEY_FILE="$PWD/deploy/secrets-dev/throttle.key"
export AUDIT_CHAIN_KEYS_FILE="$PWD/deploy/secrets-dev/audit-chain-keys.json"
export AUDIT_CHAIN_KEY_ID=dev-2026-11
export JWT_EDDSA_KID=dev-2026-11
export ADMIN_ORIGIN=http://localhost:5173
export CORS_ALLOWED_ORIGINS=http://localhost:5173

cargo run -p rs-admin-api                        # API on http://localhost:8080
```

Create the first administrator (the temporary password is shown once):

```sh
printf '%s\n' 'Change-Me-Before-Use-2026!' |
  cargo run -p rs-admin-cli --bin bootstrap-super-admin -- \
    --email ops@example.com --username ops.admin --password-stdin
```

### SPA

```sh
cd frontend
bun install --frozen-lockfile
bun dev                                   # http://localhost:5173, Vite proxies /api/v1
```

The Vite proxy makes the browser same-origin in development, so the real
`__Host-refresh` cookie and CSRF rules stay active without CORS noise.

### Full stack

```sh
./scripts/generate-dev-secrets.sh
cp deploy/.env.example deploy/.env
sudo sh -c 'printf "127.0.0.1 admin.example.com api.example.com\n" >> /etc/hosts'
cd deploy && docker compose --profile local-db up
```

The gateway publishes only `80/443`; `api`, `web` and `db` are reachable only
on the Compose networks. See [deploy/README.md](deploy/README.md) for the
production sequence and [docs/runbooks/first-deploy.md](docs/runbooks/first-deploy.md)
for host preparation and live smoke checks.

## Tests and acceptance

```sh
# Backend: format, lint, unit/integration tests (a PostgreSQL 18 schema is
# created per #[sqlx::test] test).
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
DATABASE_URL=postgres://rs_admin:rs_admin_dev@127.0.0.1:5432/rs_admin \
  cargo test --workspace --all-targets

# Frontend: lint, format, types, i18n keys, browser-mode tests, build.
cd frontend && bun run check

# Full-stack nine journeys (isolated Compose project, real browser/cookies).
./scripts/e2e.sh

# M6 hardening evidence.
./scripts/bench-argon2.sh --out docs/acceptance/argon2-2cpu.md
./scripts/perf.sh
./scripts/security-verify.sh
./scripts/fault-injection.sh
```

The acceptance harnesses always use an isolated Compose project
(`rs-admin-accept`, own networks and secrets) and destroy its volumes. Never
point them at production. Generated artifacts (`backend/openapi.json` and the
orval client) are checked for drift in CI and must be regenerated with the
commands in [CONTRIBUTING.md](CONTRIBUTING.md).

## Documentation

- [Specification](docs/specs/login-system.md) — authoritative implementation
  spec, including the 62 acceptance criteria in §14.
- [ADRs](docs/adr/) — token/session, RBAC, lifecycle, API contract, security
  baseline, frontend integration, pruning/i18n, deployment/observability/CI.
- [CONTEXT.md](CONTEXT.md) — domain terminology; use these names in code and
  docs.
- [Runbooks](docs/runbooks/README.md) — first deploy, release/rollback, secret
  rotation, backup/restore, PostgreSQL upgrade, alerting, break-glass,
  certificate renewal.
- [Acceptance records](docs/acceptance/README.md) — performance, security,
  fault injection, scans and the 62-AC matrix.
- [Research](docs/research/) — technology selection and security background.

## License

The SPA is based on `satnaing/shadcn-admin` (MIT); see
[frontend/LICENSE](frontend/LICENSE) and
[frontend/THIRD-PARTY-NOTICES.md](frontend/THIRD-PARTY-NOTICES.md). The rest of
the repository is `UNLICENSED` and not for redistribution.
