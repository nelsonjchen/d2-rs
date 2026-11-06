// Copyright 2017–2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned mathjax-go tex/commands.go physicsBraket, braket, physicsFollowingKet;
// tex/parser.go fenced/leftRightFenced and svg/stretch.go.
use super::*;

fn token(code: u32, class: usize) -> Atom {
    Atom::FenceDelimiter {
        code,
        class,
        color: None,
    }
}

// tex/left_right.go: retain the closing item across Style/Over unwinding.
#[derive(Clone)]
pub(super) struct RowDelimiter {
    pub middle: bool,
    pub delimiter: u32,
    pub color: Option<String>,
}

// tex/inheritance.go cleanMathMLAttributes removes a redundant mathcolor.
pub(super) fn explicit_color(color: Option<&str>) -> Option<String> {
    color.and_then(|color| {
        INHERITED_COLOR.with(|inherited| {
            (inherited.borrow().last().map(String::as_str) != Some(color)).then(|| color.to_owned())
        })
    })
}

fn empty_class(class: usize) -> Atom {
    Atom::Class {
        body: Box::new(Atom::Sequence(Vec::new())),
        class,
    }
}

fn ord(body: Atom) -> Atom {
    Atom::Ord(Box::new(body))
}
fn fenced(open: u32, body: Vec<Atom>, close: u32, stretchy: bool, inner: bool) -> Atom {
    Atom::Fenced {
        open,
        body,
        close,
        close_color: None,
        stretchy,
        inner,
    }
}
// tex/delimiters.go lookupDelimiter and source_tables_{base,ams,mathtools}_maps.go.
pub(super) fn delimiter(key: &str) -> Option<u32> {
    Some(match key {
        "." => 0,
        "(" | r"\lparen" => 0x28,
        ")" | r"\rparen" => 0x29,
        "[" | r"\lbrack" => 0x5b,
        "]" | r"\rbrack" => 0x5d,
        "<" | r"\lt" | r"\langle" => 0x27e8,
        ">" | r"\gt" | r"\rangle" => 0x27e9,
        "/" => 0x2f,
        "|" | r"\vert" | r"\lvert" | r"\rvert" => 0x7c,
        r"\|" | r"\Vert" | r"\lVert" | r"\rVert" | r"\Arrowvert" => 0x2016,
        r"\\" | r"\backslash" => 0x5c,
        r"\{" | r"\lbrace" => 0x7b,
        r"\}" | r"\rbrace" => 0x7d,
        r"\lceil" => 0x2308,
        r"\rceil" => 0x2309,
        r"\lfloor" => 0x230a,
        r"\rfloor" => 0x230b,
        r"\lmoustache" => 0x23b0,
        r"\rmoustache" => 0x23b1,
        r"\lgroup" => 0x27ee,
        r"\rgroup" => 0x27ef,
        r"\arrowvert" => 0x23d0,
        r"\bracevert" => 0x23aa,
        r"\uparrow" => 0x2191,
        r"\downarrow" => 0x2193,
        r"\updownarrow" => 0x2195,
        r"\Uparrow" => 0x21d1,
        r"\Downarrow" => 0x21d3,
        r"\Updownarrow" => 0x21d5,
        r"\ulcorner" => 0x231c,
        r"\urcorner" => 0x231d,
        r"\llcorner" => 0x231e,
        r"\lrcorner" => 0x231f,
        _ => return None,
    })
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
    fn following_ket(&mut self) -> Result<Option<(Atom, bool)>, String> {
        self.skip_whitespace();
        if self.chars.get(self.index) != Some(&'\\') {
            return Ok(None);
        }
        let start = self.index;
        self.index += 1;
        if self.capture_control_sequence() != "ket" {
            self.index = start;
            return Ok(None);
        }
        // commands.go physicsFollowingKet commits the recognized command,
        // but restores its star/argument lookahead if there is no brace.
        let after_name = self.index;
        let star = self.braket_star();
        self.skip_whitespace();
        if self.chars.get(self.index) != Some(&'{') {
            self.index = after_name;
            return Ok(Some((Atom::Sequence(vec![]), false)));
        }
        let raw = self.padded_raw_argument("bra")?;
        Ok(self.padded_parse_string(&raw).map(|body| (body, star)))
    }
    pub(super) fn native_braket(&mut self, name: &str) -> Option<Atom> {
        // symbols.go initializes these macros before command dispatch;
        // macro_expansion.go preserves control words at substitution and
        // continuation boundaries.
        let recipe = match name {
            "Bra" => Some("{\\left\\langle {#1} \\right\\vert}"),
            "Ket" => Some("{\\left\\vert {#1} \\right\\rangle}"),
            _ => None,
        };
        if let Some(recipe) = recipe {
            let raw = match self.padded_raw_argument(name) {
                Ok(s) => s,
                Err(e) => return Some(Atom::ErrorDisplay(e)),
            };
            let expansion = (|| {
                let mut expansion = paired_delimiters::substitute(recipe, &[raw])?;
                paired_delimiters::add_args(
                    &mut expansion,
                    &self.chars[self.index..].iter().collect::<String>(),
                )?;
                Ok::<_, String>(expansion)
            })();
            self.chars = match expansion {
                Ok(value) => value.chars().collect(),
                Err(message) => return Some(Atom::ErrorDisplay(message)),
            };
            self.index = 0;
            return self.atom();
        }
        if matches!(name, "Braket" | "Set" | "set") {
            // braket.go owns the caller's group/token stream, so commands
            // consume their arguments before the closing brace is delivered.
            self.skip_whitespace();
            if self.index == self.chars.len() {
                return Some(Atom::ErrorDisplay(format!("Missing argument for \\{name}")));
            }
            let content = self.atom_or_group()?;
            if cd::first_error(&content).is_some() {
                return Some(content);
            }
            let body = match content {
                Atom::Sequence(v) => v,
                a => vec![a],
            };
            let (open, close) = match name {
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
                let following = match self.following_ket() {
                    Ok(value) => value,
                    Err(message) => return Some(Atom::ErrorDisplay(message)),
                };
                if let Some((right, ket_star)) = following {
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
    pub(super) fn native_delimiter(
        &mut self,
        command: &str,
        brace_ok: bool,
    ) -> Result<u32, String> {
        self.skip_whitespace();
        let error = || format!("Missing or unrecognized delimiter for \\{command}");
        let Some(c) = self.chars.get(self.index).copied() else {
            return Err(error());
        };
        // parser.go readDelimiter consults exact SymbolMap keys, including
        // the slash on control sequences, and accepts braces only for Big.
        let key = if c == '{' && brace_ok {
            self.padded_raw_argument(command)?.trim().to_owned()
        } else {
            self.index += 1;
            if c == '\\' {
                format!("\\{}", self.capture_control_sequence())
            } else {
                c.to_string()
            }
        };
        delimiter(&key).ok_or_else(error)
    }

    pub(super) fn left_right(&mut self) -> Option<Atom> {
        let open = match self.native_delimiter("left", false) {
            Ok(c) => c,
            Err(e) => return Some(Atom::ErrorDisplay(e)),
        };
        let auto = self.physics_auto.take();
        let outer = self.row_delimiter.take();
        let content = self.left_right_body(open);
        self.row_delimiter = outer;
        self.physics_auto = auto;
        content
    }

    fn left_right_body(&mut self, open: u32) -> Option<Atom> {
        let mut children = Vec::new();
        loop {
            let content = self.sequence_with_prefix(Some('\0'), false, children)?;
            if cd::first_error(&content).is_some() {
                return Some(content);
            }
            let Some(item) = self.row_delimiter.take() else {
                return Some(Atom::ErrorDisplay("Extra \\left or missing \\right".into()));
            };
            children = match content {
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
            if !item.middle {
                return Some(Atom::Fenced {
                    open,
                    body: children,
                    close: item.delimiter,
                    close_color: item.color,
                    stretchy: true,
                    inner: true,
                });
            }
            children.extend([
                empty_class(5),
                Atom::FenceDelimiter {
                    code: item.delimiter,
                    class: 0,
                    color: item.color,
                },
                empty_class(4),
            ]);
        }
    }
    pub(super) fn middle(&mut self) -> Option<Atom> {
        match self.native_delimiter("middle", false) {
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
        intrinsic_width_em: None,
        width: 0.0,
        ascent: 0.0,
        depth: 0.0,
        glyphs: vec![],
        rules: vec![],
        groups: vec![],
    }
}

// svg/mo.go stretchRowChildren runs for every mrow, including the inferred
// rows inside fractions. Sample nonstretchy siblings before resizing their
// delimiters; a delimiter must not contribute its natural height to that
// sample unless every child stretches.
pub(super) fn row_dimensions(
    children: &[Atom],
    scale: f64,
    display: bool,
    prime: bool,
) -> Option<Option<(f64, f64)>> {
    let stretches = |atom: &Atom| matches!(atom, Atom::FenceDelimiter { code, .. } if table::can_stretch(*code));
    if children.len() <= 1 || !children.iter().any(stretches) {
        return Some(None);
    }
    let all = children.iter().all(stretches);
    let (mut height, mut depth) = (0.0_f64, 0.0_f64);
    for child in children {
        // An empty mo has EmptyBBox's negative H/D. It cannot increase the
        // row's zero-initialized sample; its visible frame is emitted later.
        if matches!(child, Atom::FenceDelimiter { code: 0, .. }) {
            continue;
        }
        if !all && stretches(child) {
            continue;
        }
        let (h, d) = if all && let Atom::FenceDelimiter { code, class, .. } = child {
            let symmetric = *class != 0
                || operator_dictionary::lookup(*code, operator_dictionary::Form::Infix)
                    .flag("symmetric");
            let (_, h, d, _) = table::natural_fence(*code, scale, symmetric)?;
            (h, d)
        } else {
            let mut local = blank();
            local.width = layout_at_with_style(child, 0., 0., scale, &mut local, display, prime)?;
            (local.ascent, local.depth)
        };
        height = height.max(h / 1000. / scale);
        depth = depth.max(d / 1000. / scale);
    }
    Some(Some((height, depth)))
}

pub(super) fn layout_stretched_token(
    code: u32,
    class: usize,
    color: Option<&str>,
    height: f64,
    depth: f64,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
) -> Option<f64> {
    let symmetric = class != 0
        || operator_dictionary::lookup(code, operator_dictionary::Form::Infix).flag("symmetric");
    let (width, height, depth, mut group) =
        table::fence_with_symmetry(code, height, depth, scale, symmetric)?;
    group.x = x;
    group.baseline = baseline;
    group.before_glyph = out.glyphs.len();
    group.color = explicit_color(color);
    out.ascent = out.ascent.max(baseline + height);
    out.depth = out.depth.max(depth - baseline);
    out.width_em = Some(width / 1000.);
    out.groups.push(group);
    Some(width)
}

pub(super) fn layout(
    open: u32,
    body: &[Atom],
    close: u32,
    close_color: Option<&str>,
    stretchy: bool,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    display: bool,
    prime: bool,
) -> Option<f64> {
    let mut row = blank();
    let mut children = vec![token(open, 4)];
    children.extend_from_slice(body);
    children.push(Atom::FenceDelimiter {
        code: close,
        class: 5,
        color: close_color.map(str::to_owned),
    });
    // The outer fences and middles share one row's sample. Measuring the
    // body as another row first can resize a middle twice and promote its
    // first selected glyph's height into the outer row's sample.
    let (h, d) = if stretchy {
        row_dimensions(&children, scale, display, prime)?.unwrap_or((0., 0.))
    } else {
        (0., 0.)
    };
    let classes = sequence_tex_classes(&children);
    let mut cursor = 0.0;
    let mut cursor_em = 0.0;
    let mut previous = None;
    for (i, a) in children.iter().enumerate() {
        let mut spacing = 0.;
        if !operator_dictionary::spacelike(a) {
            if let Some(j) = previous {
                // As in the row layout: getSpace reads effectiveTeXClass,
                // which treats a null class (mfrac, msqrt) as ORD.
                spacing = tex_class_spacing(
                    return_tex_class(&children[j], classes[j]),
                    classes::adjustment_class(a, classes[i]).or(Some(0)),
                    scale < 1.0,
                );
            }
            previous = Some(i);
        }
        let (left_space, right_space) = mml_spacing::row_spaces(&children, i, scale);
        if mml_spacing::core(a).is_some() {
            spacing = left_space * 1000.;
        }
        cursor += spacing * scale;
        cursor_em += spacing / 1000.;
        // Nonstretchy zero delimiters stand for an unfenced explicit row
        // in cfrac/genfrac; they contribute no SVG child node.
        if !stretchy && matches!(a, Atom::FenceDelimiter { code: 0, .. }) {
            continue;
        }
        // Original \middle (tex/parser.go middle) brackets its mo with empty
        // TeXAtom CLOSE/OPEN nodes; texAtomToSVG still emits each as a node.
        if let Atom::FenceDelimiter { code: 0, class, .. } = a
            && i != 0
            && i + 1 != children.len()
            && *class != 0
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
            if let Atom::FenceDelimiter { class, color, .. } = a {
                group.color = explicit_color(color.as_deref());
                group.own_translate = (*class != 0).then_some(250.0);
            }
            row.groups.push(group);
            continue;
        }
        let delim = match a {
            Atom::FenceDelimiter { code, .. } if *code != 0 => Some(*code),
            _ => None,
        };
        if let Some(code) = delim.filter(|code| stretchy && table::can_stretch(*code)) {
            let symmetric = !matches!(a, Atom::FenceDelimiter { class: 0, .. })
                || operator_dictionary::lookup(code, operator_dictionary::Form::Infix)
                    .flag("symmetric");
            let (mut w, mut fh, mut fd, mut group) = table::fence_with_symmetry(
                code,
                if stretchy { h } else { 0.0 },
                if stretchy { d } else { 0.0 },
                scale,
                symmetric,
            )?;
            // wrapper.getMathMLSpacing asks the preceding embellished
            // sibling for its bbox during construction. A fixed-size stretch
            // changes its paint variant without invalidating that cached bbox.
            if children
                .get(i + 1)
                .is_some_and(|next| mml_spacing::core(next).is_some())
                && group.delimiter.is_some_and(|d| d.fixed)
            {
                let mut natural = blank();
                natural.width = layout_token(code, 0, 0., 0., scale, &mut natural, display)?;
                w = natural.width;
                fh = natural.ascent;
                fd = natural.depth;
                group.body.ascent = fh;
                group.body.depth = fd;
                group.body.bounds_em = Some(styles::em_bounds(&natural));
                if let Some(delimiter) = &mut group.delimiter {
                    delimiter.height = 0.;
                }
            }
            group.x = cursor;
            group.before_glyph = row.glyphs.len();
            if let Atom::FenceDelimiter { color, .. } = a {
                group.color = explicit_color(color.as_deref());
            }
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
        cursor += right_space * 1000. * scale;
        cursor_em += right_space;
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
        if class == 0 {
            out.groups
                .push(empty_group("mo", x, scale, out.glyphs.len()));
        }
        return Some(0.0);
    }
    let definition = if class == 0 {
        operator_dictionary::lookup(code, operator_dictionary::Form::Infix)
    } else {
        operator_dictionary::Definition {
            class: class as i8,
            left: 0,
            right: 0,
            properties: &[("symmetric", operator_dictionary::Property::Bool(true))],
        }
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
        // MakeBig's present dot is an empty symmetric mo, not an absent
        // delimiter. moToSVG retains its own axis-centering translation.
        out.groups.push(LayoutGroup {
            data_cramped: None,
            own_translate: Some(250.),
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
            kind: "mo",
            delimiter: None,
            arrow: None,
            body: Box::new(general_fraction::blank()),
            lines: Vec::new(),
        });
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
