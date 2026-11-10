# Migrations

SQLx applies these files with `sqlx::migrate!` from the one-shot `migrate`
container. The API never migrates on startup.

## Rules

- **Forward only.** There are no down migrations. A failed release rolls the
  code back to the previous image tag and keeps the new schema.
- **Immutable.** Once a migration is in a released commit, never edit, rename
  or delete it; SQLx records a checksum in `_sqlx_migrations`. Add a new
  migration instead.
- **UTC timestamp prefix.** `YYYYMMDDHHMMSS_short_description.sql`; versions
  must be unique and must sort after every existing migration.
- **Expand/contract.** Add nullable columns, new tables and new indexes first;
  keep code compatible with both shapes, backfill in a later release, and drop
  the old object only after no supported code version reads it. A destructive
  migration needs an on-demand backup and an explicit release note.

`scripts/check-migrations.sh` enforces immutability and ordering and applies
the migrations to an empty database and to the previous release schema. CI
runs it against PostgreSQL 18 before the test suite:

```sh
./scripts/check-migrations.sh --base-ref origin/main --database-url "$DATABASE_URL"
```
