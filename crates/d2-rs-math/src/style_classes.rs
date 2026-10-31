// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Source: mathjax-go v0.1.0 internal/svg/texclass.go, wrapper.go.
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
    spans: HashMap<usize, (usize, usize)>,
    row_ends: Vec<usize>,
}
impl Plan {
    fn collect(&mut self, atom: &Atom, script: bool) {
        let start = self.classes.len();
        match atom {
            Atom::Sequence(children) => {
                for child in children {
                    self.collect(child, script);
                }
                let empty_size_tail = children.last().is_some_and(|a| {
                    matches!(a, Atom::DeclaredSize { .. }) && self.span(a).0 == self.span(a).1
                });
                if self.classes.len() > start && !empty_size_tail {
                    self.row_ends.push(self.classes.len() - 1);
                }
            }
            Atom::DeclaredStyle { body, level, .. } => self.collect(body, *level > 0),
            Atom::DeclaredSize { body, .. } => self.collect(body, script),
            Atom::ApplyFunction | Atom::Space(_) | Atom::Kern(_) | Atom::Strut(..) => {}
            // An empty mstyle takes no part in TeX-class spacing.
            Atom::Color { body, .. } if matches!(body.as_ref(), Atom::Sequence(v) if v.is_empty()) => {}
            _ => {
                self.classes.push(atom_tex_class(atom));
                self.adjustments
                    .push(classes::adjustment_class(atom, atom_tex_class(atom)));
                self.returns
                    .push(return_tex_class(atom, atom_tex_class(atom)));
                self.levels.push(script && !xarrow::accepts_space(atom));
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
                let mut cursor = x;
                let mut width_em = 0.0;
                let mut previous = None;
                for child in children {
                    let mut boundary_space = 0.0;
                    let mut boundary_em = 0.0;
                    let mut size_boundary_em = 0.0;
                    let (start, end) = self.span(child);
                    if start < end {
                        if let Some(previous_index) = previous {
                            let own_scale = if xarrow::accepts_space(child) {
                                1.0
                            } else {
                                match child {
                                    Atom::DeclaredStyle { level, .. } => styles::scale(*level),
                                    Atom::DeclaredSize { factor, .. } => scale * factor,
                                    _ => scale,
                                }
                            };
                            let own_script = if xarrow::accepts_space(child) {
                                false
                            } else {
                                match child {
                                    Atom::DeclaredStyle { level, .. } => *level > 0,
                                    _ => scale < 1.0,
                                }
                            };
                            let spacing = tex_class_spacing(
                                self.returns[previous_index],
                                classes::adjustment_class(child, self.classes[start]).or(Some(0)),
                                own_script || self.levels[previous_index],
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
                    out.width_em = None;
                    let before = cursor;
                    let mut external_space = 0.0;
                    if let Some(spaced) = classes::with_script_space(child, boundary_space)
                        .or_else(|| xarrow::with_space(child, boundary_space))
                    {
                        cursor += layout_at_with_style(
                            &spaced, cursor, baseline, scale, out, display, prime,
                        )?;
                    } else {
                        cursor += boundary_space;
                        external_space = boundary_space;

                        cursor +=
                            self.layout(child, cursor, baseline, scale, out, display, prime)?;
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
                    width_em += contribution;
                }
                out.width_em = Some(width_em);
                Some(cursor - x)
            }
            Atom::DeclaredSize { body, factor } => {
                let mut local = Layout {
                    bounds_em: None,
                    width_em: None,
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
                let width = local.width;
                out.width_em = local.width_em;
                out.ascent = out.ascent.max(baseline + local.ascent);
                out.depth = out.depth.max(local.depth - baseline);
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
        classes: Vec::new(),
        adjustments: Vec::new(),
        returns: Vec::new(),
        levels: Vec::new(),
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
