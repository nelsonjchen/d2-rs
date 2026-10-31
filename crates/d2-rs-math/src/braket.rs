// Copyright 2017–2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned mathjax-go tex/commands.go physicsBraket, braket, physicsFollowingKet;
// tex/parser.go fenced/leftRightFenced and svg/stretch.go.
use super::*;

fn token(code: u32, class: usize) -> Atom {
    Atom::FenceDelimiter { code, class }
}

fn ord(body: Atom) -> Atom {
    Atom::Ord(Box::new(body))
}
fn fenced(open: u32, body: Vec<Atom>, close: u32, stretchy: bool, inner: bool) -> Atom {
    Atom::Fenced {
        open,
        body,
        close,
        stretchy,
        inner,
    }
}
impl Parser {
    fn skip_whitespace(&mut self) {
        while self
            .chars
            .get(self.index)
            .is_some_and(|c| c.is_whitespace())
        {
            self.index += 1;
        }
    }
    fn braket_star(&mut self) -> bool {
        self.skip_whitespace();
        if self.chars.get(self.index) == Some(&'*') {
            self.index += 1;
            true
        } else {
            false
        }
    }
    fn following_ket(&mut self) -> Option<(Atom, bool)> {
        let start = self.index;
        self.skip_whitespace();
        let result = (|| {
            if self.chars.get(self.index) != Some(&'\\') {
                return None;
            }
            self.index += 1;
            if self.capture_control_sequence() != "ket" {
                return None;
            }
            let star = self.braket_star();
            self.skip_whitespace();
            if self.chars.get(self.index) != Some(&'{') {
                return None;
            }
            let raw = self.padded_raw_argument("ket").ok()?;
            Some((self.padded_parse_string(&raw)?, star))
        })();
        if result.is_none() {
            self.index = start;
        }
        result
    }
    pub(super) fn native_braket(&mut self, name: &str) -> Option<Atom> {
        // symbols.go initializes these macros before command dispatch. Their
        // substituted text joins the remaining input without a control-word
        // separator, just as invokeMacro does in the owning source parser.
        let recipe = match name {
            "Bra" => Some("\\left\\langle #1\\right|"),
            "Ket" => Some("\\left|#1\\right\\rangle"),
            "Braket" => Some("\\left\\langle #1\\right\\rangle"),
            "Set" => Some("\\left\\{#1\\right\\}"),
            _ => None,
        };
        if let Some(recipe) = recipe {
            let raw = match self.padded_raw_argument(name) {
                Ok(s) => s,
                Err(e) => return Some(Atom::ErrorDisplay(e)),
            };
            self.chars
                .splice(self.index..self.index, recipe.replace("#1", &raw).chars());
            return self.atom();
        }
        if matches!(name, "Bra" | "Ket" | "Braket" | "Set" | "set") {
            let content = self.padded_argument(name)?;
            let body = match content {
                Atom::Sequence(v) => v,
                a => vec![a],
            };
            let (open, close) = match name {
                "Bra" => (0x27e8, 0x7c),
                "Ket" => (0x7c, 0x27e9),
                "Set" | "set" => (0x7b, 0x7d),
                _ => (0x27e8, 0x27e9),
            };
            return Some(fenced(open, body, close, name != "set", true));
        }
        let star = if matches!(name, "bra" | "ket" | "braket" | "innerproduct" | "ip") {
            self.braket_star()
        } else {
            false
        };
        let left = self.padded_argument(name)?;
        if matches!(left, Atom::ErrorDisplay(_)) {
            return Some(left);
        }
        let mut body = vec![ord(left.clone())];
        let (open, close, stretchy, inner) = match name {
            "bra" => {
                if let Some((right, ket_star)) = self.following_ket() {
                    // \left\langle{bra}\middle\vert{ket}\right\rangle versus
                    // the starred \langle{bra}\vert{ket}\rangle expansion.
                    if star || ket_star {
                        body.push(token(0x7c, 0));
                    } else {
                        body.extend([token(0, 5), token(0x7c, 0), token(0, 4)]);
                    }
                    body.push(ord(right));
                    (0x27e8, 0x27e9, !(star || ket_star), !(star || ket_star))
                } else {
                    (0x27e8, 0x7c, !star, !star)
                }
            }
            "ket" => (0x7c, 0x27e9, !star, !star),
            "braket" | "innerproduct" | "ip" => {
                self.skip_whitespace();
                let right = if self.chars.get(self.index) == Some(&'{') {
                    self.padded_argument(name)?
                } else {
                    left
                };
                if star {
                    body.push(token(0x7c, 0));
                } else {
                    body.extend([token(0, 5), token(0x7c, 0), token(0, 4)]);
                }
                body.push(ord(right));
                (0x27e8, 0x27e9, !star, !star)
            }
            _ => return None,
        };
        let result = fenced(open, body, close, stretchy, inner);
        // The source expansion of an independent bra ends with an empty group.
        Some(if name == "bra" && close == 0x7c {
            Atom::Sequence(vec![
                result,
                self.attach_scripts(ord(Atom::Sequence(vec![])))?,
            ])
        } else {
            result
        })
    }
    pub(super) fn native_delimiter(&mut self) -> Result<u32, String> {
        self.skip_whitespace();
        let Some(c) = self.chars.get(self.index).copied() else {
            return Err("Missing or unrecognized delimiter".into());
        };
        self.index += 1;
        let escaped = c == '\\';
        let name = if escaped {
            self.capture_control_sequence()
        } else {
            c.to_string()
        };
        Ok(match name.as_str() {
            "." => 0,
            "(" | "lparen" => 0x28,
            ")" | "rparen" => 0x29,
            "[" | "lbrack" => 0x5b,
            "]" | "rbrack" => 0x5d,
            "{" | "lbrace" => 0x7b,
            "}" | "rbrace" => 0x7d,
            "|" if !escaped => 0x7c,
            "vert" | "lvert" | "rvert" => 0x7c,
            "|" | "Vert" | "lVert" | "rVert" => 0x2016,
            "langle" => 0x27e8,
            "rangle" => 0x27e9,
            "lceil" => 0x2308,
            "rceil" => 0x2309,
            "lfloor" => 0x230a,
            "rfloor" => 0x230b,
            // Reachable parser::readDelimiter uses delimiterSymbols (U+2216).
            "backslash" => 0x2216,
            "uparrow" => 0x2191,
            "downarrow" => 0x2193,
            "updownarrow" => 0x2195,
            "Uparrow" => 0x21d1,
            "Downarrow" => 0x21d3,
            "Updownarrow" => 0x21d5,
            _ => return Err(format!("Missing or unrecognized delimiter for \\{name}")),
        })
    }
    pub(super) fn left_right(&mut self) -> Option<Atom> {
        let open = match self.native_delimiter() {
            Ok(c) => c,
            Err(e) => return Some(Atom::ErrorDisplay(e)),
        };
        let content = self.sequence(Some('\0'))?;
        if cd::first_error(&content).is_some() {
            return Some(content);
        }
        if self.chars.get(self.index) != Some(&'\\') {
            return Some(Atom::ErrorDisplay("Missing \\right".into()));
        }
        self.index += 1;
        if self.capture_control_sequence() != "right" {
            return Some(Atom::ErrorDisplay("Missing \\right".into()));
        }
        let close = match self.native_delimiter() {
            Ok(c) => c,
            Err(e) => return Some(Atom::ErrorDisplay(e)),
        };
        let body = match content {
            Atom::Sequence(v) => v
                .into_iter()
                .flat_map(|a| {
                    if let Atom::Sequence(v) = a {
                        v
                    } else {
                        vec![a]
                    }
                })
                .collect(),
            a => vec![a],
        };
        Some(fenced(open, body, close, true, true))
    }
    pub(super) fn middle(&mut self) -> Option<Atom> {
        match self.native_delimiter() {
            Ok(c) => Some(Atom::Sequence(vec![token(0, 5), token(c, 0), token(0, 4)])),
            Err(e) => Some(Atom::ErrorDisplay(e)),
        }
    }
}

