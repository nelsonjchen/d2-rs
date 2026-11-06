// Copyright 2017-2022 The MathJax Consortium; Apache-2.0.
// Source: pinned mathjax-go tex/ams_tags.go and ams_tags_environments.go;
// svg/{mtable,mtr,typesetter,wrapper}.go.
use super::*;
use std::{cell::RefCell, collections::HashMap, rc::Rc};
pub(super) type Shared = Rc<RefCell<State>>;
#[derive(Clone, Debug)]
pub(super) struct Tag {
    pub raw: String,
    pub text: String,
    pub id: String,
    pub content: Option<Box<Atom>>,
}
#[derive(Clone, Default)]
pub(super) struct Label {
    pub text: String,
    pub id: String,
}
#[derive(Default)]
pub(super) struct Scope {
    pub environment: String,
    pub label: String,
    pub tag: Option<Tag>,
    pub no_tag: bool,
    pub row_tags: Vec<Option<Tag>>,
}
#[derive(Default)]
pub(super) struct State {
    pub current: Scope,
    // amscdCommand stores dimensions in compilation-wide parse state.
    pub cd_minimums: Option<Minimums>,
    pub paired_macro_count: usize,
    pub paired_delimiters: HashMap<String, super::paired_delimiters::Definition>,
    stack: Vec<Scope>,
    pub labels: HashMap<String, Label>,
    pub options: HashMap<String, String>,
    // Parse-state macros defined by \DeclareMathOperator (no parameters).
    pub macros: HashMap<String, String>,
}
impl State {
    pub fn option(&self, key: &str) -> String {
        self.options.get(key).cloned().unwrap_or_else(|| {
            match key {
                "multlinegap" => "1em",
                "multlined-pos" | "smallmatrix-align" => "c",
                "shortvdotsadjustabove" | "shortvdotsadjustbelow" => ".2em",
                "centercolon" | "use-unicode" => "false",
                "centercolon-offset" => ".04em",
                "thincolon-dx" => "-.04em",
                "thincolon-dw" => "-.08em",
                "allow-mathtoolsset" => "true",
                _ => "",
            }
            .into()
        })
    }
    pub fn set_options(&mut self, raw: &str) -> Result<(), String> {
        let settings = keyval_options(raw)?;
        for (key, _) in &settings {
            if !matches!(
                key.as_str(),
                "multlinegap"
                    | "multlined-pos"
                    | "firstline-afterskip"
                    | "lastline-preskip"
                    | "smallmatrix-align"
                    | "shortvdotsadjustabove"
                    | "shortvdotsadjustbelow"
                    | "centercolon"
                    | "centercolon-offset"
                    | "thincolon-dx"
                    | "thincolon-dw"
                    | "use-unicode"
                    | "prescript-sub-format"
                    | "prescript-sup-format"
                    | "prescript-arg-format"
                    | "pairedDelimiters"
            ) {
                return Err(format!("Invalid option: {key}"));
            }
        }
        self.options.extend(settings);
        Ok(())
    }
    pub fn start(&mut self, environment: &str) {
        let prior = std::mem::replace(
            &mut self.current,
            Scope {
                environment: environment.into(),
                ..Scope::default()
            },
        );
        self.stack.push(prior);
    }
    pub fn end(&mut self) {
        self.current = self.stack.pop().unwrap_or_default();
    }
    pub fn label(&mut self, label: String) -> Result<(), String> {
        if label.is_empty() {
            return Ok(());
        }
        if !self.current.label.is_empty() {
            return Err("Multiple \\label".into());
        }
        self.current.label = label.clone();
        if self.labels.contains_key(&label) {
            return Err(format!("Label '{label}' multiply defined"));
        }
        self.labels.insert(
            label,
            Label {
                text: "???".into(),
                id: String::new(),
            },
        );
        Ok(())
    }
    pub fn set_tag(&mut self, raw: String, star: bool) {
        let raw = raw.trim().to_owned();
        self.current.tag = Some(Tag {
            text: if star {
                raw.clone()
            } else {
                format!("({raw})")
            },
            raw,
            id: String::new(),
            content: None,
        });
        self.current.no_tag = false;
    }
    pub fn notag(&mut self) {
        self.set_tag(String::new(), true);
        self.current.no_tag = true;
    }
    pub fn make_tag(&mut self) -> Tag {
        let mut tag = self.current.tag.clone().unwrap();
        let id_source = if self.current.label.is_empty() {
            &tag.raw
        } else {
            &self.current.label
        };
        tag.id = format!(
            "mjx-eqn:{}",
            id_source
                .chars()
                .map(|c| if c.is_whitespace() || c == '\u{feff}' {
                    '_'
                } else {
                    c
                })
                .collect::<String>()
        );
        if !self.current.label.is_empty() {
            self.labels.insert(
                self.current.label.clone(),
                Label {
                    text: tag.raw.clone(),
                    id: tag.id.clone(),
                },
            );
        }
        self.current.tag.as_mut().unwrap().id = tag.id.clone();
        tag
    }
}
// Release the state borrow before executing the tag's fresh parser: tag text
// can itself access labels, commands and the live tag ID.
pub(super) fn make_tag(shared: &Shared) -> Option<Tag> {
    let mut tag = shared.borrow_mut().make_tag();
    let parser = Parser {
        in_root: false,
        root_moves: Default::default(),
        physics_after: None,
        physics_auto: None,
        command_position: None,
        pending_cell: None,
        pending_infix: None,
        row_delimiter: None,
        active_color: None,
        equation_seen: Default::default(),
        array_cell: None,
        vector_alias: false,
        vector_font: None,
        ams: shared.clone(),
        splice_inferred: false,
        chars: Vec::new(),
        index: 0,
        active_font: None,
        normal_font: false,
        calligraphic_font: false,
        multi_letter_font: false,
        operator_letters: false,
        operator_pattern: false,
        no_auto_operator: false,
    };
    let source = format!("\\text{{{}}}", tag.text);
    let mut content = parser.parse_internal_math(&source)?;
    if cd::first_error(&content).is_none() {
        table::inherit_columns(&mut content, 'c');
        containers(&mut content, "center");
        styles::resolve_sizes(&mut content, 1.);
        styles::resolve(&mut content, true, 0, false);
        operator_dictionary::inherit(&mut content, operator_dictionary::Form::Infix);
    }
    tag.id = shared
        .borrow()
        .current
        .tag
        .as_ref()
        .map_or_else(String::new, |t| t.id.clone());
    tag.content = Some(Box::new(content));
    Some(tag)
}
pub(super) fn get_tag(shared: &Shared) -> Option<Option<Tag>> {
    let eligible = {
        let state = shared.borrow();
        let scope = &state.current;
        scope.tag.as_ref().is_some_and(|t| !t.raw.is_empty())
            && !scope.no_tag
            && matches!(scope.environment.as_str(), "equation" | "align" | "align*")
    };
    if eligible {
        Some(Some(make_tag(shared)?))
    } else {
        Some(None)
    }
}
pub(super) fn end_row(shared: &Shared) -> Option<()> {
    let tag = get_tag(shared)?;
    let mut state = shared.borrow_mut();
    state.current.row_tags.push(tag);
    state.current.label.clear();
    state.current.tag = None;
    state.current.no_tag = false;
    Some(())
}
pub(super) fn uri(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char)
        } else {
            out.push_str(&format!("%{b:02X}"))
        }
    }
    out
}
pub(super) struct LabelLayout {
    pub id: String,
    pub body: Box<Layout>,
}
pub(super) struct TaggedLayout {
    pub labels: Vec<Option<LabelLayout>>,
    pub label_width: f64,
    pub pad: f64,
    pub top: bool,
    pub align: &'static str,
    pub wrap_width: Option<f64>,
}
pub(super) fn copy_tagged(t: &TaggedLayout, scale: f64) -> TaggedLayout {
    TaggedLayout {
        labels: t
            .labels
            .iter()
            .map(|l| {
                l.as_ref().map(|l| LabelLayout {
                    id: l.id.clone(),
                    body: Box::new(layout_in_parent_frame(&l.body, scale)),
                })
            })
            .collect(),
        label_width: t.label_width / scale,
        pad: t.pad / scale,
        top: t.top,
        align: t.align,
        wrap_width: t.wrap_width.map(|w| w / scale),
    }
}
pub(super) fn layout_tagged(
    rows: &[Vec<Atom>],
    tags: &[Option<Tag>],
    environment: &str,
    align: &'static str,
    spacing: Option<f64>,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
) -> Option<f64> {
    let mut measured = Vec::new();
    let mut label_width = 0.0_f64;
    for tag in tags {
        let label = if let Some(t) = tag {
            let mut body = Layout {
                bounds_em: None,
                width_em: None,
                intrinsic_width_em: None,
                width: 0.0,
                ascent: 0.0,
                depth: 0.0,
                glyphs: vec![],
                rules: vec![],
                groups: vec![],
            };
            // An empty tag text (\notag, \tag*{}) leaves the label mtd empty.
            if !t.text.is_empty() {
                body.width = layout_at_with_style(
                    t.content.as_deref().unwrap_or(&Atom::Text(t.text.clone())),
                    0.0,
                    0.0,
                    scale,
                    &mut body,
                    true,
                    false,
                )?;
            }
            label_width = label_width.max(body.width);
            Some(LabelLayout {
                id: t.id.clone(),
                body: Box::new(body),
            })
        } else {
            None
        };
        measured.push(label);
    }
    let mut local = Layout {
        bounds_em: None,
        width_em: None,
        intrinsic_width_em: None,
        width: 0.0,
        ascent: 0.0,
        depth: 0.0,
        glyphs: vec![],
        rules: vec![],
        groups: vec![],
    };
    local.width = table::layout(
        rows,
        environment,
        "",
        None,
        spacing,
        0.0,
        0.0,
        scale,
        &mut local,
        false,
    )?;
    let mut group = local.groups.pop()?;
    for (i, row) in group.body.groups.iter_mut().enumerate() {
        if measured.get(i).is_some_and(Option::is_some) {
            row.kind = "mlabeledtr";
        }
    }
    let center = align == "center";
    // Source stores body/label dimensions and L/R padding in em, then scales
    // the composed outer box once at the parent boundary.
    let pad_em = label_width / (1000.0 * scale) + 0.8;
    let pad = pad_em * scale * 1000.0;
    let width = (local.width / (1000.0 * scale) + if center { 2.0 * pad_em } else { pad_em })
        * scale
        * 1000.0;
    group.tagged = Some(TaggedLayout {
        labels: measured,
        label_width,
        pad,
        top: true,
        align,
        wrap_width: None,
    });
    group.x = x + if center { pad } else { 0.0 };
    group.baseline = baseline;
    group.before_glyph = out.glyphs.len();
    out.ascent = out.ascent.max(baseline + local.ascent);
    out.depth = out.depth.max(local.depth - baseline);
    out.groups.push(group);
    Some(width)
}
pub(super) fn mark(layout: &mut Layout, top: bool) {
    for g in &mut layout.groups {
        if let Some(t) = &mut g.tagged {
            t.top = top;
        }
        if let Some(t) = &mut g.percentage {
            t.top = top;
        }
        let transparent = matches!(g.kind, "" | "mrow" | "#inferred-place")
            || matches!(g.kind, "TeXAtom" | "mstyle")
                && g.body.groups.len() == 1
                && g.body.glyphs.is_empty()
                && g.body.rules.is_empty();
        mark(&mut g.body, top && transparent);
    }
}
pub(super) fn responsive(layout: &Layout) -> bool {
    layout
        .groups
        .iter()
        .any(|g| g.tagged.as_ref().is_some_and(|t| t.top) || responsive(&g.body))
}
fn inner(layout: &Layout) -> String {
    let svg = render_layout(layout);
    let marker = "<g data-mml-node=\"math\">";
    let start = svg.find(marker).unwrap() + marker.len();
    let end = svg.len() - "</g></g></svg>".len();
    svg[start..end].to_owned()
}
pub(super) fn render_tagged(svg: &mut String, g: &LayoutGroup) {
    let t = copy_tagged(g.tagged.as_ref().unwrap(), g.scale);
    let body = layout_in_parent_frame(&g.body, g.scale);
    let w = body.width;
    let h = body.ascent;
    let d = body.depth;
    let labels = t
        .labels
        .iter()
        .enumerate()
        .filter_map(|(i, l)| l.as_ref().map(|l| (i, l)))
        .map(|(i, l)| {
            let y = body.groups.get(i).map_or(0.0, |r| r.baseline);
            let x = t.label_width - l.body.width;
            // wrapper.place invokes handleID only for a nonzero placement.
            // Keep the anchor at the top of the cell and compensate its content.
            let positioned = x != 0.0 || y != 0.0;
            let h = if positioned && !l.id.is_empty() {
                l.body.ascent
            } else {
                0.0
            };
            let placement = if positioned {
                format!(
                    " transform=\"translate({},{})\"",
                    root_number(x),
                    root_number(y + h)
                )
            } else {
                String::new()
            };
            let content = if positioned && !l.id.is_empty() {
                format!(
                    "<text data-id-align=\"true\"></text><g data-idbox=\"true\" transform=\"translate(0,{})\">{}</g>",
                    root_number(-h),
                    inner(&l.body)
                )
            } else {
                inner(&l.body)
            };
            format!(
                "<g data-mml-node=\"mtd\" id=\"{}\"{}>{}</g>",
                escape(&l.id),
                placement,
                content
            )
        })
        .collect::<String>();
    let placement = if g.x == 0.0 && g.baseline == 0.0 {
        String::new()
    } else {
        format!(
            "translate({},{})",
            root_number(g.x),
            root_number(g.baseline)
        )
    };
    let scale = if g.scale == 1.0 {
        String::new()
    } else {
        format!(" scale({})", number(g.scale))
    };
    let cancel = if t.top {
        format!(" translate({},0)", root_number(-t.pad))
    } else {
        String::new()
    };
    let transform = format!("{placement}{cancel}{scale}").trim().to_owned();
    let attr = if transform.is_empty() {
        String::new()
    } else {
        format!(" transform=\"{transform}\"")
    };
    svg.push_str(&format!("<g data-mml-node=\"mtable\"{attr}>"));
    if t.top {
        svg.push_str(&format!("<g transform=\"translate(0 {}) matrix(1 0 0 -1 0 0) scale(55.25)\"><svg data-table=\"true\" preserveAspectRatio=\"xMidYMid\" viewBox=\"{} {} 1 {}\"><g transform=\"matrix(1 0 0 -1 0 0)\">{}</g></svg><svg data-labels=\"true\" preserveAspectRatio=\"xMaxYMid\" viewBox=\"{} {} 1 {}\"><g data-labels=\"true\" transform=\"matrix(1 0 0 -1 0 0)\">{}</g></svg></g>",root_number(h),root_number(w/2.0),root_number(-h),root_number(h+d),inner(&body),root_number(t.label_width),root_number(-h),root_number(h+d),labels));
    } else {
        svg.push_str(&inner(&body));
        let full_width = w + if t.align == "center" {
            2.0 * t.pad
        } else {
            t.pad
        };
        let extra = t.wrap_width.unwrap_or(full_width).max(full_width) - full_width;
        let x = w + t.pad - t.label_width
            + match t.align {
                "left" => extra,
                "center" => extra / 2.0,
                _ => 0.0,
            };
        let attr = if x == 0.0 {
            String::new()
        } else {
            format!(" transform=\"translate({},0)\"", root_number(x))
        };
        svg.push_str(&format!("<g data-labels=\"true\"{attr}>{labels}</g>"));
    }
    svg.push_str("</g>");
}

