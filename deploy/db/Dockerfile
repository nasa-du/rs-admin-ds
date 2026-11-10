# PostgreSQL 18.6 with the pinned pgBackRest used for WAL archiving, the daily
# full backup and the restore drill. The database and the one-shot backup job
# share this image so pgBackRest versions and configuration cannot drift.

FROM postgres:18.6-alpine

# Alpine 3.24.4 (the postgres:18.6-alpine base) ships pgBackRest 2.58.0-r0.
# Pin the exact package version; do not use a floating `pgbackrest`.
ARG PGBACKREST_VERSION=2.58.0-r0
RUN apk add --no-cache "pgbackrest=${PGBACKREST_VERSION}"

COPY pgbackrest.conf /etc/pgbackrest/pgbackrest.conf
COPY pgbackrest-env.sh /usr/local/bin/rs-admin-pgbackrest-env
COPY backup.sh /usr/local/bin/rs-admin-backup
COPY restore-drill-start.sh /usr/local/bin/rs-admin-restore-drill-start

RUN chmod 0755 \
    /usr/local/bin/rs-admin-pgbackrest-env \
    /usr/local/bin/rs-admin-backup \
    /usr/local/bin/rs-admin-restore-drill-start
