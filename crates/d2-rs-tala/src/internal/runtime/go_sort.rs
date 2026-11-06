// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// Third-party portions retain their terms; see THIRD_PARTY_NOTICES.md.

//! Compatibility shim for the shared Go 1.27.0 `sort.Slice` translation.
//!
//! Keeping this module path preserves TALA's existing internal call sites and
//! tests while the implementation lives in `d2-rs-core` for reuse by the
//! compiler's graph ordering boundary.

pub(super) use d2_rs_core::go_sort::sort_by;

#[cfg(test)]
mod tests {
    use super::sort_by;

    #[test]
    fn short_equal_runs_remain_in_insertion_order() {
        let mut values = vec![(2, 'a'), (1, 'b'), (1, 'c'), (2, 'd')];
        sort_by(&mut values, |left, right| left.0 < right.0);
        assert_eq!(values, vec![(1, 'b'), (1, 'c'), (2, 'a'), (2, 'd')]);
    }

    #[test]
    fn long_equal_runs_match_go_pdqsort_permutation() {
        let areas = [
            12_000, 12_000, 12_000, 12_000, 12_000, 24_000, 8_000, 16_000, 16_000, 48_800, 69_600,
            6_006, 7_722, 5_808, 6_270, 9_174,
        ];
        let mut values: Vec<_> = areas.into_iter().enumerate().collect();
        sort_by(&mut values, |left, right| left.1 > right.1);
        assert_eq!(
            values.iter().map(|value| value.0).collect::<Vec<_>>(),
            vec![10, 9, 5, 8, 7, 4, 0, 3, 2, 1, 15, 6, 12, 14, 11, 13]
        );
    }

    #[test]
    fn unbalanced_equal_partition_matches_go_1_27_pattern_breaking() {
        let areas = [
            10_000, 10_000, 10_000, 10_000, 10_000, 10_000, 10_000, 10_000, 10_000, 10_000, 10_000,
            6_864, 10_000, 10_000,
        ];
        let mut values: Vec<_> = areas.into_iter().enumerate().collect();
        sort_by(&mut values, |left, right| left.1 > right.1);
        assert_eq!(
            values.iter().map(|value| value.0).collect::<Vec<_>>(),
            vec![6, 12, 2, 3, 4, 5, 8, 0, 1, 9, 10, 13, 7, 11]
        );
    }
}
