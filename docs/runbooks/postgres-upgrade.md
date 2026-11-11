# PostgreSQL minor and major upgrades

This runbook covers ADR-0008 §11 and AC-OPS-09:

- the pinned production tag (`postgres:18.6-alpine`) and the minor-version
  process;
- the 18 -> 19 upgrade rehearsal with the newest backup, `pg_upgrade --check`,
  and the default `--copy` mode;
- the production maintenance-window procedure, rollback path and verification
  of the migration version, key data, extensions and audit chain;
- the measured drill record.

The executable drill is [`scripts/upgrade-drill.sh`](../../scripts/upgrade-drill.sh).
The drill maintenance image is
[`deploy/db/upgrade/Dockerfile`](../../deploy/db/upgrade/Dockerfile).

## 1. Version policy

| Item | Value |
|---|---|
| Production PostgreSQL | `postgres:18.6-alpine` (pinned in `deploy/db/Dockerfile` and CI) |
| Target for the next major upgrade | `postgres:19rc1-alpine` until PG19 GA, then the newest `postgres:19.x-alpine` |
| Minor upgrade deadline | within 30 days of the upstream release |
| Pin rule | never use `18-alpine`, `19-alpine` or `latest` |

`deploy/db/Dockerfile` is the single source of the production tag; the CI
service container and `scripts/check-deploy.sh` assert the same value. The
upgrade image tag is a build argument (`PG_NEW_IMAGE`) with the same default in
the Dockerfile, `.env.example` and this runbook.

## 2. Minor version upgrade (same major)

1. Read the PostgreSQL release notes for every version between the running
   minor and the target. Check the pgBackRest compatibility matrix and the
   pinned package version in `deploy/db/Dockerfile`.
2. Update only the `FROM postgres:18.x-alpine` line (and the comment) in
   `deploy/db/Dockerfile`. Do not change `PGBACKREST_VERSION` unless the release
   notes require it.
3. Run the normal CI gates and publish a new `db` image tag. Do not rebuild on
   the host.
4. Take an on-demand full backup and confirm WAL archiving is current:
   `docker compose --profile local-db --profile tools run --rm backup`.
5. Announce a short maintenance window. Stop the API, then the database:
   ```sh
   docker compose -f deploy/compose.yaml stop api
   docker compose -f deploy/compose.yaml --profile local-db stop db
   ```
6. Set `DB_IMAGE` to the new tag, start the database and wait for healthy:
   ```sh
   docker compose -f deploy/compose.yaml --profile local-db up -d db
   docker compose -f deploy/compose.yaml exec db pg_isready -U rs_admin -d rs_admin
   ```
7. Start the API and run the release smoke checks.
8. **Verification** (all must pass):
   ```sh
   docker compose exec db psql -U rs_admin -d rs_admin -tAc \
     "SELECT current_setting('server_version')"           # expected 18.x
   docker compose exec db psql -U rs_admin -d rs_admin -tAc \
     "SELECT extname || '=' || extversion FROM pg_extension ORDER BY 1"
   docker compose exec db psql -U rs_admin -d rs_admin -tAc \
     "SELECT COUNT(*) FILTER (WHERE NOT success) FROM _sqlx_migrations"   # 0
   curl -fsS https://api.example.com/health/ready            # 404 on the public edge
   docker compose exec api curl -fsS http://127.0.0.1:8080/health/ready
   docker compose --profile tools run --rm --no-deps \
     --entrypoint /usr/local/bin/rs-admin-verify-audit-chain api
   ```
9. Record the old/new tag and verification output in the release ticket. A
   minor rollback is a tag revert plus a database restart; the data directory is
   compatible within the major version.

PostgreSQL minor upgrades are binary-compatible: the old image can reopen the
data directory. If the database does not start or the ready probe stays at
`503`, revert `DB_IMAGE` to the previous tag, start the database, and follow the
release rollback in [ci-cd.md](./ci-cd.md). Do not run migrations backwards.

## 3. Major upgrade preflight (18 -> 19)

Do this before scheduling the maintenance window. Every item is a gate.

1. Read the PostgreSQL 19 release notes, the extension changes (`pg_trgm`,
   `pg_stat_statements`, `plpgsql`) and the pgBackRest compatibility matrix.
   The currently pinned `pgbackrest=2.58.0-r0` must list PostgreSQL 19 support;
   if it does not, bump `PGBACKREST_VERSION` in `deploy/db/Dockerfile` and
   `deploy/db/upgrade/Dockerfile` (and the CI build arg) in the same change,
   then rerun the drill. The drill records the maintenance image's pgBackRest
   version so the window uses the reviewed binary.
