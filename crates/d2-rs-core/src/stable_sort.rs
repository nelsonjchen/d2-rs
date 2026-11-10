// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Stable slice sorting with one sort instantiation per element type.
//!
//! `slice::sort_by` and `slice::sort_by_key` instantiate the whole standard
//! library stable sort for every comparator closure. Across the workspace
//! that was 576 copies and about 3.4 MB of the CLI's machine code. These
//! methods pass the comparator as `&mut dyn FnMut`, so each element type gets
//! one copy of the sort.
//!
//! The order is exactly the standard library's: the same sort runs and calls
//! the comparator (or key function) on the same pairs in the same order.

use core::cmp::Ordering;

#[inline(never)]
fn sort_dyn<T>(values: &mut [T], compare: &mut dyn FnMut(&T, &T) -> Ordering) {
    values.sort_by(|left, right| compare(left, right));
}

/// Stable sorts that share one instantiation per element type. Use these
/// instead of `sort_by`, `sort_by_key` and `sort` outside hot loops.
pub trait StableSort<T> {
    /// Same order as [`slice::sort_by`].
    fn stable_sort_by<F>(&mut self, compare: F)
    where
        F: FnMut(&T, &T) -> Ordering;

    /// Same order as [`slice::sort_by_key`].
    fn stable_sort_by_key<K, F>(&mut self, key: F)
    where
        K: Ord,
        F: FnMut(&T) -> K;

    /// Same order as [`slice::sort`].
    fn stable_sort(&mut self)
    where
        T: Ord;
}

impl<T> StableSort<T> for [T] {
    #[inline]
    fn stable_sort_by<F>(&mut self, mut compare: F)
    where
        F: FnMut(&T, &T) -> Ordering,
    {
        sort_dyn(self, &mut compare);
    }

    #[inline]
    fn stable_sort_by_key<K, F>(&mut self, mut key: F)
    where
        K: Ord,
        F: FnMut(&T) -> K,
    {
        // slice::sort_by_key is sort_by(|a, b| f(a).cmp(&f(b))).
        sort_dyn(self, &mut |left, right| key(left).cmp(&key(right)));
    }

    #[inline]
    fn stable_sort(&mut self)
    where
        T: Ord,
    {
        // slice::sort orders by T::lt; the stable sort only asks whether the
        // comparison is Less, so mapping lt onto Less/Greater is equivalent.
        sort_dyn(self, &mut |left, right| {
            if left.lt(right) {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::StableSort;

    #[test]
    fn matches_standard_library_order_including_ties() {
        let mut state = 0x2545_f491_u64;
        let values: Vec<(u8, u16)> = (0..2000u16)
            .map(|index| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                ((state % 7) as u8, index)
            })
            .collect();

        let mut expected = values.clone();
        expected.sort_by_key(|value| value.0);
        let mut actual = values.clone();
        actual.stable_sort_by_key(|value| value.0);
        assert_eq!(actual, expected);

        let mut expected = values.clone();
        expected.sort_by(|left, right| right.0.cmp(&left.0));
        let mut actual = values.clone();
        actual.stable_sort_by(|left, right| right.0.cmp(&left.0));
        assert_eq!(actual, expected);

        let mut expected = values.clone();
        expected.sort();
        let mut actual = values;
        actual.stable_sort();
        assert_eq!(actual, expected);
    }

    #[test]
    fn calls_the_comparator_on_the_same_pairs() {
        let values: Vec<u32> = (0..300).map(|index| (index * 7919) % 101).collect();
        let mut expected_calls = Vec::new();
        let mut expected = values.clone();
        expected.sort_by(|left, right| {
            expected_calls.push((*left, *right));
            left.cmp(right)
        });
        let mut actual_calls = Vec::new();
        let mut actual = values;
        actual.stable_sort_by(|left, right| {
            actual_calls.push((*left, *right));
            left.cmp(right)
        });
        assert_eq!(actual, expected);
        assert_eq!(actual_calls, expected_calls);
    }
}
