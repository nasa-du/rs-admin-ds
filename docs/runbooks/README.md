# Operations runbooks

This directory is the operator entry point for the single-host deployment
described in [deploy/README.md](../../deploy/README.md) and ADR-0008. Every
runbook follows the same shape: preconditions, numbered procedure, and a
**Verification** section with commands and expected results. Record the
operator, UTC timestamps and the evidence path in the incident or release
ticket; never paste credentials, tokens, cookies or key material.

## Required runbooks

| Topic | Runbook | Primary evidence |
|---|---|---|
| First deployment | [first-deploy.md](./first-deploy.md) | release record, bootstrap audit event, smoke output |
| Release and rollback | [ci-cd.md](./ci-cd.md) | `$RELEASE_STATE_DIR` record and workflow run |
| Secret rotation | [secret-rotation.md](./secret-rotation.md) | rotation record and `crypto_key_rotated` audit event |
| Backup and restore | [backup-restore.md](./backup-restore.md) | `$RESTORE_DRILL_STATE_DIR` record, `RTO` |
| PostgreSQL upgrade | [postgres-upgrade.md](./postgres-upgrade.md) | `$UPGRADE_DRILL_STATE_DIR` record and measured phases |
| Alert troubleshooting | [alerting.md](./alerting.md) | incident record with the alert name and runbook anchor |
| Break-glass recovery | [break-glass.md](./break-glass.md) | `system_recover_super_admin` audit event |
| Certificate renewal and external uptime | [certificate-renewal.md](./certificate-renewal.md) | `openssl s_client` output and probe metric |

The alert catalog in
[deploy/observability/prometheus/rules/rs-admin.yml](../../deploy/observability/prometheus/rules/rs-admin.yml)
carries a `runbook_url`. Most alerts land on the matching section of
[alerting.md](./alerting.md); backup and certificate alerts link directly to
their dedicated runbooks. `ExternalUptime` is provider-owned and is documented
in [certificate-renewal.md](./certificate-renewal.md#externaluptime) and
[deploy/observability/uptime/README.md](../../deploy/observability/uptime/README.md).

## Drill scripts

| Script | Purpose | State directory |
|---|---|---|
| [`scripts/restore-drill.sh`](../../scripts/restore-drill.sh) | Restore the newest backup, verify schema/key rows/audit chain, record RTO | `RESTORE_DRILL_STATE_DIR` (`/var/lib/rs-admin/restore-drills`) |
| [`scripts/upgrade-drill.sh`](../../scripts/upgrade-drill.sh) | Restore the newest backup, verify it, run `pg_upgrade --check` and `--copy`, verify the upgraded cluster and record each phase | `UPGRADE_DRILL_STATE_DIR` (`/var/lib/rs-admin/upgrade-drills`) |

Run the restore drill quarterly (the systemd timer does it) and both drills
before any PostgreSQL major-version maintenance window. The upgrade drill
always uses `--copy`; `--link` is not supported because it removes the
old-cluster rollback path.

## Conventions

- Host time is UTC. `date -u` is used in every record.
- All records are JSON and retained with the same policy as backup and release
  records (at least one year; keep the last four quarterly restore drills and
  every upgrade drill).
- The drill containers share no network with production, publish no ports and
  are removed on exit (`--keep` is for incident debugging only).
- A failed drill is an incident: keep the record and logs, open a ticket, and do
  not proceed with the maintenance window.