fn empty_group(kind: &'static str, x: f64, scale: f64, before_glyph: usize) -> LayoutGroup {
    LayoutGroup {
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
        before_glyph,
        x,
        baseline: 0.0,
        scale,
        color: None,
        background: None,
        kind,
        delimiter: None,
        arrow: None,
        body: Box::new(blank()),
        lines: vec![],
    }
}
fn blank() -> Layout {
    Layout {
        bounds_em: None,
        width_em: None,
        width: 0.0,
        ascent: 0.0,
        depth: 0.0,
        glyphs: vec![],
        rules: vec![],
        groups: vec![],
    }
}
pub(super) fn layout(
    open: u32,
    body: &[Atom],
    close: u32,
    stretchy: bool,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    display: bool,
    prime: bool,
) -> Option<f64> {
    let mut measure = blank();
    measure.width = layout_at_with_style(
        &Atom::Sequence(body.to_vec()),
        0.0,
        0.0,
        scale,
        &mut measure,
        display,
        prime,
    )?;
    let (h, d) = (
        measure.ascent / 1000.0 / scale,
        measure.depth / 1000.0 / scale,
    );
    let mut row = blank();
    let mut children = vec![token(open, 4)];
    children.extend_from_slice(body);
    children.push(token(close, 5));
    let classes = sequence_tex_classes(&children);
    let mut cursor = 0.0;
    let mut cursor_em = 0.0;
    let mut previous = None;
    for (i, a) in children.iter().enumerate() {
        if !operator_dictionary::spacelike(a) {
            if let Some(j) = previous {
                // As in the row layout: getSpace reads effectiveTeXClass,
                // which treats a null class (mfrac, msqrt) as ORD.
                let spacing = tex_class_spacing(
                    return_tex_class(&children[j], classes[j]),
                    classes::adjustment_class(a, classes[i]).or(Some(0)),
                    scale < 1.0,
                );
                cursor += spacing * scale;
                cursor_em += spacing / 1000.0;
            }
            previous = Some(i);
        }
        // Nonstretchy zero delimiters stand for an unfenced explicit row
        // in cfrac/genfrac; they contribute no SVG child node.
        if !stretchy && matches!(a, Atom::FenceDelimiter { code: 0, .. }) {
            continue;
        }
        // Original \middle (tex/parser.go middle) brackets its mo with empty
        // TeXAtom CLOSE/OPEN nodes; texAtomToSVG still emits each as a node.
        if let Atom::FenceDelimiter { code: 0, class } = a
            && i != 0
            && i + 1 != children.len()
        {
            let mut group = empty_group("TeXAtom", cursor, scale, row.glyphs.len());
            group.tex_class = *class;
            row.groups.push(group);
            continue;
        }
        // An outer "." delimiter is a symmetric empty mo; moToSVG centers its
        // EmptyBBox (H = D = -BigDimen), so u = axis height 0.25em.
        if matches!(a, Atom::FenceDelimiter { code: 0, .. }) {
            let mut group = empty_group("mo", cursor, scale, row.glyphs.len());
            group.own_translate = Some(250.0);
            row.groups.push(group);
            continue;
        }
        let delim = match a {
            Atom::FenceDelimiter { code, .. } if *code != 0 => Some(*code),
            _ => None,
        };
        if let Some(code) = delim.filter(|code| stretchy && table::can_stretch(*code)) {
            let (w, fh, fd, mut group) = table::fence(
                code,
                if stretchy { h } else { 0.0 },
                if stretchy { d } else { 0.0 },
                scale,
            )?;
            group.x = cursor;
            group.before_glyph = row.glyphs.len();
            row.groups.push(group);
            row.ascent = row.ascent.max(fh);
            row.depth = row.depth.max(fd);
            cursor_em += w / (1000.0 * scale);
            cursor += w;
        } else {
            let mut local = blank();
            local.width = layout_at_with_style(a, 0.0, 0.0, scale, &mut local, display, prime)?;
            row.ascent = row.ascent.max(local.ascent);
            row.depth = row.depth.max(local.depth);
            let w = local.width;
            cursor_em += local.width_em.unwrap_or(w / 1000.0) / scale;
            // Source place() translates the child's own semantic frame. Keep
            // that transform ownership rather than inserting another frame.
            if local.glyphs.is_empty() && local.rules.is_empty() && local.groups.len() == 1 {
                let mut group = local.groups.pop()?;
                group.x += cursor;
                group.before_glyph = row.glyphs.len();
                row.groups.push(group);
            } else if local.glyphs.len() == 1 && local.rules.is_empty() && local.groups.is_empty() {
                // A single token child is placed directly (wrapper.place on
                // its own node), like the single-group case above.
                let mut glyph = local.glyphs.pop()?;
                glyph.x += cursor;
                row.glyphs.push(glyph);
            } else {
                row.groups.push(LayoutGroup {
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
                    before_glyph: row.glyphs.len(),
                    x: cursor,
                    baseline: 0.0,
                    scale,
                    color: None,
                    background: None,
                    kind: "",
                    delimiter: None,
                    arrow: None,
                    body: Box::new(local),
                    lines: vec![],
                });
            }
            cursor += w;
        }
    }
    if open == 0 && close == 0 && body.is_empty() {
        row.ascent = 250.0 * scale;
        row.depth = -250.0 * scale;
    }
    row.width = cursor;
    row.width_em = Some(cursor_em * scale);
    out.width_em = Some(cursor_em * scale);
    out.ascent = out.ascent.max(baseline + row.ascent);
    out.depth = if out.glyphs.is_empty() && out.groups.is_empty() && out.rules.is_empty() {
        row.depth - baseline
    } else {
        out.depth.max(row.depth - baseline)
    };
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
        x,
        baseline,
        scale,
        color: None,
        background: None,
        kind: "mrow",
        delimiter: None,
        arrow: None,
        body: Box::new(row),
        lines: vec![],
    });
    Some(cursor)
}

