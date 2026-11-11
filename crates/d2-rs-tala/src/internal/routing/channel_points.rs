// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Identity-preserving channelPointProposal/channelWireLength translation.
//! IDs refer to actual shared point slots, never coordinate-based deduplication.

// Translation of Go 1.27 src/math/hypot.go, used on the pinned ARM64 oracle.
// The ARM64 Go implementation lowers Sqrt to the platform FSQRTD instruction;
// keeping this helper explicit makes the operation order in Hypot visible.
// Copyright 2010 The Go Authors. All rights reserved.
// BSD terms: see THIRD_PARTY_NOTICES.md.
pub(in crate::internal) fn go_sqrt(x: f64) -> f64 {
    x.sqrt()
}

pub(in crate::internal) fn go_hypot(mut p: f64, mut q: f64) -> f64 {
    p = p.abs();
    q = q.abs();
    if p == f64::INFINITY || q == f64::INFINITY {
        return f64::INFINITY;
    }
    if p.is_nan() || q.is_nan() {
        return f64::from_bits(0x7ff8_0000_0000_0001);
    }
    if p < q {
        std::mem::swap(&mut p, &mut q);
    }
    if p == 0.0 {
        return 0.0;
    }
    q /= p;
    // Go's ARM64 compiler fuses `1+q*q` in math.hypot into an FMA before
    // calling FSQRTD. Preserve that intermediate exactly; a separate multiply
    // and add can be one ULP lower.
    p * go_sqrt(d2_rs_core::fused_mul_add(q, q, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_hypot_matches_go_arm64_result() {
        assert_eq!(
            go_sqrt((142.0_f64 / 283.5).mul_add(142.0_f64 / 283.5, 1.0)).to_bits(),
            0x3ff1e5156cabb585
        );
        assert_eq!(go_hypot(-283.5, -142.0).to_bits(), 0x4073d13139d82785);
        assert_eq!(go_hypot(-212.5, -142.0).to_bits(), 0x406ff281277ca342);
    }
}
