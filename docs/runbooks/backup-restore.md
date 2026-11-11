# Backup, restore drill and maintenance

This runbook covers the scheduled operations required by ADR-0008 §5.6,
§6.6 and §7 and AC-OPS-05/07:

- daily pgBackRest full backup and continuous WAL archiving to an
  S3-compatible repository;
- the quarterly restore drill and its RTO record;
- retention cleanup, the advisory lock and the audit purge anchor;
- what to do when a backup, archive or drill fails.

The stack and its files are described in
[deploy/README.md](../../deploy/README.md).

## 1. Targets and schedule

| Item | Value |
|---|---|
| RPO | ≤ 15 minutes (WAL archived continuously; idle databases switch WAL every 60 s) |
| RTO | ≤ 2 hours (measured by every restore drill) |
| Full backup | daily 03:00 UTC via `rs-admin-backup.timer` |
| Retention | 30 days, time-based (`repo1-retention-full-type=time`) |
| Maintenance | daily 04:00 UTC via `rs-admin-maintenance.timer` |
| Restore drill | quarterly, 1st of Jan/Apr/Jul/Oct at 05:30 UTC |

Host time must be UTC. Timers use `Persistent=true`, so a missed run catches up
after the host restarts; `RandomizedDelaySec` avoids exact-hour bursts.

## 2. Repository configuration

`deploy/db/pgbackrest.conf` defines the `[rs-admin]` stanza:

- `repo1-type=s3`, `repo1-path=/rs-admin`, URL-style addressing;
- `repo1-cipher-type=aes-256-cbc`; the passphrase is
  `backup_repo_cipher_pass` in the secrets directory. **Losing it makes every
  backup unreadable.** Keep an offline copy in the password manager.
- `repo1-retention-full-type=time`, `repo1-retention-full=30`.

The non-secret coordinates (`BACKUP_S3_BUCKET`, `BACKUP_S3_ENDPOINT`,
`BACKUP_S3_REGION`, `BACKUP_S3_URI_STYLE`, `BACKUP_RETENTION_DAYS`) come from
`deploy/.env`. The access key, secret key and cipher pass live in
`backup_s3_access_key`, `backup_s3_secret_key` and
`backup_repo_cipher_pass`; they must be `0440 root:70` because the backup
container runs as the postgres uid 70 and Compose preserves host file modes.
Create the bucket before the first run and scope the key to
list/get/put/delete on that bucket prefix only.

The first-initialization script grants the backup role the PostgreSQL 18
non-superuser backup functions, including `pg_create_restore_point` (used by
`pgbackrest check`). A cluster created before this grant was added needs it
applied once by the bootstrap superuser:

```sql
GRANT EXECUTE ON FUNCTION pg_create_restore_point(text) TO rs_admin_backup;
```

Verify the repository and the stanza without taking a backup:

```sh
cd deploy
docker compose --profile local-db --profile tools run --rm \
    --entrypoint sh backup -c \
    '. /usr/local/bin/rs-admin-pgbackrest-env; pgbackrest --stanza=rs-admin check'
```

For an S3-compatible service with a private CA, mount the CA into the `db`
and `backup` containers and add
`repo1-storage-ca-file=/path/to/ca.pem` to `pgbackrest.conf` (or set
`PGBACKREST_REPO1_STORAGE_CA_FILE`). Keep `repo1-storage-verify-tls` at its
default (`y`).

## 3. Daily backup

Run it manually at any time:

```sh
cd deploy
docker compose --profile local-db --profile tools run --rm backup
```

The script runs `stanza-create` (idempotent, also bootstraps a new repository),
`check` (stanza, archive and repository credentials), then a full backup. It
writes `$RS_ADMIN_METRICS_DIR/rs-admin-backup.prom`:

```
pgbackrest_backup_last_success_timestamp <unix seconds>
pgbackrest_backup_last_attempt_success 1|0
pgbackrest_backup_last_attempt_timestamp <unix seconds>
```

