# Alerting and observability runbook

This runbook covers every alert in `deploy/observability/prometheus/rules/rs-admin.yml`.
Each Prometheus rule carries the anchor of its section as `runbook_url`, so the
notification always links to the matching procedure.

## Notification path

```
Prometheus rule
  -> Alertmanager (severity route)
  -> alert-webhook adapter
  -> Feishu / DingTalk / WeCom / generic chat webhook
```

- **critical** is delivered immediately (resolved notifications included).
- **warning** is buffered by the adapter and delivered once per day at
  `ALERT_DIGEST_HOUR_UTC` (default 09:00 UTC). Resolved warnings stay in the
  digest until the next flush, so a warning that fires and resolves between
  digests is still reported.
- A critical alert inhibits the same-name warning while both are firing.
- Notifications contain the environment, alert name, instance, start time,
  summary and this runbook link. They never contain credentials, tokens or
  arbitrary alert labels.
- The adapter URL is the secret `alert_webhook_url`; an empty file disables
  delivery without stopping the stack.
- The warning buffer is in memory. If the adapter restarts before the digest
  hour, pending warnings are re-collected when Alertmanager repeats them
  (24 hours); a failed digest send is retried every minute instead of being
  dropped.
- The external uptime provider notifies independently of this host; see
  [`deploy/observability/uptime/README.md`](../../deploy/observability/uptime/README.md).

## ApiDown

**Meaning:** Prometheus has failed to scrape `api:9100` for two minutes.

1. `docker compose -f deploy/compose.yaml ps api` and inspect
   `docker compose -f deploy/compose.yaml logs --tail=200 api`.
2. From the Compose network, check `GET /health/live` and `GET /health/ready`.
   A live but unready API usually means the database is unreachable or the
   schema is behind the embedded migrator.
3. Check the gateway: `curl --cacert <CA> https://api.example.com/health/ready`
   must return `404` (not exposed) while `https://admin.example.com/` still
   serves the SPA.
4. If the container is restarting, look for configuration or key-material
   startup failures. Do not bypass audit or authentication checks to restore
   service.
5. Escalate to the database runbook (`PgDown`) when readiness reports a
   database problem. `ApiUnready` fires when the process is live but this
   readiness check fails.

## ApiUnready

**Meaning:** the internal blackbox_exporter probe of
`http://api:8080/health/ready` has returned non-200 for two minutes.

1. The API process is alive (metrics still scrape) but cannot serve business
   traffic. Inspect `docker compose -f deploy/compose.yaml logs --tail=200 api`.
2. Check the database: `docker compose ps db`, `PgDown`, and the pool gauges.
3. Check the migration version reported by `/health/ready` against the
   embedded migrator. A database behind the code returns 503 until the
   one-shot `migrate` job runs.
4. Do not restart-loop the API or force readiness; fix the dependency first.
5. This probe is deliberately not public. The public gateway continues to
   return 404 for `/health/*`; external uptime probes only the SPA.

## Api5xxHigh

**Meaning:** the 5xx ratio exceeded 5% for 5 minutes (critical) or 1% for
30 minutes (warning).

1. Use the API dashboard to find the affected `route` and `status`.
2. Correlate with `audit_write_failures_total`, `db_pool_connections`, and the
   request log lines for the same `request_id`.
3. Check PostgreSQL availability and recent migrations/releases.
4. If the errors started with a release, follow the rollback flow in
   [`ci-cd.md`](./ci-cd.md). Never roll back a database migration manually.
5. Capture the error codes and a sample `request_id` in the incident record;
   do not paste request bodies, cookies or tokens.

## ApiLatencyHigh

**Meaning:** API p95 latency exceeded 300 ms for 10 minutes (warning) or 1 s
for 10 minutes (critical).

1. Check the API dashboard for the slowest route and the pool connection
   states.
2. Check host CPU, memory and disk pressure on the PostgreSQL/Host dashboard.
3. Inspect PostgreSQL for long-running queries and lock waits with a read-only
   session; `log_min_duration_statement=500ms` lines are in Loki.
4. Verify the Argon2 semaphore is not saturated by a login burst
   (`LoginLatencyHigh`).
5. If the host is beyond the ADR-0008 capacity envelope, open a capacity
   review rather than raising the pool limit without recalculating
   `max_connections`.

## LoginLatencyHigh

**Meaning:** login p95 exceeded 1 second for 10 minutes.

