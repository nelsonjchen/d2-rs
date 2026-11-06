// Copyright 2010 The Go Authors. All rights reserved.
// BSD-3-Clause; see THIRD_PARTY_NOTICES.md.
// Source: Go 1.27 math/sincos.go, sin.go and tan.go.
// Cephes coefficients: Copyright Stephen L. Moshier, freely usable as
// retained in the original Go source. Only reduced [-pi,pi] inputs are used.
pub(super) fn sincos(angle: f64) -> (f64, f64) {
    if angle == 0.0 {
        return (angle, 1.0);
    }
    const PI: f64 = std::f64::consts::PI;
    const PI4A: f64 = 7.853_981_256_484_985e-1;
    const PI4B: f64 = 3.774_894_707_930_798e-8;
    const PI4C: f64 = 2.695_151_429_079_059_5e-15;
    const SIN: [f64; 6] = [
        1.589_623_015_765_465_7e-10,
        -2.505_074_776_285_780_7e-8,
        2.755_731_362_138_572_5e-6,
        -1.984_126_982_958_954e-4,
        8.333_333_333_322_119e-3,
        -1.666_666_666_666_663e-1,
    ];
    const COS: [f64; 6] = [
        -1.135_853_652_138_768_2e-11,
        2.087_570_084_197_473e-9,
        -2.755_731_417_929_674e-7,
        2.480_158_728_885_170_5e-5,
        -1.388_888_888_887_305_6e-3,
        4.166_666_666_666_659_e-2,
    ];
    let (x, mut sin_negative) = if angle < 0.0 {
        (-angle, true)
    } else {
        (angle, false)
    };
    let mut cos_negative = false;
    let mut j = (x * (4.0 / PI)).trunc() as u64;
    let mut y = j as f64;
    if j & 1 == 1 {
        j += 1;
        y += 1.0;
    }
    j &= 7;
    #[cfg(not(target_arch = "aarch64"))]
    let z = ((x - y * PI4A) - y * PI4B) - y * PI4C;
    #[cfg(target_arch = "aarch64")]
    let z = (-y).mul_add(PI4C, (-y).mul_add(PI4B, (-y).mul_add(PI4A, x)));
    if j > 3 {
        j -= 4;
        sin_negative = !sin_negative;
        cos_negative = !cos_negative;
    }
    if j > 1 {
        cos_negative = !cos_negative;
    }
    let zz = z * z;
    #[cfg(not(target_arch = "aarch64"))]
    let (mut sin, mut cos) = {
        let poly = |coeff: [f64; 6]| {
            ((((coeff[0] * zz + coeff[1]) * zz + coeff[2]) * zz + coeff[3]) * zz + coeff[4]) * zz
                + coeff[5]
        };
        (z + z * zz * poly(SIN), 1.0 - 0.5 * zz + zz * zz * poly(COS))
    };
    #[cfg(target_arch = "aarch64")]
    let cos_poly = COS[0]
        .mul_add(zz, COS[1])
        .mul_add(zz, COS[2])
        .mul_add(zz, COS[3])
        .mul_add(zz, COS[4])
        .mul_add(zz, COS[5]);
    #[cfg(target_arch = "aarch64")]
    let sin_poly = SIN[0]
        .mul_add(zz, SIN[1])
        .mul_add(zz, SIN[2])
        .mul_add(zz, SIN[3])
        .mul_add(zz, SIN[4])
        .mul_add(zz, SIN[5]);
    #[cfg(target_arch = "aarch64")]
    let mut cos = (zz * zz).mul_add(cos_poly, (-0.5_f64).mul_add(zz, 1.0));
    #[cfg(target_arch = "aarch64")]
    let mut sin = (z * zz).mul_add(sin_poly, z);
    if j == 1 || j == 2 {
        std::mem::swap(&mut sin, &mut cos);
    }
    if cos_negative {
        cos = -cos;
    }
    if sin_negative {
        sin = -sin;
    }
    (sin, cos)
}

pub(super) fn tan(mut x: f64) -> f64 {
    if x == 0. {
        return x;
    }
    let negative = x < 0.;
    if negative {
        x = -x;
    }
    let mut j = (x * (4. / std::f64::consts::PI)) as u64;
    let mut y = j as f64;
    if j & 1 == 1 {
        j += 1;
        y += 1.;
    }
    #[cfg(not(target_arch = "aarch64"))]
    let z = ((x - y * 7.85398125648498535156e-1) - y * 3.77489470793079817668e-8)
        - y * 2.69515142907905952645e-15;
    #[cfg(target_arch = "aarch64")]
    let z = (-y).mul_add(
        2.695_151_429_079_059_5e-15,
        (-y).mul_add(
            3.774_894_707_930_798e-8,
            (-y).mul_add(7.853_981_256_484_985e-1, x),
        ),
    );
    let zz = z * z;
    let mut result = if zz > 1e-14 {
        #[cfg(not(target_arch = "aarch64"))]
        {
            let numerator = zz
                * ((-1.30936939181383777646e4 * zz + 1.15351664838587416140e6) * zz
                    - 1.79565251976484877988e7);
            let denominator = (((zz + 1.36812963470692954678e4) * zz - 1.32089234440210967447e6)
                * zz
                + 2.50083801823357915839e7)
                * zz
                - 5.38695755929454629881e7;
            z + z * (numerator / denominator)
        }
        #[cfg(target_arch = "aarch64")]
        {
            let numerator = zz
                * (-1.309_369_391_813_837_9e4_f64)
                    .mul_add(zz, 1.153_516_648_385_874_2e6)
                    .mul_add(zz, -1.795_652_519_764_848_8e7);
            let denominator = z
                .mul_add(z, 1.368_129_634_706_929_6e4)
                .mul_add(zz, -1.320_892_344_402_109_7e6)
                .mul_add(zz, 2.500_838_018_233_579e7)
                .mul_add(zz, -5.386_957_559_294_546_4e7);
            z.mul_add(numerator / denominator, z)
        }
    } else {
        z
    };
    if j & 2 == 2 {
        result = -1. / result;
    }
    if negative {
        result = -result;
    }
    result
}