On failure the previous success timestamp is preserved,
`last_attempt_success` becomes `0`, and the script exits non-zero. The systemd
unit fails and `rs-admin-alert@.service` posts to
`$ALERT_WEBHOOK_URL_FILE` when configured. The authoritative alert is
`BackupStale`: `time() - pgbackrest_backup_last_success_timestamp > 93600`
(26 hours). `pgbackrest_backup_last_attempt_success == 0` flags a failed
attempt before the 26-hour threshold.

Useful commands:

```sh
# List backups and WAL ranges (JSON for tooling).
docker compose --profile local-db --profile tools run --rm \
    --entrypoint sh backup -c \
    '. /usr/local/bin/rs-admin-pgbackrest-env; pgbackrest --stanza=rs-admin info'

# Check the archived WAL stream without a full backup.
docker compose --profile local-db --profile tools run --rm \
    --entrypoint sh backup -c \
    '. /usr/local/bin/rs-admin-pgbackrest-env; pgbackrest --stanza=rs-admin check'
```

PostgreSQL reports archive failures in `pg_stat_archiver`; the
`PgArchiveFailure` alert is backed by postgres_exporter. A failing
`archive_command` retains WAL in `pg_wal`, so a long outage can fill the disk:
check `pg_stat_archiver.failed_count` and `pg_wal` size before a maintenance
window.

## 4. Restore drill

Run quarterly (the timer does this automatically) or after any change to the
backup configuration:

```sh
./scripts/restore-drill.sh
# Override the live-database comparison explicitly:
./scripts/restore-drill.sh --expected-migration-version 20261108000000
```

Without the flag, the scheduled drill reads `MAX(version)` from the running
local `db` service and fails when the restored backup is at a different
migration. If the local database is not running (for example during host
maintenance), the script logs a warning and records the version without
comparing it.

The drill:

1. restores the newest backup from the S3 repository into a throwaway
   PostgreSQL 18.6 container (loopback only, trust authentication, network
   namespace shared with the verifier);
2. checks `_sqlx_migrations` (all migrations succeeded, version present),
   `users ≥ 1`, `roles ≥ 3`;
3. runs `verify-audit-chain`, which recomputes every HMAC row hash, the chain
   links and the purge anchors;
4. records `started_at`, `finished_at`, `rto_seconds`, row counts, migration
   version, the pgBackRest `info` JSON, the verifier JSON and the result;
5. removes the container and volume.

Records are written to `$RESTORE_DRILL_STATE_DIR`
(`/var/lib/rs-admin/restore-drills`) and the unit output prints
`RESTORE_DRILL_RECORD=...`. Keep them; they are the RTO evidence.

Inspect the newest record:

```sh
latest="$(find /var/lib/rs-admin/restore-drills -name '*.json' | sort | tail -n 1)"
jq '{result, rto_seconds, verification}' "$latest"
```

Interpretation:

- `result: success` and `rto_seconds ≤ 7200` meets RTO. A drill above the
  target is an incident: open a ticket, keep the record.
- A chain failure (`verification` null and a verification error) means the
  restored data is not trustworthy. Do not accept the backup; investigate
  before the next backup ages out.
- `--keep` leaves the container and volume running for inspection. Remove them
  with `docker rm -f rs-admin-restore-drill-<pid>` and
  `docker volume rm rs-admin-restore-drill-<pid>`.
- To rehearse the full disaster-recovery path, use the procedure below.

### 4.1 Actual restore (disaster recovery)

Only on the deployment host, during a maintenance window:

1. Announce the outage and stop the API so no writes occur:
   `docker compose -f deploy/compose.yaml stop api`.
2. Take an on-demand backup if the cluster is still readable:
   `docker compose --profile local-db --profile tools run --rm backup`.
3. Keep the old data directory. The `db` service already mounts the `pgdata`
   volume at `/var/lib/postgresql`:
   ```sh
   docker compose -f deploy/compose.yaml --profile local-db stop db
   docker compose -f deploy/compose.yaml --profile local-db run --rm --no-deps \
       --entrypoint sh db -c \
       'mv /var/lib/postgresql/18/docker /var/lib/postgresql/18/docker.pre-restore'
   ```
