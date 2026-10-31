// SPDX-License-Identifier: MIT
// Copyright (c) 2020 Nor Khasyatillah.
// Translated from pinned csscolorparser v0.1.8; see THIRD_PARTY_NOTICES.md.
use std::{collections::BTreeMap, sync::OnceLock};
fn floor0(n: f64) -> f64 {
    if n.is_nan() {
        n
    } else if n <= 0. {
        0.
    } else {
        n
    }
}
fn clamp(n: f64) -> f64 {
    if n < 0. {
        0.
    } else if n > 1. {
        1.
    } else {
        n
    }
}
fn float(s: &str) -> Option<f64> {
    d2_rs_core::parse_d2_float(s.trim())
}
fn percent(s: &str, denominator: f64) -> Option<(f64, bool)> {
    if let Some(n) = s.strip_suffix('%') {
        Some((float(n)? / 100., true))
    } else {
        Some((float(s)? / denominator, false))
    }
}
fn angle(s: &str) -> Option<f64> {
    for suffix in ["deg", "grad", "rad", "turn"] {
        if let Some(n) = s.strip_suffix(suffix) {
            let n = float(n)?;
            return Some(match suffix {
                "grad" => n / 400. * 360.,
                "rad" => n / std::f64::consts::PI * 180.,
                "turn" => n * 360.,
                _ => n,
            });
        }
    }
    float(s)
}
fn normalize(n: f64) -> f64 {
    let n = n % 360.;
    if n < 0. { n + 360. } else { n }
}
fn hue(a: f64, b: f64, h: f64) -> f64 {
    let h = (h % 6. + 6.) % 6.;
    if h < 1. {
        (b - a).mul_add(h, a)
    } else if h < 3. {
        b
    } else if h < 4. {
        (b - a).mul_add(4. - h, a)
    } else {
        a
    }
}
fn hsl(h: f64, s: f64, l: f64) -> [f64; 3] {
    let h = normalize(h) / 60.;
    let s = clamp(s);
    let l = clamp(l);
    if s == 0. {
        return [l; 3];
    }
    let b = if l < 0.5 {
        l * (1. + s)
    } else {
        (-l).mul_add(s, l + s)
    };
    let a = 2_f64.mul_add(l, -b);
    [
        clamp(hue(a, b, h + 2.)),
        clamp(hue(a, b, h)),
        clamp(hue(a, b, h - 2.)),
    ]
}
fn linear(n: f64) -> f64 {
    if n >= 0.0031308 {
        1.055_f64.mul_add(n.powf(5. / 12.), -0.055)
    } else {
        12.92 * n
    }
}
fn oklab(l: f64, a: f64, b: f64) -> [f64; 3] {
    let x = 0.2158037573_f64
        .mul_add(b, 0.3963377774_f64.mul_add(a, l))
        .powi(3);
    let y = (-0.0638541728_f64)
        .mul_add(b, (-0.1055613458_f64).mul_add(a, l))
        .powi(3);
    let z = (-1.2914855480_f64)
        .mul_add(b, (-0.0894841775_f64).mul_add(a, l))
        .powi(3);
    [
        linear(0.2309699292_f64.mul_add(z, (-3.3077115913_f64).mul_add(y, 4.0767416621 * x))),
        linear((-0.3413193965_f64).mul_add(z, 2.6097574011_f64.mul_add(y, -1.2684380046 * x))),
        linear(1.7076147010_f64.mul_add(z, (-0.7034186147_f64).mul_add(y, -0.0041960863 * x))),
    ]
}
fn lab(l: f64, a: f64, b: f64) -> [f64; 3] {
    let fy = (l + 16.) / 116.;
    let fx = fy + a / 500.;
    let fz = fy - b / 200.;
    let e = 216. / 24389.;
    let k = 24389. / 27.;
    let y = if l > 8. { fy.powi(3) } else { l / k };
    let x = if fx.powi(3) > e {
        0.95047 * fx.powi(3)
    } else {
        0.95047 * 116_f64.mul_add(fx, -16.) / k
    };
    let z = if fz.powi(3) > e {
        1.08883 * fz.powi(3)
    } else {
        1.08883 * 116_f64.mul_add(fz, -16.) / k
    };
    [
        linear(clamp(
            (-0.498531_f64).mul_add(z, (-1.537177_f64).mul_add(y, 3.2404537 * x)),
        )),
        linear(clamp(
            0.041556_f64.mul_add(z, 1.876010_f64.mul_add(y, -0.969266 * x)),
        )),
        linear(clamp(
            1.057225_f64.mul_add(z, (-0.204025_f64).mul_add(y, 0.055643 * x)),
        )),
    ]
}
fn hex(s: &str) -> Option<[f64; 4]> {
    let bytes = s.as_bytes();
    let mut result = [0., 0., 0., 1.];
    if !matches!(bytes.len(), 3 | 4 | 6 | 8) {
        return None;
    }
    let short = bytes.len() < 5;
    let digit = |b: u8| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    };
    for index in 0..if short { bytes.len() } else { bytes.len() / 2 } {
        result[index] = f64::from(if short {
            digit(bytes[index])? * 17
        } else {
            digit(bytes[index * 2])? * 16 + digit(bytes[index * 2 + 1])?
        }) / 255.;
    }
    Some(result)
}
/// csscolorparser v0.1.8 `Parse`: straight RGBA components in [0, 1].
pub fn parse(raw: &str) -> Option<[f64; 4]> {
    static NAMES: OnceLock<BTreeMap<String, [u8; 3]>> = OnceLock::new();
    let names =
        NAMES.get_or_init(|| serde_json::from_str(include_str!("css_named_colors.json")).unwrap());
    let lowered = String::from_utf8(d2_rs_core::ir::go_lowercase(raw.as_bytes())).unwrap();
    let s = lowered.trim();
    if s == "transparent" {
        return Some([0.; 4]);
    }
    if let Some(rgb) = names.get(s) {
        return Some([
            f64::from(rgb[0]) / 255.,
            f64::from(rgb[1]) / 255.,
            f64::from(rgb[2]) / 255.,
            1.,
        ]);
    }
    if let Some(s) = s.strip_prefix('#') {
        return hex(s);
    }
    if let Some(open) = s.find('(').filter(|_| s.ends_with(')')) {
        let name = s[..open].trim();
        let fields: Vec<_> = s[open + 1..s.len() - 1]
            .split(|c| matches!(c, ',' | '/' | ' '))
            .filter(|p| !p.is_empty())
            .collect();
        if !matches!(fields.len(), 3 | 4) {
            return None;
        }
        let alpha = if fields.len() == 4 {
            clamp(percent(fields[3], 1.)?.0)
        } else {
            1.
        };
        let rgb = match name {
            "rgb" | "rgba" => [
                clamp(percent(fields[0], 255.)?.0),
                clamp(percent(fields[1], 255.)?.0),
                clamp(percent(fields[2], 255.)?.0),
            ],
            "hsl" | "hsla" => hsl(
                angle(fields[0])?,
                percent(fields[1], 1.)?.0,
                percent(fields[2], 1.)?.0,
            ),
            "hwb" | "hwba" => {
                let h = angle(fields[0])?;
                let w = clamp(percent(fields[1], 1.)?.0);
                let b = clamp(percent(fields[2], 1.)?.0);
                if w + b >= 1. {
                    [w / (w + b); 3]
                } else {
                    let mut rgb = hsl(h, 1., 0.5);
                    for n in &mut rgb {
                        *n = n.mul_add(1. - w - b, w);
                    }
                    rgb
                }
            }
            "hsv" | "hsva" => {
                let h = angle(fields[0])?;
                let s = clamp(percent(fields[1], 1.)?.0);
                let v = clamp(percent(fields[2], 1.)?.0);
                let l = (2. - s) * v / 2.;
                let sat = if l == 0. {
                    s
                } else if l == 1. {
                    0.
                } else if l < 0.5 {
                    s * v / (l * 2.)
                } else {
                    s * v / (-2_f64).mul_add(l, 2.)
                };
                hsl(h, sat, l)
            }
            "oklab" | "lab" => {
                let (mut l, lp) = percent(fields[0], 1.)?;
                let (mut a, ap) = percent(fields[1], 1.)?;
                let (mut b, bp) = percent(fields[2], 1.)?;
                let limit = if name == "lab" { 125. } else { 0.4 };
                if ap {
                    a = (a + 1.).mul_add(limit, -limit);
                }
                if bp {
                    b = (b + 1.).mul_add(limit, -limit);
                }
                if name == "lab" && lp {
                    l *= 100.;
                }
                if name == "lab" {
                    lab(floor0(l), a, b)
                } else {
                    oklab(floor0(l), a, b)
                }
            }
            "oklch" | "lch" => {
                let (mut l, lp) = percent(fields[0], 1.)?;
                let (mut c, cp) = percent(fields[1], 1.)?;
                let h = angle(fields[2])? * std::f64::consts::PI / 180.;
                if cp {
                    c *= if name == "lch" { 150. } else { 0.4 };
                }
                if name == "lch" && lp {
                    l *= 100.;
                }
                let c = floor0(c);
                let (sn, cs) = h.sin_cos();
                if name == "lch" {
                    lab(floor0(l), c * cs, c * sn)
                } else {
                    oklab(floor0(l), c * cs, c * sn)
                }
            }
            _ => return hex(s),
        };
        return Some([rgb[0], rgb[1], rgb[2], alpha]);
    }
    hex(s)
}
/// Original importer parseSolidColor boundary, returning straight NRGBA bytes.
pub fn solid(value: &str) -> Result<[u8; 4], String> {
    if value.len() > 4096 {
        return Err("solid color is too long".into());
    }
    let color = parse(value.trim()).ok_or_else(|| "unsupported solid color".to_owned())?;
    // Pinned Go ARM64 RGBA255 emits FMA then FCVTZSDW (signed int32),
    // followed by the low byte. Preserve saturation and NaN handling.
    Ok(color.map(|v| v.mul_add(255., 0.5) as i32 as u8))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_solid_color_boundary() {
        let rows: serde_json::Value =
            serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/svg-color-v090.json")).unwrap();
        for row in rows.as_array().unwrap() {
            let result = solid(row["value"].as_str().unwrap());
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            if let Ok(color) = result {
                assert_eq!(serde_json::to_value(color).unwrap(), row["rgba"], "{row}");
            }
        }
    }
}
