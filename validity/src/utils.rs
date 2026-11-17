use anyhow::{anyhow, Result};
use op_succinct_host_utils::fetcher::BlockInfo;
use std::{collections::HashMap, ops::Range};

/// Identifies gaps not covered by the given sub-ranges in the overall range.
///
/// This function takes an overall block range and a list of sorted sub-ranges.
/// It identifies and returns the gaps between these sub-ranges in the overall range.
///
/// # Example
///
/// ```
/// use op_succinct_validity::find_gaps;
///
/// let overall_start = 1;
/// let overall_end = 10;
/// let ranges = [(2, 5), (7, 9)];
///
/// let gaps = find_gaps(overall_start, overall_end, &ranges);
/// assert_eq!(gaps, [1..2, 5..7, 9..10]);
/// ```
pub fn find_gaps(overall_start: i64, overall_end: i64, ranges: &[(i64, i64)]) -> Vec<Range<i64>> {
    let mut gaps = Vec::new();
    let mut current_start = overall_start;

    for &(start, end) in ranges {
        if current_start < start {
            gaps.push(current_start..start);
        }
        current_start = end.max(current_start); // Ensure current_start doesn't move backward
    }

    if current_start < overall_end {
        gaps.push(current_start..overall_end);
    }

    merge_ranges(gaps)
}

/// Determines the block ranges to be proven based on a set of ranges and a gas limit.
///
/// Given a set of block ranges that overlap at most on the boundaries and a gas limit, this
/// function calculates and returns the specific block ranges that need to be proven based on
/// the gas used in each block. Ensures that all disjoint ranges are fully covered by creating
/// ranges that don't exceed the gas limit.
///
/// # Arguments
/// * `disjoint_ranges` - The ranges to split based on gas usage
/// * `gas_limit` - Maximum gas allowed per range
/// * `block_infos` - Map of block number to BlockInfo for all blocks in the ranges
///
/// # Example
///
/// ```ignore
/// use op_succinct_validity::get_ranges_to_prove_by_gas;
/// use std::collections::HashMap;
///
/// let disjoint_ranges = [(0, 4)];
/// let gas_limit = 100_000_000; // 100 million gas
/// let mut block_infos = HashMap::new();
/// // Add BlockInfo for blocks 1-4...
///
/// // If blocks 1-4 use 30M gas each:
/// let ranges_to_prove = get_ranges_to_prove_by_gas(&disjoint_ranges, gas_limit, &block_infos)?;
/// // Result: [(0, 3), (3, 4)] - first 3 blocks (90M gas), then last block (30M gas)
/// ```
pub fn get_ranges_to_prove_by_gas(
    disjoint_ranges: &[Range<i64>],
    evm_gas_limit: u64,
    range_proof_interval: i64,
    block_infos: &HashMap<i64, BlockInfo>,
) -> Result<Vec<Range<i64>>> {
    get_ranges_to_prove_by_weighted_gas(
        disjoint_ranges,
        evm_gas_limit,
        0,
        range_proof_interval,
        None,
        block_infos,
    )
}

/// [MANTLE] Submission checkpoints for gas-based splitting: `base + k × interval` (k ≥ 1), where
/// `base` is the latest proposed block and `interval` the effective submission interval, and `tip`
/// is the highest provable block (the end of the last gap). See
/// [`get_ranges_to_prove_by_weighted_gas`].
#[derive(Debug, Clone, Copy)]
pub struct SubmissionCheckpoints {
    pub base: i64,
    pub interval: i64,
    pub tip: i64,
}

impl SubmissionCheckpoints {
    fn is_checkpoint(&self, block: i64) -> bool {
        self.interval > 0 && block > self.base && (block - self.base) % self.interval == 0
    }
}

