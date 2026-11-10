#!/bin/sh
#
# One-shot migration entrypoint: apply embedded SQLx migrations with the
# migrator role, then apply the table/role grants. Both steps must succeed
# before the release continues.

set -eu

: "${DATABASE_URL_FILE:=/run/secrets/database_url_migrator}"

if [ ! -r "$DATABASE_URL_FILE" ]; then
    echo "migration failed: cannot read $DATABASE_URL_FILE" >&2
    exit 1
fi

/usr/local/bin/rs-admin-migrate

DATABASE_URL="$(cat "$DATABASE_URL_FILE")"
export DATABASE_URL

psql "$DATABASE_URL" \
    --no-psqlrc \
    --set=ON_ERROR_STOP=1 \
    --file=/opt/rs-admin/grants.sql
