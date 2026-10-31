// SPDX-License-Identifier: MPL-2.0
//! Native Markdown measurement shared by source sizing and SVG callouts.
//! Maps D2 v0.9.0 lib/textmeasure's Markdown sizing. Paint and measurement
//! intentionally retain distinct CSS versus legacy ruler font-box contracts.

use crate::{
    markdown::{
        html::{render_markdown_html as render_html, unescape_html},
        html_names::normalize_html_names,
        plain_text::plain_text,
    },
    measure::TrueTypeFont,
};
use d2_rs_core::Size;

#[doc(hidden)]
pub mod legacy_inline;

/// Borrowed primary and monospace ruler faces, including custom CLI faces.
pub struct MarkdownMeasurer<'a> {
    pub font: &'a TrueTypeFont,
    pub bold_font: &'a TrueTypeFont,
    pub italic_font: &'a TrueTypeFont,
    pub semibold_font: &'a TrueTypeFont,
    pub mono_font: &'a TrueTypeFont,
    pub mono_bold_font: &'a TrueTypeFont,
    pub mono_semibold_font: &'a TrueTypeFont,
    pub mono_italic_font: &'a TrueTypeFont,
}

fn is_markdown_thematic_break(line: &str) -> bool {
    let Some(first) = line.chars().next() else {
        return false;
    };
    matches!(first, '-' | '*' | '_')
        && line
            .chars()
            .all(|character| character == first || character.is_ascii_whitespace())
        && line.chars().filter(|character| *character == first).count() >= 3
}