/// [MANTLE] [`get_ranges_to_prove_by_gas`], with each block weighted as
/// `gas_used + tx_gas_weight × user transactions` (`transaction_count − 1`: every block carries
/// one L1-attributes deposit). Proving cost has a per-transaction part that gas alone misses: at
/// the same gas, an all-transfer range costs ~3× a mixed one (6.1B vs 18.6B gas per 2280s proof),
/// so a gas-only limit either over-splits mixed traffic or lets transfer-heavy ranges through.
///
/// With `checkpoints`, no range crosses a submission checkpoint, so aggregations land on the same
/// `latest proposed + k × interval` grid as with block-based splitting:
/// 1. a block that would push the range over `evm_gas_limit` starts a new range;
/// 2. a range always ends at a checkpoint, whether or not it reached the limit;
/// 3. `range_proof_interval` still caps the blocks per range (counted from the last cut);
/// 4. the last piece of a gap is inserted, except at the tip (gap ending at `tip`): there it waits
///    for the next loop unless it reached the limit or `range_proof_interval`. A gap ending at an
///    existing range (e.g. a failed range being rebuilt) gets no more blocks, so it must be
///    inserted in full.
///
/// `tx_gas_weight = 0` with `checkpoints = None` is exactly `get_ranges_to_prove_by_gas`. See
/// MANTLE_CHANGES.md §3.15.
pub fn get_ranges_to_prove_by_weighted_gas(
    disjoint_ranges: &[Range<i64>],
    evm_gas_limit: u64,
    tx_gas_weight: u64,
    range_proof_interval: i64,
    checkpoints: Option<SubmissionCheckpoints>,
    block_infos: &HashMap<i64, BlockInfo>,
) -> Result<Vec<Range<i64>>> {
    let mut ranges = Vec::new();

    for &Range { start, end } in disjoint_ranges {
        let mut current_start = start;
        let mut accumulated_gas = 0u64;

        // Process blocks in the range (start+1 to end inclusive)
        // Note: We don't prove the start block, only use its hash as starting point
        for block_num in (start + 1)..=end {
            let block_info = block_infos.get(&(block_num)).ok_or(anyhow!(
                "Missing BlockInfo for block {} in range ({}, {})",
                block_num,
                start,
                end
            ))?;

            // Validate block number consistency
            if block_info.block_number as i64 != block_num {
                return Err(anyhow!(
                    "BlockInfo has inconsistent block number: expected {}, got {}",
                    block_num,
                    block_info.block_number
                ));
            }

            let block_weight = block_info.gas_used.saturating_add(
                tx_gas_weight.saturating_mul(block_info.transaction_count.saturating_sub(1)),
            );

            // Check if adding this block would exceed gas limit or block count limit
            if (accumulated_gas > 0 && accumulated_gas.saturating_add(block_weight) > evm_gas_limit) ||
                (range_proof_interval > 0 && block_num - current_start > range_proof_interval)
            {
                // Create a range from current_start to the previous block
                ranges.push(current_start..block_num - 1);
                current_start = block_num - 1;
                accumulated_gas = block_weight;
            } else {
                accumulated_gas = accumulated_gas.saturating_add(block_weight);
            }

            // [MANTLE] Rule 2: a range never crosses a submission checkpoint.
            if checkpoints.is_some_and(|c| c.is_checkpoint(block_num)) {
                ranges.push(current_start..block_num);
                current_start = block_num;
                accumulated_gas = 0;
            }
        }

        // Handle remaining blocks
        if current_start < end {
            // [MANTLE] Rule 4: at the tip, an unfinished piece waits for the next loop.
            let open_at_tip = checkpoints.is_some_and(|c| end == c.tip) &&
                accumulated_gas < evm_gas_limit &&
                (range_proof_interval <= 0 || end - current_start < range_proof_interval);
            if !open_at_tip {
                ranges.push(current_start..end);
            }
        }
    }

    // Remove the last range if it's empty
    if let Some(r) = ranges.last() {
        if r.is_empty() {
            ranges.pop();
        }
    }

    Ok(ranges)
}

