-- [MANTLE] Roll back migration 7 (`07_chain_lock_owner.sql`) so a release without it — e.g.
-- `mantle-v1.6.x` — can start on a database the newer release has already migrated.
--
-- Why this is needed: sqlx refuses to start when the database records a migration the binary does
-- not ship ("migration 7 was previously applied but is missing in the resolved migrations"), so
-- the older release exits at startup.
--
-- Run it with every proposer for this database STOPPED, after stopping the newer release and
-- before starting the older one:
--
--     psql "$DATABASE_URL" -f validity/migrations-rollback/07_chain_lock_owner.sql
--
-- What it does:
-- - forgets migration 7, so the older release's migration check passes;
-- - clears `owner_id` / `lease_expires_at`, restoring the legacy row the older release expects:
--   on a later re-upgrade the newer release then treats the row as held for one lease after
--   `locked_at` (its legacy rule) instead of trusting a lease the older release never maintained.
-- The two columns themselves stay: the older release ignores them, and migration 7 uses
-- `ADD COLUMN IF NOT EXISTS`, so re-upgrading simply re-applies it.
--
-- Covered by `rollback_sql_lets_a_release_without_migration_7_start` in validity/src/db/client.rs.
BEGIN;
DELETE FROM _sqlx_migrations WHERE version = 7;
UPDATE chain_locks SET owner_id = NULL, lease_expires_at = NULL;
COMMIT;