// CommonMtable.findContainer: inferred rows/styles/TeXAtoms are transparent;
// fractions and ordinary matrix cells center, while enclosures default left.
pub(super) fn containers(a: &mut Atom, align: &'static str) {
    match a {
        Atom::TaggedTable {
            rows,
            container_align,
            environment,
            ..
        } => {
            *container_align = align;
            for row in rows {
                for (i, c) in row.iter_mut().enumerate() {
                    let aligned = if environment.contains("align") || environment == "split" {
                        if i % 2 == 0 { "right" } else { "left" }
                    } else {
                        "center"
                    };
                    containers(c, aligned);
                }
            }
        }
        Atom::Matrix {
            rows,
            environment,
            column_spec,
            ..
        } => {
            let aligns: Vec<_> = if environment == "aligned" {
                Vec::new()
            } else {
                column_spec
                    .chars()
                    .filter(|c| matches!(c, 'l' | 'r' | 'c'))
                    .collect()
            };
            for row in rows {
                for (i, c) in row.iter_mut().enumerate() {
                    let aligned = aligns.get(i).or_else(|| aligns.last()).map_or(
                        if environment.contains("align") || environment == "split" {
                            if i % 2 == 0 { "right" } else { "left" }
                        } else {
                            "center"
                        },
                        |c| match c {
                            'l' => "left",
                            'r' => "right",
                            _ => "center",
                        },
                    );
                    containers(c, aligned)
                }
            }
        }
        Atom::Sequence(v)
        | Atom::InferredRow(v)
        | Atom::ExplicitRow(v)
        | Atom::Identifier(v)
        | Atom::Number(v)
        | Atom::OperatorToken(v) => {
            for c in v {
                containers(c, align)
            }
        }
        Atom::Fraction(a, b) | Atom::SplitFraction(a, b, _) | Atom::GeneralFraction(a, b, _) => {
            containers(a, "center");
            containers(b, "center")
        }
        Atom::Boxed(a)
        | Atom::Sqrt(a)
        | Atom::Root { body: a, .. }
        | Atom::SizedSqrt { body: a, .. }
        | Atom::Cancel { body: a, .. }
        | Atom::ColorBox { body: a, .. } => containers(a, "left"),
        Atom::Mrow(a)
        | Atom::Ord(a)
        | Atom::Class { body: a, .. }
        | Atom::DeclaredSize { body: a, .. }
        | Atom::DeclaredStyle { body: a, .. }
        | Atom::CrampedStyle(a, _)
        | Atom::Color { body: a, .. }
        | Atom::Reference { body: a, .. }
        | Atom::MovesScripts(a)
        | Atom::ScriptPolicy { body: a, .. } => containers(a, align),
        Atom::Padded { body: a, .. } | Atom::Phantom(a) => containers(a, "left"),
        Atom::Multiscripts {
            base,
            sub,
            sup,
            post,
            ..
        } => {
            for a in [base, sub, sup] {
                containers(a, "left");
            }
            if let Some(v) = post {
                for a in v {
                    containers(a, "left");
                }
            }
        }
        Atom::Scripts { base, sub, sup, .. } => {
            containers(base, "left");
            for c in [sub, sup].into_iter().flatten() {
                containers(c, "left")
            }
        }
        _ => {}
    }
}