/// Determines the block ranges to be proven based on a set of ranges and a specified interval.
///
/// Given a set block ranges that overlap at most on the boundaries and a range proof interval, this
/// function calculates and returns the specific block ranges that need to be proven. Ensures that
/// all disjoint ranges are fully covered by conditionally inserting a single range smaller than the
/// range proof interval if necessary.
///
/// # Example
///
/// ```
/// use op_succinct_validity::get_ranges_to_prove_by_blocks;
///
/// let disjoint_ranges = [0..50, 100..200, 200..210];
/// let range_proof_interval = 25;
///
/// let ranges_to_prove = get_ranges_to_prove_by_blocks(&disjoint_ranges, range_proof_interval);
/// assert_eq!(ranges_to_prove, [0..25, 25..50, 100..125, 125..150, 150..175, 175..200]);
/// ```
pub fn get_ranges_to_prove_by_blocks(
    disjoint_ranges: &[Range<i64>],
    range_proof_interval: i64,
) -> Vec<Range<i64>> {
    let mut ranges = Vec::new();

    for &Range { start, end } in disjoint_ranges {
        let mut current_start = start;
        while current_start < end {
            let current_end = std::cmp::min(current_start + range_proof_interval, end);
            ranges.push(current_start..current_end);
            current_start = current_end;
        }
    }

    // For the last range, remove it if it's less than range_proof_interval. This is to ensure when
    // inserting the ranges near the tip, only requests of size range_proof_interval are
    // inserted.
    if let Some(r) = ranges.last() {
        if r.end - r.start < range_proof_interval {
            ranges.pop();
        }
    }

    ranges
}

