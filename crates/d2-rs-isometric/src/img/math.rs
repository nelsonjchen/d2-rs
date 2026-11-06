// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
// Go math/asin.go: Copyright 2009 The Go Authors. All rights reserved.
// Go BSD-3-Clause terms are retained in crates/d2-rs-render/THIRD_PARTY_NOTICES.md.
//! Scalar adapters missing from the shared Go math facade.
pub use d2_rs_render::go_float::gomath::*;
pub fn ceil(x: f64) -> f64 {
    x.ceil()
}
pub fn floor(x: f64) -> f64 {
    x.floor()
}
pub fn sqrt(x: f64) -> f64 {
    x.sqrt()
}
/// Go math/asin.go; atan2's positive-quadrant kernel is the same satan.
pub fn asin(mut x: f64) -> f64 {
    if x == 0.0 {
        return x;
    }
    let negative = x < 0.0;
    if negative {
        x = -x;
    }
    if x > 1.0 {
        return f64::from_bits(0x7ff8000000000001);
    }
    let temp = d2_rs_render::go_float::go_fmsub(x, x, 1.0).sqrt();
    let result = if x > 0.7 {
        std::f64::consts::FRAC_PI_2 - atan2(temp, x)
    } else {
        atan2(x, temp)
    };
    if negative { -result } else { result }
}
