-- Records the L1 identity used by range proofs and logical invalidation after an L1 reorg.
--
-- [MANTLE] Renumbered from upstream's `05_` — that number is already taken by
-- `05_add_requests_indexes.sql` here, and deployed databases have recorded a checksum for ours.
-- The filenames differ, so git merges both without reporting a conflict; sqlx indexes by version
-- number and the two cannot coexist. `IF NOT EXISTS` matches the style of `02_` and keeps this
-- re-runnable if the columns were ever added out of band.
ALTER TABLE requests
ADD COLUMN IF NOT EXISTS l1_head_block_hash BYTEA,
ADD COLUMN IF NOT EXISTS invalidated_at TIMESTAMP;
