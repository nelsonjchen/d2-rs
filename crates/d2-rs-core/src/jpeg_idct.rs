// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
// Copyright 2025 The Go Authors. All rights reserved.
// Go BSD-3-Clause conditions and disclaimer: THIRD_PARTY_NOTICES.md.
//! Go1.27 image/jpeg/dct.go integer inverse DCT, including signed overflow.
//! This is the native block reconstruction stage; JPEG entropy decoding remains separate.
fn add(a: i32, b: i32) -> i32 {
    a.wrapping_add(b)
}
fn sub(a: i32, b: i32) -> i32 {
    a.wrapping_sub(b)
}
fn mul(a: i32, b: i32) -> i32 {
    a.wrapping_mul(b)
}
fn rotation(a: i32, b: i32, cos: i32, sin: i32) -> (i32, i32) {
    let sum = mul(cos, add(a, b));
    (
        add(sum, mul(sub(sin, cos), b)),
        sub(sum, mul(add(cos, sin), a)),
    )
}

/// Transforms a natural-order 8x8 dequantized coefficient block in place.
#[doc(hidden)]
pub fn inverse(block: &mut [i32; 64]) {
    for row in block.chunks_exact_mut(8) {
        let [
            mut x0,
            mut x7,
            mut x2,
            mut x5,
            mut x1,
            mut x6,
            mut x3,
            mut x4,
        ] = *<&[i32; 8]>::try_from(&*row).unwrap();
        x0 = x0.wrapping_shl(17);
        x1 = x1.wrapping_shl(17);
        (x0, x1) = (add(x0, x1), sub(x0, x1));
        (x2, x3) = rotation(x2, x3, 70936, -171254);
        (x1, x2) = (add(x1, x2), sub(x1, x2));
        (x0, x3) = (add(x0, x3), sub(x0, x3));
        x4 = x4.wrapping_shl(7);
        x7 = x7.wrapping_shl(7);
        (x7, x4) = (add(x7, x4), sub(x7, x4));
        x6 = mul(x6, 181);
        x5 = mul(x5, 181);
        (x7, x5) = (add(x7, x5), sub(x7, x5));
        (x4, x6) = (add(x4, x6), sub(x4, x6));
        (x4, x7) = rotation(x4 >> 2, x7 >> 2, 3406, -2276);
        (x5, x6) = rotation(x5 >> 2, x6 >> 2, 4017, -799);
        (x0, x7) = (add(x0, x7), sub(x0, x7));
        (x1, x6) = (add(x1, x6), sub(x1, x6));
        (x2, x5) = (add(x2, x5), sub(x2, x5));
        (x3, x4) = (add(x3, x4), sub(x3, x4));
        row.copy_from_slice(&[x0, x1, x2, x3, x4, x5, x6, x7]);
    }
    for i in 0..8 {
        let [
            mut x0,
            mut x7,
            mut x2,
            mut x5,
            mut x1,
            mut x6,
            mut x3,
            mut x4,
        ] = std::array::from_fn(|j| block[j * 8 + i]);
        x0 = add(x0, 1 << 19);
        (x0, x1) = (add(x0, x1) >> 2, sub(x0, x1) >> 2);
        (x2, x3) = rotation(x2 >> 13, x3 >> 13, 1108, -2676);
        (x1, x2) = (add(x1, x2), sub(x1, x2));
        (x0, x3) = (add(x0, x3), sub(x0, x3));
        (x7, x4) = (add(x7, x4), sub(x7, x4));
        x5 = mul(x5 >> 13, 11585);
        x6 = mul(x6 >> 13, 11585);
        (x7, x5) = (add(x7, x5), sub(x7, x5));
        (x4, x6) = (add(x4, x6), sub(x4, x6));
        (x4, x7) = rotation(x4 >> 14, x7 >> 14, 3406, -2276);
        (x5, x6) = rotation(x5 >> 14, x6 >> 14, 4017, -799);
        (x0, x7) = (add(x0, x7), sub(x0, x7));
        (x1, x6) = (add(x1, x6), sub(x1, x6));
        (x2, x5) = (add(x2, x5), sub(x2, x5));
        (x3, x4) = (add(x3, x4), sub(x3, x4));
        for (j, v) in [x0, x1, x2, x3, x4, x5, x6, x7].into_iter().enumerate() {
            block[j * 8 + i] = v >> 18;
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    #[test]
    fn source_raw_blocks_include_basis_signs_rounding_and_wrapping() {
        let fixtures: Vec<serde_json::Value> = serde_json::from_str(
            d2_rs_testdata::oracle_fixture!("tests/jpeg-idct-go127.json"),
        )
        .unwrap();
        assert_eq!(fixtures.len(), 705);
        for (i, f) in fixtures.iter().enumerate() {
            let mut block: [i32; 64] =
                std::array::from_fn(|j| f["input"][j].as_i64().unwrap() as i32);
            super::inverse(&mut block);
            let expected: [i32; 64] =
                std::array::from_fn(|j| f["output"][j].as_i64().unwrap() as i32);
            assert_eq!(block, expected, "source block {i}");
        }
    }
}
