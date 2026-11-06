// Copyright 2009, 2010, 2011, 2018 The Go Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license (Go BSD-3-Clause;
// the full text is retained in the repository root's THIRD_PARTY_NOTICES.md).
// Cephes Math Library Release 2.8. Copyright 1984, 1987, 1989, 1992, 2000
// by Stephen L. Moshier: "The readme file at http://netlib.sandia.gov/cephes/
// says: Some software in this archive may be from the book _Methods and
// Programs for Mathematical Functions_ (Prentice-Hall or Simon & Schuster
// International, 1989) or from the Cephes Mathematical Library, a commercial
// product. In either event, it is copyrighted by the author. What you see
// here may be used freely but it comes with no support or guarantee."
//! Pinned Go 1.27 `math` functions used by rough-go: sin/cos (sin.go,
//! trig_reduce.go), tan (tan.go), asin/atan (asin.go, atan.go). Go implements
//! these in Go rather than via the platform libm, so they can differ from
//! Rust's `f64` methods in the last bit. The same translation is used by
//! d2-rs-render's `d2svg/geo/gomath.rs`; every contraction follows the
//! pinned darwin/arm64 build (`go build -gcflags=math=-S`), e.g. tan.go:129
//! contracts `zz+_tanQ[1]` with the unrounded `z*z`.

#![allow(clippy::excessive_precision)]

use std::f64::consts::PI;

/// `a + x*y` as Go ARM64 `FMADDD` (one rounding); separate ops elsewhere.
#[inline]
pub(crate) fn fmadd(x: f64, y: f64, a: f64) -> f64 {
    #[cfg(target_arch = "aarch64")]
    {
        x.mul_add(y, a)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        a + x * y
    }
}

/// `a - x*y` as Go ARM64 `FMSUBD`; separate ops elsewhere.
#[inline]
pub(crate) fn fmsub(x: f64, y: f64, a: f64) -> f64 {
    #[cfg(target_arch = "aarch64")]
    {
        (-x).mul_add(y, a)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        a - x * y
    }
}

fn nan() -> f64 {
    f64::from_bits(0x7ff8000000000001)
}

// ---------------------------------------------------------------- sin.go
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
const PI4A: f64 = f64::from_bits(0x3fe921fb40000000);
const PI4B: f64 = f64::from_bits(0x3e64442d00000000);
const PI4C: f64 = f64::from_bits(0x3ce8469898cc5170);
const FOUR_OVER_PI: f64 = f64::from_bits(0x3ff45f306dc9c883);
const REDUCE_THRESHOLD: f64 = 536870912.0;

fn polynomial(coefficients: &[u64; 6], z: f64) -> f64 {
    let mut p = f64::from_bits(coefficients[0]);
    for &bits in &coefficients[1..] {
        p = fmadd(p, z, f64::from_bits(bits))
    }
    p
}

/// Go `math.Sin`.
pub(crate) fn sin(x: f64) -> f64 {
    compute(x, false)
}

/// Go `math.Cos`.
pub(crate) fn cos(x: f64) -> f64 {
    compute(x, true)
}

