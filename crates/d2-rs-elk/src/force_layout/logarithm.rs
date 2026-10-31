// Copyright 2009 The Go Authors. All rights reserved.
// BSD-3-Clause; see ../../GO-MATH-LICENSE.txt.
// Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.
// Developed at SunPro, a Sun Microsystems, Inc. business.
// Permission to use, copy, modify, and distribute this software is freely
// granted, provided that this notice is preserved.
// Translation of pinned Go1.27 math/log.go and math/frexp.go. The arm64
// fused evaluation order follows the retained math.log disassembly.

fn product_sum(a: f64, b: f64, c: f64) -> f64 {
    #[cfg(target_arch = "aarch64")]
    {
        a.mul_add(b, c)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        a * b + c
    }
}
pub(super) fn log(mut x: f64) -> f64 {
    if x.is_nan() || x == f64::INFINITY {
        return x;
    }
    if x < 0.0 {
        return f64::from_bits(0x7ff8000000000001);
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    let mut exponent = 0;
    if x < f64::MIN_POSITIVE {
        x *= 4503599627370496.0;
        exponent = -52;
    }
    let bits = x.to_bits();
    exponent += ((bits >> 52) & 0x7ff) as i32 - 1022;
    let mut fraction = f64::from_bits((bits & 0x000fffffffffffff) | 0x3fe0000000000000);
    if fraction < std::f64::consts::FRAC_1_SQRT_2 {
        fraction *= 2.0;
        exponent -= 1;
    }
    let f = fraction - 1.0;
    let k = exponent as f64;
    let s = f / (2.0 + f);
    let s2 = s * s;
    let s4 = s2 * s2;
    let l1 = f64::from_bits(0x3fe5555555555593);
    let l2 = f64::from_bits(0x3fd999999997fa04);
    let l3 = f64::from_bits(0x3fd2492494229359);
    let l4 = f64::from_bits(0x3fcc71c51d8e78af);
    let l5 = f64::from_bits(0x3fc7466496cb03de);
    let l6 = f64::from_bits(0x3fc39a09d078c69f);
    let l7 = f64::from_bits(0x3fc2f112df3e5244);
    let odd = product_sum(s4, product_sum(s4, product_sum(s4, l7, l5), l3), l1);
    let even = product_sum(s4, product_sum(s4, l6, l4), l2);
    let r = product_sum(s2, odd, s4 * even);
    let half_f = 0.5 * f;
    let sum = product_sum(
        s,
        product_sum(half_f, f, r),
        k * f64::from_bits(0x3dea39ef35793c76),
    );
    let correction = product_sum(half_f, f, -sum) - f;
    product_sum(k, f64::from_bits(0x3fe62e42fee00000), -correction)
}

#[cfg(all(test, target_arch = "aarch64"))]
mod tests {
    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn pinned_go_arm64_logarithm_bits() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "src/fixtures/original_force_logarithm.json"
        ))
        .unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 1011);
        for row in rows {
            let input = row["input_bits"].as_u64().unwrap();
            let expected = row["output_bits"].as_u64().unwrap();
            assert_eq!(
                super::log(f64::from_bits(input)).to_bits(),
                expected,
                "Go arm64 Log for input {input:016x}"
            );
        }
    }
}