4. Restore into the live volume and start the temporary server. `RESTORE_ROOT`
   points at the `pgdata` volume and `RESTORE_PGDATA` at the cluster directory
   the regular `db` service uses:
   ```sh
   docker compose -f deploy/compose.yaml --profile local-db run -d \
       --name rs-admin-live-restore \
       --no-deps \
       -e RESTORE_ROOT=/var/lib/postgresql \
       -e RESTORE_PGDATA=/var/lib/postgresql/18/docker \
       --entrypoint /usr/local/bin/rs-admin-restore-drill-start db
   until docker exec rs-admin-live-restore pg_isready \
       -h 127.0.0.1 -U rs_admin -d rs_admin; do sleep 2; done
   ```
5. Run the same checks as the drill: the key table counts through `psql` and
   the audit chain through the API image:
   ```sh
   api_image="$(sed -n 's/^API_IMAGE=//p' deploy/.env | tail -n 1)"
   audit_key_id="$(sed -n 's/^AUDIT_CHAIN_KEY_ID=//p' deploy/.env | tail -n 1)"
   secrets_dir="$(sed -n 's/^RS_ADMIN_SECRETS_DIR=//p' deploy/.env | tail -n 1)"
   docker exec -e PGSSLMODE=require rs-admin-live-restore \
       psql -h 127.0.0.1 -U rs_admin -d rs_admin \
       -c "SELECT (SELECT COUNT(*) FROM users), (SELECT COUNT(*) FROM roles);"
   docker run --rm --network container:rs-admin-live-restore --read-only \
       -v "$secrets_dir/audit-chain-keys.json:/run/secrets/audit_chain_keys:ro" \
       -v "$secrets_dir/internal_ca.pem:/run/secrets/internal_ca:ro" \
       -e APP_ENV=test \
       -e DATABASE_URL='postgres://rs_admin@127.0.0.1:5432/rs_admin?sslmode=verify-full&sslrootcert=/run/secrets/internal_ca' \
       -e AUDIT_CHAIN_KEY_ID="$audit_key_id" \
       -e AUDIT_CHAIN_KEYS_FILE=/run/secrets/audit_chain_keys \
       --entrypoint /usr/local/bin/rs-admin-verify-audit-chain \
       "$api_image"
   ```
   Stop the temporary server with `docker rm -f rs-admin-live-restore` when the
   checks pass. Keep `docker.pre-restore` until the retention window ends.
6. Start the stack and confirm readiness:
   `docker compose -f deploy/compose.yaml --profile local-db up -d db`, wait
   for healthy, `docker compose -f deploy/compose.yaml up -d --remove-orphans`,
   then
   `curl -fsS https://api.example.com/health/ready`.
7. Confirm `pgbackrest check` and the next scheduled backup succeed.
8. Record the incident: restore start/end time, backup label used, data-loss
   window (last archived WAL vs. incident), verification output.

The restored cluster's `postgresql.auto.conf` keeps a
`restore_command`; the regular `db` service starts with `archive_mode=on` and
resumes archiving. Never leave the drill server running against production
data.

## 5. Maintenance and retention

Run manually:

```sh
cd deploy
docker compose --profile tools run --rm maintenance
```

The command takes `pg_try_advisory_lock` on a fixed key. If another run holds
it, it prints `another instance holds the lock; skipped` and exits 0. Crash or
container removal releases the lock with the connection.

Order and retention:

1. refresh tokens: expired or revoked more than 30 days ago, plus tokens of
   sessions that are themselves past retention;
2. sessions: revoked or expired more than 30 days ago and with no remaining
   refresh tokens (FK RESTRICT);
3. `auth_login_failures` / `auth_login_throttle`: older than 30 days and not
   currently locked (`locked_until` null or in the past);
4. audit rows: one bounded prefix per run. Rows stay until their severity tier
   expires (`info`/`warning` 180 days, `error`/`critical` 365 days, configurable
   upward only via `AUDIT_RETENTION_*`). The deletion stops at the first row
   that is still inside its tier, so the surviving chain is contiguous.