2. Confirm the production database is within the `--copy` envelope: database
   size < 50 GB **and** the data volume has at least 2x the database size plus
   the backup staging space. Above that, resize the volume; do not switch to
   `--link` in production.
   - Spec §11.4 allows `--link` when disk or time is insufficient. This
     runbook intentionally tightens that rule: disk is a preflight gate and
     `--link` is never used because it removes the old-cluster rollback path.
     The ADR-0008 2026-11-11 amendment records the decision. If the volume
     cannot hold the copy, resize it or use the logical migration in section 7;
     do not trade the rollback path for disk space.
3. Run the pre-upgrade restore drill and keep its record:
   ```sh
   ./scripts/restore-drill.sh --operator "$USER"
   ```
   Require `result: success` and `rto_seconds <= 7200`. This proves the newest
   backup, WAL chain, migration version, key rows and audit chain are restorable
   before any destructive step.
4. Build the maintenance image and run the full upgrade drill against the same
   backup:
   ```sh
   ./scripts/upgrade-drill.sh --operator "$USER"
   ```
   The default target is `postgres:19rc1-alpine`. After PG19 GA, pass
   `--new-image postgres:19.0-alpine` (or set `PG_UPGRADE_NEW_IMAGE`) and update
   the Dockerfile default in the same change.
5. Inspect the newest record and require `result: success`,
   `extensions_match: true`, and equal source/destination migration versions and
   row counts:
   ```sh
   latest="$(find "${UPGRADE_DRILL_STATE_DIR:-/var/lib/rs-admin/upgrade-drills}" \
     -name '*.json' | sort | tail -n 1)"
   jq '{result, source_version, destination_version, check_seconds,
        upgrade_seconds, total_seconds, extensions_match,
        source_migration_version, destination_migration_version,
        restore: .restore_verification.row_counts,
        upgraded: .upgrade_verification.row_counts}' "$latest"
   ```
6. If any step fails, do not schedule the window. Keep the record and logs,
   read the failing phase log, open a ticket, and rerun after the fix.

### What the drill proves

The script:

1. restores the newest pgBackRest backup into a throwaway PostgreSQL 18
   container (loopback only, no published ports, trust over TLS inside the
   private namespace);
2. runs `rs-admin-verify-audit-chain` against the restored cluster, which
   checks `_sqlx_migrations`, the built-in role count, key row counts and the
   HMAC audit chain;
3. stops the restored cluster and requires `pg_controldata` to report
   `shut down`, which is a `pg_upgrade` precondition;
4. initializes an empty PostgreSQL 19 cluster with the source encoding and
   locale (pg_upgrade does not run `initdb` itself);
5. runs `pg_upgrade --check --copy` with
   `--old-bindir=/opt/pg18/bin --new-bindir=/usr/local/bin`;
6. initializes a fresh target cluster and runs the real upgrade with `--copy`
   (explicit; never `--link`);
7. starts PostgreSQL 19, runs `ANALYZE`, applies `update_extensions.sql` when
   `pg_upgrade` produced it, and verifies the migration version, key row
   counts, extension names and the audit chain again;
8. writes a JSON record with the measured `restore_seconds`, `initdb_seconds`,
   `check_seconds`, `upgrade_seconds`, `analyze_seconds`,
   `verification_seconds` and `total_seconds`, plus the source/destination
   versions, the maintenance image's pgBackRest version, encoding/locale,
   extension versions, whether the extension update script ran, and both
   verifier outputs.

The drill deliberately does **not** run `stanza-upgrade` or a full backup
against the production repository: those commands update repository metadata
to the temporary cluster's system identifier and would create a backup of the
drill cluster in the production chain. The pgBackRest binary in the
maintenance image is version-recorded by the drill; the real stanza upgrade,
archive check and first PG19 full backup are production step 10.

### Measured drill (2026-11-11, development host)

The full drill ran against a temporary PostgreSQL 18.6 cluster restored from a
real pgBackRest backup (30.9 MB, 1,334 files) of a bootstrapped dataset: one
super administrator, three built-in roles, one audit row, migration
`20261108000000`, and the `pg_trgm` / `pg_stat_statements` / `plpgsql`
extensions. Target: `postgres:19rc1-alpine`. The record reported
`result: success`, `extension_updates_applied: true` (the generated script
updated `pg_stat_statements` 1.12 -> 1.13), and identical key row counts before
and after.

| Phase | Seconds |
|---|---|
| Restore + pre-upgrade verification | 6 |
| `initdb` for the check and upgrade target clusters | 3 |
| `pg_upgrade --check` | 1 |
| `pg_upgrade --copy` | 4 |
| `ANALYZE` (plus the extension update script) | 0 |
| Post-upgrade verification | 0 |
| Total drill (including image and directory preparation) | 18 |

