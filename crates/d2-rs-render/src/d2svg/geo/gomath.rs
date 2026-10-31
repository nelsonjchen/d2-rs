// Copyright 2009, 2010, 2011, 2018 The Go Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license (Go BSD-3-Clause;
// the full text is retained in crates/d2-rs-render/THIRD_PARTY_NOTICES.md).
// Cephes Math Library Release 2.8. Copyright 1984, 1987, 1989, 1992, 2000
// by Stephen L. Moshier: "The readme file at http://netlib.sandia.gov/cephes/
// says: Some software in this archive may be from the book _Methods and
// Programs for Mathematical Functions_ (Prentice-Hall or Simon & Schuster
// International, 1989) or from the Cephes Mathematical Library, a commercial
// product. In either event, it is copyrighted by the author. What you see
// here may be used freely but it comes with no support or guarantee."
// log.go: Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.
// Developed at SunPro, a Sun Microsystems, Inc. business. Permission to use,
// copy, modify, and distribute this software is freely granted, provided that
// this notice is preserved.
//! Pinned Go 1.27 `math` functions used by lib/geo, lib/shape and lib/svg:
//! sin/cos (sin.go, trig_reduce.go), atan/atan2/asin/acos (atan.go,
//! atan2.go, asin.go), log/log10 (log.go, log10.go), exp (exp_arm64.s),
//! pow (pow.go), frexp/ldexp/modf. Go implements these in Go (or arm64
//! assembly for Exp), not via the platform libm, so results differ from
//! Rust's `f64` methods in the last bit. Fused operations follow the pinned
//! arm64 build (FMADDD/FMSUBD), as in the d2-rs-elk translations these were
//! taken from (go_trig.rs, go_math.rs, force_layout/logarithm.rs).

#![allow(clippy::excessive_precision)]

