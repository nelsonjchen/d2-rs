// Copyright 2011, 2018 The Go Authors. All rights reserved.
// BSD-3-Clause; see ../GO-MATH-LICENSE.txt.
// Cephes Math Library Release 2.8. Copyright 1984, 1987, 1989, 1992, 2000
// by Stephen L. Moshier. See ../GO-TRIG-NOTICE.txt for the retained notice.
// Pinned Go1.27 src/math/{sin,trig_reduce}.go. Used by elk-go's fdlibm
// wrapper once its medium-argument reduction declines an input.
use crate::js_trig::product_sum;
const SIN: [u64; 6] = [
    0x3de5d8fd1fd19ccd,
    0xbe5ae5e5a9291f5d,
    0x3ec71de3567d48a1,
    0xbf2a01a019bfdf03,
    0x3f8111111110f7d0,
    0xbfc5555555555548,
];
const COS: [u64; 6] = [
    0xbda8fa49a0861a9b,
    0x3e21ee9d7b4e3f05,
    0xbe927e4f7eac4bc6,
    0x3efa01a019c844f5,
    0xbf56c16c16c14f91,
    0x3fa555555555554b,
];
fn polynomial(coefficients: &[u64; 6], z: f64) -> f64 {
    let mut p = f64::from_bits(coefficients[0]);
    for &bits in &coefficients[1..] {
        p = product_sum(p, z, f64::from_bits(bits))
    }
    p
}
pub(crate) fn sin(x: f64) -> f64 {
    compute(x, false)
}
pub(crate) fn cos(x: f64) -> f64 {
    compute(x, true)
}
fn compute(mut x: f64, cosine: bool) -> f64 {
    if x.is_nan() || x.is_infinite() {
        return if x.is_nan() && !cosine {
            x
        } else {
            f64::from_bits(0x7ff8000000000001)
        };
    }
    if x == 0.0 && !cosine {
        return x;
    }
    let mut sign = !cosine && x < 0.0;
    x = x.abs();
    let (mut j, z) = if x >= 536870912.0 {
        reduce(x)
    } else {
        let mut j = (x * f64::from_bits(0x3ff45f306dc9c883)) as u64;
        let mut y = j as f64;
        if j & 1 == 1 {
            j += 1;
            y += 1.0
        }
        j &= 7;
        let z = product_sum(-y, f64::from_bits(0x3fe921fb40000000), x);
        let z = product_sum(-y, f64::from_bits(0x3e64442d00000000), z);
        let z = product_sum(-y, f64::from_bits(0x3ce8469898cc5170), z);
        (j, z)
    };
    if j > 3 {
        j -= 4;
        sign = !sign
    }
    if cosine && j > 1 {
        sign = !sign
    }
    let zz = z * z;
    let sine_kernel = if cosine {
        j == 1 || j == 2
    } else {
        j != 1 && j != 2
    };
    let mut y = if sine_kernel {
        product_sum(z * zz, polynomial(&SIN, zz), z)
    } else {
        let base = product_sum(-0.5, zz, 1.0);
        product_sum(zz * zz, polynomial(&COS, zz), base)
    };
    if sign {
        y = -y
    }
    y
}
fn shl(a: u64, b: u32) -> u64 {
    if b >= 64 { 0 } else { a << b }
}
fn shr(a: u64, b: u32) -> u64 {
    if b >= 64 { 0 } else { a >> b }
}
fn reduce(x: f64) -> (u64, f64) {
    const DIGITS: [u64; 20] = [
        0x0000000000000001,
        0x45f306dc9c882a53,
        0xf84eafa3ea69bb81,
        0xb6c52b3278872083,
        0xfca2c757bd778ac3,
        0x6e48dc74849ba5c0,
        0x0c925dd413a32439,
        0xfc3bd63962534e7d,
        0xd1046bea5d768909,
        0xd338e04d68befc82,
        0x7323ac7306a673e9,
        0x3908bf177bf25076,
        0x3ff12fffbc0b301f,
        0xde5e2316b414da3e,
        0xda6cfd9e4f96136e,
        0x9e8c7ecd3cbfd45a,
        0xea4f758fd7cbe2f6,
        0x7a0e73ef14a525d4,
        0xd7f6bf623f1aba10,
        0xac06608df8f6d757,
    ];
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32 - 1023 - 52;
    let mantissa = (bits & 0x000fffffffffffff) | (1 << 52);
    let digit = ((exp + 61) as usize) / 64;
    let shift = ((exp + 61) as u32) % 64;
    let z0 = shl(DIGITS[digit], shift) | shr(DIGITS[digit + 1], 64 - shift);
    let z1 = shl(DIGITS[digit + 1], shift) | shr(DIGITS[digit + 2], 64 - shift);
    let z2 = shl(DIGITS[digit + 2], shift) | shr(DIGITS[digit + 3], 64 - shift);
    let z2hi = ((z2 as u128 * mantissa as u128) >> 64) as u64;
    let z1product = z1 as u128 * mantissa as u128;
    let (z1hi, z1lo) = ((z1product >> 64) as u64, z1product as u64);
    let z0lo = z0.wrapping_mul(mantissa);
    let (lo, carry) = z1lo.overflowing_add(z2hi);
    let mut hi = z0lo.wrapping_add(z1hi).wrapping_add(u64::from(carry));
    let mut j = hi >> 61;
    hi = hi << 3 | lo >> 61;
    let lz = hi.leading_zeros();
    let exponent = 1023 - (lz + 1) as u64;
    hi = shl(hi, lz + 1) | shr(lo, 64u32.wrapping_sub(lz + 1));
    hi >>= 12;
    hi |= exponent << 52;
    let mut z = f64::from_bits(hi);
    if j & 1 == 1 {
        j += 1;
        j &= 7;
        z -= 1.0
    }
    (j, z * (std::f64::consts::PI / 4.0))
}
