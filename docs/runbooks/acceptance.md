# Acceptance and performance harnesses

Use this runbook for the M6 acceptance pass: calibrate Argon2id, measure the
capacity SLOs, verify the security boundary, and inject the three failure
modes on the **isolated acceptance stack**. The harnesses create and destroy
their own Compose project (`rs-admin-accept`) and must never touch the
production project.

The 62-criterion matrix and the latest development-host record are in
[docs/acceptance/](../acceptance/README.md).

## 1. Preconditions

- A checkout of the release commit on the host that will run the measurement.
  The production host is the only valid target for the final record; a
  development host is a methodology baseline.
- Docker Engine 27+ with Compose v2, `cargo` 1.99.1, Bun 1.4.2, `jq`, `curl`,
  and enough disk for the images and a fresh PostgreSQL volume.
- No `rs-admin-accept` or `rs-admin-e2e` project running. The harness uses
  the production host ports 80/443, its own network ranges
  (`172.32.0.0/24`, `172.32.1.0/24`, `172.32.2.0/24`) and its own secrets
  directory (`deploy/secrets-acceptance`, copied from `secrets-dev` when
  present).
- The host clock is UTC.

## 2. Argon2id calibration

Pin the same CPU budget as the API container (the Compose file limits `api`
to 2 CPUs). On a 4-vCPU production host, also record the host baseline:

```sh
./scripts/bench-argon2.sh --cpus 0-1 --out docs/acceptance/argon2-prod-2cpu.md
./scripts/bench-argon2.sh --cpus 0-3 --out docs/acceptance/argon2-prod-4cpu.md
```

Exit codes:

| Exit | Meaning | Action |
|---|---|---|
| 0 | policy set (`m=64 MiB,t=3,p=4`) p95 ≤ 300 ms | keep the constants; archive the report |
| 3 | policy set missed 300 ms but a fallback is under 1 s | update `ARGON2_MEMORY_KIB`/`ARGON2_TIME_COST`/`ARGON2_PARALLELISM` in `backend/crates/security/src/password.rs`, re-run the full suite, and record the final parameters |
| 4 | every candidate exceeds 1 s | the CPU budget is unusable; resize the host/container before deploying |

Do not skip the second run when the first is on target: the report records
the measurement budget, and a later container-limit change invalidates it.

## 3. Performance and capacity

```sh
./scripts/perf.sh --out docs/acceptance/performance-prod.md
```

The script boots the isolated stack, creates 50 fixture users through the real
API (including the forced password change), then measures 20-concurrency
logins, 50-concurrency authenticated list requests, 20-concurrency refreshes
of one session, and samples the runtime role's active connections from
`pg_stat_activity`. It fails if any p95 or the pool saturation bound is
missed, or if the concurrent refreshes produce more than one successor cookie
or any reuse-detection event.

Thresholds are spec §10.10: login p95 ≤ 1 s, API p95 ≤ 300 ms, refresh p95
≤ 200 ms, pool active/max < 0.8. A miss is a capacity-review trigger; see
[alerting.md](./alerting.md#capacity-review).

## 4. Security verification

```sh
./scripts/security-verify.sh --out docs/acceptance/security-prod.md
```

It checks API headers/problem details, exact-Origin CORS, CSRF rejection,
`__Host-refresh` attributes, the live account-lockout threshold, and scans
API/gateway/PostgreSQL logs, the audit table and metrics for credential
values, password hashes, Authorization/Cookie headers and identifiers. It
also runs `scripts/check-deploy-edge.sh` (TLS, HSTS, CSP, SPA fallback,
production 60r/m edge limit) and `scripts/security-scan.sh` (`cargo audit`,
`bun audit`, Trivy). A CRITICAL scan finding is a release blocker; open a
ticket with the command printed in the report.

## 5. Fault injection

```sh
./scripts/fault-injection.sh --out docs/acceptance/fault-injection-prod.md
```

Scenarios:

1. Stop `db`: authenticated requests and login must return 503
   `service_unavailable`; restarting the database restores the existing
   session without a new login.
2. Revoke `INSERT` on `audit_logs` from `rs_admin_runtime`: a login with the
   correct password must return 503 and leave no new `auth_sessions` row;
   restoring the grant makes the next login succeed.
3. Stop `api` and load the SPA through the gateway (Playwright): the
   cold-start bootstrap must show the retryable "Can't restore your session"
   page, not a false sign-out; Retry works after the API recovers.

Never point this at production: it plants deliberate failures and revokes
privileges temporarily.

## 6. Record the result

Copy the four reports into `docs/acceptance/` (or archive them with the
release record), update [matrix.md](../acceptance/matrix.md) if a status
changes, and record the operator, UTC timestamps, host/container CPU budget,
image tags and commit in the release ticket. A failed result is a defect or a
new risk acceptance; do not silently re-run until it passes.

## Verification

```sh
# All four reports exist and carry a verdict.
for report in docs/acceptance/performance.md docs/acceptance/security.md \
    docs/acceptance/fault-injection.md docs/acceptance/security-scan.md; do
    grep -q 'Verdict\|Pass\|Risk accepted\|Critical findings' "$report" ||
        echo "missing verdict: $report"
done

# The matrix still covers all 62 IDs with a decided status.
./scripts/check-acceptance-docs.sh
```
