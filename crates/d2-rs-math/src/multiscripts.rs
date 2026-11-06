// Copyright 2018-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Source: pinned mathjax-go svg/mmultiscripts.go and msubsup.go.
use super::*;
pub(super) fn empty_pair() -> [Box<Atom>; 2] {
    [
        Box::new(Atom::EmptyNode("none")),
        Box::new(Atom::EmptyNode("none")),
    ]
}

// tex/base_ams_handlers.go splitSideSet retains the unscripted remainder.
// A parsed argument's inferred row is transparent only when it has one child.
fn split_side_set(mut atom: Atom) -> (Option<Atom>, Option<Atom>) {
    if let Atom::Sequence(v) = &mut atom {
        if v.is_empty() {
            return (None, None);
        }
        if v.len() == 1 {
            atom = v.pop().unwrap();
        }
    }
    fn empty_base(atom: &Atom) -> bool {
        match atom {
            Atom::Scripts { base, .. } | Atom::Multiscripts { base, .. } => {
                matches!(base.as_ref(), Atom::Identifier(v) if v.is_empty())
            }
            _ => false,
        }
    }
    if empty_base(&atom) {
        return (Some(atom), None);
    }
    if let Atom::Sequence(v) = &mut atom {
        if v.first().is_some_and(empty_base) {
            let scripts = v.remove(0);
            return (Some(scripts), Some(atom));
        }
    }
    (None, Some(atom))
}

// SideSet consumes literal child slots. Eager msup represents an absent sub
// slot; a genuine prime msup instead supplies its child 1 as the subscript.
fn side_scripts(atom: Atom) -> [Box<Atom>; 2] {
    let [empty_sub, empty_sup] = empty_pair();
    let script = |value: Option<Box<Atom>>, empty| {
        let body = value.unwrap_or(empty);
        if matches!(body.as_ref(), Atom::Sequence(_)) {
            Box::new(Atom::Ord(body))
        } else {
            body
        }
    };
    match atom {
        Atom::Scripts {
            sub: None,
            sup,
            eager: false,
            ..
        } => [script(sup, empty_sub), empty_sup],
        Atom::Scripts { sub, sup, .. } => [script(sub, empty_sub), script(sup, empty_sup)],
        Atom::Multiscripts {
            post: Some(pair), ..
        } => pair,
        _ => [empty_sub, empty_sup],
    }
}

fn replace_base(atom: Atom, base: Atom) -> Atom {
    match atom {
        Atom::Scripts {
            sub,
            sup,
            eager,
            limits,
            leading_space,
            accent,
            ..
        } => Atom::Scripts {
            base: Box::new(base),
            sub,
            sup,
            eager,
            limits,
            leading_space,
            accent,
        },
        Atom::Multiscripts {
            sub,
            sup,
            post,
            prescript,
            mathsize,
            ..
        } => Atom::Multiscripts {
            base: Box::new(base),
            sub,
            sup,
            post,
            prescript,
            mathsize,
        },
        atom => atom,
    }
}

pub(super) fn side_set(pre: Atom, post: Atom, base: Atom) -> Atom {
    let (pre_scripts, pre_rest) = split_side_set(pre);
    let (post_scripts, post_rest) = split_side_set(post);
    let mut children = Vec::new();
    let body = if let Some(pre_scripts) = pre_scripts {
        if pre_rest.is_some() {
            let phantom = Atom::Phantom(Box::new(Atom::Padded {
                body: Box::new(base.clone()),
                padding: padded::Padding {
                    width_zero: true,
                    ..Default::default()
                },
            }));
            children.push(replace_base(pre_scripts, phantom));
            children.push(pre_rest.unwrap());
            if let Some(post_scripts) = post_scripts {
                replace_base(post_scripts, base)
            } else {
                base
            }
        } else {
            let [sub, sup] = side_scripts(pre_scripts);
            Atom::Multiscripts {
                base: Box::new(base),
                sub,
                sup,
                post: post_scripts.map(side_scripts),
                prescript: false,
                mathsize: 1.0,
            }
        }
    } else {
        if let Some(rest) = pre_rest {
            children.push(rest);
        }
        if let Some(post_scripts) = post_scripts {
            replace_base(post_scripts, base)
        } else {
            base
        }
    };
    children.push(body);
    if let Some(rest) = post_rest {
        children.push(rest);
    }
    Atom::Sequence(children)
}