These timings are a baseline for a small dataset, not a production estimate.
Rerun the drill on the production host and replace the table with the
production record before scheduling the window. The production window budget is
the drill total multiplied by the data-growth factor plus 30 minutes for the
announcement, smoke checks and rollback decision. A drill above 2 hours is an
incident and blocks the window.

## 4. Production upgrade procedure (18 -> 19, copy mode)

This procedure assumes the drill passed and the maintenance window is approved.
It replaces the data directory path only after the new image is ready; the old
`18/docker` directory stays intact for rollback.

1. **Freeze releases** so the running code and the migration set do not change
   during the upgrade. Announce the outage and open an incident record.
2. **Take the final backup** and verify it:
   ```sh
   docker compose --profile local-db --profile tools run --rm backup
   docker compose --profile local-db --profile tools run --rm \
       --entrypoint sh backup -c \
       '. /usr/local/bin/rs-admin-pgbackrest-env; pgbackrest --stanza=rs-admin check'
   ```
3. **Record the source state**:
   ```sh
   docker compose exec db psql -U rs_admin -d rs_admin -c \
     "SELECT current_setting('server_version'), MAX(version) FROM _sqlx_migrations"
   docker compose exec db psql -U rs_admin -d rs_admin -c \
     "SELECT extname, extversion FROM pg_extension ORDER BY extname"
   docker compose exec db psql -U rs_admin -d rs_admin -c \
     "SELECT encoding, datcollate, datctype FROM pg_database
       WHERE datname = current_database()"
   ```
   Keep the output with the incident record. The encoding and locale values
   are used to initialize the PostgreSQL 19 cluster in the next step.
4. **Stop writes and the database**:
   ```sh
   docker compose -f deploy/compose.yaml stop api
   docker compose -f deploy/compose.yaml --profile local-db stop db
   ```
   Wait for a clean stop (`docker compose ps db` shows `Exited (0)`). Do not
   continue if the exit code is non-zero.