/// Merges overlapping or contiguous ranges into a single range.
///
/// Takes a vector of ranges and combines any that overlap or touch, returning
/// a sorted vector of non-overlapping ranges.
fn merge_ranges(mut ranges: Vec<Range<i64>>) -> Vec<Range<i64>> {
    if ranges.is_empty() {
        return ranges;
    }

    // Sort by start, then end if starts are equal.
    ranges.sort_unstable_by(|a, b| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));

    let mut merged = Vec::with_capacity(ranges.len());
    let mut current = ranges[0].clone();

    for range in ranges.into_iter().skip(1) {
        if range.start <= current.end {
            // Ranges overlap or are contiguous, merge them
            current.end = current.end.max(range.end);
        } else {
            // No overlap, push current and start new range
            merged.push(current);
            current = range;
        }
    }

    merged.push(current);
    merged
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;
    use op_succinct_host_utils::fetcher::BlockInfo;
    use std::collections::HashMap;

    // Tests for find_gaps
    macro_rules! test_find_gaps {
        ($name:ident, $overall_start:expr, $overall_end:expr, $ranges:expr, $expected:expr) => {
            #[test]
            fn $name() {
                let gaps = find_gaps($overall_start, $overall_end, $ranges);
                assert_eq!(gaps, $expected);
            }
        };
    }

    macro_rules! test_merge_ranges {
        ($name:ident, $disjoint_ranges:expr, $expected:expr) => {
            #[test]
            fn $name() {
                let ranges = merge_ranges($disjoint_ranges);
                assert_eq!(ranges, $expected);
            }
        };
    }

    test_find_gaps!(test_find_gaps_no_gaps, 1, 4, &[(1, 2), (2, 3), (3, 4)], &[]);
    test_find_gaps!(
        test_find_gaps_with_gaps,
        1,
        10,
        &[(1, 2), (4, 5), (7, 8)],
        &[2..4, 5..7, 8..10]
    );
    test_find_gaps!(test_find_gaps_at_start, 1, 6, &[(3, 4), (5, 6)], &[1..3, 4..5]);
    test_find_gaps!(test_find_gaps_at_end, 1, 5, &[(1, 2), (2, 3)], &[3..5]);
    test_find_gaps!(test_find_gaps_empty_ranges, 1, 5, &[], &[1..5]);
    test_find_gaps!(test_find_gaps_single_range, 1, 5, &[(2, 4)], &[1..2, 4..5]);

    test_merge_ranges!(test_merge_ranges_contiguous, vec![3..4, 4..7], vec![3..7]);
    test_merge_ranges!(test_merge_ranges_overlap, vec![3..6, 4..7], vec![3..7]);
    test_merge_ranges!(test_merge_ranges_disjoint, vec![3..5, 7..9], vec![3..5, 7..9]);

    // Tests for get_ranges_to_prove_by_blocks
    macro_rules! test_get_ranges_to_prove_by_blocks {
        ($name:ident, $disjoint_ranges:expr, $range_proof_interval:expr, $expected:expr) => {
            #[test]
            fn $name() {
                let result = get_ranges_to_prove_by_blocks($disjoint_ranges, $range_proof_interval);
                assert_eq!(result, $expected);
            }
        };
    }

    test_get_ranges_to_prove_by_blocks!(
        test_get_ranges_to_prove_case_1,
        &[0..50, 100..200, 200..210],
        25,
        &[0..25, 25..50, 100..125, 125..150, 150..175, 175..200]
    );

    test_get_ranges_to_prove_by_blocks!(
        test_get_ranges_to_prove_case_2,
        &[0..30, 40..70],
        10,
        &[0..10, 10..20, 20..30, 40..50, 50..60, 60..70]
    );

    test_get_ranges_to_prove_by_blocks!(
        test_get_ranges_to_prove_case_3,
        &[0..100],
        20,
        &[0..20, 20..40, 40..60, 60..80, 80..100]
    );

    test_get_ranges_to_prove_by_blocks!(
        test_get_ranges_to_prove_case_4,
        &[0..15, 20..35],
        5,
        &[0..5, 5..10, 10..15, 20..25, 25..30, 30..35]
    );

    test_get_ranges_to_prove_by_blocks!(
        test_get_ranges_to_prove_case_5,
        &[0..5, 10..15, 20..25],
        3,
        &[0..3, 3..5, 10..13, 13..15, 20..23]
    );

    test_get_ranges_to_prove_by_blocks!(
        test_get_ranges_to_prove_case_interval_larger_than_range,
        &[0..5, 10..15, 20..25],
        30,
        &[0..5, 10..15]
    );

    // Tests for get_ranges_to_prove_by_gas
    fn create_block_infos(block_data: Vec<(i64, i64)>) -> HashMap<i64, BlockInfo> {
        block_data
            .into_iter()
            .map(|(block_number, gas_used)| {
                (
                    block_number,
                    BlockInfo {
                        block_number: block_number as u64,
                        transaction_count: 10,
                        gas_used: gas_used as u64,
                        total_l1_fees: 1000,
                        total_tx_fees: 2000,
                    },
                )
            })
            .collect()
    }

    // [MANTLE] Tests for get_ranges_to_prove_by_weighted_gas (§3.15).
    fn weighted_block_infos(blocks: &[(i64, u64, u64)]) -> HashMap<i64, BlockInfo> {
        blocks
            .iter()
            .map(|&(block_number, gas_used, transaction_count)| {
                (
                    block_number,
                    BlockInfo {
                        block_number: block_number as u64,
                        transaction_count,
                        gas_used,
                        total_l1_fees: 0,
                        total_tx_fees: 0,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn weighted_with_zero_weight_equals_gas_only() {
        let blocks = [
            (1, 50_000_000, 900),
            (2, 20_000_000, 10),
            (3, 40_000_000, 1),
            (4, 10_000_000, 5000),
            (5, 60_000_000, 2),
        ];
        let infos = weighted_block_infos(&blocks);
        for interval in [0, 2, 600] {
            assert_eq!(
                get_ranges_to_prove_by_weighted_gas(
                    &[0..5],
                    100_000_000,
                    0,
                    interval,
                    None,
                    &infos
                )
                .unwrap(),
                get_ranges_to_prove_by_gas(&[0..5], 100_000_000, interval, &infos).unwrap(),
            );
        }
    }

    /// Transfer-heavy blocks: low gas but many transactions. Gas alone (60M) fits in one range;
    /// with 72k gas per user transaction each block weighs 20M + 72k × 999 ≈ 92M.
    #[test]
    fn weighted_splits_transaction_heavy_blocks() {
        let infos = weighted_block_infos(&[
            (1, 20_000_000, 1000),
            (2, 20_000_000, 1000),
            (3, 20_000_000, 1000),
        ]);
        assert_eq!(
            get_ranges_to_prove_by_gas(&[0..3], 100_000_000, 600, &infos).unwrap(),
            vec![0..3]
        );
        assert_eq!(
            get_ranges_to_prove_by_weighted_gas(&[0..3], 100_000_000, 72_000, 600, None, &infos)
                .unwrap(),
            vec![0..1, 1..2, 2..3]
        );
    }

    /// Gas-heavy blocks with few transactions split (almost) as with gas alone.
    #[test]
    fn weighted_leaves_gas_heavy_blocks_as_gas_only() {
        let infos =
            weighted_block_infos(&[(1, 40_000_000, 3), (2, 40_000_000, 3), (3, 40_000_000, 3)]);
        assert_eq!(
            get_ranges_to_prove_by_weighted_gas(&[0..3], 100_000_000, 72_000, 600, None, &infos)
                .unwrap(),
            get_ranges_to_prove_by_gas(&[0..3], 100_000_000, 600, &infos).unwrap(),
        );
    }

    /// The L1-attributes deposit (one per block) is not weighted; an empty block weighs nothing
    /// beyond its gas.
    #[test]
    fn weighted_ignores_the_system_transaction() {
        let infos = weighted_block_infos(&[(1, 0, 1), (2, 0, 1), (3, 0, 0)]);
        assert_eq!(
            get_ranges_to_prove_by_weighted_gas(&[0..3], 1, u64::MAX, 600, None, &infos).unwrap(),
            vec![0..3]
        );
    }

    /// A single block heavier than the limit still forms its own range; the block interval still
    /// applies; huge weights saturate instead of overflowing.
    #[test]
    fn weighted_single_heavy_block_interval_and_saturation() {
        let infos = weighted_block_infos(&[
            (1, 1_000, 10),
            (2, 1_000, u64::MAX),
            (3, 1_000, 10),
            (4, 1_000, 10),
            (5, 1_000, 10),
        ]);
        assert_eq!(
            get_ranges_to_prove_by_weighted_gas(&[0..5], 100_000, u64::MAX, 600, None, &infos)
                .unwrap(),
            vec![0..1, 1..2, 2..3, 3..4, 4..5]
        );
        let light = weighted_block_infos(&[
            (1, 1_000, 2),
            (2, 1_000, 2),
            (3, 1_000, 2),
            (4, 1_000, 2),
            (5, 1_000, 2),
        ]);
        assert_eq!(
            get_ranges_to_prove_by_weighted_gas(&[0..5], 100_000_000, 72_000, 2, None, &light)
                .unwrap(),
            vec![0..2, 2..4, 4..5]
        );
    }

    // [MANTLE] Checkpoint-aligned splitting (§3.15). Blocks 1..=n with the given weights (gas only,
    // one tx each so the tx weight never applies); checkpoints at base + k × interval.
    fn cp_infos(weights: &[u64]) -> HashMap<i64, BlockInfo> {
        let blocks: Vec<(i64, u64, u64)> =
            weights.iter().enumerate().map(|(i, &w)| (i as i64 + 1, w, 1)).collect();
        weighted_block_infos(&blocks)
    }

    fn cp(base: i64, interval: i64, tip: i64) -> Option<SubmissionCheckpoints> {
        Some(SubmissionCheckpoints { base, interval, tip })
    }

    fn split(
        gaps: &[Range<i64>],
        limit: u64,
        interval: i64,
        checkpoints: Option<SubmissionCheckpoints>,
        infos: &HashMap<i64, BlockInfo>,
    ) -> Vec<Range<i64>> {
        get_ranges_to_prove_by_weighted_gas(gaps, limit, 0, interval, checkpoints, infos).unwrap()
    }

    /// Idle / low load: one range per checkpoint period, same as block-based splitting.
    #[test]
    fn checkpoint_idle_one_range_per_period() {
        let infos = cp_infos(&[1; 36]);
        assert_eq!(split(&[0..36], 1_000, 18, cp(0, 18, 36), &infos), vec![0..18, 18..36]);
        assert_eq!(get_ranges_to_prove_by_blocks(&[0..36], 18), vec![0..18, 18..36]);
    }

    /// High load: the limit cuts at blocks 7 and 14 inside an 18-block period; the last piece ends
    /// at the checkpoint and is inserted at once.
    #[test]
    fn checkpoint_high_load_cuts_inside_the_period() {
        let infos = cp_infos(&[10; 18]);
        assert_eq!(split(&[0..18], 70, 18, cp(0, 18, 18), &infos), vec![0..7, 7..14, 14..18]);
    }

    /// The case that motivated this: the 18th block of the period would push the range over the
    /// limit. It becomes its own one-block range ending at the checkpoint and is not carried into
    /// the next period.
    #[test]
    fn checkpoint_last_block_of_period_is_not_carried_over() {
        let mut w = vec![1u64; 36];
        w[17] = 100; // block 18
        let infos = cp_infos(&w);
        assert_eq!(split(&[0..36], 100, 18, cp(0, 18, 36), &infos), vec![0..17, 17..18, 18..36]);
    }

    /// Catching up: several periods in one pass, no range crosses a checkpoint.
    #[test]
    fn checkpoint_catch_up_never_crosses_a_checkpoint() {
        let infos = cp_infos(&[10; 54]);
        let ranges = split(&[0..54], 100, 18, cp(0, 18, 54), &infos);
        assert_eq!(ranges, vec![0..10, 10..18, 18..28, 28..36, 36..46, 46..54]);
        for r in &ranges {
            assert!((r.start + 1..r.end).all(|b| b % 18 != 0), "{r:?} crosses a checkpoint");
        }
    }

    /// At the tip, a piece that reached neither the limit nor a checkpoint waits for the next loop;
    /// once a checkpoint is reached it is inserted.
    #[test]
    fn checkpoint_open_tail_at_tip_waits() {
        let infos = cp_infos(&[1; 18]);
        assert_eq!(split(&[0..10], 1_000, 18, cp(0, 18, 10), &infos), Vec::<Range<i64>>::new());
        assert_eq!(split(&[0..18], 1_000, 18, cp(0, 18, 18), &infos), vec![0..18]);
        // A piece at the tip that already reached the limit is inserted (block 2 alone is 120);
        // one still under it (60) waits.
        let heavy = cp_infos(&[60, 120]);
        assert_eq!(split(&[0..2], 100, 18, cp(0, 18, 2), &heavy), vec![0..1, 1..2]);
        let under = cp_infos(&[60, 60]);
        assert_eq!(split(&[0..2], 100, 18, cp(0, 18, 2), &under), vec![0..1]);
    }

    /// A gap that ends at an existing range (e.g. a failed range being rebuilt) gets no more
    /// blocks, so its last piece is inserted even though it reached neither limit.
    #[test]
    fn checkpoint_gap_ending_at_an_existing_range_is_inserted() {
        let infos = cp_infos(&[1; 40]);
        // Gaps 0..5 (mid, bounded by an existing range 5..30) and 30..40 (tip at 40).
        assert_eq!(split(&[0..5, 30..40], 1_000, 18, cp(0, 18, 40), &infos), vec![0..5, 30..36]);
    }

    /// RANGE_PROOF_INTERVAL smaller than the submission interval still caps blocks per range;
    /// ranges stay inside the period.
    #[test]
    fn checkpoint_with_smaller_range_proof_interval() {
        let infos = cp_infos(&[1; 18]);
        assert_eq!(
            split(&[0..18], 1_000, 5, cp(0, 18, 18), &infos),
            vec![0..5, 5..10, 10..15, 15..18]
        );
    }

    /// Checkpoints are counted from the latest proposed block, not from multiples of the interval.
    #[test]
    fn checkpoint_base_is_the_latest_proposed_block() {
        let blocks: Vec<(i64, u64, u64)> = (1235..=1270).map(|b| (b, 1, 1)).collect();
        let infos = weighted_block_infos(&blocks);
        assert_eq!(
            split(&[1234..1270], 1_000, 18, cp(1234, 18, 1270), &infos),
            vec![1234..1252, 1252..1270]
        );
    }

    #[test]
    fn test_basic_gas_splitting() {
        // Test case: 4 blocks of 30M gas each with 100M limit
        // Expected: First 3 blocks (90M gas), then last block (30M gas)
        let block_infos = create_block_infos(vec![
            (1, 30_000_000),
            (2, 30_000_000),
            (3, 30_000_000),
            (4, 30_000_000),
        ]);

        let result = get_ranges_to_prove_by_gas(&[0..4], 100_000_000, 600, &block_infos).unwrap();
        assert_eq!(result, vec![0..3, 3..4]);
    }

    #[test]
    fn test_varying_gas_amounts() {
        // Test case: blocks with varying gas amounts
        let block_infos = create_block_infos(vec![
            (1, 50_000_000), // 50M
            (2, 20_000_000), // 70M total - still under limit
            (3, 40_000_000), // 110M total - would exceed, so create range (0, 2)
            (4, 10_000_000), // 50M total with block 3
            (5, 60_000_000), // 110M total - would exceed, so create range (2, 4)
        ]);

        let result = get_ranges_to_prove_by_gas(&[0..5], 100_000_000, 600, &block_infos).unwrap();
        assert_eq!(result, vec![0..2, 2..4, 4..5]);
    }

    #[test]
    fn test_exact_gas_limit_match() {
        // Test case: blocks that exactly match gas limit
        let block_infos = create_block_infos(vec![
            (1, 50_000_000),  // 50M
            (2, 50_000_000),  // 100M total - exactly at limit
            (3, 100_000_000), // Would be 200M total - exceeds limit
        ]);

        let result = get_ranges_to_prove_by_gas(&[0..3], 100_000_000, 600, &block_infos).unwrap();
        assert_eq!(result, vec![0..2, 2..3]);
    }

    #[test]
    fn test_multiple_ranges() {
        // Test case: multiple disjoint ranges
        let block_infos = create_block_infos(vec![
            // First range
            (1, 40_000_000), // 40M
            (2, 40_000_000), // 80M total
            (3, 40_000_000), // 120M total - exceeds limit
            // Gap from 4-5
            // Second range
            (6, 60_000_000), // 60M
            (7, 60_000_000), // 120M total - exceeds limit
        ]);

        let result =
            get_ranges_to_prove_by_gas(&[0..3, 5..7], 100_000_000, 600, &block_infos).unwrap();
        assert_eq!(result, vec![0..2, 2..3, 5..6, 6..7]);
    }

    #[test]
    fn test_max_interval_and_some_empty_blocks() {
        // Test case: max interval and some empty blocks
        let block_infos = create_block_infos(vec![
            // First range
            (1, 40_000_000), // 40M
            (2, 40_000_000), // 80M total
            // Second range
            (3, 40_000_000), // 40M
            (4, 0),
            (5, 0),
            // Third range
            (6, 0),
            (7, 60_000_000), // 60M
        ]);

        let result = get_ranges_to_prove_by_gas(&[0..7], 100_000_000, 3, &block_infos).unwrap();
        assert_eq!(result, vec![0..2, 2..5, 5..7]);
    }

    #[test]
    fn test_max_interval_and_small_blocks() {
        // Test case: multiple small blocks
        let block_infos = create_block_infos(vec![
            // First range
            (1, 1_000),
            (2, 1_000),
            (3, 1_000),
            // Second range
            (4, 1_000),
            (5, 1_000),
            (6, 1_000),
            // Third range
            (7, 1_000),
            (8, 1_000),
        ]);

        let result = get_ranges_to_prove_by_gas(&[0..8], 100_000_000, 3, &block_infos).unwrap();
        assert_eq!(result, vec![0..3, 3..6, 6..8]);
    }

    #[test]
    fn test_zero_max_interval() {
        // Test case: multiple small blocks
        let block_infos = create_block_infos(vec![
            // First range
            (1, 40_000),
            (2, 20_000),
            (3, 20_000),
            // Second range
            (4, 40_000),
            (5, 20_000),
            (6, 20_000),
            // Third range
            (7, 40_000),
            (8, 40_000),
        ]);

        let result = get_ranges_to_prove_by_gas(&[0..8], 100_000, 0, &block_infos).unwrap();
        assert_eq!(result, vec![0..3, 3..6, 6..8]);
    }

    #[test]
    fn test_single_large_block() {
        // Test case: single block exceeding gas limit
        let block_infos = create_block_infos(vec![(1, 150_000_000)]);

        let result = get_ranges_to_prove_by_gas(&[0..1], 100_000_000, 600, &block_infos).unwrap();
        assert_eq!(result, vec![0..1]);
    }

    #[test]
    fn test_single_small_block() {
        // Test case: single block not exceeding gas limit
        let block_infos = create_block_infos(vec![(1, 10_000)]);

        let result = get_ranges_to_prove_by_gas(&[0..1], 100_000_000, 600, &block_infos).unwrap();
        assert_eq!(result, vec![0..1]);
    }

    #[test]
    fn test_empty_range() {
        // Test case: empty range (start == end)
        let block_infos = HashMap::new();

        let result = get_ranges_to_prove_by_gas(&[0..0], 100_000_000, 600, &block_infos).unwrap();
        assert_eq!(result, vec![]);
    }

    #[test]
    fn test_missing_block_info_validation() {
        // Test case: missing block info should return error
        let block_infos = create_block_infos(vec![(1, 30_000_000)]); // Missing block 2

        let result = get_ranges_to_prove_by_gas(&[0..2], 100_000_000, 600, &block_infos);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Missing BlockInfo for block 2"));
    }

    #[test]
    fn test_inconsistent_block_number_validation() {
        // Test case: block info with wrong block number should return error
        let mut block_infos = HashMap::new();
        block_infos.insert(
            1,
            BlockInfo {
                block_number: 99, // Wrong block number
                transaction_count: 10,
                gas_used: 30_000_000,
                total_l1_fees: 1000,
                total_tx_fees: 2000,
            },
        );

        let result = get_ranges_to_prove_by_gas(&[0..1], 100_000_000, 600, &block_infos);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("inconsistent block number"));
    }

    // Block-based splitting tests

    #[test]
    fn test_basic_block_splitting() {
        let result = get_ranges_to_prove_by_blocks(&[0..50, 100..200, 200..210], 25);
        assert_eq!(result, vec![0..25, 25..50, 100..125, 125..150, 150..175, 175..200]);
    }

    #[test]
    fn test_block_splitting_exact_intervals() {
        let result = get_ranges_to_prove_by_blocks(&[0..30, 40..70], 10);
        assert_eq!(result, vec![0..10, 10..20, 20..30, 40..50, 50..60, 60..70]);
    }

    #[test]
    fn test_block_splitting_removes_small_last_range() {
        let result = get_ranges_to_prove_by_blocks(&[0..5, 10..15, 20..25], 30);
        assert_eq!(result, vec![0..5, 10..15]);
    }
}
