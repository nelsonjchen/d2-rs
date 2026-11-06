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
fn palette(code: u32, class: usize) -> Atom {
    let fixed = |size| {
        Box::new(Atom::Class {
            body: Box::new(Atom::FixedDelimiter { code, size }),
            class,
        })
    };
    Atom::Choice([fixed(2.047), fixed(1.2), fixed(1.2), fixed(1.2)])
}
// tex/parser.go readInfixSpec/finishInfixFraction (the Over family).
pub(super) fn is_infix(name: &str) -> bool {
    matches!(
        name,
        "over"
            | "atop"
            | "above"
            | "choose"
            | "brace"
            | "brack"
            | "overwithdelims"
            | "atopwithdelims"
            | "abovewithdelims"
    )
}
pub(super) struct InfixSpec {
    thickness: String,
    open: u32,
    close: u32,
    converted: bool,
}

// parser.go closes every local StyleItem before pushing OverItem. Keep the
// already-read arguments and lexical environment while recursive styles unwind.
pub(super) struct PendingInfix {
    pub name: String,
    pub spec: InfixSpec,
    pub font: Option<&'static str>,
    pub color: Option<String>,
}
impl Parser {
    fn infix_palette(&self, code: u32, side: &str) -> Option<Atom> {
        self.genfrac_palette(&char::from_u32(code)?.to_string(), side)
    }
    pub(super) fn finish_infix(&self, left: Atom, right: Atom, spec: InfixSpec) -> Option<Atom> {
        let with_delims = spec.open != 0 || spec.close != 0;
        let fraction = Atom::GeneralFraction(
            Box::new(left),
            Box::new(right),
            Spec::new(&spec.thickness, with_delims),
        );
        if !with_delims {
            return Some(fraction);
        }
        let mut body = Vec::new();
        if spec.open != 0 {
            let fence = if spec.converted {
                self.infix_palette(spec.open, "l")?
            } else {
                palette(spec.open, 4)
            };
            if cd::first_error(&fence).is_some() {
                return Some(fence);
            }
            body.push(fence);
        }
        body.push(fraction);
        if spec.close != 0 {
            let fence = if spec.converted {
                self.infix_palette(spec.close, "r")?
            } else {
                palette(spec.close, 5)
            };
            if cd::first_error(&fence).is_some() {
                return Some(fence);
            }
            body.push(fence);
        }
        Some(Atom::Mrow(Box::new(Atom::Sequence(body))))
    }
    pub(super) fn read_infix_spec(&mut self, name: &str) -> Result<InfixSpec, String> {
        let (mut open, mut close) = (0, 0);
        if name.ends_with("withdelims") {
            open = self.native_delimiter(name, false)?;
            close = self.native_delimiter(name, false)?;
        }
        let thickness = if matches!(name, "above" | "abovewithdelims") {
            self.read_dimension(name)?
        } else if matches!(
            name,
            "atop" | "atopwithdelims" | "choose" | "brace" | "brack"
        ) {
            "0".into()
        } else {
            "medium".into()
        };
        (open, close) = match name {
            "choose" => (0x28, 0x29),
            "brace" => (0x7b, 0x7d),
            "brack" => (0x5b, 0x5d),
            _ => (open, close),
        };
        Ok(InfixSpec {
            thickness,
            open,
            close,
            converted: name.ends_with("withdelims"),
        })
    }
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
            close_color: None,
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
    // commands.go:binomial uses direct token palettes, unlike Genfrac's
    // parsed palettes. Registered big/mathchoice commands cannot intercept it.
    pub(super) fn binomial_command(&mut self, name: &str) -> Option<Atom> {
        let numerator = self.padded_argument(name)?;
        if cd::first_error(&numerator).is_some() {
            return Some(numerator);
        }
        let denominator = self.padded_argument(name)?;
        if cd::first_error(&denominator).is_some() {
            return Some(denominator);
        }
        let fraction = Atom::GeneralFraction(
            Box::new(numerator),
            Box::new(denominator),
            Spec::new("0", true),
        );
        let content = Atom::Fenced {
            close_color: None,
            open: 0,
            close: 0,
            stretchy: false,
            inner: false,
            body: vec![palette(0x28, 4), fraction, palette(0x29, 5)],
        };
        Some(if name == "binom" {
            content
        } else {
            Atom::DeclaredStyle {
                body: Box::new(content),
                display: name == "dbinom",
                level: 0,
            }
        })
    }
    // source_tables_ams_maps.go registers dfrac/tfrac as Genfrac with a
    // fixed style argument. The mstyle owns only the fraction, not the
    // remaining parser row (base_ams_handlers.go:amsGenfrac).
    pub(super) fn styled_fraction(&mut self, name: &str) -> Option<Atom> {
        let numerator = self.padded_argument(name)?;
        if cd::first_error(&numerator).is_some() {
            return Some(numerator);
        }
        let denominator = self.padded_argument(name)?;
        if cd::first_error(&denominator).is_some() {
            return Some(denominator);
        }
        Some(Atom::DeclaredStyle {
            body: Box::new(Atom::GeneralFraction(
                Box::new(numerator),
                Box::new(denominator),
                Spec::new("medium", false),
            )),
            display: name == "dfrac",
            level: 0,
        })
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
pub(super) fn blank() -> Layout {
    Layout {
        bounds_em: None,
        width_em: None,
        intrinsic_width_em: None,
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
    } else if scale >= 1. {
        styles::SCRIPT_MULTIPLIER
    } else {
        styles::SCRIPT_MULTIPLIER * styles::SCRIPT_MULTIPLIER
    };
    let n = probe(num, child_scale, prime)?;
    let d = probe(den, child_scale, true)?;
    let unit = 1000. * scale;
    // mfrac reads em BBoxes before any conversion to SVG font units.
    let (nh_em, nd_em) = styles::em_bounds(&n);
    let (dh_em, dd_em) = styles::em_bounds(&d);
    // wrapper.go caps scriptlevel at two before getScale. The paint
    // frame can still carry styles::scale's rounded level-two value.
    let wrapper_scale = if scale >= 1. {
        1.
    } else if scale >= styles::SCRIPT_MULTIPLIER {
        styles::SCRIPT_MULTIPLIER
    } else {
        styles::SCRIPT_MULTIPLIER * styles::SCRIPT_MULTIPLIER
    };
    let child_ratio = child_scale / wrapper_scale;
    let h = (nh_em / child_scale) * child_ratio;
    let depth_n = (nd_em / child_scale) * child_ratio;
    let dh = (dh_em / child_scale) * child_ratio;
    let dd = (dd_em / child_scale) * child_ratio;
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
        let q = (u - nd_em * spec.mathsize) - (dh_em * spec.mathsize - v);
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

#[cfg(test)]
mod infix_contract {
    use super::*;

    #[test]
    fn infix_arguments_precede_ambiguity_and_groups_reset_pending_over() {
        // Pinned parser.go readInfixSpec precedes OverItem ambiguity;
        // real groups start a fresh logical row, unlike declarations.
        for (tex, error) in [
            (
                r"a\over b\above bad",
                r"Missing dimension or its units for \above",
            ),
            (
                r"a\over b\overwithdelims?x",
                r"Missing or unrecognized delimiter for \overwithdelims",
            ),
            (
                r"a\over b\abovewithdelims()bad",
                r"Missing dimension or its units for \abovewithdelims",
            ),
            (r"a\over b\above{.2pt} c", r"Ambiguous use of \above"),
            (r"a\over \bf b\over c", r"Ambiguous use of \over"),
            (r"a\over \scriptstyle b\over c", r"Ambiguous use of \over"),
            (r"a\over \Huge b\over c", r"Ambiguous use of \over"),
            (r"a\over \color{red} b\over c", r"Ambiguous use of \over"),
            (r"a\above {0.2pt", "Missing close brace"),
        ] {
            assert_eq!(render_exact(tex).unwrap(), cd::render_error(error), "{tex}");
        }
        for tex in [r"a\over{b\over c}", r"{a\over b}\over c", r"a\above 0.2ptb"] {
            let svg = render_exact(tex).unwrap();
            assert!(!svg.contains("data-mjx-error"), "{tex}");
            assert!(svg.contains("data-mml-node=\"mfrac\""), "{tex}");
        }

        // Pinned parser.go:730-755 closes StyleItems before creating OverItem.
        // The numerator owns the style, and CR is replayed outside the fraction.
        // The transforms below come from the pinned Go D2 SVG renderer.
        for (style, numerator_transform, denominator_transform, spacer_transform) in [
            (
                r"\displaystyle",
                "translate(220,676)",
                "translate(270,-686)",
                "translate(969,0)",
            ),
            (
                r"\scriptstyle",
                "translate(247.5,676) scale(0.707)",
                "translate(220,-686)",
                "translate(869,0)",
            ),
            (
                r"\tiny",
                "translate(302.3,676) scale(0.5)",
                "translate(220,-686)",
                "translate(869,0)",
            ),
            (
                r"\color{red}",
                "translate(220,676)",
                "translate(270,-686)",
                "translate(969,0)",
            ),
        ] {
            let tex = format!(r"{style} a \over b \\ c");
            let svg = render_exact(&tex).unwrap();
            let doc = roxmltree::Document::parse(&svg).unwrap();
            let math = doc
                .descendants()
                .find(|node| node.attribute("data-mml-node") == Some("math"))
                .unwrap();
            let children: Vec<_> = math.children().filter(|node| node.is_element()).collect();
            assert_eq!(
                children
                    .iter()
                    .map(|node| node.attribute("data-mml-node").unwrap())
                    .collect::<Vec<_>>(),
                ["mfrac", "mspace", "mi"],
                "{tex}"
            );
            let fraction: Vec<_> = children[0]
                .children()
                .filter(|node| node.attribute("data-mml-node").is_some())
                .collect();
            assert_eq!(fraction.len(), 2, "{tex}");
            assert_eq!(fraction[0].attribute("data-mml-node"), Some("mstyle"));
            assert_eq!(fraction[1].attribute("data-mml-node"), Some("mi"));
            assert_eq!(
                fraction[0].attribute("transform"),
                Some(numerator_transform)
            );
            assert_eq!(
                fraction[1].attribute("transform"),
                Some(denominator_transform)
            );
            assert_eq!(children[1].attribute("transform"), Some(spacer_transform));
            assert_eq!(children[2].attribute("transform"), Some(spacer_transform));
        }

        // svg/wrapper.go standardSVG installs RScale before handleColor;
        // place updates that transform without changing its attribute order.
        let svg =
            render_exact(r"\begin{matrix}\color{red} a \over b \\ c & d\end{matrix}").unwrap();
        assert!(svg.contains(
            r#"<g data-mml-node="mstyle" transform="translate(220,394) scale(0.707)" fill="red" stroke="red">"#
        ));
    }

    #[test]
    fn infix_fixed_fences_follow_inherited_palette_and_converted_delimiter_errors() {
        // fixed_infix_fences_test.go and parser.go fixedFence: fixed palette,
        // ORD mrow ownership, and no null-delimiter padding.
        for (style, viewport) in [
            ("", "0 -1449.5 2001 2399"),
            (r"\textstyle ", "0 -849.5 1290.1 1202.3"),
            (r"\scriptstyle ", "0 -600.7 912.2 850.1"),
            (r"\scriptscriptstyle ", "0 -442.5 722.5 620.5"),
        ] {
            let svg = render_exact(&format!("{style}{{a\\choose b}}")).unwrap();
            let doc = roxmltree::Document::parse(&svg).unwrap();
            assert_eq!(doc.root_element().attribute("viewBox"), Some(viewport));
            for class in ["OPEN", "CLOSE"] {
                assert_eq!(
                    doc.descendants()
                        .filter(|n| n.attribute("data-mjx-texclass") == Some(class))
                        .count(),
                    1
                );
            }
        }
        // Converted glyphs are reparsed by the generated biggl/bigl child;
        // raw angle glyphs are not delimiter-map keys in the pin.
        assert_eq!(
            render_exact(r"a\overwithdelims\langle\rangle b").unwrap(),
            cd::render_error(r"Missing or unrecognized delimiter for \biggl")
        );
    }
}