pub(super) fn layout_token(
    code: u32,
    class: usize,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    display: bool,
) -> Option<f64> {
    if code == 0 {
        return Some(0.0);
    }
    let definition = operator_dictionary::Definition {
        class: class as i8,
        left: 0,
        right: 0,
        properties: &[("symmetric", operator_dictionary::Property::Bool(true))],
    };
    operator_dictionary::layout(
        &Atom::Glyph(code),
        definition,
        x,
        baseline,
        scale,
        out,
        display,
    )
}

// Original commands.go bigDelimiter minsize=maxsize skips delimiter shortfall.
pub(super) fn fixed_size(name: &str) -> Option<f64> {
    Some(match name.trim_end_matches(['l', 'r', 'm']) {
        "big" => 1.2,
        "Big" => 1.623,
        "bigg" => 2.047,
        "Bigg" => 2.470,
        _ => return None,
    })
}
pub(super) fn layout_fixed(
    code: u32,
    size: f64,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
) -> Option<f64> {
    if code == 0 {
        return Some(0.0);
    }
    if !table::can_stretch(code) {
        return layout_token(code, 0, x, baseline, scale, out, true);
    }
    let (width, height, depth, mut group) = table::fixed_fence(code, size, scale)?;
    group.x = x;
    group.baseline = baseline;
    group.before_glyph = out.glyphs.len();
    out.ascent = out.ascent.max(baseline + height);
    out.depth = out.depth.max(depth - baseline);
    out.groups.push(group);
    Some(width)
}
