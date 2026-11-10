#!/bin/sh
#
# Daily full backup plus WAL archive check (spec §10.7, AC-OPS-05).
#
# Runs as the postgres user in a one-shot container that shares the database
# data directory and Unix socket read-only-overlapped volumes. Writes the
# Prometheus textfile collected by node_exporter:
#
#   pgbackrest_backup_last_success_timestamp
#   pgbackrest_backup_last_attempt_success
#   pgbackrest_backup_last_attempt_timestamp
#
# A failed attempt leaves the last success timestamp untouched, sets
# last_attempt_success to 0 and exits non-zero so the systemd unit fails and
# the BackupStale alert fires.

set -eu

stanza=rs-admin
metrics_file="${BACKUP_METRICS_FILE:-/var/lib/rs-admin/metrics/rs-admin-backup.prom}"

. /usr/local/bin/rs-admin-pgbackrest-env

mkdir -p "$(dirname "$metrics_file")"

last_success=0
if [ -r "$metrics_file" ]; then
    previous="$(sed -n 's/^pgbackrest_backup_last_success_timestamp //p' "$metrics_file" | tail -n 1)"
    case "$previous" in
        '' | *[!0-9]*) ;;
        *) last_success="$previous" ;;
    esac
fi

attempt_started="$(date +%s)"
status=0

# stanza-create is idempotent and makes the first scheduled run self-contained.
if ! pgbackrest --stanza="$stanza" stanza-create; then
    status=1
elif ! pgbackrest --stanza="$stanza" check; then
    status=1
elif ! pgbackrest --stanza="$stanza" backup --type=full; then
    status=1
fi

attempt_finished="$(date +%s)"
attempt_success=1
if [ "$status" -ne 0 ]; then
    attempt_success=0
else
    last_success="$attempt_finished"
fi

tmp="${metrics_file}.tmp.$$"
{
    printf '# HELP pgbackrest_backup_last_success_timestamp Unix time of the last successful full backup.\n'
    printf '# TYPE pgbackrest_backup_last_success_timestamp gauge\n'
    printf 'pgbackrest_backup_last_success_timestamp %s\n' "$last_success"
    printf '# HELP pgbackrest_backup_last_attempt_success Whether the most recent backup attempt succeeded (1/0).\n'
    printf '# TYPE pgbackrest_backup_last_attempt_success gauge\n'
    printf 'pgbackrest_backup_last_attempt_success %s\n' "$attempt_success"
    printf '# HELP pgbackrest_backup_last_attempt_timestamp Unix time of the most recent backup attempt.\n'
    printf '# TYPE pgbackrest_backup_last_attempt_timestamp gauge\n'
    printf 'pgbackrest_backup_last_attempt_timestamp %s\n' "$attempt_finished"
} >"$tmp"
mv "$tmp" "$metrics_file"

if [ "$status" -ne 0 ]; then
    echo "backup failed; last success remains ${last_success}" >&2
    exit 1
fi

echo "backup completed; last success ${last_success}"
