// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Translated from pinned mathjax-go v0.1.0 tex/commands.go and
// svg/{menclose,menclose_notation,menclose_shapes}.go.
use super::*;
use std::collections::BTreeMap;
#[derive(Clone, Debug, Default)]
pub(super) struct Options {
    values: BTreeMap<String, String>,
}
#[derive(Clone, Copy)]
pub(super) struct Arrow {
    width: f64,
    height: f64,
    depth: f64,
    thickness: f64,
    arrow_x: f64,
    arrow_y: f64,
    arrow_dx: f64,
}
impl Parser {
    fn enclose_brackets(&mut self) -> Result<String, String> {
        while self
            .chars
            .get(self.index)
            .is_some_and(|c| c.is_whitespace())
        {
            self.index += 1;
        }
        if self.chars.get(self.index) != Some(&'[') {
            return Ok(String::new());
        }
        self.index += 1;
        let start = self.index;
        let mut depth = 0usize;
        while let Some(c) = self.chars.get(self.index).copied() {
            match c {
                '\\' => {
                    self.index += 1;
                    if self.index < self.chars.len() {
                        self.index += 1;
                    }
                    continue;
                }
                '{' => depth += 1,
                '}' => {
                    if depth == 0 {
                        return Err("Extra close brace while looking for ']'".into());
                    }
                    depth -= 1;
                }
                ']' if depth == 0 => {
                    let raw = self.chars[start..self.index].iter().collect();
                    self.index += 1;
                    return Ok(raw);
                }
                _ => {}
            }
            self.index += 1;
        }
        Err("Could not find closing ']' for argument".into())
    }
    pub(super) fn enclose_command(&mut self, name: &str) -> Option<Atom> {
        let raw = match self.enclose_brackets() {
            Ok(s) => s,
            Err(e) => return Some(Atom::ErrorDisplay(e)),
        };
        let target = if name == "cancelto" {
            let a = self.padded_argument(name)?;
            if cd::first_error(&a).is_some() {
                return Some(a);
            }
            Some(a)
        } else {
            None
        };
        let body = self.padded_argument(name)?;
        if cd::first_error(&body).is_some() {
            return Some(body);
        }
        let options = match Options::parse(&raw) {
            Ok(o) => o,
            Err(e) => return Some(Atom::ErrorDisplay(e)),
        };
        let base = Atom::Cancel {
            body: Box::new(body),
            up: name == "cancel" || name == "xcancel",
            down: name == "bcancel" || name == "xcancel",
            arrow: name == "cancelto",
            options,
        };
        Some(if let Some(target) = target {
            Atom::Scripts {
                limits: false,
                accent: None,
                eager: false,
                leading_space: 0.,
                base: Box::new(base),
                sub: None,
                sup: Some(Box::new(Atom::Padded {
                    body: Box::new(target),
                    padding: padded::Padding {
                        height_add: 0.1,
                        depth_add: -0.1,
                        voffset_em: 0.1,
                        ..Default::default()
                    },
                })),
            }
        } else {
            base
        })
    }
}
impl Options {
    fn parse(raw: &str) -> Result<Self, String> {
        let mut values = BTreeMap::new();
        if raw.trim().is_empty() {
            return Ok(Self { values });
        }
        let mut start = 0;
        let mut depth = 0usize;
        let mut escaped = false;
        let mut entries = Vec::new();
        for (i, c) in raw.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if c == '\\' {
                escaped = true;
                continue;
            }
            if c == '{' {
                depth += 1;
            } else if c == '}' {
                depth = depth.saturating_sub(1);
            } else if c == ',' && depth == 0 {
                entries.push(&raw[start..i]);
                start = i + 1;
            }
        }
        entries.push(&raw[start..]);
        for entry in entries {
            let (key, value) = entry
                .split_once('=')
                .map_or((entry, "true"), |(k, v)| (k, v));
            let key = key.trim();
            if ![
                "data-arrowhead",
                "color",
                "mathcolor",
                "background",
                "mathbackground",
                "data-padding",
                "data-thickness",
            ]
            .contains(&key)
            {
                // commands.go keyvalOptions disables errorOnUnknown.
                continue;
            }
            values.insert(key.into(), value.trim_matches(['{', '}']).trim().into());
        }
        Ok(Self { values })
    }
    fn dimension(&self, name: &str, fallback: f64, scale: f64) -> f64 {
        self.values
            .get(name)
            .map_or(fallback, |raw| length::dimension(raw).em(fallback, scale))
    }
}
fn blank() -> Layout {
    Layout {
        bounds_em: None,
        width_em: None,
        intrinsic_width_em: None,
        width: 0.0,
        ascent: 0.0,
        depth: 0.0,
        glyphs: Vec::new(),
        rules: Vec::new(),
        groups: Vec::new(),
    }
}
pub(super) fn layout_node(
    body: &Atom,
    up: bool,
    down: bool,
    arrow: bool,
    options: &Options,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    display: bool,
    prime: bool,
) -> Option<f64> {
    let mut child = blank();
    child.width = layout_at_with_style(body, 0.0, 0.0, scale, &mut child, display, prime)?;
    retain_negative_glyph_depth(&mut child);
    let (w, h, d) = (
        child.width / (1000.0 * scale),
        child.ascent / (1000.0 * scale),
        child.depth / (1000.0 * scale),
    );
    let padding = options.dimension("data-padding", 0.2, scale);
    let thickness = options.dimension("data-thickness", 0.067, scale);
    let parts = options
        .values
        .get("data-arrowhead")
        .map(|v| v.split_whitespace().collect::<Vec<_>>())
        .unwrap_or_default();
    let part = |index: usize, default: f64| parts.get(index).map_or(default, |s| float_prefix(s));
    let (ax, ay, adx) = (part(0, 4.0), part(1, 2.0), part(2, 1.0));
    let (dy, dx) = if arrow {
        let r = thickness * (ax + adx.max(1.0));
        let height = h + d;
        let radius = (height * height + w * w).sqrt();
        let x = go_max(padding, r * w / radius);
        let y = go_max(padding, r * height / radius);
        let a = (height + 2.0 * y).atan2(w + 2.0 * x);
        let b = ay.atan2(ax + adx);
        let radius = ((ax + adx) * (ax + adx) + ay * ay).sqrt();
        (
            y + if b > a {
                thickness * radius * (b - a).sin()
            } else {
                0.0
            },
            x + if b > std::f64::consts::FRAC_PI_2 - a {
                thickness * radius * (b + a - std::f64::consts::FRAC_PI_2).sin()
            } else {
                0.0
            },
        )
    } else {
        (padding + thickness, padding + thickness)
    };
    // getBBoxExtenders maximizes each notation against an initial zero;
    // negative and NaN extenders do not change that zero.
    let dx = if dx > 0.0 { dx } else { 0.0 };
    let dy = if dy > 0.0 { dy } else { 0.0 };
    let (width, height, depth) = (w + 2.0 * dx, h + dy, d + dy);
    let mut local = blank();
    local.width = width * 1000.0 * scale;
    local.ascent = height * 1000.0 * scale;
    local.depth = depth * 1000.0 * scale;
    local.groups.push(LayoutGroup {
        x_frame: None,
        data_cramped: None,
        own_translate: None,
        accent_offset: None,
        box_border: None,
        reference: None,
        tagged: None,
        percentage: None,
        table_line: None,
        mathsize: None,
        tex_class: 0,
        before_glyph: 0,
        scale,
        x: dx * 1000.0 * scale,
        baseline: 0.0,
        color: None,
        background: None,
        kind: "",
        delimiter: None,
        arrow: None,
        body: Box::new(child),
        lines: Vec::new(),
    });
    let t = thickness * 1000.0 * scale;
    let half = t / 2.0;
    let mut lines = Vec::new();
    if up {
        lines.push((
            half,
            half - local.depth,
            local.width - half,
            local.ascent - half,
            t,
        ));
    }
    if down {
        lines.push((
            half,
            local.ascent - half,
            local.width - half,
            half - local.depth,
            t,
        ));
    }
    let color = options
        .values
        .get("mathcolor")
        .or_else(|| options.values.get("color"))
        .cloned();
    let background = options
        .values
        .get("mathbackground")
        .or_else(|| options.values.get("background"))
        .map(|c| (c.clone(), local.width, local.ascent, local.depth));
    let width = local.width;
    out.ascent = out.ascent.max(baseline + local.ascent);
    out.depth = out.depth.max(local.depth - baseline);
    out.groups.push(LayoutGroup {
        x_frame: None,
        data_cramped: None,
        own_translate: None,
        accent_offset: None,
        box_border: None,
        reference: None,
        tagged: None,
        percentage: None,
        table_line: None,
        mathsize: None,
        tex_class: 0,
        before_glyph: out.glyphs.len(),
        scale,
        x,
        baseline,
        color,
        background,
        kind: "menclose",
        delimiter: None,
        arrow: arrow.then_some(Arrow {
            width: width / (1000.0 * scale),
            height,
            depth,
            thickness,
            arrow_x: ax,
            arrow_y: ay,
            arrow_dx: adx,
        }),
        body: Box::new(local),
        lines,
    });
    Some(width)
}
fn float_prefix(raw: &str) -> f64 {
    static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        regex::Regex::new(r"^[+-]?(?:Infinity|(?:[0-9]+\.?[0-9]*|\.[0-9]+)(?:[eE][+-]?[0-9]+)?)")
            .unwrap()
    });
    pattern
        .find(raw.trim())
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(f64::NAN)
}
pub(super) fn render_arrow(svg: &mut String, a: Arrow) {
    let angle = (a.height + a.depth).atan2(a.width);
    let length = (a.width * a.width + (a.height + a.depth) * (a.height + a.depth)).sqrt();
    let dw = (length - a.width) / 2.0;
    let mid = (a.height - a.depth) / 2.0;
    let t = a.thickness;
    let half = t / 2.0;
    let (x, y, dx) = (t * a.arrow_x, t * a.arrow_y, t * a.arrow_dx);
    let f = |v| root_number(v * 1000.0);
    let path = format!(
        "M {} {} l {} {} l {} {} L {} {} l 0 {} L {} {} l {} {} Z",
        f(a.width + dw),
        f(mid),
        f(-(x + dx)),
        f(y),
        f(dx),
        f(half - y),
        f(-dw),
        f(mid + half),
        f(-t),
        f(a.width + dw - x),
        f(mid - half),
        f(-dx),
        f(half - y)
    );
    let transform = if angle == 0.0 {
        String::new()
    } else {
        format!(
            " transform=\"rotate({} {} {})\"",
            angle_number(angle * 180.0 / std::f64::consts::PI),
            f(a.width / 2.0),
            f(mid)
        )
    };
    svg.push_str(&format!("<path d=\"{path}\"{transform}></path>"));
}

