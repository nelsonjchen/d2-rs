// SPDX-License-Identifier: MPL-2.0
// Original D2 v0.9.0 lib/textmeasure/markdown_svg.go:
// wrapInlineTokens, atomizeInlineTokens, appendInlineTextAtoms,
// inlineUnbrokenWidth, inlineTrailingPaddingWidth, splitInlineTokenAtWidth.
use super::NativeMarkdownRun;
use d2_rs_text::uniseg::{grapheme_boundaries, line_boundaries};
use std::collections::HashSet;

// normalizeMarkdownWhitespaceTree retains the first text node owning a
// pending collapsible space, including across style/link boundaries.
pub(super) fn normalize_whitespace(runs: &mut Vec<NativeMarkdownRun>) {
    let mut outputs = vec![String::new(); runs.len()];
    let mut pending = None;
    let mut at_start = true;
    for (index, run) in runs.iter().enumerate() {
        if run.line_break {
            pending = None;
            at_start = true;
            continue;
        }
        for ch in run.text.chars() {
            if matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{c}') {
                if pending.is_none() {
                    pending = Some(index);
                }
                continue;
            }
            if let Some(owner) = pending.take() {
                if !at_start {
                    outputs[owner].push(' ');
                }
            }
            outputs[index].push(ch);
            at_start = false;
        }
    }
    for (run, text) in runs.iter_mut().zip(outputs) {
        run.text = text;
        if run.code && run.text.is_empty() && !run.line_break {
            run.padding_only = true;
        }
    }
    runs.retain(|run| run.line_break || !run.text.is_empty() || run.code);
}

