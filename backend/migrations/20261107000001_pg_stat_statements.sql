-- pg_stat_statements requires shared_preload_libraries. Create it when the
-- server is configured accordingly (development Compose and production);
-- test servers that do not preload the library skip it.
DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM pg_settings
        WHERE name = 'shared_preload_libraries'
          AND setting LIKE '%pg_stat_statements%'
    ) THEN
        CREATE EXTENSION IF NOT EXISTS pg_stat_statements;
    END IF;
END
$$;