1. Check `auth_password_hash_seconds` on the Auth dashboard.
2. Confirm `PASSWORD_HASH_CONCURRENCY` still matches the container CPU limit
   and that queue waits are not returning 503.
3. Check the account/IP/global throttle tables for a lock storm.
4. Do not increase Argon2 concurrency without recalculating memory:
   `64 MiB × concurrency` must fit the 1 GiB API limit.

## RefreshReuseDetected

**Meaning:** a rotated refresh token was presented outside the 30-second grace
window. The token family and session were revoked automatically.

1. Open the critical `authn_refresh_reuse_detected` audit event; it contains
   the session id, rotation counter and actor.
2. Determine whether the cause is a stolen token, a broken client retry loop,
   or a clock problem.
3. If a client is replaying an old token, fix the client. Do not re-enable the
   revoked session.
4. If compromise is suspected, revoke the user's other sessions and reset
   credentials through the normal admin flow (break-glass only when the admin
   account itself is unavailable).
5. Record the incident against the user/actor in the audit trail.

## LoginLockoutSpike

**Meaning:** lockouts are occurring more than 10 times per minute for
5 minutes.

1. Check the Auth dashboard for the lockout scope (`account`, `ip`,
   `global`).
2. Inspect `auth_login_failures` and `auth_login_throttle` counts and recent
   audit events.
3. A single IP usually means a brute-force attempt: confirm the gateway edge
   limiter is returning 429 and consider a firewall block with the security
   owner.
4. A global spike means many users are failing: check a recent deploy, client
   configuration and Argon2 availability.
5. Do not unlock accounts in bulk while an attack is active; unlock after the
   source is contained.

## GlobalLoginCircuitOpen

**Meaning:** `auth_global_circuit_open == 1`; all login and reauthentication
requests return 429 until the circuit releases.

1. This is a critical availability event. Check `LoginLockoutSpike` and the
   audit log for the failure source.
2. The circuit releases automatically after `LOGIN_GLOBAL_LOCK_SECONDS`.
   Do not disable the circuit in production.
3. If the cause is a misconfigured client, stop that client before the circuit
   releases.
4. Confirm the release by watching the gauge return to 0 and a successful
   login test with a development account (not a production admin).

## AuditWriteFailure

**Meaning:** `audit_write_failures_total` increased. Business actions fail
closed when the audit chain cannot be written.

1. Treat this as a data-integrity incident. Check PostgreSQL availability,
   `audit_chain_head` contention and disk space.
2. Inspect the API logs for the stable audit error message (never SQL or
   parameters).
3. Verify the chain with `rs-admin-verify-audit-chain` once writes succeed.
4. Do not disable audit or make the writer best-effort. If the chain is
   damaged, follow the backup/restore runbook and open a security review.

## DbPoolSaturated

**Meaning:** active connections exceeded 80% of the configured maximum for
5 minutes.

1. Check the API dashboard and PostgreSQL `pg_stat_activity` for slow queries
   and lock waits.
2. Confirm the pool formula: `replicas × DATABASE_MAX_CONNECTIONS + one-shot
   jobs + monitoring < max_connections`.
3. Add capacity (host/replicas) only after the ADR-0008 §10 review; raising
   the pool alone can exhaust PostgreSQL.
4. Look for a leak: long-lived transactions in the API logs or audit writer.

## PgArchiveFailure

**Meaning:** `pg_stat_archiver_failed_count` increased. WAL segments are not
reaching the pgBackRest repository and the RPO is at risk.

1. Check `docker compose logs db | grep archive` and the backup container
   output.
2. Verify S3 credentials, endpoint reachability and the repository cipher
   pass. A failed push retries and retains segments in `pg_wal`.
3. Check disk space: a full `pg_wal` eventually stops the database.
4. Do not delete WAL files manually. Restore archiving and let PostgreSQL
   drain the queue.
5. Run `pgbackrest --stanza=rs-admin check` and a full backup once fixed.

## PgDown

**Meaning:** postgres_exporter cannot reach PostgreSQL for two minutes.

1. Check the `db` container, its healthcheck and the data network.
2. Inspect PostgreSQL logs for startup, disk, TLS or authentication failures.
3. Verify the monitoring role still exists and `monitor_password` matches.
4. If PostgreSQL is down, follow the backup/restore and incident procedures;
   do not start the API against a partially initialized database.

## BackupStale

**Meaning:** no successful full backup in 26 hours, or the metric is absent.

