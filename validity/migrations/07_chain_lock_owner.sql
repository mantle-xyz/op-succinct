-- [MANTLE] Owner-scoped chain lock lease.
--
-- The lock used to be a bare `locked_at` timestamp, checked and written by two separate
-- statements and refreshed by an unconditional UPDATE: two proposers could both pass the check,
-- and either one could keep the other's lease alive. `owner_id` names the holding process, and
-- `lease_expires_at` is the database-clock deadline that acquisition and renewal compare against
-- (see `DriverDBClient::try_acquire_chain_lock` / `renew_chain_lock`).
--
-- Both columns are nullable so a row written by an older binary (owner_id NULL) stays valid; the
-- acquire statement treats such a row as held until `locked_at` is older than one lease.
-- Additive only — an older binary ignores both columns.
ALTER TABLE chain_locks ADD COLUMN IF NOT EXISTS owner_id TEXT;
ALTER TABLE chain_locks ADD COLUMN IF NOT EXISTS lease_expires_at TIMESTAMPTZ;
