#!/bin/sh
#
# First-initialization script for the local PostgreSQL container.
#
# Creates the least-privilege roles required by ADR-0008 §5.4, makes the
# migrator the schema owner, creates the extensions as superuser (so untrusted
# extensions do not need migrator superuser rights), and restricts CONNECT to
# the application roles. Table-level grants run after migrations in
# deploy/db/grants.sql because the tables do not exist yet at this point.

set -eu

secret_dir=/run/rs-admin-secrets

psql \
    -v ON_ERROR_STOP=1 \
    --username "$POSTGRES_USER" \
    --dbname "$POSTGRES_DB" \
    --set=dbname="$POSTGRES_DB" \
    --set=migrator_password="$(cat "$secret_dir/migrator_password")" \
    --set=runtime_password="$(cat "$secret_dir/runtime_password")" \
    --set=cleanup_password="$(cat "$secret_dir/cleanup_password")" \
    --set=backup_password="$(cat "$secret_dir/backup_password")" \
    --set=monitor_password="$(cat "$secret_dir/monitor_password")" <<'SQL'
CREATE ROLE rs_admin_migrator
    LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT
    PASSWORD :'migrator_password';

CREATE ROLE rs_admin_runtime
    LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT
    PASSWORD :'runtime_password';

CREATE ROLE rs_admin_cleanup
    LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT
    PASSWORD :'cleanup_password';

CREATE ROLE rs_admin_backup
    LOGIN REPLICATION NOSUPERUSER NOCREATEDB NOCREATEROLE
    PASSWORD :'backup_password';

-- postgres_exporter uses this role. pg_monitor grants read-only access to the
-- statistics views (connections, archiver, replication) without table reads.
-- INHERIT (the default) is required: the exporter uses pg_monitor directly.
CREATE ROLE rs_admin_monitor
    LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE
    PASSWORD :'monitor_password';

GRANT pg_read_all_data, pg_read_all_settings, pg_monitor TO rs_admin_backup;
GRANT pg_monitor TO rs_admin_monitor;

-- pgBackRest uses the non-superuser physical backup protocol on PostgreSQL
-- 15+. These built-in functions are owned by the bootstrap superuser, so the
-- grants belong here rather than in the migrator-owned grants.sql.
GRANT EXECUTE ON FUNCTION pg_backup_start(text, boolean) TO rs_admin_backup;
GRANT EXECUTE ON FUNCTION pg_backup_stop(boolean) TO rs_admin_backup;
GRANT EXECUTE ON FUNCTION pg_create_restore_point(text) TO rs_admin_backup;
GRANT EXECUTE ON FUNCTION pg_switch_wal() TO rs_admin_backup;

-- The migrator owns the database and public schema so it can apply DDL
-- without superuser rights.
ALTER DATABASE :"dbname" OWNER TO rs_admin_migrator;
ALTER SCHEMA public OWNER TO rs_admin_migrator;

-- Untrusted extensions are installed here by the superuser; the migrator's
-- IF NOT EXISTS statements then become no-ops.
CREATE EXTENSION IF NOT EXISTS pg_trgm;
CREATE EXTENSION IF NOT EXISTS pg_stat_statements;

-- No anonymous connections; only the application and monitoring roles may
-- connect.
REVOKE CONNECT ON DATABASE :"dbname" FROM PUBLIC;
GRANT CONNECT ON DATABASE :"dbname"
    TO rs_admin_migrator, rs_admin_runtime, rs_admin_cleanup, rs_admin_backup,
       rs_admin_monitor;
SQL
