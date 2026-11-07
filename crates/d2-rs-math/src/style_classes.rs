// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Source: pinned mathjax-go 94d80186bfd7 internal/svg/texclass.go, wrapper.go.
use super::*;
use std::collections::HashMap;

// Transparent rows and declarations share the surrounding previous-class
// traversal. Their wrapper takes its first child's class, while traversal
// returns its last child. Keep both identities through layout.
struct Plan {
    classes: Vec<Option<usize>>,
    adjustments: Vec<Option<usize>>,
    returns: Vec<Option<usize>>,
    levels: Vec<bool>,
    // adjustTeXClass reads the current mo/TeXAtom's inherited level rather
    // than the predecessor's returned level. An arrow's explicit zero
    // belongs to its inner mstyle, so retain both values through declarations.
    operator_previous_levels: Vec<Option<bool>>,
    automatic_operators: Vec<bool>,
    spans: HashMap<usize, (usize, usize)>,
    row_ends: Vec<usize>,
}
// texclass.go:adjustTeXClass stores the CURRENT inherited level for mo
// and TeXAtom, while setPreviousClass on other nodes reads the predecessor.
pub(super) fn adjusts_current_level(atom: &Atom) -> bool {
    match atom {
        Atom::RawOperator { .. }
        | Atom::Class { .. }
        | Atom::Ord(_)
        | Atom::Operator(_)
        | Atom::AutoOperator(_) => true,
        Atom::Scripts { base, .. } | Atom::Multiscripts { base, .. } => adjusts_current_level(base),
        Atom::Glyph(_) | Atom::LiteralGlyph(_) => atom_tex_class(atom).is_some_and(|c| c != 0),
        _ => false,
    }
}
// setTeXClass returns a transparent wrapper's final child. setPreviousClass
// on a following identifier reads that child's inherited scriptlevel, even
// when the wrapper itself belongs to the surrounding display-level row.
pub(super) fn returned_script_level(atom: &Atom, script: bool) -> bool {
    match atom {
        Atom::DeclaredStyle { body, level, .. } => returned_script_level(body, *level > 0),
        Atom::DeclaredSize { body, .. }
        | Atom::Color { body, .. }
        | Atom::Phantom(body)
        | Atom::Padded { body, .. }
        | Atom::CrampedStyle(body, _)
        | Atom::ScriptPolicy { body, .. } => returned_script_level(body, script),
        Atom::Sequence(children) | Atom::InferredRow(children) | Atom::ExplicitRow(children) => {
            children
                .iter()
                .rev()
                .find(|child| !operator_dictionary::spacelike(child))
                .map_or(script, |child| returned_script_level(child, script))
        }
        _ => script && !xarrow::returns_level_zero_core(atom),
    }
}
impl Plan {
    fn collect(&mut self, atom: &Atom, script: bool) {
        let start = self.classes.len();
        match atom {
            Atom::Sequence(children) => {
                for child in children {
                    self.collect(child, script);
                }
                // isLastTeXAtom tests the actual final child, including
                // spacers. A transparent trailing mspace must not make the
                // preceding binary operator the row's final TeX atom.
                let final_atom = children.last().is_some_and(|a| {
                    let (start, end) = self.span(a);
                    start < end
                });
                if self.classes.len() > start && final_atom {
                    self.row_ends.push(self.classes.len() - 1);
                }
            }
            Atom::DeclaredStyle { body, level, .. } => self.collect(body, *level > 0),
            Atom::DeclaredSize { body, .. } => self.collect(body, script),
            Atom::ApplyFunction
            | Atom::Space(_)
            | Atom::Kern(_)
            | Atom::RuleSpace { .. }
            | Atom::EmptyNode("mspace")
            | Atom::Strut(..) => {}
            Atom::Padded { body, .. } | Atom::Phantom(body)
                if operator_dictionary::spacelike(body) => {}
            // An empty mstyle takes no part in TeX-class spacing.
            Atom::Color { body, .. } if matches!(body.as_ref(), Atom::Sequence(v) if v.is_empty()) =>
                {}
            _ => {
                self.classes.push(atom_tex_class(atom));
                self.adjustments
                    .push(classes::adjustment_class(atom, atom_tex_class(atom)));
                self.returns
                    .push(return_tex_class(atom, atom_tex_class(atom)));
                self.levels.push(returned_script_level(atom, script));
                self.operator_previous_levels.push(
                    (xarrow::returns_level_zero_core(atom) || adjusts_current_level(atom))
                        .then_some(script),
                );
                self.automatic_operators
                    .push(matches!(atom, Atom::AutoOperator(_)));
            }
        }
        self.spans
            .insert(atom as *const Atom as usize, (start, self.classes.len()));
    }
    fn span(&self, atom: &Atom) -> (usize, usize) {
        self.spans[&(atom as *const Atom as usize)]
    }
    fn prepare(&mut self) {
        for index in 0..self.classes.len() {
            if index > 0
                && self.automatic_operators[index - 1]
                && matches!(self.adjustments[index], Some(2 | 3))
            {
                self.classes[index - 1] = Some(0);
                self.returns[index - 1] = Some(0);
                self.adjustments[index - 1] = Some(0);
            }
            let previous = if index == 0 {
                None
            } else {
                self.returns[index - 1]
            };
            if self.adjustments[index] == Some(2)
                && (matches!(previous, None | Some(1 | 2 | 3 | 4 | 6))
                    || self.row_ends.contains(&index))
            {
                self.classes[index] = Some(0);
                self.returns[index] = Some(0);
                self.adjustments[index] = Some(0);
            } else if matches!(self.adjustments[index], Some(3 | 5 | 6)) && previous == Some(2) {
                self.classes[index - 1] = Some(0);
                self.returns[index - 1] = Some(0);
            }
        }
    }
    fn layout(
        &self,
        atom: &Atom,
        x: f64,
        baseline: f64,
        scale: f64,
        out: &mut Layout,
        display: bool,
        prime: bool,
    ) -> Option<f64> {
        match atom {
            Atom::Sequence(children) => {
                let stretch = braket::row_dimensions(children, scale, display, prime)?;
                let mut cursor = x;
                let mut local_cursor_em = 0.0;
                let mut width_em = 0.0;
                let mut previous = None;
                for (index, child) in children.iter().enumerate() {
                    let mut boundary_space = 0.0;
                    let mut boundary_em = 0.0;
                    let mut size_boundary_em = 0.0;
                    let (start, end) = self.span(child);
                    if start < end {
                        if let Some(previous_index) = previous {
                            let own_scale = match child {
                                Atom::DeclaredStyle { level, .. } => styles::scale(*level),
                                Atom::DeclaredSize { factor, .. } => scale * factor,
                                _ => scale,
                            };
                            let own_script = match child {
                                Atom::DeclaredStyle { level, .. } => *level > 0,
                                _ => scale < 1.0,
                            };
                            let spacing = tex_class_spacing(
                                self.returns[previous_index],
                                classes::adjustment_class(child, self.classes[start]).or(Some(0)),
                                own_script
                                    || self.operator_previous_levels[start]
                                        .unwrap_or(self.levels[previous_index]),
                            );
                            boundary_space = spacing * own_scale;
                            boundary_em = spacing / 1000.0 * own_scale;
                            size_boundary_em = spacing / 1000.0 * scale;
                        }
                        previous = Some(end - 1);
                    }
                    if table::replaces_leading_space(child) || matches!(child, Atom::InferredRow(_))
                    {
                        boundary_space = 0.0;
                        boundary_em = 0.0;
                        size_boundary_em = 0.0;
                    }
                    let (left_space, right_space) = mml_spacing::row_spaces(children, index, scale);
                    if mml_spacing::core(child).is_some() {
                        boundary_space = left_space * 1000.0 * scale;
                        boundary_em = left_space * scale;
                        size_boundary_em = boundary_em;
                    }
                    out.width_em = None;
                    let before = cursor;
                    cursor += boundary_space;
                    let external_space = boundary_space;
                    let group_start = out.groups.len();
                    let child_x = cursor;
                    let local_x = (local_cursor_em + boundary_em / scale) * 1000.0;
                    out.intrinsic_width_em = None;
                    cursor += if let (
                        Some((height, depth)),
                        Atom::FenceDelimiter { code, class, color },
                    ) = (stretch, child)
                        && table::can_stretch(*code)
                    {
                        braket::layout_stretched_token(
                            *code,
                            *class,
                            color.as_deref(),
                            height,
                            depth,
                            cursor,
                            baseline,
                            scale,
                            out,
                        )?
                    } else {
                        self.layout(child, cursor, baseline, scale, out, display, prime)?
                    };
                    // wrapper.addChildren keeps x in the row's em frame;
                    // place converts it only after applying the child's RScale.
                    // Retain that coordinate when the paint plan is later
                    // divided back out of an inherited script scale.
                    for group in &mut out.groups[group_start..] {
                        if group.x == child_x {
                            group.x_frame = Some((x / scale + local_x, scale));
                        }
                    }
                    let content = out
                        .width_em
                        .unwrap_or((cursor - before - external_space) / 1000.0);
                    // BBox.Append adds child W/L/R before applying RScale.
                    // A mathsize wrapper retains its unscaled body explicitly.
                    let contribution = if let Atom::DeclaredSize { factor, .. } = child {
                        let body = &out.groups.last()?.body;
                        (body.width_em.unwrap_or(body.width / 1000.0)
                            + if external_space == 0.0 {
                                0.0
                            } else {
                                size_boundary_em
                            })
                            * factor
                    } else {
                        content
                            + if external_space == 0.0 {
                                0.0
                            } else {
                                boundary_em
                            }
                    };
                    width_em += contribution + right_space * scale;
                    local_cursor_em += if let Atom::DeclaredSize { factor, .. } = child {
                        let body = &out.groups.last()?.body;
                        (body.intrinsic_width_em.unwrap_or_else(|| {
                            body.width_em.unwrap_or(body.width / 1000.0) / scale
                        }) + size_boundary_em / scale)
                            * factor
                    } else {
                        out.intrinsic_width_em.unwrap_or(content / scale) + boundary_em / scale
                    };
                    local_cursor_em += right_space;
                    cursor += right_space * 1000.0 * scale;
                }
                out.width_em = Some(width_em);
                out.intrinsic_width_em = Some(local_cursor_em);
                Some(cursor - x)
            }
            Atom::DeclaredSize { body, factor } => {
                let mut local = Layout {
                    bounds_em: None,
                    width_em: None,
                    intrinsic_width_em: None,
                    width: 0.0,
                    ascent: 0.0,
                    depth: 0.0,
                    glyphs: Vec::new(),
                    rules: Vec::new(),
                    groups: Vec::new(),
                };
                local.width = self.layout(body, 0.0, 0.0, scale, &mut local, display, prime)?;
                Some(styles::place_size(local, *factor, x, baseline, scale, out))
            }
            Atom::DeclaredStyle {
                body,
                display,
                level,
            } => {
                let own_scale = styles::scale(*level);
                let mut local = Layout {
                    bounds_em: None,
                    width_em: None,
                    intrinsic_width_em: None,
                    width: 0.0,
                    ascent: 0.0,
                    depth: 0.0,
                    glyphs: Vec::new(),
                    rules: Vec::new(),
                    groups: Vec::new(),
                };
                local.width =
                    self.layout(body, 0.0, 0.0, own_scale, &mut local, *display, false)?;
                retain_negative_glyph_depth(&mut local);
                // BBox.Append applies the wrapper's getScale result to
                // its intrinsic em advance before converting to SVG units.
                let wrapper_scale = styles::SCRIPT_MULTIPLIER.powi((*level).min(2) as i32);
                let intrinsic_width_em = local
                    .intrinsic_width_em
                    .unwrap_or_else(|| local.width_em.unwrap_or(local.width / 1000.) / own_scale);
                let width_em = intrinsic_width_em * wrapper_scale;
                let width = width_em * 1000.;
                local.width = width;
                local.width_em = Some(width_em);
                out.width_em = Some(width_em);
                // Keep intrinsic em bounds independent of the font-unit paint frame.
                // BBox.Append applies the declaration scale before SVG unit conversion.
                if own_scale != wrapper_scale {
                    local.bounds_em = Some(styles::em_bounds(&local));
                    local.ascent = (local.ascent / own_scale / 1000.) * wrapper_scale * 1000.;
                    local.depth = (local.depth / own_scale / 1000.) * wrapper_scale * 1000.;
                }
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
                    scale: own_scale,
                    x,
                    baseline,
                    color: None,
                    background: None,
                    kind: "mstyle",
                    delimiter: None,
                    arrow: None,
                    body: Box::new(local),
                    lines: Vec::new(),
                });
                Some(width)
            }
            _ => {
                let group_start = out.groups.len();
                let width = layout_at_with_style(atom, x, baseline, scale, out, display, prime)?;
                let (start, end) = self.span(atom);
                if start < end && atom_tex_class(atom) == Some(2) && self.classes[start] == Some(0)
                {
                    classes::demote_group(&mut out.groups[group_start..]);
                }
                Some(width)
            }
        }
    }
}
pub(super) fn layout(
    atom: &Atom,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    display: bool,
    prime: bool,
) -> Option<f64> {
    let mut plan = Plan {
        automatic_operators: Vec::new(),
        classes: Vec::new(),
        adjustments: Vec::new(),
        returns: Vec::new(),
        levels: Vec::new(),
        operator_previous_levels: Vec::new(),
        spans: HashMap::new(),
        row_ends: Vec::new(),
    };
    plan.collect(atom, scale < 1.0);
    plan.prepare();
    plan.layout(atom, x, baseline, scale, out, display, prime)
}
pub(super) fn has_declaration(atom: &Atom) -> bool {
    match atom {
        Atom::DeclaredStyle { .. } | Atom::DeclaredSize { .. } => true,
        Atom::Sequence(children) => children
            .iter()
            .any(|a| matches!(a, Atom::Sequence(_)) || has_declaration(a)),
        _ => false,
    }
}