use super::go_fmadd as product_sum;
use std::f64::consts::PI;

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
fn polynomial(coefficients: &[u64; 6], z: f64) -> f64 {
    let mut p = f64::from_bits(coefficients[0]);
    for &bits in &coefficients[1..] {
        p = product_sum(p, z, f64::from_bits(bits))
    }
    p
}
pub fn sin(x: f64) -> f64 {
    compute(x, false)
}
pub fn cos(x: f64) -> f64 {
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

/// Go `math.Tan` (tan.go), as in d2-rs-sketch's gomath: on arm64 the
/// numerator chain contracts, the leading `zz+_tanQ[1]` reuses the unrounded
/// `z*z`, and the final `z + z*q` contracts.
pub fn tan(mut x: f64) -> f64 {
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
    let (j, z) = if x >= 536870912.0 {
        reduce(x)
    } else {
        let mut j = (x * f64::from_bits(0x3ff45f306dc9c883)) as u64;
        let mut y = j as f64;
        if j & 1 == 1 {
            j += 1;
            y += 1.0;
        }
        let z = product_sum(-y, f64::from_bits(0x3fe921fb40000000), x);
        let z = product_sum(-y, f64::from_bits(0x3e64442d00000000), z);
        (j, product_sum(-y, f64::from_bits(0x3ce8469898cc5170), z))
    };
    let zz = z * z;
    let mut y = if zz > 1e-14 {
        let numerator =
            zz * product_sum(product_sum(TAN_P[0], zz, TAN_P[1]), zz, TAN_P[2]);
        let denominator = product_sum(
            product_sum(product_sum(product_sum(z, z, TAN_Q[1]), zz, TAN_Q[2]), zz, TAN_Q[3]),
            zz,
            TAN_Q[4],
        );
        product_sum(z, numerator / denominator, z)
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

// ------------------------------------------------- atan.go, atan2.go, asin.go
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
        p = product_sum(p, z, c)
    }
    let mut q = z + Q[0];
    for &c in &Q[1..] {
        q = product_sum(q, z, c)
    }
    let z = z * p / q;
    product_sum(x, z, x)
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
fn atan(x: f64) -> f64 {
    if x == 0.0 {
        x
    } else if x > 0.0 {
        satan(x)
    } else {
        -satan(-x)
    }
}
pub fn atan2(y: f64, x: f64) -> f64 {
    if y.is_nan() || x.is_nan() {
        return nan();
    }
    if y == 0.0 {
        return if x >= 0.0 && !x.is_sign_negative() {
            0.0_f64.copysign(y)
        } else {
            PI.copysign(y)
        };
    }
    if x == 0.0 {
        return (PI / 2.0).copysign(y);
    }
    if x.is_infinite() {
        return if x > 0.0 {
            if y.is_infinite() {
                (PI / 4.0).copysign(y)
            } else {
                0.0_f64.copysign(y)
            }
        } else if y.is_infinite() {
            (3.0 * PI / 4.0).copysign(y)
        } else {
            PI.copysign(y)
        };
    }
    if y.is_infinite() {
        return (PI / 2.0).copysign(y);
    }
    let q = atan(y / x);
    if x < 0.0 {
        if q <= 0.0 { q + PI } else { q - PI }
    } else {
        q
    }
}
pub fn acos(mut x: f64) -> f64 {
    let asin = if x == 0.0 {
        x
    } else {
        let negative = x < 0.0;
        if negative {
            x = -x
        }
        if x > 1.0 {
            return nan();
        }
        let temp = product_sum(-x, x, 1.0).sqrt();
        let temp = if x > 0.7 {
            PI / 2.0 - satan(temp / x)
        } else {
            satan(x / temp)
        };
        if negative { -temp } else { temp }
    };
    PI / 2.0 - asin
}
pub fn hypot(mut p: f64, mut q: f64) -> f64 {
    p = p.abs();
    q = q.abs();
    if p.is_infinite() || q.is_infinite() {
        return f64::INFINITY;
    }
    if p.is_nan() || q.is_nan() {
        return nan();
    }
    if p < q {
        std::mem::swap(&mut p, &mut q)
    }
    if p == 0.0 {
        return 0.0;
    }
    q /= p;
    p * product_sum(q, q, 1.0).sqrt()
}

// ---------------------------------------------------------------- log.go
pub fn log(mut x: f64) -> f64 {
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

/// log10.go `log10`: `Log(x) * (1 / Ln10)`. `1/Ln10` is an exact Go
/// constant expression, i.e. the correctly rounded log10(e).
pub fn log10(x: f64) -> f64 {
    log(x) * std::f64::consts::LOG10_E
}

// --------------------------------------------------------- exp_arm64.s

/// exp_arm64.s `archExp` (Go's `math.Exp` on arm64).
pub fn exp(x: f64) -> f64 {
    const LN2_HI: f64 = 6.93147180369123816490e-01;
    const LN2_LO: f64 = 1.90821492927058770002e-10;
    const LOG2E: f64 = 1.44269504088896338700e+00;
    const OVERFLOW: f64 = 7.09782712893383973096e+02;
    const UNDERFLOW: f64 = -7.45133219101941108420e+02;
    const P1: f64 = 1.66666666666666657415e-01;
    const P2: f64 = -2.77777777770155933842e-03;
    const P3: f64 = 6.61375632143793436117e-05;
    const P4: f64 = -1.65339022054652515390e-06;
    const P5: f64 = 4.13813679705723846039e-08;
    if x.is_nan() {
        return x;
    }
    if x > OVERFLOW {
        return f64::INFINITY;
    }
    if x < UNDERFLOW {
        return 0.0;
    }
    if x.abs() < f64::from_bits(0x3e30000000000000) {
        return 1.0 + x;
    }
    // FNMSUBD/FMADDD: Log2e*x -+ 0.5, then FCVTZSD truncation.
    let k_float = if x < 0.0 {
        super::go_fnmsub(LOG2E, x, 0.5)
    } else {
        product_sum(LOG2E, x, 0.5)
    };
    let k = k_float as i64;
    let kf = k as f64;
    let hi = super::go_fmsub(kf, LN2_HI, x);
    let lo = kf * LN2_LO;
    let r = hi - lo;
    let t = r * r;
    let mut c = product_sum(t, P5, P4);
    c = product_sum(t, c, P3);
    c = product_sum(t, c, P2);
    c = product_sum(t, c, P1);
    c = super::go_fmsub(t, c, r);
    let y = 1.0 - ((lo - (r * c) / (2.0 - c)) - hi);
    // Inline Ldexp(y, k) without special-case checks.
    let bits = y.to_bits();
    let fraction = bits & 0x000f_ffff_ffff_ffff;
    let mut exponent = ((bits >> 52) as i64).wrapping_add(k);
    let mut m = 1.0;
    if exponent < 1 {
        exponent += 52;
        m = f64::from_bits(0x3cb0000000000000);
    }
    let out = f64::from_bits(((exponent as u64) << 52) | fraction);
    m * out
}

// ------------------------------------------- frexp.go, ldexp.go, modf.go

fn normalize(x: f64) -> (f64, i32) {
    const SMALLEST_NORMAL: f64 = 2.2250738585072014e-308;
    if x.abs() < SMALLEST_NORMAL {
        return (x * (1u64 << 52) as f64, -52);
    }
    (x, 0)
}

/// frexp.go `frexp`.
pub fn frexp(f: f64) -> (f64, i32) {
    if f == 0.0 || f.is_infinite() || f.is_nan() {
        return (f, 0);
    }
    let (f, mut exp) = normalize(f);
    let mut x = f.to_bits();
    exp += ((x >> 52) & 0x7ff) as i32 - 1023 + 1;
    x &= !(0x7ffu64 << 52);
    x |= 1022u64 << 52;
    (f64::from_bits(x), exp)
}

/// ldexp.go `ldexp`.
pub fn ldexp(frac: f64, mut exp: i32) -> f64 {
    if frac == 0.0 || frac.is_infinite() || frac.is_nan() {
        return frac;
    }
    let (frac, e) = normalize(frac);
    exp += e;
    let mut x = frac.to_bits();
    exp += ((x >> 52) & 0x7ff) as i32 - 1023;
    if exp < -1075 {
        return 0f64.copysign(frac);
    }
    if exp > 1023 {
        return if frac < 0.0 {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    let mut m = 1.0;
    if exp < -1022 {
        exp += 53;
        m = 1.0 / (1u64 << 53) as f64;
    }
    x &= !(0x7ffu64 << 52);
    x |= ((exp + 1023) as u64) << 52;
    m * f64::from_bits(x)
}

/// modf.go `Modf` (arm64 uses the generic Trunc/Copysign form).
pub fn modf(f: f64) -> (f64, f64) {
    let integer = f.trunc();
    (integer, (f - integer).copysign(f))
}

fn is_odd_int(x: f64) -> bool {
    if x.abs() >= (1u64 << 53) as f64 {
        return false;
    }
    let (xi, xf) = modf(x);
    xf == 0.0 && (xi as i64) & 1 == 1
}

// ---------------------------------------------------------------- pow.go

/// pow.go `pow`.
pub fn pow(x: f64, y: f64) -> f64 {
    if y == 0.0 || x == 1.0 {
        return 1.0;
    }
    if y == 1.0 {
        return x;
    }
    if x.is_nan() || y.is_nan() {
        return nan();
    }
    if x == 0.0 {
        if y < 0.0 {
            if x.is_sign_negative() && is_odd_int(y) {
                return f64::NEG_INFINITY;
            }
            return f64::INFINITY;
        } else if y > 0.0 {
            if x.is_sign_negative() && is_odd_int(y) {
                return x;
            }
            return 0.0;
        }
    }
    if y.is_infinite() {
        if x == -1.0 {
            return 1.0;
        }
        if (x.abs() < 1.0) == (y > 0.0) {
            return 0.0;
        }
        return f64::INFINITY;
    }
    if x.is_infinite() {
        if x < 0.0 {
            return pow(1.0 / x, -y);
        }
        if y < 0.0 {
            return 0.0;
        } else if y > 0.0 {
            return f64::INFINITY;
        }
    }
    if y == 0.5 {
        return x.sqrt();
    }
    if y == -0.5 {
        return 1.0 / x.sqrt();
    }

    let (mut yi, mut yf) = modf(y.abs());
    if yf != 0.0 && x < 0.0 {
        return nan();
    }
    if yi >= 9223372036854775808.0 {
        if x == -1.0 {
            return 1.0;
        }
        if (x.abs() < 1.0) == (y > 0.0) {
            return 0.0;
        }
        return f64::INFINITY;
    }

    let mut a1 = 1.0;
    let mut ae: i32 = 0;
    if yf != 0.0 {
        if yf > 0.5 {
            yf -= 1.0;
            yi += 1.0;
        }
        a1 = exp(yf * log(x));
    }

    let (mut x1, mut xe) = frexp(x);
    let mut i = yi as i64;
    while i != 0 {
        if !(-(1 << 12)..=(1 << 12)).contains(&xe) {
            ae += xe;
            break;
        }
        if i & 1 == 1 {
            a1 *= x1;
            ae += xe;
        }
        x1 *= x1;
        xe <<= 1;
        if x1 < 0.5 {
            x1 += x1;
            xe -= 1;
        }
        i >>= 1;
    }

    if y < 0.0 {
        a1 = 1.0 / a1;
        ae = -ae;
    }
    ldexp(a1, ae)
}
