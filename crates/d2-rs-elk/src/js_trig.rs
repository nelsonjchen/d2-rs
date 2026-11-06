// Adapted from fdlibm. Copyright (C) 1993 Sun Microsystems, Inc.
// All rights reserved. Developed at SunSoft, a Sun Microsystems, Inc. business.
// Permission to use, copy, modify, and distribute this software is freely
// granted, provided that this notice is preserved.
// Modified significantly by Google Inc.
// Copyright 2016 the V8 project authors. All rights reserved.
// Copyright (c) 2026 The elk-go Authors.
// SunPro AND BSD-3-Clause; see ../V8-BSD-3-Clause.txt.
// Pinned elk-go/internal/javacompat/jsmath.go.

// fdlibm returns x - x, which is NaN for infinities and NaN.
#![allow(clippy::eq_op)]
pub(crate) fn product_sum(a: f64, b: f64, c: f64) -> f64 {
    #[cfg(target_arch = "aarch64")]
    {
        a.mul_add(b, c)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        a * b + c
    }
}
fn high(x: f64) -> u32 {
    (x.to_bits() >> 32) as u32
}
pub(crate) fn sin(x: f64) -> f64 {
    let ix = high(x) & 0x7fffffff;
    if ix <= 0x3fe921fb {
        return kernel_sin(x, 0.0, false);
    }
    if ix >= 0x7ff00000 {
        return x - x;
    }
    let Some((n, y0, y1)) = reduce(x) else {
        return crate::go_trig::sin(x);
    };
    match n & 3 {
        0 => kernel_sin(y0, y1, true),
        1 => kernel_cos(y0, y1),
        2 => -kernel_sin(y0, y1, true),
        _ => -kernel_cos(y0, y1),
    }
}
pub(crate) fn cos(x: f64) -> f64 {
    let ix = high(x) & 0x7fffffff;
    if ix <= 0x3fe921fb {
        return kernel_cos(x, 0.0);
    }
    if ix >= 0x7ff00000 {
        return x - x;
    }
    let Some((n, y0, y1)) = reduce(x) else {
        return crate::go_trig::cos(x);
    };
    match n & 3 {
        0 => kernel_cos(y0, y1),
        1 => -kernel_sin(y0, y1, true),
        2 => -kernel_cos(y0, y1),
        _ => kernel_sin(y0, y1, true),
    }
}
fn reduce(x: f64) -> Option<(i32, f64, f64)> {
    const INV: f64 = 6.366_197_723_675_814e-1;
    const P1: f64 = 1.570_796_326_734_125_6;
    const P1T: f64 = 6.077_100_506_506_192e-11;
    const P2: f64 = 6.077_100_506_303_966e-11;
    const P2T: f64 = 2.022_266_248_795_950_6e-21;
    const P3: f64 = 2.022_266_248_711_166_5e-21;
    const P3T: f64 = 8.478_427_660_368_9e-32;
    const HIGH: [u32; 32] = [
        0x3ff921fb, 0x400921fb, 0x4012d97c, 0x401921fb, 0x401f6a7a, 0x4022d97c, 0x4025fdbb,
        0x402921fb, 0x402c463a, 0x402f6a7a, 0x4031475c, 0x4032d97c, 0x40346b9c, 0x4035fdbb,
        0x40378fdb, 0x403921fb, 0x403ab41b, 0x403c463a, 0x403dd85a, 0x403f6a7a, 0x40407e4c,
        0x4041475c, 0x4042106c, 0x4042d97c, 0x4043a28c, 0x40446b9c, 0x404534ac, 0x4045fdbb,
        0x4046c6cb, 0x40478fdb, 0x404858eb, 0x404921fb,
    ];
    let hx = high(x);
    let ix = hx & 0x7fffffff;
    if ix <= 0x3fe921fb {
        return Some((0, x, 0.0));
    }
    if ix < 0x4002d97c {
        if hx as i32 > 0 {
            let mut z = x - P1;
            if ix != 0x3ff921fb {
                let y = z - P1T;
                return Some((1, y, (z - y) - P1T));
            }
            z -= P2;
            let y = z - P2T;
            return Some((1, y, (z - y) - P2T));
        }
        let mut z = x + P1;
        if ix != 0x3ff921fb {
            let y = z + P1T;
            return Some((-1, y, (z - y) + P1T));
        }
        z += P2;
        let y = z + P2T;
        return Some((-1, y, (z - y) + P2T));
    }
    if ix > 0x413921fb {
        return None;
    }
    let mut t = x.abs();
    let n = (t * INV + 0.5) as i32;
    let nf = n as f64;
    let mut r = t - nf * P1;
    let mut w = nf * P1T;
    let mut y = r - w;
    if !(n < 32 && ix != HIGH[(n - 1) as usize]) {
        let j = (ix >> 20) as i32;
        let mut i = j - ((high(y) >> 20) & 0x7ff) as i32;
        if i > 16 {
            t = r;
            w = nf * P2;
            r = t - w;
            w = nf * P2T - ((t - r) - w);
            y = r - w;
            i = j - ((high(y) >> 20) & 0x7ff) as i32;
            if i > 49 {
                t = r;
                w = nf * P3;
                r = t - w;
                w = nf * P3T - ((t - r) - w);
                y = r - w;
            }
        }
    }
    let tail = (r - y) - w;
    if (hx as i32) < 0 {
        Some((-n, -y, -tail))
    } else {
        Some((n, y, tail))
    }
}
fn kernel_cos(x: f64, y: f64) -> f64 {
    const C: [f64; 6] = [
        4.166_666_666_666_66e-2,
        -1.388_888_888_887_411e-3,
        2.480_158_728_947_673e-5,
        -2.755_731_435_139_066_3e-7,
        2.087_572_321_298_175e-9,
        -1.135_964_755_778_819_5e-11,
    ];
    let ix = high(x) & 0x7fffffff;
    if ix < 0x3e400000 && x as i64 == 0 {
        return 1.0;
    }
    let z = x * x;
    let mut polynomial = C[4] + z * C[5];
    for i in (0..4).rev() {
        polynomial = C[i] + z * polynomial
    }
    let r = z * polynomial;
    let zr = z * r - x * y;
    if ix < 0x3fd33333 {
        return 1.0 - (0.5 * z - zr);
    }
    let qx = if ix <= 0x3fe90000 {
        f64::from_bits(((ix - 0x00200000) as u64) << 32)
    } else {
        0.28125
    };
    let iz = 0.5 * z - qx;
    let a = 1.0 - qx;
    a - (iz - zr)
}
fn kernel_sin(x: f64, y: f64, tail: bool) -> f64 {
    const S: [f64; 6] = [
        -1.666_666_666_666_663_2e-1,
        8.333_333_333_322_49e-3,
        -1.984_126_982_985_795e-4,
        2.755_731_370_707_006_8e-6,
        -2.505_076_025_340_686_3e-8,
        1.589_690_995_211_55e-10,
    ];
    if high(x) & 0x7fffffff < 0x3e400000 && x as i64 == 0 {
        return x;
    }
    let z = x * x;
    let v = z * x;
    let r = product_sum(
        z,
        product_sum(z, product_sum(z, product_sum(z, S[5], S[4]), S[3]), S[2]),
        S[1],
    );
    if !tail {
        return product_sum(v, product_sum(z, r, S[0]), x);
    }
    let a = product_sum(-v, r, 0.5 * y);
    let b = product_sum(z, a, -y);
    let c = product_sum(-v, S[0], b);
    x - c
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_go_javascript_trigonometry_bits() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "src/fixtures/original_radial_trig.json"
        ))
        .unwrap();
        let mut differences = vec![];
        assert_eq!(rows.as_array().unwrap().len(), 2393);
        for row in rows.as_array().unwrap() {
            let bits = row["input_bits"].as_u64().unwrap();
            let x = f64::from_bits(bits);
            for (name, actual) in [("sin", super::sin(x)), ("cos", super::cos(x))] {
                let expected = row[format!("{name}_bits")].as_u64().unwrap();
                if actual.to_bits() != expected {
                    differences.push(format!(
                        "{name}({x:?}) [{bits:016x}]: {:016x} != {expected:016x}",
                        actual.to_bits()
                    ))
                }
            }
        }
        assert!(
            differences.is_empty(),
            "{} scalar differences:\n{}",
            differences.len(),
            differences
                .iter()
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