fn own_scale(atom: &Atom, scale: f64) -> f64 {
    match atom {
        Atom::Sequence(v) if v.len() == 1 => own_scale(&v[0], scale),
        Atom::DeclaredStyle { level, .. } => styles::scale(*level),
        Atom::DeclaredSize { factor, .. } => scale * factor,
        _ => scale,
    }
}
fn blank() -> Layout {
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
fn probe(atom: &Atom, scale: f64, display: bool, prime: bool) -> Option<Layout> {
    let mut result = blank();
    result.width = layout_at_with_style(atom, 0., 0., scale, &mut result, display, prime)?;
    retain_negative_glyph_depth(&mut result);
    Some(result)
}
fn place_child(mut child: Layout, x: f64, scale: f64, out: &mut Layout) {
    // A one-child argument is the child itself. An explicit style wrapper keeps
    // its actual scale rather than an extra script-scale frame.
    if child.glyphs.is_empty()
        && child.rules.is_empty()
        && child.groups.len() == 1
        && child.groups[0].kind == "mstyle"
        && child.groups[0].x == 0.
        && child.groups[0].baseline == 0.
    {
        let mut g = child.groups.pop().unwrap();
        g.x = x;
        g.before_glyph = out.glyphs.len();
        out.groups.push(g);
    } else {
        classes::place(child, x, 0., scale, out);
    }
}
fn group(child: Layout, x: f64, y: f64, scale: f64, kind: &'static str, out: &mut Layout) {
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
        baseline: y,
        color: None,
        background: None,
        kind,
        delimiter: None,
        arrow: None,
        body: Box::new(child),
        lines: Vec::new(),
    });
}
pub(super) fn layout(
    base: &Atom,
    sub: &Atom,
    sup: &Atom,
    post: Option<&[Box<Atom>; 2]>,
    prescript: bool,
    mathsize: f64,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    display: bool,
    prime: bool,
) -> Option<f64> {
    let base_box = probe(base, scale, display, prime)?;
    // wrapper.go caps inherited scriptlevel at two before getScale computes
    // Pow(scriptsizemultiplier, level). At the default metrics, 8px is below
    // that level-two scale; rounding it up to 0.5 changes nested script bounds.
    let child_scale = (scale * styles::SCRIPT_MULTIPLIER)
        .max(styles::SCRIPT_MULTIPLIER * styles::SCRIPT_MULTIPLIER);
    let sub_box = probe(sub, child_scale, false, true)?;
    let sup_box = probe(sup, child_scale, false, prime)?;
    let post_boxes = if let Some(pair) = post {
        Some((
            probe(&pair[0], child_scale, false, true)?,
            probe(&pair[1], child_scale, false, prime)?,
        ))
    } else {
        None
    };
    let (core, core_scale, core_display, core_prime) =
        styles::script_core(base, scale, display, prime);
    let core_box = probe(core, core_scale, core_display, core_prime)?;
    let character = core_scale == scale
        && match core {
            Atom::Glyph(c)
            | Atom::LiteralGlyph(c)
            | Atom::VariantGlyph(c)
            | Atom::CalligraphicGlyph(c)
            | Atom::ItalicGlyph(c)
            | Atom::Unit(c) => !is_large_operator(*c),
            Atom::FontGlyph { paint, .. } => !is_large_operator(*paint),
            Atom::RawOperator { definition, .. } => !definition.flag("largeop"),
            Atom::Identifier(v) | Atom::Number(v) => v.len() == 1,
            _ => false,
        };
    let unit = 1000. * scale;
    let parent_bounds = |value: &Layout| {
        let (h, d) = styles::em_bounds(value);
        (h / scale, d / scale)
    };
    let parent_width = |value: &Layout| value.width_em.unwrap_or(value.width / 1000.) / scale;
    let (core_h, core_d) = parent_bounds(&core_box);
    // The first postscript, or mprescripts when absent, remains the inherited
    // scriptChild. Combined pre/post boxes are used only for pair clearance.
    let zero = |value| if character { 0. } else { value };
    let primary_rscale = post.map_or(1.0 / (scale * mathsize), |pair| {
        own_scale(&pair[0], child_scale) / scale
    });
    let (primary_h, primary_d) = post_boxes
        .as_ref()
        .map_or((0., 0.), |(sub, _)| parent_bounds(sub));
    let get_u = zero(core_h - 0.386 * primary_rscale)
        .max(if prime {
            0.289
        } else if display {
            0.413
        } else {
            0.363
        })
        .max(primary_d + 0.1105);
    let get_v = zero(core_d + 0.05 * primary_rscale)
        .max(0.15)
        .max(primary_h - 0.8 * 0.442);
    let (pre_sh, pre_sd) = parent_bounds(&sub_box);
    let (pre_uh, pre_ud) = parent_bounds(&sup_box);
    let (post_sh, post_sd, post_uh, post_ud) = post_boxes.as_ref().map_or(
        (-1_000_000., -1_000_000., -1_000_000., -1_000_000.),
        |(sub, sup)| {
            let (sh, sd) = parent_bounds(sub);
            let (uh, ud) = parent_bounds(sup);
            (sh, sd, uh, ud)
        },
    );
    let sh = pre_sh.max(post_sh);
    let sd = pre_sd.max(post_sd);
    let uh = pre_uh.max(post_uh);
    let ud = pre_ud.max(post_ud);
    // CommonMmultiscripts.getUVQ intentionally keeps these single-script
    // branches, including the sub-only result in the superscript baseline.
    let (u, v) = if sh == 0. && sd == 0. {
        (get_u, 0.)
    } else if uh == 0. && ud == 0. {
        (-get_v, 0.)
    } else {
        let mut u = get_u;
        let mut down = zero(core_d + 0.05).max(0.247);
        let q = (u - ud) - (sh - down);
        if q < 3. * 0.06 {
            down += 3. * 0.06 - q;
            let p = 0.8 * 0.442 - (u - ud);
            if p > 0. {
                u += p;
                down -= p;
            }
        }
        (u, -down)
    };

    // computeMultiscriptsBBox composes all dimensions in parent ems before
    // any SVG-unit conversion. Pre/post heights stay separate while Combine
    // chooses the logical maxima (paint and BBox are independent passes).
    let pre_sub_width = parent_width(&sub_box);
    let pre_sup_width = parent_width(&sup_box);
    let pre_width = pre_sub_width.max(pre_sup_width);
    let base_width = parent_width(&base_box);
    let base_end = 0.05 + pre_width + base_width;
    let mut width = base_end;
    let (base_h, base_d) = parent_bounds(&base_box);
    let mut height = (u + pre_uh).max(v + pre_sh).max(base_h);
    let mut depth = (pre_ud - u).max(pre_sd - v).max(base_d);
    if let Some((sub, sup)) = &post_boxes {
        width += parent_width(sub).max(parent_width(sup));
        width += 0.05;
        height = height.max(u + post_uh).max(v + post_sh);
        depth = depth.max(post_ud - u).max(post_sd - v);
    }

    // addMultiscripts emits two rows, preserving each child's own RScale.
    // SideSet sets left alignment; Prescript keeps the default right alignment
    // even after authored right scripts have filled its initially absent slots.
    let mut local = blank();
    let mut sup_row = blank();
    place_child(
        sup_box,
        if prescript {
            (pre_width - pre_sup_width) * unit
        } else {
            0.
        },
        child_scale,
        &mut sup_row,
    );
    let mut sub_row = blank();
    place_child(
        sub_box,
        if prescript {
            (pre_width - pre_sub_width) * unit
        } else {
            0.
        },
        child_scale,
        &mut sub_row,
    );
    group(sup_row, 0.05 * unit, u * unit, scale, "", &mut local);
    group(sub_row, 0.05 * unit, v * unit, scale, "", &mut local);
    classes::place(base_box, (0.05 + pre_width) * unit, 0., scale, &mut local);
    if let Some((sub, sup)) = post_boxes {
        let mut sup_row = blank();
        place_child(sup, 0., child_scale, &mut sup_row);
        let mut sub_row = blank();
        place_child(sub, 0., child_scale, &mut sub_row);
        group(sup_row, base_end * unit, u * unit, scale, "", &mut local);
        group(sub_row, base_end * unit, v * unit, scale, "", &mut local);
    }
    local.width_em = Some(width * scale);
    local.bounds_em = Some((height * scale, depth * scale));
    local.width = width * unit;
    local.ascent = height * scale * 1000.;
    local.depth = depth * scale * 1000.;
    out.width_em = local.width_em;
    out.ascent = out.ascent.max(baseline + local.ascent);
    out.depth = out.depth.max(local.depth - baseline);
    group(local, x, baseline, scale, "mmultiscripts", out);
    Some(width * unit)
}

