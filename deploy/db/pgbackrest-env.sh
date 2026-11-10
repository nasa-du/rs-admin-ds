#!/bin/sh
#
# Export the pgBackRest repository credentials from mounted secret files.
# Sourced by archive-push, the daily backup and the restore drill.
#
# The S3 bucket/endpoint/region and retention are non-secret and arrive as
# PGBACKREST_* environment variables from Compose.

set -eu

secret_dir="${RS_ADMIN_SECRETS_DIR:-/run/secrets}"

read_secret() {
    local name="$1"
    local file="$secret_dir/$name"

    if [ ! -r "$file" ]; then
        echo "pgBackRest secret is missing: $name" >&2
        exit 1
    fi
    cat "$file"
}

PGBACKREST_REPO1_S3_KEY="$(read_secret backup_s3_access_key)"
PGBACKREST_REPO1_S3_KEY_SECRET="$(read_secret backup_s3_secret_key)"
PGBACKREST_REPO1_CIPHER_PASS="$(read_secret backup_repo_cipher_pass)"
export PGBACKREST_REPO1_S3_KEY
export PGBACKREST_REPO1_S3_KEY_SECRET
export PGBACKREST_REPO1_CIPHER_PASS
