// Copyright 2009-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Source: mathjax-go tex/{parser,commands,base_ams_handlers}.go and svg/mfrac.go.
use super::*;
#[derive(Clone, Copy, Debug)]
pub(super) struct Spec {
    thickness: Dimension,
    pad: bool,
    align: i8,
    pub mathsize: f64,
}
impl Spec {
    pub fn new(thickness: &str, with_delims: bool) -> Self {
        Self {
            thickness: length::dimension(thickness),
            pad: !with_delims,
            align: 0,
            mathsize: 1.,
        }
    }
}
pub(super) fn fence(body: Atom, open: u32, close: u32) -> Atom {
    Atom::Fenced {
        open,
        body: vec![body],
        close,
        stretchy: true,
        inner: false,
    }
}
fn style(body: Atom, raw: &str) -> Result<Atom, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(body);
    }
    let (display, level) = match raw {
        "0" => (true, 0),
        "1" => (false, 0),
        "2" => (false, 1),
        "3" => (false, 2),
        _ => return Err("Bad math style for \\genfrac".into()),
    };
    Ok(Atom::DeclaredStyle {
        body: Box::new(body),
        display,
        level,
    })
}
fn palette(code: u32, class: usize) -> Atom {
    let fixed = |size| {
        Box::new(Atom::Class {
            body: Box::new(Atom::FixedDelimiter { code, size }),
            class,
        })
    };
    Atom::Choice([fixed(2.047), fixed(1.2), fixed(1.2), fixed(1.2)])
}
impl Parser {
    pub(super) fn continued_fraction(&mut self) -> Option<Atom> {
        let align = match self.padded_brackets() {
            Ok(a) => a,
            Err(e) => return Some(Atom::ErrorDisplay(e)),
        };
        let n = self.padded_argument("cfrac")?;
        if cd::first_error(&n).is_some() {
            return Some(n);
        }
        let d = self.padded_argument("cfrac")?;
        if cd::first_error(&d).is_some() {
            return Some(d);
        }
        let row = |a| Atom::Fenced {
            open: 0,
            close: 0,
            stretchy: false,
            inner: false,
            body: vec![
                Atom::FractionStrut(1.),
                Atom::DeclaredStyle {
                    body: Box::new(Atom::Ord(Box::new(a))),
                    display: false,
                    level: 0,
                },
            ],
        };
        let mut spec = Spec::new("medium", false);
        spec.align = match align.as_str() {
            "l" => -1,
            "r" => 1,
            _ => 0,
        };
        Some(Atom::GeneralFraction(
            Box::new(row(n)),
            Box::new(row(d)),
            spec,
        ))
    }
    pub(super) fn insert_fraction_macro(&mut self, name: &str) -> Result<(), String> {
        let n = self.padded_raw_argument(name)?;
        let d = self.padded_raw_argument(name)?;
        let frac = format!("\\frac{{{n}}}{{{d}}}");
        let expansion = match name {
            "binom" => format!("\\left({frac}\\right)"),
            "dbinom" => format!("\\displaystyle\\binom{{{n}}}{{{d}}}"),
            "tbinom" => format!("\\textstyle\\binom{{{n}}}{{{d}}}"),
            "dfrac" => format!("\\displaystyle{frac}"),
            "tfrac" => format!("\\textstyle{frac}"),
            _ => unreachable!(),
        };
        self.chars.splice(self.index..self.index, expansion.chars());
        Ok(())
    }
    fn fraction_delimiter(&self, raw: &str) -> Result<u32, String> {
        let raw = raw.trim();
        if raw.is_empty() || raw == "." {
            return Ok(0);
        }
        let raw = if raw == "|" || (!raw.starts_with('\\') && raw.chars().count() > 1) {
            format!("\\{raw}")
        } else {
            raw.to_string()
        };
        let mut p = Parser {
            ams: self.ams.clone(),
            splice_inferred: false,
            chars: raw.chars().collect(),
            index: 0,
            normal_font: false,
            calligraphic_font: false,
            multi_letter_font: false,
            cd_minimums: self.cd_minimums,
        };
        let c = p
            .native_delimiter()
            .map_err(|_| "Missing or unrecognized delimiter".to_string())?;
        if p.index != p.chars.len() {
            return Err("Missing or unrecognized delimiter".into());
        }
        Ok(c)
    }
    pub(super) fn general_fraction_command(&mut self, name: &str) -> Option<Atom> {
        let mut raws = Vec::new();
        if name == "genfrac" {
            for _ in 0..4 {
                match self.padded_raw_argument(name) {
                    Ok(v) => raws.push(v),
                    Err(e) => return Some(Atom::ErrorDisplay(e)),
                }
            }
        }
        let n = self.padded_argument(name)?;
        if cd::first_error(&n).is_some() {
            return Some(n);
        }
        let d = self.padded_argument(name)?;
        if cd::first_error(&d).is_some() {
            return Some(d);
        }
        if name == "genfrac" {
            let left = match self.fraction_delimiter(&raws[0]) {
                Ok(v) => v,
                Err(e) => return Some(Atom::ErrorDisplay(e)),
            };
            let right = match self.fraction_delimiter(&raws[1]) {
                Ok(v) => v,
                Err(e) => return Some(Atom::ErrorDisplay(e)),
            };
            let thickness = if raws[2].is_empty() {
                "medium"
            } else {
                &raws[2]
            };
            let fraction = Atom::GeneralFraction(
                Box::new(n),
                Box::new(d),
                Spec::new(thickness, left != 0 || right != 0),
            );
            let content = if left != 0 || right != 0 {
                let mut v = Vec::new();
                if left != 0 {
                    v.push(palette(left, 4));
                }
                v.push(fraction);
                if right != 0 {
                    v.push(palette(right, 5));
                }
                Atom::Fenced {
                    open: 0,
                    body: v,
                    close: 0,
                    stretchy: false,
                    inner: false,
                }
            } else {
                fraction
            };
            return Some(match style(content, &raws[3]) {
                Ok(a) => a,
                Err(e) => Atom::ErrorDisplay(e),
            });
        }
        unreachable!("only genfrac reaches the native handler")
    }
}
pub(super) fn strut(
    mathsize: f64,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    display: bool,
    prime: bool,
) -> Option<f64> {
    let padding = padded::Padding {
        width_zero: true,
        height_add: length::dimension("8.6pt").em(0., scale * mathsize),
        depth_add: length::dimension("3pt").em(0., scale * mathsize),
        ..Default::default()
    };
    padded::layout_node(
        &Atom::EmptyNode("mrow"),
        Some(&padding),
        false,
        x,
        baseline,
        scale,
        out,
        display,
        prime,
    )
}
fn blank() -> Layout {
    Layout {
        bounds_em: None,
        width_em: None,
        width: 0.,
        ascent: 0.,
        depth: 0.,
        glyphs: Vec::new(),
        rules: Vec::new(),
        groups: Vec::new(),
    }
}
fn probe(a: &Atom, scale: f64, prime: bool) -> Option<Layout> {
    let mut l = blank();
    l.width = layout_at_with_style(a, 0., 0., scale, &mut l, false, prime)?;
    retain_negative_glyph_depth(&mut l);
    Some(l)
}
pub(super) fn layout(
    num: &Atom,
    den: &Atom,
    spec: Spec,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    display: bool,
    prime: bool,
) -> Option<f64> {
    let display = display && scale == 1.;
    let child_scale = if display {
        scale
    } else {
        (scale * std::f64::consts::FRAC_1_SQRT_2).max(0.5)
    };
    let n = probe(num, child_scale, prime)?;
    let d = probe(den, child_scale, true)?;
    let unit = 1000. * scale;
    let h = n.ascent / unit;
    let depth_n = n.depth / unit;
    let dh = d.ascent / unit;
    let dd = d.depth / unit;
    let content = n
        .width_em
        .unwrap_or(n.width / 1000.)
        .max(d.width_em.unwrap_or(d.width / 1000.))
        / scale;
    let pad = if spec.pad { 0.12 } else { 0. };
    let t = spec.thickness.em(0.06, scale * spec.mathsize);
    let (u, v, width_em, mut nx, mut dx) = if t == 0. {
        let mut u = if display { 0.676 } else { 0.444 };
        let mut v = if display { 0.686 } else { 0.345 };
        let q = (u - depth_n * (scale * spec.mathsize)) - (dh * (scale * spec.mathsize) - v);
        let min = if display { 7. * 0.06 } else { 3. * 0.06 };
        if q < min {
            u += (min - q) / 2.;
            v += (min - q) / 2.;
        }
        (
            u,
            -v,
            content + 2. * pad,
            (content - n.width / unit) / 2. + pad,
            (content - d.width / unit) / 2. + pad,
        )
    } else {
        let big_t = if display { 3.5 * t } else { 1.5 * t };
        let u = (if display { 0.676 } else { 0.394 }) - 0.25 - big_t;
        let v = (if display { 0.686 } else { 0.345 }) + 0.25 - big_t;
        (
            0.25 + big_t + depth_n.max(u),
            0.25 - big_t - dh.max(v),
            content + (2. * pad + 0.2),
            (content - n.width / unit) / 2. + 0.1 + pad,
            (content - d.width / unit) / 2. + 0.1 + pad,
        )
    };
    if spec.align != 0 {
        let inset = pad + if t == 0. { 0. } else { 0.1 };
        nx = inset
            + if spec.align > 0 {
                content - n.width / unit
            } else {
                0.
            };
        dx = inset
            + if spec.align > 0 {
                content - d.width / unit
            } else {
                0.
            };
    }
    let ascent = (h + u).max(dh + v);
    let depth = (depth_n - u).max(dd - v);
    let mut local = blank();
    classes::place(n, nx * unit, u * unit, child_scale, &mut local);
    classes::place(d, dx * unit, v * unit, child_scale, &mut local);
    if t != 0. {
        local.rules.push((
            pad * unit,
            (0.25 - t / 2.) * unit,
            (content + 0.2) * unit,
            t * unit,
        ));
    }
    local.width_em = Some(width_em);
    local.width = width_em * unit;
    local.ascent = ascent * unit;
    local.depth = depth * unit;
    local.bounds_em = Some((ascent * scale, depth * scale));
    ams::set_wrap_width(&mut local, content * unit);
    ams::set_percentage_wrap(&mut local, content * unit, "center");
    out.groups.push(LayoutGroup {
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
        color: None,
        background: None,
        kind: "mfrac",
        delimiter: None,
        arrow: None,
        body: Box::new(local),
        lines: Vec::new(),
    });
    out.ascent = out.ascent.max(baseline + ascent * unit);
    out.depth = out.depth.max(depth * unit - baseline);
    out.width_em = Some(width_em * scale);
    Some(width_em * unit)
}
