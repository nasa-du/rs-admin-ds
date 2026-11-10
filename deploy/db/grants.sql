-- Role grants applied by the one-shot migrate job after the SQLx migrations.
-- Run as rs_admin_migrator, which owns every application table.
--
-- Runtime: DML only, no DDL. audit_logs is append-only and the chain head may
-- only be read and advanced. _sqlx_migrations is readable for readiness.

GRANT USAGE ON SCHEMA public TO rs_admin_runtime, rs_admin_cleanup;
GRANT SELECT ON TABLE _sqlx_migrations TO rs_admin_runtime;

DO $$
DECLARE
    table_name text;
BEGIN
    FOR table_name IN
        SELECT tablename
        FROM pg_tables
        WHERE schemaname = 'public'
          AND tablename NOT IN ('_sqlx_migrations', 'audit_logs', 'audit_chain_head')
    LOOP
        EXECUTE format(
            'GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE public.%I TO rs_admin_runtime',
            table_name
        );
    END LOOP;
END
$$;

GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO rs_admin_runtime;

-- Audit rows are immutable for the runtime role.
REVOKE ALL ON TABLE audit_logs FROM rs_admin_runtime;
GRANT SELECT, INSERT ON TABLE audit_logs TO rs_admin_runtime;

REVOKE ALL ON TABLE audit_chain_head FROM rs_admin_runtime;
GRANT SELECT, UPDATE ON TABLE audit_chain_head TO rs_admin_runtime;

-- The cleanup role may delete expired rows (including audit rows) and advance
-- the chain head after inserting the purge audit event. It only needs access
-- to the retention-managed tables, not to users or roles.
GRANT SELECT, DELETE ON TABLE
    auth_refresh_tokens,
    auth_sessions,
    auth_login_failures,
    auth_login_throttle
TO rs_admin_cleanup;
GRANT SELECT, INSERT, DELETE ON TABLE audit_logs TO rs_admin_cleanup;
GRANT SELECT, UPDATE ON TABLE audit_chain_head TO rs_admin_cleanup;

-- Future tables created by the migrator inherit the same runtime rights.
ALTER DEFAULT PRIVILEGES IN SCHEMA public
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO rs_admin_runtime;
ALTER DEFAULT PRIVILEGES IN SCHEMA public
    GRANT USAGE, SELECT ON SEQUENCES TO rs_admin_runtime;