// tex/commands.go MathtoolsUtil.getScript, before fix_prescripts.go's final
// empty-row normalization. Formatting is selected independently for each slot.
impl Parser {
    pub(super) fn prescript_argument(&mut self, position: &str) -> Option<Atom> {
        let raw = match self.padded_raw_argument("prescript") {
            Ok(raw) => raw,
            Err(error) => return Some(Atom::ErrorDisplay(error)),
        };
        let mut arg = raw.trim_matches(|c| Self::prime_space(&c)).to_owned();
        if arg.ends_with('\\') && raw.ends_with(' ') {
            arg.push(' ');
        }
        if arg.is_empty() {
            return Some(Atom::EmptyNode("none"));
        }
        let format = self
            .ams
            .borrow()
            .option(&format!("prescript-{position}-format"));
        if !format.is_empty() && format != "false" {
            arg = format!("{format}{{{arg}}}");
        }
        let atom = self.padded_parse_string(&arg)?;
        Some(
            if position != "arg" && matches!(&atom, Atom::Sequence(v) if v.is_empty()) {
                Atom::EmptyNode("none")
            } else {
                atom
            },
        )
    }
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    #[test]
    fn prescripts_preserve_empty_and_authored_post_slots() {
        assert_eq!(measure(r"\prescript{}{}{X}"), measure("X"));
        let parsed = parse(r"\prescript{a}{b}{x} ^c_d").unwrap();
        let Atom::Sequence(items) = parsed else {
            panic!("expected root row")
        };
        let Atom::Multiscripts {
            post: Some([sub, sup]),
            prescript: true,
            ..
        } = &items[0]
        else {
            panic!("authored scripts must fill the prescript's own post slots")
        };
        assert!(!matches!(sub.as_ref(), Atom::EmptyNode("none")));
        assert!(!matches!(sup.as_ref(), Atom::EmptyNode("none")));
        assert_eq!(
            measure(r"\prescript{a}{b}{x}^c_d"),
            measure(r"\prescript{a}{b}{x} ^c_d")
        );
    }
}
