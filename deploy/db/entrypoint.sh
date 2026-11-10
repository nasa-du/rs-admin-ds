#!/bin/sh
#
# The official PostgreSQL entrypoint runs as root and then drops to the
# `postgres` user. Compose mounts secrets as root-owned files, so copy the
# server key material and role passwords into a postgres-owned directory
# before handing control to the upstream entrypoint.

set -eu

secret_dir=/run/rs-admin-secrets

mkdir -p "$secret_dir"
chown postgres:postgres "$secret_dir"
chmod 0700 "$secret_dir"

copy_secret() {
    local name="$1"
    local source="/run/secrets/$name"

    if [ ! -r "$source" ] && [ -r "$source.pem" ]; then
        source="$source.pem"
    fi

    if [ ! -r "$source" ]; then
        echo "database startup failed: missing secret $name" >&2
        exit 1
    fi

    cp "$source" "$secret_dir/$name"
    chown postgres:postgres "$secret_dir/$name"
    chmod 0600 "$secret_dir/$name"
}

for name in \
    postgres_server_cert \
    postgres_server_key \
    internal_ca \
    migrator_password \
    runtime_password \
    cleanup_password \
    backup_password \
    backup_s3_access_key \
    backup_s3_secret_key \
    backup_repo_cipher_pass
do
    copy_secret "$name"
done

exec /usr/local/bin/docker-entrypoint.sh "$@"
