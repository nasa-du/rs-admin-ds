# Full-stack E2E journeys

Playwright drives a real Chromium against the production Compose stack
(`gateway` → `web`/`api` → PostgreSQL) to verify the nine journeys in spec
§12.5. Nothing is mocked: the browser uses the real hosts, TLS edge, CORS
preflights, `__Host-refresh` cookie, CSRF header and PostgreSQL.

The browser resolves `admin.example.com` and `api.example.com` to the local
gateway with Chromium's `--host-resolver-rules`, so no `/etc/hosts` edit is
needed. It also forces `--proxy-server=direct://`, so a host or CI
`HTTP(S)_PROXY` cannot intercept loopback traffic. `ignoreHTTPSErrors` accepts
the self-signed development certificate; cookie attributes stay at their
production values. Certificate and SNI wiring is covered by
`scripts/check-deploy-edge.sh`, not by these journeys.

The isolated project raises the gateway auth ceiling
(`AUTH_RATE_LIMIT=600r/m`, `AUTH_RATE_BURST=100`) because the suite makes
credential calls much faster than a human. Production keeps the 60r/m +
burst 10 defaults; the edge limiter itself is covered by
`scripts/check-deploy-edge.sh`.

## Run

```sh
./scripts/e2e.sh
```

The script:

1. generates isolated `deploy/secrets-e2e` key material when it is missing;
2. resets the isolated Compose project `rs-admin-e2e` **including volumes**;
3. builds `api`, `web` and `db`, then waits for `db`, `migrate`, `api`, `web`
   and `gateway` to be healthy;
4. runs `bootstrap-super-admin` with the test password;
5. runs the Playwright suite and removes the stack on success.

Options:

| Option | Effect |
|---|---|
| `--keep` | Leave the stack running after success (it is always kept on failure). |
| `--no-build` | Reuse the local `rs-admin-api:local`, `rs-admin-web:local` and `rs-admin-db:local` images. |
| `--headed` | Show the browser window. |
| `-- <args>` | Pass extra arguments to `playwright test`, for example `-- --grep "7. audit"`. |

After a failure the stack is left in place and the API/gateway logs are
printed. Stop it with:

```sh
RS_ADMIN_SECRETS_DIR=./secrets-e2e docker compose \
  --project-name rs-admin-e2e --env-file deploy/.env.example \
  -f deploy/compose.yaml -f deploy/compose.e2e.yaml \
  --profile local-db down --volumes
```

To iterate on a single journey without resetting, keep the stack from a
successful run and run Playwright directly:

```sh
cd e2e
E2E_ADMIN_ORIGIN=https://admin.example.com \
E2E_API_ORIGIN=https://api.example.com \
E2E_BOOTSTRAP_USERNAME=ops.e2e \
E2E_BOOTSTRAP_PASSWORD='E2E-Bootstrap-Anchor-2026!' \
E2E_ADMIN_PASSWORD='E2E-Admin-Harbor-2026!' \
bunx playwright test --grep "7. audit"
```

The suite is serial and stateful: journey 1 changes the bootstrap password,
and later journeys use the users and roles created earlier. A fresh full run
is the meaningful result; a single-test rerun assumes the earlier journeys
already ran against the same database.

`tests/fault-injection.spec.ts` is skipped in this suite. It is the browser
half of `scripts/fault-injection.sh`: stop the API, verify the retryable
session error page through the gateway, restart the API and retry. Run it with
that script, not directly.

## Journeys

| # | Test | What it proves |
|---|---|---|
| 1 | first deploy | Bootstrap login, forced password change, Dashboard, `__Host-refresh` attributes (`HttpOnly; Secure; SameSite=Strict; Path=/`, no `Domain`). |
| 2 | create user | Admin creates a user with a generated temporary password; the new user signs in, changes it and opens Users. |
| 3 | custom role | Role + permission matrix, role assignment, then menu/route gating after a fresh sign-in; direct `/users` becomes `/403`. |
| 4 | disable / enable | Disable blocks the next request with `account_disabled`, sign-in is 403, re-enable allows a new sign-in but never restores the old session. |
| 5 | password reset | Reset revokes the old session and token; the one-time temporary password forces a change at the next sign-in. |
| 6 | lockout | The first four failures return 401; the fifth returns 429 `login_locked` + `Retry-After` and starts the form countdown; admin unlock restores sign-in. |
| 7 | audit trail | The audit page finds the bootstrap/lifecycle events; opening a detail twice writes two `audit_log_accessed` rows. |
| 8 | sessions | Page reload cold-start refresh, multi-tab BroadcastChannel sign-out, and sign-out everywhere across independent contexts. |
| 9 | token invalidation | Old access tokens die after self-service change, admin session revocation and a refresh replay outside the 30-second grace window; replay revokes the family and writes `authn_refresh_reuse_detected`. |

## Requirements

- Docker Engine 27+ with Compose v2, Bun 1.4.2, and `openssl` (the secret
  generator).
- Playwright Chromium. The script downloads the browser but not the Linux
  system libraries; on a fresh host run
  `cd e2e && bunx playwright install --with-deps chromium` once (the CI
  workflow does).
- Host ports 80 and 443 free. The E2E override uses its own ranges
  (`172.31.0.0/24`, `172.31.1.0/24`, `172.31.2.0/24`), so it can run beside a
  production-range stack; those ranges must be free.
- About 2 GiB of free memory (PostgreSQL is configured with a 1 GiB
  `shared_buffers` in the production Compose file).

This suite destroys the `rs-admin-e2e` project's volumes and creates test
users. Never point it at production. The manual `e2e` GitHub workflow runs the
same script on an ephemeral runner.

The host defaults (`admin.example.com` / `api.example.com`) are the supported
path. Overriding `E2E_ADMIN_ORIGIN`/`E2E_API_ORIGIN` also requires a matching
web image (`VITE_API_BASE_URL`), gateway hosts and API CORS/Origin settings in
the Compose environment.