The audit purge commits in one transaction with a `system_audit_logs_purged`
event. Its `details` contains `from_seq`, `to_seq`, `count`,
`last_purged_hash`, `retention_days` and `retention_critical_days`. The hash is
the anchor the verifier uses for the first surviving row; if the event write
fails, the whole deletion rolls back.

Inspect recent purges:

```sql
SELECT occurred_at, chain_seq, severity, details
FROM audit_logs
WHERE action = 'system_audit_logs_purged'
ORDER BY chain_seq DESC
LIMIT 5;
```

Verify the chain at any time (read-only; the `api` service already mounts the
runtime DSN and audit keyring):

```sh
docker compose --profile tools run --rm --no-deps \
    --entrypoint /usr/local/bin/rs-admin-verify-audit-chain api
```

A bounded batch may leave a backlog after a long gap; the next runs drain it.
Monitor table size and cleanup duration if `audit_logs` grows unexpectedly. The
job writes `maintenance_last_success_timestamp`,
`maintenance_last_attempt_success`, `maintenance_duration_seconds` and
`maintenance_rows_deleted{kind}` to
`$RS_ADMIN_MAINTENANCE_METRICS_DIR/rs-admin-maintenance.prom`; collect it with
the node_exporter textfile collector alongside the backup metric.

## 6. Alert response

| Alert / symptom | First action |
|---|---|
| `BackupStale` (>26 h) | Run the backup manually; check `pgbackrest check`, S3 credentials and egress on the `storage` network |
| `pgbackrest_backup_last_attempt_success == 0` | Read `journalctl -u rs-admin-backup`; fix the repository, then rerun |
| `PgArchiveFailure` | Check S3 reachability and the cipher pass; WAL accumulates in `pg_wal` — extend disk or fix before it fills |
| Drill failed | Keep the record (`--keep` if needed), open a ticket, do not accept the backup as restorable |
| Maintenance failed | Check `journalctl -u rs-admin-maintenance`; a missing `_sqlx_migrations` grant or cleanup DSN is the usual cause |
| Audit chain verification failed | Treat as tampering or corruption: stop writes, preserve the database and audit keys, escalate; do not purge |

Every alert links to this runbook. Record the drill/backup outcome in the
operations log and keep the last four quarterly drill records.

## 7. Verification

Use these checks after a backup, restore drill or actual restore:

```sh
# 1. The repository and WAL archive are current.
cd deploy
docker compose --profile local-db --profile tools run --rm \
    --entrypoint sh backup -c \
    '. /usr/local/bin/rs-admin-pgbackrest-env; pgbackrest --stanza=rs-admin check'
docker compose --profile local-db --profile tools run --rm \
    --entrypoint sh backup -c \
    '. /usr/local/bin/rs-admin-pgbackrest-env; pgbackrest --stanza=rs-admin info --output=json' |
    jq '.[0].backup[-1] | {label, type, timestamp}'

# 2. The backup age metric is fresh and the attempt succeeded.
grep -E 'pgbackrest_backup_(last_success_timestamp|last_attempt_success)' \
    "${RS_ADMIN_METRICS_DIR:-/var/lib/rs-admin/metrics}/rs-admin-backup.prom"

# 3. The newest drill record succeeded and met the RTO.
latest="$(find "${RESTORE_DRILL_STATE_DIR:-/var/lib/rs-admin/restore-drills}" \
    -name '*.json' | sort | tail -n 1)"
jq '{result, rto_seconds, verification}' "$latest"

# 4. The live database still verifies (after a restore or maintenance change).
docker compose --profile tools run --rm --no-deps \
    --entrypoint /usr/local/bin/rs-admin-verify-audit-chain api

# 5. The scheduled timers are armed and their last runs succeeded.
systemctl list-timers 'rs-admin-*'
systemctl status rs-admin-backup.timer rs-admin-maintenance.timer rs-admin-restore-drill.timer
```

The `BackupStale` and `PgArchiveFailure` alerts must be resolved after the
verification. A drill record with `rto_seconds > 7200` is an incident, not a
pass, even when `result` is `success`.
