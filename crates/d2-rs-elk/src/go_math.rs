// Copyright 2009, 2010 The Go Authors. All rights reserved.
// BSD-3-Clause; see ../GO-MATH-LICENSE.txt.
// Cephes Math Library Release 2.8. Copyright 1984, 1987, 1989, 1992, 2000
// by Stephen L. Moshier. See ../GO-TRIG-NOTICE.txt for the retained notice.
// Pinned Go1.27 src/math/{atan,atan2,asin,hypot}.go.
use crate::js_trig::product_sum;
use std::f64::consts::PI;
fn nan() -> f64 {
    f64::from_bits(0x7ff8000000000001)
}
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
pub(crate) fn atan2(y: f64, x: f64) -> f64 {
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
pub(crate) fn acos(mut x: f64) -> f64 {
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
pub(crate) fn hypot(mut p: f64, mut q: f64) -> f64 {
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

#[cfg(test)]
mod tests {
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn pinned_go_radial_inverse_trigonometry_and_hypot_bits() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "src/fixtures/original_radial_math.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 2009);
        let mut errors = vec![];
        for row in rows.as_array().unwrap() {
            let x = f64::from_bits(row["x_bits"].as_u64().unwrap());
            let y = f64::from_bits(row["y_bits"].as_u64().unwrap());
            for (name, result) in [
                ("acos", super::acos(x)),
                ("atan2", super::atan2(y, x)),
                ("hypot", super::hypot(x, y)),
            ] {
                let expected = row[format!("{name}_bits")].as_u64().unwrap();
                if result.to_bits() != expected {
                    errors.push(format!(
                        "{name}({x:?},{y:?}): {:016x} != {expected:016x}",
                        result.to_bits()
                    ))
                }
            }
        }
        assert!(
            errors.is_empty(),
            "{} scalar differences:\n{}",
            errors.len(),
            errors
                .iter()
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
