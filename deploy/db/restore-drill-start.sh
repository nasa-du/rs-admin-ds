#!/bin/sh
#
# Restore-drill container entrypoint (ADR-0008 §5.6, AC-OPS-05).
#
# Restores the latest repository backup into a temporary data directory and
# starts a loopback-only PostgreSQL server with trust authentication. The
# drill driver then runs SQL checks and verify-audit-chain against it and
# removes the container and volume.
#
# Runs as root only long enough to copy mounted secrets into a postgres-owned
# directory and prepare the restore volume; the postmaster runs as postgres.

set -eu

restore_root="${RESTORE_ROOT:-/restore}"
restore_pgdata="${RESTORE_PGDATA:-$restore_root/pgdata}"
secret_dir="/run/rs-admin-secrets"

mkdir -p "$secret_dir" "$restore_root/socket"

for name in \
    backup_s3_access_key \
    backup_s3_secret_key \
    backup_repo_cipher_pass \
    internal_ca \
    postgres_server_cert \
    postgres_server_key
do
    source="/run/secrets/$name"
    if [ ! -r "$source" ] && [ -r "$source.pem" ]; then
        source="$source.pem"
    fi
    if [ ! -r "$source" ]; then
        echo "restore drill: missing secret $name" >&2
        exit 1
    fi
    cp "$source" "$secret_dir/$name"
done

chown -R postgres:postgres "$secret_dir" "$restore_root"
chmod 0700 "$secret_dir" "$restore_root/socket"
# PostgreSQL refuses a private key that is group/world readable.
chmod 0600 "$secret_dir"/*

# Refuse to overwrite an existing cluster. The drill always uses a fresh
# volume; the disaster-recovery procedure moves the old data directory aside
# first.
if [ -e "$restore_pgdata" ] && [ -n "$(ls -A "$restore_pgdata" 2>/dev/null)" ]; then
    echo "restore drill: $restore_pgdata is not empty; move it aside before restoring" >&2
    exit 1
fi
rm -rf "$restore_pgdata"
mkdir -p "$restore_pgdata"
chown postgres:postgres "$restore_pgdata"

cat >"$restore_root/pg_hba.conf" <<'HBA'
# Restore drill only: the server listens on loopback inside a private network
# namespace shared with the verifier container. TCP still requires TLS, matching
# the production rule that sslmode=disable/prefer is never used.
local all all trust
hostssl all all 127.0.0.1/32 trust
hostssl all all ::1/128 trust
HBA
chown postgres:postgres "$restore_root/pg_hba.conf"

export RS_ADMIN_SECRETS_DIR="$secret_dir"
. /usr/local/bin/rs-admin-pgbackrest-env

# Restore the newest full backup and replay its WAL to the first consistent
# point, then promote. The server below overrides the archived configuration
# so the drill never writes WAL back to the repository.
gosu postgres pgbackrest --stanza=rs-admin --pg1-path="$restore_pgdata" \
    --type=immediate --target-action=promote restore

chown -R postgres:postgres "$restore_pgdata"

exec gosu postgres postgres -D "$restore_pgdata" \
    -c listen_addresses=127.0.0.1 \
    -c port=5432 \
    -c hba_file="$restore_root/pg_hba.conf" \
    -c ssl=on \
    -c ssl_cert_file="$secret_dir/postgres_server_cert" \
    -c ssl_key_file="$secret_dir/postgres_server_key" \
    -c ssl_ca_file="$secret_dir/internal_ca" \
    -c ssl_min_protocol_version=TLSv1.2 \
    -c archive_mode=off \
    -c archive_command='' \
    -c unix_socket_directories="$restore_root/socket" \
    -c logging_collector=off