#[derive(Clone)]
pub(super) struct Token {
    pub run: NativeMarkdownRun,
    pub width: f64,
    pub padding: bool,
    pub leading_padding: bool,
    pub break_line: bool,
    pub break_before: bool,
    pub break_after_soft_hyphen: bool,
    pub soft_hyphen_end: bool,
    pub discretionary_hyphen: bool,
}
impl Token {
    fn new(run: NativeMarkdownRun, width: f64) -> Self {
        Self {
            run,
            width,
            padding: false,
            leading_padding: false,
            break_line: false,
            break_before: false,
            break_after_soft_hyphen: false,
            soft_hyphen_end: false,
            discretionary_hyphen: false,
        }
    }
}
fn atomize(tokens: &[Token], measure: &impl Fn(&NativeMarkdownRun, &str) -> f64) -> Vec<Token> {
    let mut atoms = Vec::new();
    let mut start = 0;
    while start < tokens.len() {
        if tokens[start].break_line {
            atoms.push(tokens[start].clone());
            start += 1;
            continue;
        }
        let mut end = start;
        let mut combined = String::new();
        while end < tokens.len() && !tokens[end].break_line {
            if !tokens[end].padding {
                combined.push_str(&tokens[end].run.text);
            }
            end += 1;
        }
        let graphemes: HashSet<_> = grapheme_boundaries(&combined).into_iter().collect();
        let breaks: HashSet<_> = line_boundaries(&combined)
            .into_iter()
            .map(|(offset, _)| offset)
            .filter(|offset| *offset != combined.len() && graphemes.contains(offset))
            .collect();
        let mut text_offset = 0;
        for token in &tokens[start..end] {
            if token.padding || token.run.text.is_empty() {
                atoms.push(token.clone());
                continue;
            }
            let token_start = text_offset;
            let token_end = token_start + token.run.text.len();
            let mut piece_start = token_start;
            for offset in token_start + 1..=token_end {
                if offset != token_end && !breaks.contains(&offset) {
                    continue;
                }
                let mut piece = &token.run.text[piece_start - token_start..offset - token_start];
                let mut atom = token.clone();
                atom.break_before = breaks.contains(&piece_start);
                atom.break_after_soft_hyphen =
                    atom.break_before && combined[..piece_start].ends_with('\u{ad}');
                if piece.ends_with('\u{ad}') {
                    piece = piece.strip_suffix('\u{ad}').unwrap();
                    atom.soft_hyphen_end = true;
                }
                if piece.is_empty() && atom.soft_hyphen_end {
                    atom.run.text.clear();
                    atom.width = 0.0;
                    atoms.push(atom);
                } else {
                    let content = piece.trim_end_matches(' ');
                    if !content.is_empty() {
                        let mut content_atom = atom.clone();
                        content_atom.run.text = content.to_owned();
                        content_atom.width = measure(&content_atom.run, content);
                        atoms.push(content_atom);
                        atom.break_before = false;
                    }
                    for _ in 0..piece.len() - content.len() {
                        let mut space = atom.clone();
                        space.run.text = " ".to_owned();
                        space.width = measure(&space.run, " ");
                        atoms.push(space);
                        atom.break_before = false;
                    }
                }
                piece_start = offset;
            }
            text_offset = token_end;
        }
        start = end;
    }
    atoms
}
fn unbroken_width(atoms: &[Token], index: usize) -> f64 {
    let mut width = 0.0;
    for (i, atom) in atoms.iter().enumerate().skip(index) {
        if atom.break_line
            || (i > index && !atom.padding && (atom.break_before || atom.run.text == " "))
        {
            break;
        }
        if i == index && atom.run.text == " " && !atom.padding {
            break;
        }
        width += atom.width;
    }
    width
}
fn split(
    token: &Token,
    max_width: f64,
    force: bool,
    measure: &impl Fn(&NativeMarkdownRun, &str) -> f64,
) -> Option<(Token, Token)> {
    let boundaries = grapheme_boundaries(&token.run.text);
    if boundaries.len() < 2 {
        return None;
    }
    let mut best = None;
    for (index, boundary) in boundaries.iter().enumerate() {
        if measure(&token.run, &token.run.text[..*boundary]) > max_width + 0.001 {
            break;
        }
        best = Some(index);
    }
    let mut best = best?;
    if best == boundaries.len() - 1 {
        if !force {
            return None;
        }
        best = best.checked_sub(1)?;
    }
    let boundary = boundaries[best];
    let mut head = token.clone();
    let mut tail = token.clone();
    head.run.text = token.run.text[..boundary].to_owned();
    head.width = measure(&head.run, &head.run.text);
    head.soft_hyphen_end = false;
    tail.run.text = token.run.text[boundary..].to_owned();
    tail.width = measure(&tail.run, &tail.run.text);
    tail.break_before = false;
    tail.break_after_soft_hyphen = false;
    Some((head, tail))
}
fn inline_tokens(
    parsed: &[Vec<NativeMarkdownRun>],
    code_padding: f64,
    measure: &impl Fn(&NativeMarkdownRun, &str) -> f64,
) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut fallback_group = parsed
        .iter()
        .flatten()
        .map(|run| run.code_group)
        .max()
        .unwrap_or(0);
    for (index, runs) in parsed.iter().enumerate() {
        if index > 0 {
            let Some(run) = parsed.iter().flatten().next() else {
                continue;
            };
            let mut token = Token::new(run.clone(), 0.0);
            token.run.text.clear();
            token.break_line = true;
            tokens.push(token);
        }
        let mut position = 0;
        while position < runs.len() {
            let mut run = runs[position].clone();
            if run.code {
                if run.code_group == 0 {
                    fallback_group += 1;
                    run.code_group = fallback_group;
                }
                let group = run.code_group;
                let mut padding = Token::new(run.clone(), code_padding);
                padding.run.text.clear();
                padding.run.link = run.code_link.clone();
                padding.run.link_title = run.code_link_title.clone();
                padding.padding = true;
                padding.leading_padding = true;
                tokens.push(padding.clone());
                let original_group = runs[position].code_group;
                loop {
                    run.code_group = group;
                    if !run.padding_only {
                        tokens.push(Token::new(run.clone(), measure(&run, &run.text)));
                    }
                    position += 1;
                    if position >= runs.len()
                        || original_group == 0
                        || !runs[position].code
                        || runs[position].code_group != original_group
                    {
                        break;
                    }
                    run = runs[position].clone();
                }
                padding.leading_padding = false;
                tokens.push(padding);
            } else {
                tokens.push(Token::new(run.clone(), measure(&run, &run.text)));
                position += 1;
            }
        }
    }
    tokens
}

