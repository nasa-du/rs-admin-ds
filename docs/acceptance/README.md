# Acceptance records

The 62 acceptance criteria in [spec §14](../specs/login-system.md#14-验收标准)
are closed out here. Every criterion is either **Pass** with linked evidence or
**Risk accepted** with the reason, owner and trigger. The matrix is in
[matrix.md](./matrix.md); the most recent executed run is in
[2026-11-11.md](./2026-11-11.md).

## How the evidence is produced

| Harness | Covers | Record |
|---|---|---|
| `cargo test --workspace --all-targets` | Rust unit/integration, contract, audit chain, migrations | CI + run record |
| `cd frontend && bun run check` | Frontend unit/browser-mode tests, i18n, build | CI + run record |
| `./scripts/e2e.sh` | Nine full-stack journeys on the Compose stack | [e2e run record](./2026-11-11.md#e2e) |
| `./scripts/bench-argon2.sh` | Argon2id calibration at the API container CPU budget (`--container-cpus 2` or `taskset`) | [argon2-container-2cpu.md](./argon2-container-2cpu.md), [argon2-2cpu.md](./argon2-2cpu.md), [argon2-4cpu.md](./argon2-4cpu.md) |
| `./scripts/perf.sh` | 20-concurrency login, 50-concurrency API, 20-concurrency refresh, pool saturation | [performance.md](./performance.md); [analysis and login-concurrency risk acceptance](./performance-analysis.md) |
| `./scripts/security-verify.sh` | Headers, CORS, CSRF, cookies, lockout, log/audit/metric redaction, edge checks, scans | [security.md](./security.md) |
| `./scripts/security-scan.sh` | `cargo audit`, `bun audit`, Trivy image scan; CRITICAL opens a ticket | [security-scan.md](./security-scan.md), [critical disposition](./security-findings.md), weekly [security workflow](../../.github/workflows/security.yml) |
| `./scripts/fault-injection.sh` | DB down fail-closed, audit write failure rollback, gateway 5xx error page | [fault-injection.md](./fault-injection.md) |

`scripts/perf.sh`, `security-verify.sh` and `fault-injection.sh` share an
isolated Compose project (`rs-admin-accept`, edge `172.32.0.0/24`, data
`172.32.1.0/24`, its own secrets directory). They destroy that project's
volumes and must never target production.

## Running on the production host

The operator procedure is
[docs/runbooks/acceptance.md](../runbooks/acceptance.md). The Argon2 and
performance records in this directory were produced on the
development host with the production container CPU budget (2 CPUs) and the
4-vCPU host baseline. **They are a methodology baseline, not a production
waiver.** Before the first production deployment, and after any host or
container CPU-limit change:

```sh
./scripts/bench-argon2.sh --container-cpus 2 --out docs/acceptance/argon2-prod.md
# or, when a cgroup quota is not available:
./scripts/bench-argon2.sh --cpus <production-affinity> --out docs/acceptance/argon2-prod-affinity.md
./scripts/perf.sh --out docs/acceptance/performance-prod.md
./scripts/security-verify.sh --out docs/acceptance/security-prod.md
./scripts/fault-injection.sh --out docs/acceptance/fault-injection-prod.md
```

Archive the four reports with the release record and update the matrix status
if a result changes. The login 20-concurrency target is currently a recorded
risk acceptance (see
[performance-analysis.md](./performance-analysis.md)); a production run that
misses it must follow the mitigation path there rather than being filed as a
pass. A miss is a defect or a risk acceptance, not a silent
exception.

## Status definitions

- **Pass** — the stated evidence executed successfully in the recorded run;
  no residual condition.
- **Risk accepted** — the control exists and is documented, but the full
  production confirmation cannot be executed in this environment. The matrix
  names the owner, the reason, and the trigger that requires re-validation.
  Any new deployment without the re-validation is a release blocker.