// CommonMtable.containerWrapWidth: transparent frames retain the nearest real
// container's resolved width; nested tables resolve their own cell widths.
pub(super) fn set_wrap_width(layout: &mut Layout, width: f64) {
    for group in &mut layout.groups {
        if let Some(tagged) = &mut group.tagged {
            tagged.wrap_width = Some(width);
        } else if let Some(table) = &mut group.percentage {
            table.wrap_width = Some(width);
        } else if matches!(
            group.kind,
            "" | "mrow" | "TeXAtom" | "mstyle" | "#inferred-place"
        ) {
            set_wrap_width(&mut group.body, width);
        }
    }
}

pub(super) fn set_percentage_wrap(layout: &mut Layout, width: f64, align: &'static str) {
    for group in &mut layout.groups {
        if let Some(table) = &mut group.percentage {
            table.wrap_width = Some(width);
            table.container_align = align;
        } else if matches!(
            group.kind,
            "" | "mrow" | "TeXAtom" | "mstyle" | "#inferred-place"
        ) {
            set_percentage_wrap(&mut group.body, width, align);
        }
    }
}

// tex/keyval_options.go parseUtilKeyvalOptions, including Object.keys order.
// Boolean option values have the same string form as fmt.Sprint in the Go
// mathtools consumer. Parse the entire list before validating any option.
fn keyval_options(mut rest: &str) -> Result<Vec<(String, String)>, String> {
    let mut settings: Vec<(String, String)> = Vec::new();
    while !rest.is_empty() {
        let (key, end, next) = option_value(rest, "=,")?;
        rest = next;
        let value = if end == '=' {
            let (value, _, next) = option_value(rest, ",")?;
            rest = next;
            value
        } else {
            if key.is_empty() {
                continue;
            }
            "true".to_owned()
        };
        if key == "__proto__" {
            continue;
        }
        if let Some((_, old)) = settings.iter_mut().find(|(k, _)| k == &key) {
            *old = value;
        } else {
            settings.push((key, value));
        }
    }
    let index = |key: &str| {
        key.parse::<u32>()
            .ok()
            .filter(|&n| n < u32::MAX && n.to_string() == key)
    };
    settings.sort_by_key(|(key, _)| index(key).map_or((true, 0), |n| (false, n)));
    Ok(settings)
}