// Original markdownMinContentWidth, retaining code padding and Unicode breaks.
pub(super) fn content_widths(
    parsed: &[Vec<NativeMarkdownRun>],
    base_size: f64,
    measure: impl Fn(&NativeMarkdownRun, &str) -> f64,
) -> (f64, f64) {
    let tokens = inline_tokens(parsed, paragraph_code_padding(base_size), &measure);
    let mut width: f64 = 0.0;
    let mut current: f64 = 0.0;
    for token in &tokens {
        if token.break_line {
            width = width.max(current);
            current = 0.0;
        } else {
            current += token.width;
        }
    }
    width = width.max(current);
    let mut minimum: f64 = 0.0;
    let mut segment = 0.0;
    let mut pending = 0.0;
    let mut previous = None;
    for atom in atomize(&tokens, &measure) {
        if atom.break_line || (atom.run.text == " " && !atom.padding) {
            segment += pending;
            pending = 0.0;
            minimum = minimum.max(segment);
            segment = 0.0;
            previous = None;
            continue;
        }
        if atom.padding && atom.leading_padding {
            pending += atom.width;
            continue;
        }
        if !atom.padding && atom.break_before {
            minimum = minimum.max(
                segment
                    + if atom.break_after_soft_hyphen && previous.is_some() {
                        base_size / 3.0
                    } else {
                        0.0
                    },
            );
            segment = 0.0;
            previous = None;
        }
        segment += pending + atom.width;
        pending = 0.0;
        if !atom.padding {
            previous = Some(atom.run.code);
        }
    }
    minimum = minimum.max(segment + pending);
    (width, minimum)
}

/// inlineCodeHorizontalPadding: PaddingLeftRight_code_em (.4) at the
/// FontSize_pre_code_em (.85) code size.
fn paragraph_code_padding(base_size: f64) -> f64 {
    base_size * 0.85 * 0.4
}

pub(super) fn wrap(
    parsed: &[Vec<NativeMarkdownRun>],
    available_width: f64,
    base_size: f64,
    measure: impl Fn(&NativeMarkdownRun, &str) -> f64,
) -> Vec<Vec<Token>> {
    wrap_with_code_padding(
        parsed,
        available_width,
        base_size,
        paragraph_code_padding(base_size),
        measure,
    )
}

/// wrapInlineTokens with an explicit inline-code padding: headings use
/// PaddingLeftRight_heading_code_em (.2) of the heading size.
pub(super) fn wrap_with_code_padding(
    parsed: &[Vec<NativeMarkdownRun>],
    available_width: f64,
    base_size: f64,
    code_padding: f64,
    measure: impl Fn(&NativeMarkdownRun, &str) -> f64,
) -> Vec<Vec<Token>> {
    let tokens = inline_tokens(parsed, code_padding, &measure);
    let mut lines = vec![Vec::new()];
    let mut width = 0.0;
    let mut needs_wrap = false;
    for token in &tokens {
        if token.break_line {
            lines.push(Vec::new());
            width = 0.0;
            continue;
        }
        lines.last_mut().unwrap().push(token.clone());
        width += token.width;
        needs_wrap |= available_width > 0.0 && width > available_width + 0.001;
    }
    if !needs_wrap {
        return lines;
    }
    let mut atoms = atomize(&tokens, &measure);
    lines = vec![Vec::new()];
    width = 0.0;
    let mut pending_space: Option<Token> = None;
    let mut pending_padding = Vec::new();
    let mut pending_padding_width = 0.0;
    let mut has_content = false;
    let mut index = 0;
    while index < atoms.len() {
        let mut atom = atoms[index].clone();
        if atom.break_line {
            pending_space = None;
            pending_padding.clear();
            pending_padding_width = 0.0;
            lines.push(Vec::new());
            width = 0.0;
            has_content = false;
            index += 1;
            continue;
        }
        if atom.run.text == " " && !atom.padding {
            pending_space = Some(atom);
            index += 1;
            continue;
        }
        if atom.padding && atom.leading_padding {
            pending_padding_width += atom.width;
            pending_padding.push(atom);
            index += 1;
            continue;
        }
        loop {
            let trailing: f64 = atoms[index + 1..]
                .iter()
                .take_while(|next| next.padding && !next.leading_padding)
                .map(|next| next.width)
                .sum();
            let needed = unbroken_width(&atoms, index)
                + pending_padding_width
                + pending_space.as_ref().map_or(0.0, |token| token.width);
            if available_width > 0.0
                && has_content
                && !atom.padding
                && (atom.break_before || pending_space.is_some())
                && width + needed > available_width + 0.001
            {
                if atom.break_after_soft_hyphen {
                    let line = lines.last_mut().unwrap();
                    if let Some(i) = line.iter().rposition(|token: &Token| token.soft_hyphen_end) {
                        line[i].soft_hyphen_end = false;
                        let mut hyphen = line[i].clone();
                        hyphen.run.text = "-".to_owned();
                        hyphen.width = base_size / 3.0;
                        hyphen.discretionary_hyphen = true;
                        hyphen.break_before = false;
                        hyphen.break_after_soft_hyphen = false;
                        line.insert(i + 1, hyphen);
                    }
                }
                lines.push(Vec::new());
                width = 0.0;
                has_content = false;
                pending_space = None;
            }
            let occupied = width
                + pending_padding_width
                + if has_content {
                    pending_space.as_ref().map_or(0.0, |token| token.width)
                } else {
                    0.0
                };
            if available_width > 0.0
                && !atom.padding
                && occupied + atom.width + trailing > available_width + 0.001
            {
                if let Some((head, tail)) = split(
                    &atom,
                    (available_width - occupied).max(0.0),
                    trailing > 0.0,
                    &measure,
                ) {
                    atom = head;
                    atoms.insert(index + 1, tail);
                } else if has_content {
                    lines.push(Vec::new());
                    width = 0.0;
                    has_content = false;
                    pending_space = None;
                    continue;
                }
            }
            break;
        }
        if has_content {
            if let Some(space) = pending_space.take() {
                width += space.width;
                lines.last_mut().unwrap().push(space);
            }
        } else {
            pending_space = None;
        }
        lines.last_mut().unwrap().append(&mut pending_padding);
        width += pending_padding_width;
        pending_padding_width = 0.0;
        width += atom.width;
        if !atom.padding {
            has_content = true;
        }
        lines.last_mut().unwrap().push(atom);
        index += 1;
    }
    lines
}

