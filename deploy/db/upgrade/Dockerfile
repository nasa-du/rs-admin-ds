# Maintenance image for the PostgreSQL 18 -> 19 upgrade drill
# (ADR-0008 §7.4 and §11.4, AC-OPS-09).
#
# The image carries the old cluster's PostgreSQL 18 binaries copied from the
# pinned production database image and the target PostgreSQL 19 binaries from
# the pinned upstream image. pg_upgrade runs from the target version and is
# given --old-bindir=/opt/pg18/bin, so both versions come from the same image
# and cannot drift between drill runs.
#
# Keep the defaults in sync with docs/runbooks/postgres-upgrade.md. The target
# tag moves from postgres:19rc1-alpine to postgres:19.x-alpine after PG19 GA;
# do not use a floating `19-alpine` tag.

ARG PG_OLD_IMAGE=postgres:18.6-alpine
ARG PG_NEW_IMAGE=postgres:19rc1-alpine
ARG PGBACKREST_VERSION=2.58.0-r0

FROM ${PG_OLD_IMAGE} AS old
FROM ${PG_NEW_IMAGE}

ARG PGBACKREST_VERSION=2.58.0-r0

# pg_upgrade needs both versions; pgBackRest is used by the post-upgrade
# compatibility checks in the runbook and by the drill's version record.
RUN apk add --no-cache "pgbackrest=${PGBACKREST_VERSION}"

# Official PostgreSQL images install into /usr/local. Copy the old tree to a
# separate prefix and wrap its pg_config so pg_upgrade checks the old cluster's
# own libraries instead of the target version's.
COPY --from=old /usr/local /opt/pg18
COPY upgrade/pg_config /opt/pg18/bin/pg_config.wrapper
RUN mv /opt/pg18/bin/pg_config /opt/pg18/bin/pg_config.real \
    && mv /opt/pg18/bin/pg_config.wrapper /opt/pg18/bin/pg_config \
    && chmod 0755 /opt/pg18/bin/pg_config

COPY pgbackrest.conf /etc/pgbackrest/pgbackrest.conf
COPY pgbackrest-env.sh /usr/local/bin/rs-admin-pgbackrest-env
COPY upgrade/upgrade-start.sh /usr/local/bin/rs-admin-upgrade-start
RUN chmod 0755 \
    /usr/local/bin/rs-admin-pgbackrest-env \
    /usr/local/bin/rs-admin-upgrade-start

# The old tree's shared libraries must win for the /opt/pg18 binaries. The
# target binaries in /usr/local are linked against their own libraries first.
ENV LD_LIBRARY_PATH=/opt/pg18/lib:/usr/local/lib
