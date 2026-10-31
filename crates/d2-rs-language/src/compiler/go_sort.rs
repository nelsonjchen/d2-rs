// SPDX-License-Identifier: MPL-2.0
//! Shared Go 1.27 `sort.Slice` compatibility path.
//!
//! Graph lowering uses the same unstable ordering semantics as the pinned
//! Go compiler, including equal-key permutations. The implementation is
//! owned by `d2-rs-core` so TALA and the compiler cannot drift apart.

pub(super) use d2_rs_core::go_sort::{sort_by, sort_by_index};