fn compute(mut x: f64, cosine: bool) -> f64 {
    if x.is_nan() || x.is_infinite() {
        return if x.is_nan() && !cosine { x } else { nan() };
    }
    if x == 0.0 && !cosine {
        return x;
    }
    let mut sign = !cosine && x < 0.0;
    x = x.abs();
    let (mut j, z) = if x >= REDUCE_THRESHOLD {
        reduce(x)
    } else {
        let mut j = (x * FOUR_OVER_PI) as u64;
        let mut y = j as f64;
        if j & 1 == 1 {
            j += 1;
            y += 1.0
        }
        j &= 7;
        let z = fmsub(y, PI4A, x);
        let z = fmsub(y, PI4B, z);
        let z = fmsub(y, PI4C, z);
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
        fmadd(z * zz, polynomial(&SIN, zz), z)
    } else {
        let base = fmsub(0.5, zz, 1.0);
        fmadd(zz * zz, polynomial(&COS, zz), base)
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

/// trig_reduce.go `trigReduce` (Payne-Hanek).
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
    (j, z * (PI / 4.0))
}

// ---------------------------------------------------------------- tan.go
const TAN_P: [f64; 3] = [
    -1.30936939181383777646e4,
    1.15351664838587416140e6,
    -1.79565251976484877988e7,
];
const TAN_Q: [f64; 5] = [
    1.00000000000000000000e0,
    1.36812963470692954678e4,
    -1.32089234440210967447e6,
    2.50083801823357915839e7,
    -5.38695755929454629881e7,
];

/// Go `math.Tan`.
pub(crate) fn tan(mut x: f64) -> f64 {
    if x == 0.0 || x.is_nan() {
        return x;
    }
    if x.is_infinite() {
        return nan();
    }
    let mut sign = false;
    if x < 0.0 {
        x = -x;
        sign = true;
    }
    let (j, z) = if x >= REDUCE_THRESHOLD {
        reduce(x)
    } else {
        let mut j = (x * FOUR_OVER_PI) as u64;
        let mut y = j as f64;
        if j & 1 == 1 {
            j += 1;
            y += 1.0;
        }
        let z = fmsub(y, PI4A, x);
        let z = fmsub(y, PI4B, z);
        (j, fmsub(y, PI4C, z))
    };
    let zz = z * z;
    let mut y = if zz > 1e-14 {
        // tan.go:129 on arm64: the numerator chain contracts, the leading
        // `zz+_tanQ[1]` reuses the unrounded `z*z`, and the final `z + z*q`
        // contracts too.
        let numerator = zz * fmadd(fmadd(TAN_P[0], zz, TAN_P[1]), zz, TAN_P[2]);
        let denominator = fmadd(
            fmadd(fmadd(fmadd(z, z, TAN_Q[1]), zz, TAN_Q[2]), zz, TAN_Q[3]),
            zz,
            TAN_Q[4],
        );
        fmadd(z, numerator / denominator, z)
    } else {
        z
    };
    if j & 2 == 2 {
        y = -1.0 / y;
    }
    if sign {
        y = -y;
    }
    y
}

// ------------------------------------------------------ atan.go, asin.go
fn xatan(x: f64) -> f64 {
    const P: [f64; 5] = [
        -8.750608600031904122785e-01,
        -1.615753718733365076637e+01,
        -7.500855792314704667340e+01,
        -1.228866684490136173410e+02,
        -6.485021904942025371773e+01,
    ];
    const Q: [f64; 5] = [
        2.485846490142306297962e+01,
        1.650270098316988542046e+02,
        4.328810604912902668951e+02,
        4.853903996359136964868e+02,
        1.945506571482613964425e+02,
    ];
    let z = x * x;
    let mut p = P[0];
    for &c in &P[1..] {
        p = fmadd(p, z, c)
    }
    let mut q = z + Q[0];
    for &c in &Q[1..] {
        q = fmadd(q, z, c)
    }
    let z = z * p / q;
    fmadd(x, z, x)
}

fn satan(x: f64) -> f64 {
    const MORE: f64 = 6.123233995736765886130e-17;
    if x <= 0.66 {
        xatan(x)
    } else if x > 2.41421356237309504880 {
        PI / 2.0 - xatan(1.0 / x) + MORE
    } else {
        PI / 4.0 + xatan((x - 1.0) / (x + 1.0)) + 0.5 * MORE
    }
}

/// Go `math.Atan`.
pub(crate) fn atan(x: f64) -> f64 {
    if x == 0.0 {
        x
    } else if x > 0.0 {
        satan(x)
    } else {
        -satan(-x)
    }
}

/// Go `math.Asin`.
pub(crate) fn asin(mut x: f64) -> f64 {
    if x == 0.0 {
        return x;
    }
    let negative = x < 0.0;
    if negative {
        x = -x
    }
    if x > 1.0 {
        return nan();
    }
    let temp = fmsub(x, x, 1.0).sqrt();
    let temp = if x > 0.7 {
        PI / 2.0 - satan(temp / x)
    } else {
        satan(x / temp)
    };
    if negative { -temp } else { temp }
}

/// Go `math.Max`: NaN if either is NaN; +0 is greater than -0.
pub(crate) fn max(x: f64, y: f64) -> f64 {
    if x.is_infinite() && x > 0.0 || y.is_infinite() && y > 0.0 {
        return f64::INFINITY;
    }
    if x.is_nan() || y.is_nan() {
        return nan();
    }
    if x == 0.0 && x == y {
        return if x.is_sign_negative() { y } else { x };
    }
    if x > y { x } else { y }
}

/// Go `math.Min`: NaN if either is NaN; -0 is less than +0.
pub(crate) fn min(x: f64, y: f64) -> f64 {
    if x.is_infinite() && x < 0.0 || y.is_infinite() && y < 0.0 {
        return f64::NEG_INFINITY;
    }
    if x.is_nan() || y.is_nan() {
        return nan();
    }
    if x == 0.0 && x == y {
        return if x.is_sign_negative() { x } else { y };
    }
    if x < y { x } else { y }
}

#[cfg(test)]
mod tests {
    #[test]
    fn trig_matches_known_go_values() {
        // Spot values; Go's kernels agree with libm on these.
        assert_eq!(super::sin(0.5), 0.479425538604203);
        assert_eq!(super::cos(0.5), 0.8775825618903728);
        assert_eq!(super::tan(0.5), 0.5463024898437905);
        assert_eq!(super::asin(0.5), 0.5235987755982989);
    }
}