impl MarkdownMeasurer<'_> {
    fn mono_fallback(&self, bold: bool, italic: bool) -> &TrueTypeFont {
        if bold {
            self.mono_bold_font
        } else if italic {
            self.mono_italic_font
        } else {
            self.mono_font
        }
    }

    fn measure_styled_label_with_dot(
        &self,
        label: &str,
        font_size: u32,
        bold: bool,
        italic: bool,
        mono_default: bool,
    ) -> Size {
        let font = match (mono_default, bold, italic) {
            (true, true, _) => &self.mono_bold_font,
            (true, false, true) => &self.mono_italic_font,
            (true, false, false) => &self.mono_font,
            (false, true, _) => &self.bold_font,
            (false, false, true) => &self.italic_font,
            (false, false, false) => &self.font,
        };
        let metrics = font.measure_markdown_legacy(
            label,
            f64::from(font_size),
            self.mono_fallback(bold, italic),
        );
        Size {
            width: metrics.width.ceil(),
            height: metrics.height.ceil(),
        }
    }

    fn measure_styled_width_with_dot(
        &self,
        label: &str,
        font_size: u32,
        bold: bool,
        italic: bool,
        mono_default: bool,
    ) -> f64 {
        let font = match (mono_default, bold, italic) {
            (true, true, _) => &self.mono_bold_font,
            (true, false, true) => &self.mono_italic_font,
            (true, false, false) => &self.mono_font,
            (false, true, _) => &self.bold_font,
            (false, false, true) => &self.italic_font,
            (false, false, false) => &self.font,
        };
        font.measure_markdown_legacy(
            label,
            f64::from(font_size),
            self.mono_fallback(bold, italic),
        )
        .width
    }

    // Original lib/textmeasure/markdown.go:measureMarkdownNode measures the
    // Goldmark HTML text nodes independently. Font selection follows the
    // innermost element, and edge spaces use that face's atlas advance.
    fn measure_list_inline_html(
        &self,
        markup: &str,
        font_size: u32,
        mono_default: bool,
    ) -> Option<f64> {
        let html = format!(
            "<body>{}</body>",
            normalize_html_names(&render_html(markup.as_bytes()))
        );
        let document = roxmltree::Document::parse(&html).ok()?;
        fn nonempty(node: roxmltree::Node<'_, '_>) -> bool {
            node.is_element() || node.text().is_some_and(|s| !s.trim().is_empty())
        }
        fn measure(
            measurer: &MarkdownMeasurer<'_>,
            node: roxmltree::Node<'_, '_>,
            size: f64,
            style: u8,
            mono: bool,
        ) -> f64 {
            let font = match (mono, style) {
                (true, 1) => measurer.mono_bold_font,
                (true, 2) => measurer.mono_italic_font,
                (true, _) => measurer.mono_font,
                (false, 1) => measurer.bold_font,
                (false, 2) => measurer.italic_font,
                (false, _) => measurer.font,
            };
            if node.is_text() {
                let mut text = node.text().unwrap_or_default().to_owned();
                if text.trim_matches(['\n', '\t', '\u{8}']).is_empty() {
                    return 0.0;
                }
                let code = node
                    .parent()
                    .is_some_and(|p| matches!(p.tag_name().name(), "code" | "pre"));
                if code {
                    return font.measure_markdown_raw(&text, size).width * 0.85;
                }
                text = text.replace(['\n', '\t'], " ");
                let mut spaces = 0.0;
                if text.starts_with(' ') {
                    text.remove(0);
                    if node.prev_siblings().skip(1).any(nonempty) {
                        spaces += font.measure_advance(" ", size);
                    }
                }
                if text.ends_with(' ') {
                    text.pop();
                    if node.next_siblings().skip(1).any(nonempty) {
                        spaces += font.measure_advance(" ", size);
                    }
                }
                return font
                    .measure_markdown_legacy(
                        &text,
                        size,
                        if style == 1 {
                            measurer.mono_bold_font
                        } else if style == 2 {
                            measurer.mono_italic_font
                        } else {
                            measurer.mono_font
                        },
                    )
                    .width
                    + spaces;
            }
            if !node.is_element() {
                return 0.0;
            }
            let tag = node.tag_name().name();
            if tag == "img" {
                return 0.0;
            }
            let style = match tag {
                "strong" | "b" => 1,
                "em" => 2,
                "code" | "pre" => 0,
                _ => style,
            };
            let mono = mono || matches!(tag, "code" | "pre");
            let mut widest: f64 = 0.0;
            let mut line = 0.0;
            for child in node.children() {
                if child.has_tag_name("br") {
                    widest = widest.max(line);
                    line = 0.0;
                } else {
                    line += measure(measurer, child, size, style, mono);
                }
            }
            let width = widest.max(line);
            if tag == "code" {
                width + 2.0 * 0.4 * size
            } else {
                width
            }
        }
        Some(measure(
            self,
            document.root_element(),
            f64::from(font_size),
            0,
            mono_default,
        ))
    }

    fn measure_markdown_inline_line(
        &self,
        markup: &str,
        font_size: u32,
        inherited_bold: bool,
        mono_default: bool,
    ) -> f64 {
        if markup.contains("<br") {
            let split = markup
                .replace("<br />", "\n")
                .replace("<br/>", "\n")
                .replace("<br>", "\n");
            if split != markup {
                return split
                    .split('\n')
                    .map(|line| {
                        self.measure_markdown_inline_line(
                            line,
                            font_size,
                            inherited_bold,
                            mono_default,
                        )
                    })
                    .fold(0.0, f64::max);
            }
        }
        let mut width = 0.0;
        let mut bold = inherited_bold;
        let mut italic = false;
        let mut code = false;
        let mut strikethrough = false;
        let mut raw_tag_stack = Vec::new();
        let mut cursor = 0usize;
        let mut run_start = 0usize;
        let flush = |start: usize,
                     end: usize,
                     width: &mut f64,
                     bold: bool,
                     italic: bool,
                     code: bool| {
            if end <= start {
                return;
            }
            // The slice is already bounded by Markdown delimiters.  Keep its
            // whitespace intact: `plain_text` trims a trailing space, while
            // Goldmark includes that space in the preceding inline run.
            let run = String::from_utf8_lossy(&unescape_html(&markup.as_bytes()[start..end]))
                .into_owned();
            if run.is_empty() {
                return;
            }
            if code {
                // Original measureMarkdownNode keeps base-size mono ink bounds,
                // scales the content by85%, then adds unscaled legacy padding.
                let size = f64::from(font_size);
                let content_width = self.mono_font.measure_markdown_raw(&run, size).width * 0.85;
                *width += content_width + 2.0 * 0.4 * size;
            } else {
                *width +=
                    self.measure_styled_width_with_dot(&run, font_size, bold, italic, mono_default);
            }
        };
        while cursor < markup.len() {
            let bytes = markup.as_bytes();
            if bytes[cursor..].starts_with(b"![") && !code {
                let Some(label_end_relative) = markup[cursor + 2..].find(']') else {
                    cursor += 1;
                    continue;
                };
                let label_end = cursor + 2 + label_end_relative;
                if markup
                    .get(label_end + 1..)
                    .is_some_and(|rest| rest.starts_with('('))
                {
                    let Some(destination_end_relative) = markup[label_end + 2..].find(')') else {
                        cursor += 1;
                        continue;
                    };
                    flush(run_start, cursor, &mut width, bold, italic, code);
                    cursor = label_end + 2 + destination_end_relative + 1;
                    // The legacy D2 ruler measures the text nodes on either
                    // side of the omitted image independently.  That leaves
                    // the collapsible separator after the image in the
                    // measured box even though native painting emits only
                    // the single visible separator before the following run.
                    run_start = cursor;
                    continue;
                }
            }
            if bytes[cursor..].starts_with(b"**") || bytes[cursor..].starts_with(b"__") {
                flush(run_start, cursor, &mut width, bold, italic, code);
                bold = !bold;
                cursor += 2;
                run_start = cursor;
                continue;
            }
            if bytes[cursor] == b'`' {
                flush(run_start, cursor, &mut width, bold, italic, code);
                code = !code;
                cursor += 1;
                run_start = cursor;
                continue;
            }
            if bytes[cursor..].starts_with(b"~~")
                && !code
                && (strikethrough || markup[cursor + 2..].contains("~~"))
            {
                flush(run_start, cursor, &mut width, bold, italic, code);
                strikethrough = !strikethrough;
                cursor += 2;
                run_start = cursor;
                continue;
            }
            if (bytes[cursor] == b'*' || bytes[cursor] == b'_') && !code {
                flush(run_start, cursor, &mut width, bold, italic, code);
                italic = !italic;
                cursor += 1;
                run_start = cursor;
                continue;
            }
            if bytes[cursor] == b'<' && !code {
                let Some(relative_end) = markup[cursor + 1..].find('>') else {
                    cursor += 1;
                    continue;
                };
                let end = cursor + 1 + relative_end;
                let destination = &markup[cursor + 1..end];
                let is_uri = destination.starts_with("http://")
                    || destination.starts_with("https://")
                    || destination.starts_with("mailto:");
                let is_email = !destination.contains([':', '/'])
                    && destination
                        .split_once('@')
                        .is_some_and(|(local, host)| !local.is_empty() && !host.is_empty());
                if !destination.is_empty()
                    && !destination.chars().any(char::is_whitespace)
                    && (is_uri || is_email)
                {
                    flush(run_start, cursor, &mut width, bold, italic, code);
                    width += self.measure_styled_width_with_dot(
                        destination,
                        font_size,
                        bold,
                        italic,
                        mono_default,
                    );
                    cursor = end + 1;
                    run_start = cursor;
                    continue;
                }

                // Supported raw inline tags carry style but do not contribute
                // glyphs to the Markdown ruler. Goldmark preserves these tags
                // in the HTML projection, so counting their source spelling
                // would make a label much wider than the Go renderer.
                let tag = &markup[cursor..=end];
                let closing = tag.as_bytes().get(1) == Some(&b'/');
                let name_start = if closing { 2 } else { 1 };
                let name_end = tag[name_start..]
                    .find(|character: char| {
                        character.is_ascii_whitespace() || matches!(character, '/' | '>')
                    })
                    .map_or(tag.len(), |offset| name_start + offset);
                let name = tag[name_start..name_end].to_ascii_lowercase();
                if matches!(
                    name.as_str(),
                    "a" | "b" | "br" | "code" | "del" | "em" | "i" | "s" | "strike" | "strong"
                ) {
                    flush(run_start, cursor, &mut width, bold, italic, code);
                    if closing {
                        if let Some((
                            previous_bold,
                            previous_italic,
                            previous_code,
                            previous_strike,
                        )) = raw_tag_stack.pop()
                        {
                            bold = previous_bold;
                            italic = previous_italic;
                            code = previous_code;
                            strikethrough = previous_strike;
                        }
                    } else if !tag.ends_with("/>") {
                        raw_tag_stack.push((bold, italic, code, strikethrough));
                        match name.as_str() {
                            "b" | "strong" => bold = true,
                            "code" => code = true,
                            "del" | "s" | "strike" => strikethrough = true,
                            "em" | "i" => italic = true,
                            _ => {}
                        }
                    }
                    cursor = end + 1;
                    run_start = cursor;
                    continue;
                }
            }
            if bytes[cursor] == b'['
                && !code
                && let Some(label_end_relative) = markup[cursor + 1..].find(']')
            {
                let label_end = cursor + 1 + label_end_relative;
                if markup[label_end + 1..].starts_with('(')
                    && let Some(destination_end_relative) = markup[label_end + 2..].find(')')
                {
                    flush(run_start, cursor, &mut width, bold, italic, code);
                    let label = &markup[cursor + 1..label_end];
                    width += if bold {
                        self.measure_styled_width_with_dot(
                            label,
                            font_size,
                            true,
                            italic,
                            mono_default,
                        )
                    } else {
                        self.measure_styled_width_with_dot(
                            label,
                            font_size,
                            false,
                            italic,
                            mono_default,
                        )
                    };
                    cursor = label_end + 2 + destination_end_relative + 1;
                    run_start = cursor;
                    continue;
                }
            }
            cursor += 1;
        }
        flush(run_start, markup.len(), &mut width, bold, italic, code);
        width
    }

    fn measure_markdown_heading_box(
        &self,
        markup: &str,
        font_size: u32,
        mono_default: bool,
    ) -> Size {
        let mut width = 0.0_f64;
        let mut height = 0.0_f64;
        let mut bold = false;
        let mut italic = false;
        let mut code = false;
        let mut cursor = 0;
        let mut start = 0;
        let size = f64::from(font_size);
        let flush = |start: usize,
                     end: usize,
                     bold: bool,
                     italic: bool,
                     code: bool,
                     width: &mut f64,
                     height: &mut f64| {
            if end <= start {
                return;
            }
            let text = String::from_utf8_lossy(&unescape_html(&markup.as_bytes()[start..end]))
                .into_owned();
            let font = if code {
                &self.mono_font
            } else {
                match (mono_default, bold, italic) {
                    (true, true, _) => &self.mono_bold_font,
                    (true, false, true) => &self.mono_italic_font,
                    (true, false, false) => &self.mono_semibold_font,
                    (false, true, _) => &self.bold_font,
                    (false, false, true) => &self.italic_font,
                    (false, false, false) => &self.semibold_font,
                }
            };
            let measured = if code {
                font.measure_markdown_raw(&text, size)
            } else {
                let fallback = if bold {
                    self.mono_bold_font
                } else if italic {
                    self.mono_italic_font
                } else {
                    self.mono_semibold_font
                };
                font.measure_markdown_legacy(&text, size, fallback)
            };
            *width += measured.width * if code { 0.85 } else { 1.0 };
            *height = height.max(measured.height * if code { 0.85 } else { 1.0 });
        };
        while cursor < markup.len() {
            let bytes = markup.as_bytes();
            let delimiter = if bytes[cursor] == b'`' {
                Some((1, 0))
            } else if !code
                && (bytes[cursor..].starts_with(b"**") || bytes[cursor..].starts_with(b"__"))
            {
                Some((2, 1))
            } else if !code && matches!(bytes[cursor], b'*' | b'_') {
                Some((1, 2))
            } else {
                None
            };
            if let Some((count, kind)) = delimiter {
                flush(start, cursor, bold, italic, code, &mut width, &mut height);
                match kind {
                    0 => {
                        code = !code;
                        if code {
                            width += 2.0 * 0.4 * size;
                            height = height.max(
                                self.mono_font.measure_markdown_raw("", size).height * 0.85
                                    + 2.0 * 0.2 * size,
                            )
                        }
                    }
                    1 => bold = !bold,
                    _ => italic = !italic,
                }
                cursor += count;
                start = cursor;
            } else {
                cursor += 1
            }
        }
        flush(
            start,
            markup.len(),
            bold,
            italic,
            code,
            &mut width,
            &mut height,
        );
        // Code's vertical padding surrounds the same legacy font box as its
        // content. The CSS paint path deliberately has a different heading box.
        if markup.contains('`') {
            let code_height =
                self.mono_font.measure_markdown_raw("x", size).height * 0.85 + 2.0 * 0.2 * size;
            height = height.max(code_height);
        }
        Size { width, height }
    }

    fn markdown_table_dimensions(
        &self,
        lines: &[&str],
        start: usize,
        font_size: u32,
        inherited_bold: bool,
        italic: bool,
        mono_default: bool,
    ) -> Option<(f64, f64, usize)> {
        let split_row = |line: &str| -> Option<Vec<String>> {
            let trimmed = line.trim();
            if !trimmed.contains('|') {
                return None;
            }
            let body = trimmed.strip_prefix('|').unwrap_or(trimmed);
            let body = body.strip_suffix('|').unwrap_or(body);
            let cells = body
                .split('|')
                .map(str::trim)
                .map(str::to_owned)
                .collect::<Vec<_>>();
            (!cells.is_empty() && cells.iter().all(|cell| !cell.is_empty())).then_some(cells)
        };
        let header = split_row(lines.get(start).copied()?)?;
        let separator = split_row(lines.get(start + 1).copied()?)?;
        if separator.len() != header.len()
            || !separator.iter().all(|cell| {
                let cell = cell.strip_prefix(':').unwrap_or(cell);
                let cell = cell.strip_suffix(':').unwrap_or(cell);
                !cell.is_empty() && cell.bytes().all(|byte| byte == b'-')
            })
        {
            return None;
        }
        let mut rows = vec![header];
        let mut consumed = 2usize;
        while let Some(line) = lines.get(start + consumed).copied() {
            if line.trim().is_empty() {
                break;
            }
            let row = split_row(line)?;
            if row.len() != rows[0].len() {
                return None;
            }
            rows.push(row);
            consumed += 1;
        }
        if rows.len() < 2 {
            return None;
        }
        let mut widths = vec![0.0_f64; rows[0].len()];
        for (row_index, row) in rows.iter().enumerate() {
            for (column, cell) in row.iter().enumerate() {
                let visible = plain_text(cell.as_bytes());
                let measured = if row_index == 0 {
                    let font = if mono_default {
                        &self.mono_bold_font
                    } else {
                        &self.semibold_font
                    };
                    font.measure_markdown_legacy(
                        &visible,
                        f64::from(font_size),
                        if mono_default {
                            self.mono_bold_font
                        } else {
                            self.mono_semibold_font
                        },
                    )
                    .width
                } else {
                    self.measure_styled_width_with_dot(
                        &visible,
                        font_size,
                        inherited_bold,
                        italic,
                        mono_default,
                    )
                };
                widths[column] = widths[column].max(measured + 26.0);
            }
        }
        let line_height = f64::from(font_size) * 1.5;
        let row_height = line_height + 13.0;
        let width = widths.iter().sum::<f64>() + widths.len() as f64 + 1.0;
        let height = row_height * rows.len() as f64 + 2.0;
        Some((width, height, consumed))
    }

    fn markdown_special_dimensions(
        &self,
        lines: &[&str],
        start: usize,
        font_size: u32,
        mono_default: bool,
    ) -> Option<(Size, usize)> {
        let mut index = start;
        let mut blocks = Vec::new();
        while index < lines.len() {
            let line = lines[index];
            if line.trim().is_empty() {
                index += 1;
                continue;
            }
            let trimmed = line.trim_start();
            if trimmed
                .strip_prefix("> ")
                .or_else(|| trimmed.strip_prefix(">"))
                .is_some()
            {
                let mut rows = 0usize;
                let mut width: f64 = 0.0;
                while index < lines.len() {
                    let candidate = lines[index].trim_start();
                    let Some(content) = candidate
                        .strip_prefix("> ")
                        .or_else(|| candidate.strip_prefix(">"))
                    else {
                        break;
                    };
                    let visible = plain_text(content.trim().as_bytes());
                    width = width.max(
                        self.measure_styled_width_with_dot(
                            &visible,
                            font_size,
                            false,
                            false,
                            mono_default,
                        ) + 40.0,
                    );
                    rows += 1;
                    index += 1;
                }
                blocks.push((width, 24.0 * rows as f64, false));
                continue;
            }
            if is_markdown_thematic_break(trimmed) {
                blocks.push((0.0, 4.0, true));
                index += 1;
                continue;
            }
            if trimmed.strip_prefix("```").is_some() {
                index += 1;
                let mut code_lines = Vec::new();
                let mut closed = false;
                while index < lines.len() {
                    if lines[index].trim_start().starts_with("```") {
                        closed = true;
                        index += 1;
                        break;
                    }
                    code_lines.push(lines[index].strip_prefix("    ").unwrap_or(lines[index]));
                    index += 1;
                }
                if !closed {
                    return None;
                }
                let code_size = f64::from(font_size) * 0.85;
                let width = code_lines
                    .iter()
                    .map(|line| self.mono_font.measure_markdown_raw(line, code_size).width + 32.0)
                    .fold(0.0, f64::max);
                let raw_height = if code_lines.is_empty() {
                    32.0
                } else {
                    code_lines.len() as f64 * code_size * 1.45 + 32.0
                };
                blocks.push((width, raw_height, false));
                continue;
            }
            return None;
        }
        if blocks.is_empty() {
            return None;
        }
        let width = blocks
            .iter()
            .map(|(width, _, _)| *width)
            .fold(0.0, f64::max);
        let mut height = blocks.iter().map(|(_, height, _)| *height).sum::<f64>();
        for pair in blocks.windows(2) {
            height += if pair[0].2 || pair[1].2 { 24.0 } else { 16.0 };
        }
        if blocks.len() > 1 {
            height += 18.0;
        } else if blocks[0].1 > 64.0 {
            // The legacy ruler rounds a standalone fenced block down before
            // the native painter emits its fractional CSS height.
            height = (height - 1.44).max(1.0);
        }
        Some((
            Size {
                width: width.ceil().max(1.0),
                height: height.ceil().max(1.0),
            },
            index - start,
        ))
    }

    /// Reproduce the legacy block dimensions from D2's markdown ruler without
    /// requiring a browser. Soft line breaks stay in one paragraph, headings
    /// use their CSS scale and padding, and list/paragraph blocks retain the
    /// inter-block margins that affect graph geometry.
    pub fn markdown_dimensions(
        &self,
        source: &str,
        font_size: u32,
        _inherited_bold: bool,
        _italic: bool,
        mono_default: bool,
    ) -> Size {
        // Original MeasureMarkdown accepts primary/mono families and size;
        // target-level emphasis does not seed its Markdown paint context.
        let inherited_bold = false;
        let italic = false;
        if source.trim().is_empty() {
            return Size::default();
        }
        if let Some(dimensions) = self.legacy_paragraph_dimensions(source, font_size, mono_default)
        {
            return dimensions;
        }
        let lines = source.lines().collect::<Vec<_>>();
        let line_height = f64::from(font_size) * 1.5;
        let mut blocks: Vec<(f64, f64)> = Vec::new();
        let mut index = 0usize;
        while index < lines.len() {
            let line = lines[index];
            if line.trim().is_empty() {
                index += 1;
                continue;
            }
            let trimmed = line.trim_start();
            if let Some((size, consumed)) =
                self.markdown_special_dimensions(&lines, index, font_size, mono_default)
            {
                blocks.push((size.width, size.height));
                index += consumed;
                continue;
            }
            if let Some((width, height, consumed)) = self.markdown_table_dimensions(
                &lines,
                index,
                font_size,
                inherited_bold,
                italic,
                mono_default,
            ) {
                blocks.push((width.ceil(), height.ceil()));
                index += consumed;
                continue;
            }
            let heading_level = trimmed
                .chars()
                .take_while(|character| *character == '#')
                .count();
            if (1..=6).contains(&heading_level)
                && trimmed
                    .as_bytes()
                    .get(heading_level)
                    .is_some_and(|byte| byte.is_ascii_whitespace())
            {
                let heading_markup = trimmed[heading_level..].trim_start();
                let scale = match heading_level {
                    1 => 2.0,
                    2 => 1.5,
                    3 => 1.25,
                    4 => 1.0,
                    5 => 0.875,
                    _ => 0.85,
                };
                let heading_size = (f64::from(font_size) * scale) as u32;
                let measured = self.measure_markdown_heading_box(
                    heading_markup,
                    heading_size.max(1),
                    mono_default,
                );
                let line_height = 1.25 * f64::from(heading_size);
                let heading_height = measured.height.max(line_height)
                    + if heading_level <= 2 {
                        0.3 * f64::from(heading_size) + 1.0
                    } else {
                        0.0
                    };
                // Go retains fractional block heights until the complete
                // body is measured; rounding each heading changes list viewports.
                blocks.push((measured.width.ceil(), heading_height));
                index += 1;
                continue;
            }
            let is_list = trimmed.starts_with("- ")
                || trimmed.starts_with("* ")
                || trimmed.starts_with("+ ")
                || trimmed.find(". ").is_some_and(|dot| {
                    dot > 0 && trimmed[..dot].chars().all(|c| c.is_ascii_digit())
                });
            if is_list {
                let mut width: f64 = 0.0;
                let mut count = 0usize;
                while index < lines.len() {
                    let candidate = lines[index].trim_start();
                    let marker_end = if candidate.starts_with("- ")
                        || candidate.starts_with("* ")
                        || candidate.starts_with("+ ")
                    {
                        Some(2)
                    } else {
                        candidate.find(". ").and_then(|dot| {
                            (dot > 0 && candidate[..dot].chars().all(|c| c.is_ascii_digit()))
                                .then_some(dot + 2)
                        })
                    };
                    let Some(marker_end) = marker_end else { break };
                    let item_source = &candidate[marker_end..];
                    // Original measureMarkdownNode measures each inline
                    // child's concrete style before adding list indentation.
                    // Stripping markup loses italic and makes one bold span
                    // select a bold face for the entire item.
                    let item_width = self
                        .measure_list_inline_html(item_source, font_size, mono_default)
                        .unwrap_or_else(|| {
                            self.measure_markdown_inline_line(
                                item_source,
                                font_size,
                                inherited_bold,
                                mono_default,
                            )
                        });
                    width = width.max(item_width + 2.0 * f64::from(font_size));
                    count += 1;
                    index += 1;
                }
                let height = line_height * count as f64
                    + 0.25 * f64::from(font_size) * count.saturating_sub(1) as f64;
                blocks.push((width, height));
                continue;
            }

            // A paragraph consumes consecutive nonblank, nonheading, nonlist
            // lines. Soft breaks collapse to a space under GitHub's CSS;
            // explicit two-space breaks remain separate lines.
            let mut paragraph = String::new();
            let mut paragraph_markup = String::new();
            let mut hard_lines = 0usize;
            let mut previous_emphasis_only = false;
            let mut all_emphasis_only = true;
            let mut emphasis_parts = Vec::new();
            while index < lines.len() {
                let candidate = lines[index];
                let candidate_trimmed = candidate.trim_start();
                if candidate.trim().is_empty() {
                    break;
                }
                let candidate_heading = candidate_trimmed
                    .chars()
                    .take_while(|character| *character == '#')
                    .count();
                let candidate_list = candidate_trimmed.starts_with("- ")
                    || candidate_trimmed.starts_with("* ")
                    || candidate_trimmed.starts_with("+ ")
                    || candidate_trimmed.find(". ").is_some_and(|dot| {
                        dot > 0 && candidate_trimmed[..dot].chars().all(|c| c.is_ascii_digit())
                    });
                if (1..=6).contains(&candidate_heading) || candidate_list {
                    break;
                }
                let hard_break = candidate.ends_with("  ");
                let candidate_trimmed_end = candidate.trim();
                let emphasis_only = (candidate_trimmed_end.starts_with("**")
                    && candidate_trimmed_end.ends_with("**"))
                    || (candidate_trimmed_end.starts_with("__")
                        && candidate_trimmed_end.ends_with("__"));
                all_emphasis_only &= emphasis_only;
                let visible = plain_text(candidate.trim_end().as_bytes());
                if !paragraph.is_empty() {
                    if hard_lines > 0 {
                        paragraph.push('\n');
                        paragraph_markup.push('\n');
                    } else if !(previous_emphasis_only && emphasis_only) {
                        paragraph.push(' ');
                        paragraph_markup.push(' ');
                    }
                }
                paragraph.push_str(&visible);
                paragraph_markup.push_str(candidate.trim_end());
                if emphasis_only {
                    emphasis_parts.push(visible.clone());
                }
                previous_emphasis_only = emphasis_only;
                if hard_break {
                    hard_lines += 1;
                }
                index += 1;
            }
            let metrics = if all_emphasis_only && emphasis_parts.len() > 1 && hard_lines == 0 {
                // Goldmark keeps adjacent strong text nodes as separate
                // inline runs. Their individual bounds are summed, without
                // adding the source newline as a space; measuring the joined
                // string would include a different trailing side bearing.
                Size {
                    width: emphasis_parts
                        .iter()
                        .map(|part| {
                            self.measure_styled_width_with_dot(
                                part,
                                font_size,
                                true,
                                false,
                                mono_default,
                            )
                        })
                        .sum::<f64>()
                        .ceil(),
                    height: line_height,
                }
            } else {
                // Goldmark creates separate inline runs for emphasis and
                // leaves one collapsible leading space after each explicit
                // <br>. Measure each line/run separately so an isolated strong
                // span does not turn the rest of a Markdown block bold.
                let leading_space = if hard_lines > 0 {
                    let font = match (mono_default, inherited_bold) {
                        (true, true) => &self.mono_bold_font,
                        (true, false) => &self.mono_font,
                        (false, true) => &self.bold_font,
                        (false, false) => &self.font,
                    };
                    font.advance(' ', f64::from(font_size))
                } else {
                    0.0
                };
                let width = paragraph_markup
                    .split('\n')
                    .enumerate()
                    .map(|(line_index, line)| {
                        // Source ignores a newline-only DOM text node before
                        // an inline element. A literal leading text run keeps
                        // its collapsible space after the preceding <br>.
                        let html = crate::markdown::html::render_markdown_html(line.as_bytes());
                        let first = html.strip_prefix("<p>").unwrap_or(&html);
                        let prefix = if line_index > 0 && !first.starts_with('<') {
                            leading_space
                        } else {
                            0.0
                        };
                        prefix
                            + self.measure_markdown_inline_line(
                                line,
                                font_size,
                                inherited_bold,
                                mono_default,
                            )
                    })
                    .fold(0.0, f64::max);
                Size {
                    width: width.ceil(),
                    height: line_height,
                }
            };
            let explicit_breaks = paragraph_markup.matches("<br").count();
            blocks.push((
                metrics.width,
                line_height * (hard_lines + explicit_breaks + 1) as f64,
            ));
        }

        let width = blocks.iter().map(|(width, _)| *width).fold(0.0, f64::max);
        let height = blocks.iter().map(|(_, height)| *height).sum::<f64>()
            + 16.0 * blocks.len().saturating_sub(1) as f64;
        let height = if height > 0.0 {
            height.max(f64::from(font_size) * 1.5)
        } else {
            height
        };
        Size {
            width: width.ceil().max(1.0),
            height: height.ceil().max(1.0),
        }
    }
}