5. **Build or pull the PostgreSQL 19 database image** from
   `deploy/db/Dockerfile` with `FROM postgres:19.x-alpine` and the same
   `PGBACKREST_VERSION`, and publish it as an immutable tag (see
   [Build the PG19 database image](#appendix-build-the-pg19-database-image)).
   Set `DB_IMAGE` in `deploy/.env` to the new tag. Keep the previous tag
   recorded for rollback.
6. **Build the maintenance image** if the drill did not already leave it on this
   host. The old-binary source must be the **previous PostgreSQL 18** `db`
   image tag, not the new PG19 image selected in step 5:
   ```sh
   docker build \
     --build-arg PG_OLD_IMAGE="ghcr.io/nasa-du/rs-admin-ds/db:<previous-tag>" \
     --build-arg PG_NEW_IMAGE=postgres:19rc1-alpine \
     -f deploy/db/upgrade/Dockerfile \
     -t rs-admin-pg-upgrade:drill deploy/db
   ```
   `PG_NEW_IMAGE` must be the same base tag as the new `db` image's `FROM`
   (and as `PG_UPGRADE_NEW_IMAGE` in `deploy/.env`). After PG19 GA, replace all
   three with the same `postgres:19.x-alpine` tag.
   Then **initialize the empty PostgreSQL 19 cluster** with the source encoding
   and locale. `pg_upgrade` requires an already-initialized target cluster; it
   does not run `initdb` itself.
   ```sh
   # The target directory and its /var/lib/postgresql/19 parent must exist and
   # be empty; create them as the postgres uid first.
   docker run --rm --user 70:70 \
     --volume rs-admin_pgdata:/var/lib/postgresql \
     --entrypoint sh rs-admin-pg-upgrade:drill -c \
     'mkdir -p /var/lib/postgresql/19/docker'

   docker run --rm --user 70:70 \
     --volume rs-admin_pgdata:/var/lib/postgresql \
     --workdir /var/lib/postgresql \
     --entrypoint /usr/local/bin/initdb \
     rs-admin-pg-upgrade:drill \
     -D /var/lib/postgresql/19/docker \
     --username=rs_admin \
     --encoding=UTF8 --locale=en_US.utf8
   ```
   Replace `UTF8` and `en_US.utf8` with the values recorded in step 3, and
   `rs_admin` with `POSTGRES_USER` from `deploy/.env` if it was changed. The
   directory must be empty; if a previous attempt left files, remove the
   `/var/lib/postgresql/19/docker` directory first. Do not use `--no-sync` for
   a production target.
7. **Run the upgrade** against the live `pgdata` volume. The maintenance image
   from the drill is used; no production credentials are needed because the
   data directory is local.
   ```sh
   docker run --rm --user 70:70 \
     --volume rs-admin_pgdata:/var/lib/postgresql \
     --workdir /var/lib/postgresql \
     --entrypoint /usr/local/bin/pg_upgrade \
     rs-admin-pg-upgrade:drill \
     --check --copy --username=rs_admin \
     --old-datadir=/var/lib/postgresql/18/docker \
     --new-datadir=/var/lib/postgresql/19/docker \
     --old-bindir=/opt/pg18/bin \
     --new-bindir=/usr/local/bin
   ```
   Run the same command again without `--check` after it succeeds. The volume
   name is `rs-admin_pgdata` for the default Compose project; confirm with
   `docker volume ls | grep pgdata`. The drill re-initializes the target before
   the real run; to mirror it exactly, remove and recreate
   `/var/lib/postgresql/19/docker` with the `initdb` command from step 6.
   `--check` itself does not modify the target data directory.
   - Never add `--link` to this command. `--copy` leaves the old cluster
     readable and is the rollback path.
   - If `--check` reports a problem, stop and roll back; do not retry with
     different flags.
8. **Start PostgreSQL 19**:
   ```sh
   docker compose -f deploy/compose.yaml --profile local-db up -d db
   docker compose -f deploy/compose.yaml exec db \
     pg_isready -U rs_admin -d rs_admin
   ```
   The image's entrypoint selects `/var/lib/postgresql/19/docker` for the new
   server. If it selects the old directory, the `DB_IMAGE` tag was not the PG19
   image; stop and fix it before starting the API.
9. **Apply extension updates** if `pg_upgrade` produced
   `update_extensions.sql`. The file is in the work directory on the data
   volume:
   ```sh
   if docker compose exec -T db test -s /var/lib/postgresql/update_extensions.sql; then
     docker compose exec -T db psql -U rs_admin -d postgres \
       -v ON_ERROR_STOP=1 -f /var/lib/postgresql/update_extensions.sql
   fi
   ```
   Review the file first; it contains `ALTER EXTENSION ... UPDATE` statements
   for extensions whose default version changed between major versions.
10. **Upgrade the pgBackRest stanza and prove the post-upgrade backup chain.**
    The PG19 `db` image must have `pg1-path=/var/lib/postgresql/19/docker` in
    `deploy/db/pgbackrest.conf` (see the appendix). The first archive attempts
    after startup may fail with a system-id mismatch until `stanza-upgrade`
    runs; PostgreSQL retains and retries the WAL.
    ```sh
    for command in \
      'stanza-upgrade' \
      'check' \
      'backup --type=full'; do
      docker compose exec -u postgres db sh -c \
        ". /usr/local/bin/rs-admin-pgbackrest-env && pgbackrest --stanza=rs-admin $command"
    done
    ```
    - `stanza-upgrade` updates the repository metadata to the new system
      identifier while keeping the pre-upgrade backups.
    - `check` verifies the stanza, repository and WAL archiving from the PG19
      cluster.
    - The full backup is the first backup of the PG19 chain; keep the
      pre-upgrade backups for the 30-day rollback window.
11. **Run `ANALYZE`** as the bootstrap superuser, then start the API and wait
    for readiness:
    ```sh
    docker compose exec db psql -U rs_admin -d rs_admin -c ANALYZE
    docker compose -f deploy/compose.yaml up -d --remove-orphans
    docker compose exec api curl -fsS http://127.0.0.1:8080/health/ready
    ```
12. **Verify** with section 6 below. Only then unfreeze releases and close the
    maintenance window.

## 5. Rollback path

| Situation | Action |
|---|---|
| `pg_upgrade --check` fails | Nothing has been modified. Stop; keep the old cluster running. |
| `pg_upgrade --copy` fails | The old `18/docker` directory is intact. Stop the maintenance container, set `DB_IMAGE` back to the previous tag, start `db`; the old server opens the old directory. |
| PostgreSQL 19 starts but verification fails | Stop the API and the PG19 server; set `DB_IMAGE` back to the previous tag and start `db`. If the old server refuses to start because the copy is incomplete, restore the final backup with [backup-restore.md](./backup-restore.md#41-actual-restore-disaster-recovery). |
| Verification passes but the application misbehaves | Roll back the application with [ci-cd.md](./ci-cd.md); the schema is unchanged by the major upgrade. If the database itself is the cause, use the previous `DB_IMAGE` tag and the old directory. |

Do not delete the old `18/docker` directory, the final backup, or the
`rs-admin-pg-upgrade:drill` image until at least 30 days after a successful
production upgrade. `--link` would remove the old directory and leave only the
backup as a rollback path; that is why it is not used.

## 6. Verification after the production upgrade

Run every command below; record the output in the incident ticket.

```sh
# 1. Server version and clean startup.
docker compose exec db psql -U rs_admin -d rs_admin -tAc \
  "SELECT current_setting('server_version')"          # 19.x

# 2. Migration history is complete and successful.
docker compose exec db psql -U rs_admin -d rs_admin -tAc \
  "SELECT MAX(version), COUNT(*) FILTER (WHERE NOT success)
     FROM _sqlx_migrations"                            # same version as before, 0 failures

# 3. Extensions are present with compatible versions.
docker compose exec db psql -U rs_admin -d rs_admin -c \
  "SELECT extname, extversion FROM pg_extension ORDER BY extname"
# pg_trgm, pg_stat_statements and plpgsql must match the pre-upgrade names.

# 4. Key data and the audit chain.
docker compose exec db psql -U rs_admin -d rs_admin -tAc \
  "SELECT (SELECT COUNT(*) FROM users), (SELECT COUNT(*) FROM roles),
          (SELECT COUNT(*) FROM auth_sessions), (SELECT COUNT(*) FROM audit_logs)"
docker compose --profile tools run --rm --no-deps \
  --entrypoint /usr/local/bin/rs-admin-verify-audit-chain api

# 5. Readiness, smoke and archiving.
docker compose exec api curl -fsS http://127.0.0.1:8080/health/ready
curl --cacert <CA> https://admin.example.com/                     # 200 SPA
curl --cacert <CA> https://api.example.com/api/v1/auth/me          # 401 problem+json
# 5. pgBackRest now uses the PG19 system id and the newest backup is PG19.
docker compose exec -u postgres db sh -c \
  '. /usr/local/bin/rs-admin-pgbackrest-env && pgbackrest --stanza=rs-admin check'
docker compose exec -u postgres db sh -c \
  '. /usr/local/bin/rs-admin-pgbackrest-env && \
   pgbackrest --stanza=rs-admin info --output=json' |
  jq '.[0].backup[-1] | {label, type, timestamp}'
```

The audit-chain verifier exits non-zero on a schema or chain mismatch. Treat a
failure as a data-integrity incident: stop writes, preserve the cluster and the
audit keyring, and escalate. Do not purge or rewrite audit rows.

## 7. Alternative: pg_dumpall / pg_restore

A logical migration is simpler but keeps the database unavailable for the dump
and restore. It is acceptable only when `--copy` cannot fit the disk or the
drill shows an incompatibility that a logical restore avoids. It does not
provide a fast rollback to the old cluster; the final backup is the rollback
path. Record the reason in the incident ticket. Logical replication with a
dual-write cutover is out of scope for v1.

## Appendix: build the PG19 database image

The production `db` image is built from `deploy/db/Dockerfile`. For the new
major version:

1. Change `FROM postgres:18.6-alpine` to the pinned `postgres:19.x-alpine`
   (or `19rc1-alpine` during the rehearsal period), update the comment that
   names the Alpine release and pgBackRest package, and change
   `pg1-path=/var/lib/postgresql/18/docker` in `deploy/db/pgbackrest.conf` to
   `/var/lib/postgresql/19/docker`. The backup job and the `archive-push`
   command read this path; a stale PG18 path silently backs up the retained
   old cluster instead of the new one.
2. Run `./scripts/check-deploy-db.sh` against the new image (it accepts
   `POSTGRES_TEST_IMAGE=postgres:19.x-alpine`) and the full CI gates.
3. Publish an immutable `db` image tag with the release workflow.
4. Keep the previous `db` image tag until the 30-day rollback window ends.

The maintenance image for the drill is separate: it copies the old production
image's `/usr/local` tree to `/opt/pg18` and keeps the target image's
`/usr/local` as the new version. The `pg_config` wrapper in
`deploy/db/upgrade/pg_config` makes `pg_upgrade` inspect the old cluster's own
libraries.

## Verification of this runbook

- `./scripts/check-deploy.sh` asserts the pinned image tags, the drill script's
  `--check`/`--copy` commands, the absence of `--link`, and that this runbook
  keeps the required sections.
- A real drill writes `UPGRADE_DRILL_RESULT=success` and a JSON record; the
  record is the evidence that the upgrade path was exercised with the newest
  backup.
- The maintenance image is rebuilt from pinned sources on every drill; if the
  old or new image is unavailable, the drill fails before touching any data.