fn go_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

// jscompat.Fixed(_,3) rounds the exact binary value, including decimal ties.
fn angle_number(value: f64) -> String {
    if value.abs() < 0.0006 {
        return "0".into();
    }
    if !value.is_finite() {
        return if value.is_nan() {
            "NaN".into()
        } else if value > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    let bits = value.abs().to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023 - 52;
    let numerator = (((bits & ((1_u64 << 52) - 1)) | (1_u64 << 52)) as u128) * 1000;
    let rounded = if exponent >= 0 {
        numerator.checked_shl(exponent as u32).unwrap()
    } else {
        let shift = (-exponent) as u32;
        if shift >= 128 {
            0
        } else {
            let q = numerator >> shift;
            let rest = numerator & ((1_u128 << shift) - 1);
            q + u128::from(rest >= 1_u128 << (shift - 1))
        }
    };
    let mut text = format!(
        "{}{}.{:03}",
        if value < 0.0 { "-" } else { "" },
        rounded / 1000,
        rounded % 1000
    );
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

pub(super) fn layout_box(
    body: &Atom,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    display: bool,
    prime: bool,
) -> Option<f64> {
    let width = layout_node(
        body,
        false,
        false,
        false,
        &Options::default(),
        x,
        baseline,
        scale,
        out,
        display,
        prime,
    )?;
    out.groups.last_mut()?.box_border = Some(67.0 * scale);
    Some(width)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arrow_numbers_use_go_ascii_prefixes_and_js_exact_decimal_rounding() {
        assert_eq!(float_prefix("3٢"), 3.0);
        assert!(float_prefix("٢3").is_nan());
        assert_eq!(angle_number(22.0625), "22.063");
        assert_eq!(angle_number(-22.0625), "-22.063");
        assert_eq!(angle_number(45.0), "45");
        assert_eq!(angle_number(0.0005), "0");
    }
}