// tex/empheq_handlers.go empheqReadOptionValue and empheqRemoveOptionBraces.
// The source deliberately does not give backslash an escaping role here.
fn option_value<'a>(text: &'a str, endings: &str) -> Result<(String, char, &'a str), String> {
    let mut braces = 0i32;
    let mut start = 0i32;
    let mut start_count = true;
    let mut stop_count = false;
    let clean = |mut value: &str, count: i32, stop: bool| {
        if stop {
            return "true".to_owned();
        }
        for _ in 0..count.max(0) {
            value = value.trim_matches(|c| Parser::prime_space(&c));
            if value.len() >= 2 {
                value = &value[1..value.len() - 1];
            }
        }
        value.trim_matches(|c| Parser::prime_space(&c)).to_owned()
    };
    for (index, character) in text.char_indices() {
        match character {
            ' ' => {}
            '{' => {
                if start_count {
                    start += 1;
                } else {
                    stop_count = false;
                    start = start.min(braces);
                }
                braces += 1;
            }
            '}' => {
                if braces > 0 {
                    braces -= 1;
                }
                if start_count || stop_count {
                    start -= 1;
                    stop_count = true;
                }
                start_count = false;
            }
            _ => {
                if braces == 0 && endings.contains(character) {
                    return Ok((
                        clean(&text[..index], start, stop_count),
                        character,
                        &text[index + character.len_utf8()..],
                    ));
                }
                start_count = false;
                stop_count = false;
            }
        }
    }
    if braces != 0 {
        return Err("Extra open brace or missing close brace".into());
    }
    Ok((clean(text, start, stop_count), '\0', ""))
}

#[cfg(test)]
mod keyval_contract_tests {
    use super::*;

    #[test]
    fn mathtools_keyvals_keep_nested_arguments_and_source_boolean_braces() {
        let mut state = State::default();
        state
            .set_options(r"prescript-sub-format={\color{red}},prescript-sup-format={}")
            .unwrap();
        assert_eq!(state.option("prescript-sub-format"), r"\color{red}");
        assert_eq!(state.option("prescript-sup-format"), "true");
        let before = state.options.clone();
        assert_eq!(
            state.set_options("unknown=1,prescript-arg-format={"),
            Err("Extra open brace or missing close brace".into())
        );
        assert_eq!(state.options, before);
        assert_eq!(
            keyval_options("z=one,2=two,0=zero,z=last,__proto__=x").unwrap(),
            vec![
                ("0".into(), "zero".into()),
                ("2".into(), "two".into()),
                ("z".into(), "last".into())
            ]
        );
    }
}
