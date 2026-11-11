#!/bin/sh
#
# Start the upgraded PostgreSQL 19 cluster for drill verification.
#
# The server listens on loopback inside the drill container's private network
# namespace, uses trust authentication and has WAL archiving disabled. It never
# touches the production repository or the production cluster.
#
# `shared_preload_libraries=pg_stat_statements` mirrors the production command
# line, so a missing or incompatible extension library fails the drill instead
# of being discovered during the maintenance window.

set -eu

data_dir="${PG_UPGRADE_DATA_DIR:-/upgrade/pg19}"
hba_file=/tmp/pg_hba.conf

cat >"$hba_file" <<'HBA'
# Upgrade drill only: loopback and the local socket inside a private network
# namespace shared with the verifier container.
local all all trust
host all all 127.0.0.1/32 trust
host all all ::1/128 trust
HBA

exec postgres -D "$data_dir" \
    -c listen_addresses=127.0.0.1 \
    -c port=5432 \
    -c hba_file="$hba_file" \
    -c ssl=off \
    -c archive_mode=off \
    -c archive_command='' \
    -c shared_preload_libraries=pg_stat_statements \
    -c password_encryption=scram-sha-256 \
    -c log_min_duration_statement=-1 \
    -c logging_collector=off