1. Inspect the most recent `rs-admin-backup` systemd job and its logs.
2. Run the backup manually:
   `docker compose --profile local-db --profile tools run --rm backup`.
3. Check the textfile at `$RS_ADMIN_METRICS_DIR/rs-admin-backup.prom` and the
   node_exporter textfile collector (the merge sidecar copies it every
   15 seconds).
4. If pgBackRest fails, check repository credentials, S3 reachability and the
   cipher pass. Never delete a repository to "fix" it.

## DiskSpace

**Meaning:** a host filesystem is more than 80% full (warning) or 90% full
(critical).

1. Identify the mount and largest consumers with `du`/`docker system df`.
2. Docker json-file logs are capped at 10 MB × 5 per container; Loki retention
   is 14 days and Prometheus retention is 30 days.
3. Check `pg_wal` growth (archive failures) and the pgBackRest staging area.
4. Do not delete `pgdata`, WAL segments or audit rows to free space. Expand the
   disk or move the backup repository; open an incident if the database is at
   risk.

## CertificateExpiry

**Meaning:** the gateway certificate presented on the internal probe endpoint
expires in less than 14 days (warning) or 7 days (critical), or the probe
itself cannot read the certificate.

1. If `probe_success{job="blackbox-tls"}` is 0, check that
   `blackbox_exporter` can reach `https://gateway:8443/` over the edge
   network and that the gateway TLS listener is healthy. This is a warning,
   not proof of expiry.
2. Check the certbot systemd timer and ACME challenge path.
3. Confirm DNS/port 80 reachability and the certificate SANs for both hosts.
4. Renew, then reload the gateway:
   `docker compose -f deploy/compose.yaml exec gateway nginx -s reload`.
5. Verify the new notAfter with `openssl s_client` against the public hosts and
   confirm `probe_ssl_earliest_cert_expiry` jumps.

## ExternalUptime

**Meaning:** the external uptime provider reports that the public SPA is
unavailable. This alert is owned by the provider because it must fire when this
host and Prometheus are down.

1. Check the provider's status page and the failing probe location.
2. Check DNS, the gateway container and host reachability.
3. Correlate with `ApiDown`, `PgDown` and host metrics if Prometheus is still
   running.
4. If the host is lost, follow the restore procedure in
   [`backup-restore.md`](./backup-restore.md) and update the provider only
   after the SPA returns 200.
5. Verify the provider is configured for at least two regions and that its
   notification channel is independent of this host.

## Stack operations

Start the application stack first, then the observability stack:

```sh
cd deploy
docker compose --profile local-db up -d
docker compose -f compose.observability.yaml up -d
```

Grafana is published on `127.0.0.1:3000` only:

```sh
ssh -L 3000:127.0.0.1:3000 deploy@host
# open http://127.0.0.1:3000 with the configured Grafana admin user
```

The provisioned dashboards are:

| Dashboard | Contents |
|---|---|
| rs-admin Auth | login outcomes, lockouts, refresh replay detections, password-hash p95, global circuit |
| rs-admin API | readiness, QPS, 5xx ratio, p95 latency, pool connections, per-route errors |
| rs-admin PostgreSQL / Host | pg_up, connections, WAL archive failures, disk usage, backup age, certificate days |

Prometheus keeps 30 days of metrics; Loki keeps 14 days of logs. Both use
named volumes and are not exposed through the gateway.

## Verify retention and redaction

Retention is part of the checked-in configuration:

```sh
grep -E 'retention_period|retention_enabled|delete_request_store' \
  deploy/observability/loki/loki-config.yaml
# retention_period: 336h (14 days), compactor deletion enabled
```

Promtail redacts credential-shaped fields before the line reaches Loki. To
verify on a running stack:

```sh
# Query for a known secret marker used in a controlled test; no result means
# the redaction stage removed it.
docker compose -f deploy/compose.observability.yaml exec loki \
  wget -qO- 'http://localhost:3100/loki/api/v1/query?query=%7Bservice%3D%22api%22%7D%20%7C%3D%20%22Bearer%20metricsecret%22'
```

Also rely on the automated checks:

- `backend/crates/api/tests/logging.rs` asserts request logs contain only the
  whitelisted fields and no query string, Authorization or Cookie values.
- `backend/crates/api/tests/metrics.rs` asserts metric labels never contain a
  username, email, raw path or secret.
- `scripts/check-observability.sh` asserts the 14-day retention, the redaction
  stages, alert runbook links, dashboards and pinned images.