#[cfg(all(test, feature = "oracle-fixtures"))]
pub(crate) mod source_contract {
    use super::super::*;
    pub(crate) fn primitives(svg: &str) -> serde_json::Value {
        let source = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">{svg}</svg>"#
        );
        let doc = roxmltree::Document::parse(&source).unwrap();
        let rows:Vec<_>=doc.descendants().filter(|n|n.is_element() && matches!(n.tag_name().name(),"text"|"rect"|"line")).map(|node| {
            let mut attrs=std::collections::BTreeMap::new();
            for name in ["x","y","width","height","rx","x1","y1","x2","y2","font-size","font-weight","font-style","textLength","lengthAdjust","stroke-width","stroke-linecap","stroke-linejoin","stroke-miterlimit"] {
                if let Some(value)=node.attribute(name) {attrs.insert(name,value);}
            }
            let face=node.attribute("class").and_then(|classes|classes.split_whitespace().find(|class|*class=="text" || class.starts_with("text-")));
            let link=node.ancestors().find(|n|n.has_tag_name("a")).and_then(|n|n.attribute("href"));
            let link_title=node.ancestors().find(|n|n.has_tag_name("a")).and_then(|a|a.children().find(|n|n.has_tag_name("title"))).and_then(|n|n.text());
            serde_json::json!({"kind":node.tag_name().name(),"geometry":attrs,"font_face":face,"text":node.text().unwrap_or(""),"link":link,"link_title":link_title,"fill":node.attribute("fill")})
        }).collect();
        serde_json::to_value(rows).unwrap()
    }
    #[test]
    fn complete_original_paragraph_wrap_geometry() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/paragraph-wrap-source-v090.json"
        ))
        .unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 2070);
        let mut failures = Vec::new();
        let mut results = Vec::new();
        for row in rows {
            let request = &row["request"];
            let mut svg = String::new();
            let family = match request["Family"].as_str().unwrap() {
                "mono" => MarkdownFamily::Mono,
                "hand" => MarkdownFamily::HandDrawn,
                _ => MarkdownFamily::Sans,
            };
            let supported = draw_native_markdown_paragraph(
                &mut svg,
                request["Markdown"].as_str().unwrap(),
                Rect {
                    origin: Point::default(),
                    size: Size {
                        width: request["Width"].as_f64().unwrap(),
                        height: 1024.0,
                    },
                },
                request["FontSize"].as_u64().unwrap() as u32,
                "#0A0F25",
                None,
                false,
                family,
            );
            let actual = primitives(&svg);
            let expected = primitives(row["svg"].as_str().unwrap());
            let matched = supported && actual == expected;
            if !matched {
                failures.push(request["Name"].as_str().unwrap());
            }
            results.push(serde_json::json!({"name":request["Name"],"supported":supported,"matched":matched,"actual_svg":svg,"actual":actual,"expected":expected}));
        }
        if let Some(path) = std::env::var_os("D2_RS_TEST_MD_WRAP_OUTPUT") {
            std::fs::write(path,serde_json::to_vec_pretty(&serde_json::json!({"total":rows.len(),"passed":rows.len()-failures.len(),"checks":results})).unwrap()).unwrap();
        }
        assert!(
            failures.is_empty(),
            "paragraph wrap failures ({}): {:?}",
            failures.len(),
            &failures[..failures.len().min(30)]
        );
    }
}
