-- Add a composite index on `requests` to back the proposer's hot lookup/count
-- queries. Previously the table had no secondary indexes, so every filtered
-- query (e.g. the per-loop `fetch_request_count`) did a sequential scan, which
-- degrades as the table grows and shows up as sqlx "slow statement" warnings:
--
--   SELECT COUNT(*) FROM requests
--   WHERE range_vkey_commitment = $1 AND rollup_config_hash = $2
--     AND status = $3 AND l1_chain_id = $4 AND l2_chain_id = $5
--
-- Nearly every query in validity/src/db/client.rs filters on the same
-- deployment-identity tuple (l1_chain_id, l2_chain_id, range_vkey_commitment,
-- rollup_config_hash) via equality, then narrows by status / req_type /
-- start_block. Those four leading columns are effectively constant for a single
-- deployment, so the selectivity comes from (status, req_type, start_block) --
-- keeping them in the index turns the counts and range lookups into index
-- range scans instead of full-table scans.
--
-- NOTE: plain (non-CONCURRENT) CREATE INDEX because sqlx runs each migration
-- inside a transaction. It briefly write-locks `requests` while building; that
-- is fine for the single-writer proposer. If this is ever applied to a very
-- large table where the lock matters, build it out-of-band with
-- `CREATE INDEX CONCURRENTLY` and then mark this migration as applied.
CREATE INDEX IF NOT EXISTS idx_requests_lookup
ON requests (
    l1_chain_id,
    l2_chain_id,
    range_vkey_commitment,
    rollup_config_hash,
    status,
    req_type,
    start_block
);
