# Performance analysis and login-concurrency risk acceptance

This note accompanies the raw runs in [performance.md](./performance.md) (the
production 2-CPU API limit) and
[performance-4cpu.md](./performance-4cpu.md) (a one-off 4-CPU experiment).
It explains why the login target is recorded as a risk acceptance while the
API, refresh and pool targets pass, and what must happen before the first
production deployment.

## Measured results (2026-11-11, development host)

Host: 16 logical CPUs, 15 GiB RAM, Docker 29.5.3, PostgreSQL 18.6. The API
container keeps the production limits unless noted.

| Measurement | API budget | Samples | p50 | p95 | Limit | Errors | Verdict |
|---|---|---|---|---|---|---|---|
| Login, 20 concurrent | 2 CPUs | 80 | 1352.84 ms | **2249.89 ms** | 1000 ms | 0 (earlier runs: 8–10 × 503) | **miss** |
| Login, 20 concurrent | 4 CPUs | 80 | 824.68 ms | **1422.58 ms** | 1000 ms | 0 | **miss** |
| API list, 50 concurrent | 2 CPUs | 500 | 46.28 ms | 81.12 ms | 300 ms | 0 | pass |
| API list, 50 concurrent | 4 CPUs | 500 | 40.02 ms | 57.55 ms | 300 ms | 0 | pass |
| Refresh, 20 concurrent | 2 CPUs | 60 | 27.36 ms | 50.03 ms | 200 ms | 0 | pass |
| Refresh, 20 concurrent | 4 CPUs | 60 | 26.74 ms | 42.85 ms | 200 ms | 0 | pass |
| Pool active/max, 50 concurrent | 2 CPUs | 103 samples | — | peak 3/10 = 0.30 | < 0.8 | — | pass |
| Pool active/max, 50 concurrent | 4 CPUs | 62 samples | — | peak 3/10 = 0.30 (another sample run hit 9/10) | < 0.8 | — | pass at the sampled peak; see note |

The refresh runs produced one identical successor cookie per round in both
runs, with zero reuse-detection deltas: the 30-second grace window is not
misjudged under concurrency. Argon2 calibration is in
[argon2-2cpu.md](./argon2-2cpu.md) and [argon2-4cpu.md](./argon2-4cpu.md):
policy parameters are 143.80 ms p95 at the 2-CPU budget and 114.54 ms p95 at
the 4-CPU host baseline, both inside the 100–300 ms target, so the constants
stay `m=64 MiB, t=3, p=4`.

## Why the login target is not reachable on the specified host

Argon2id at 64 MiB is memory-hard CPU work. The calibration measures one
verification at ~200 ms of single-lane CPU time (p=1), or ~114–144 ms wall
time when its four lanes run on four CPUs. Twenty simultaneous verifications
therefore need roughly **4 CPU-seconds** of Argon2 work before any HTTP,
PostgreSQL or audit overhead:

- 2-CPU API limit → ≥ 2.0 s floor; the acceptance runs measured 2.25 s,
  2.31 s and 2.52 s p95, with the slowest runs producing 8–10 requests at
  the designed 2-second hash-queue timeout, matching that floor.
- 4-CPU API limit → ≥ 1.0 s floor; the measured 1.39–1.42 s includes the
  audit chain, session and HTTP work.
- The spec's host baseline is 4 vCPU, and the API container is limited to 2
  CPUs. Both the "20 simultaneous logins, p95 ≤ 1 s" target (§10.10, §12.6)
  and "container CPU limit 2" (§10.4) cannot hold together with the approved
  Argon2 cost (ADR-0005 §1.2). The target needs at least ~8 CPU-seconds of
  available hashing capacity per second, i.e. more CPUs than the 4-vCPU
  host baseline.

The pool peak is sampling-sensitive: the production 2-CPU runs sampled
2/10, 3/10 and 6/10 (all pass), while one 4-CPU experiment sampled 9/10 (the final
gauge scrape in its report read active 10/10) and a repeat sampled 3/10.
Fifty *simultaneous* list requests are a burst above the "50 concurrent
sessions" capacity assumption; the 10-connection pool can briefly saturate
when the API is fast enough to issue all fifty at once. The production 2-CPU
configuration passes the bound, but any CPU increase must re-run the check and
revisit the connection formula.

## Risk acceptance

| Item | Value |
|---|---|
| Status | **Risk accepted** for the login 20-concurrency p95 target only |
| Owner | Operations, with security sign-off (the Argon2 cost cannot be lowered below the policy) |
| Evidence | `performance.md`, `performance-4cpu.md`, this analysis |
| Why accepted | The target is arithmetically unreachable on the specified 4-vCPU host with the approved Argon2 parameters; the harness, not a code defect, exposes the capacity conflict. Run-to-run p95 varied between 2.25 s and 2.52 s at the 2-CPU limit |
| What still passes | Argon2 100–300 ms target; API p95 ≤ 300 ms at 50 concurrency; refresh p95 ≤ 200 ms with correct grace; pool active/max < 0.8 at the production 2-CPU limit |
| Mitigation 1 | Re-run `scripts/perf.sh` on the production host before first deploy. If the target still misses, choose one of the options below and record an ADR amendment |
| Mitigation 2 (preferred) | Raise the production host to ≥ 8 vCPU and the API limit to 4–8 CPUs, or run 4 replicas × 2 CPUs (multi-instance checklist in ADR-0008 §7) and re-run the pool check |
| Mitigation 3 | Keep the host and explicitly revise the login SLO/concurrency assumption (for example, 10 simultaneous logins) in a new ADR; do **not** silently lower the Argon2 cost |
| Trigger | First production deployment; any sustained login p95 > 1 s or `ApiLatencyHigh`; any host/CPU-limit change; capacity review per spec §10.11 |

The 503 responses are the designed back-pressure at the 2-second Argon2 queue
budget (`PasswordError::Busy` → 503 `service_unavailable`), not a crash. They
are acceptable only under the risk above; the login metrics and the
`ApiLatencyHigh` / `LoginLatencyHigh` alerts make the pressure visible.
