// SPDX-License-Identifier: MIT
// Chroma v2.27.0 lexer rules, token line splitting and style
// inheritance translated to Rust. Copyright (C) 2017 Alec Thomas.
// Rust translation Copyright 2026 Nelson Chen. See assets/chroma/COPYING.
//! Native code tokenization using pinned Chroma XML rules and Go/template,
//! Python, Markdown and YAML translations.
pub mod chroma_style;
mod control;
mod haxe;
mod markdown;
mod python;
mod registry;
mod xml;
mod xml_lexers;
use control::Control;
pub use control::{CodeBudget, CodeWorkError};
use regex::Regex;
use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: &'static str,
    pub value: String,
}

struct Rule {
    regex: Regex,
    kinds: &'static [&'static str],
}
fn rule(pattern: &str, kinds: &'static [&'static str]) -> Rule {
    // Pinned regexp2 v2.6.0 charInCategories returns on the first Word
    // category. In [^\W\d] the later Nd category is never consulted:
    // this is the Source behavior, including digits in template identifiers.
    // Its word category excludes Mc/Me accepted by Rust \w and retains
    // the two join controls.
    let pattern = pattern
        .replace(r"[^\W\d]", r"[\p{L}\p{Mn}\p{Nd}\p{Pc}\u{200c}\u{200d}]")
        .replace(r"\w", r"[\p{L}\p{Mn}\p{Nd}\p{Pc}\u{200c}\u{200d}]");
    Rule {
        regex: Regex::new(&format!(r"\A(?:{pattern})")).expect("pinned Chroma pattern"),
        kinds,
    }
}
fn go_rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| vec![
        rule(r"\n", &["TextWhitespace"]),
        rule(r"\s+", &["TextWhitespace"]),
        rule(r"//[^\s\n\r][^\n\r]*", &["CommentPreproc"]),
        rule(r"//[^\n\r]*", &["CommentSingle"]),
        rule(r"/(\\\n)?[*](.|\n)*?[*](\\\n)?/", &["CommentMultiline"]),
        rule(r"(import|package)\b", &["KeywordNamespace"]),
        rule(r"(var|func|struct|map|chan|type|interface|const)\b", &["KeywordDeclaration"]),
        rule(r"(?:break|default|select|case|defer|go|else|goto|switch|fallthrough|if|range|continue|for|return)\b", &["Keyword"]),
        rule(r"(true|false|iota|nil)\b", &["KeywordConstant"]),
        rule(r"(uint|uint8|uint16|uint32|uint64|int|int8|int16|int32|int64|float|float32|float64|complex64|complex128|byte|rune|string|bool|error|uintptr|print|println|panic|recover|close|complex|real|imag|len|cap|append|copy|delete|new|make|clear|min|max)\b(\()", &["NameBuiltin", "Punctuation"]),
        rule(r"(?:uint|uint8|uint16|uint32|uint64|int|int8|int16|int32|int64|float|float32|float64|complex64|complex128|byte|rune|string|bool|error|uintptr|any)\b", &["KeywordType"]),
        rule(r"\d+i", &["LiteralNumber"]),
        rule(r"\d+\.\d*([Ee][-+]\d+)?i", &["LiteralNumber"]),
        rule(r"\.\d+([Ee][-+]\d+)?i", &["LiteralNumber"]),
        rule(r"\d+[Ee][-+]\d+i", &["LiteralNumber"]),
        rule(r"\d+(\.\d+[eE][+\-]?\d+|\.\d*|[eE][+\-]?\d+)", &["LiteralNumberFloat"]),
        rule(r"\.\d+([eE][+\-]?\d+)?", &["LiteralNumberFloat"]),
        rule(r"0[0-7]+", &["LiteralNumberOct"]),
        rule(r"0[xX][0-9a-fA-F_]+", &["LiteralNumberHex"]),
        rule(r"0b[01_]+", &["LiteralNumberBin"]),
        rule(r"(0|[1-9][0-9_]*)", &["LiteralNumberInteger"]),
        rule(r#"'(\\['"\\abfnrtv]|\\x[0-9a-fA-F]{2}|\\[0-7]{1,3}|\\u[0-9a-fA-F]{4}|\\U[0-9a-fA-F]{8}|[^\\])'"#, &["LiteralStringChar"]),
        rule(r"(`)([^`]*)(`)", &["LiteralString", "@template", "LiteralString"]),
        rule(r#""(\\\\|\\"|[^"])*""#, &["LiteralString"]),
        rule(r"(<<=|>>=|<<|>>|<=|>=|&\^=|&\^|\+=|-=|\*=|/=|%=|&=|\|=|&&|\|\||<-|\+\+|--|==|!=|:=|\.\.\.|[+\-*/%&])", &["Operator"]),
        rule(r"([a-zA-Z_]\w*)(\s*)(\()", &["NameFunction", "@self", "Punctuation"]),
        rule(r"[|^<>=!()\[\]{}.,;:~]", &["Punctuation"]),
        rule(r"[^\W\d]\w*", &["NameOther"]),
    ])
}
fn expression_rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| vec![
        rule(r"\s+", &["TextWhitespace"]),
        rule(r"(range|if|else|while|with|template|end|true|false|nil|and|call|html|index|js|len|not|or|print|printf|println|urlquery|eq|ne|lt|le|gt|ge|block|break|continue|define|slice)\b", &["Keyword"]),
        rule(r"\||:?=|,", &["Operator"]),
        rule(r"[$]?[^\W\d]\w*", &["NameOther"]),
        rule(r"\$|[$]?\.(?:[^\W\d]\w*)?", &["NameAttribute"]),
        rule(r#""(\\\\|\\"|[^"])*""#, &["LiteralString"]),
        rule(r"-?\d+i", &["LiteralNumber"]),
        rule(r"-?\d+\.\d*([Ee][-+]\d+)?i", &["LiteralNumber"]),
        rule(r"\.\d+([Ee][-+]\d+)?i", &["LiteralNumber"]),
        rule(r"-?\d+[Ee][-+]\d+i", &["LiteralNumber"]),
        rule(r"-?\d+(\.\d+[eE][+\-]?\d+|\.\d*|[eE][+\-]?\d+)", &["LiteralNumberFloat"]),
        rule(r"-?\.\d+([eE][+\-]?\d+)?", &["LiteralNumberFloat"]),
        rule(r"-?0[0-7]+", &["LiteralNumberOct"]),
        rule(r"-?0[xX][0-9a-fA-F]+", &["LiteralNumberHex"]),
        rule(r"-?0b[01_]+", &["LiteralNumberBin"]),
        rule(r"-?(0|[1-9][0-9]*)", &["LiteralNumberInteger"]),
        rule(r#"'(\\['"\\abfnrtv]|\\x[0-9a-fA-F]{2}|\\[0-7]{1,3}|\\u[0-9a-fA-F]{4}|\\U[0-9a-fA-F]{8}|[^\\])'"#, &["LiteralStringChar"]),
        rule(r"`[^`]*`", &["LiteralString"]),
    ])
}
fn push(tokens: &mut Vec<Token>, kind: &'static str, value: &str, control: &mut Control<'_>) {
    if !control.accept(value) {
        return;
    }
    // Chroma ByGroups emits even empty groups; preserve them for budgets and
    // newline splitting rather than silently coalescing adjacent tokens.
    tokens.push(Token {
        kind,
        value: value.into(),
    });
}
fn match_rule(
    source: &str,
    offset: usize,
    rules: &[Rule],
    tokens: &mut Vec<Token>,
    control: &mut Control<'_>,
) -> Option<usize> {
    for rule in rules {
        if !control.check() {
            return None;
        }
        let Some(captures) = rule.regex.captures(&source[offset..]) else {
            continue;
        };
        let matched = captures.get(0).unwrap();
        if matched.start() != 0 {
            continue;
        }
        for (index, kind) in rule.kinds.iter().enumerate() {
            let value = if rule.kinds.len() == 1 {
                matched.as_str()
            } else {
                captures.get(index + 1).map_or("", |m| m.as_str())
            };
            match *kind {
                "@self" => tokens.extend(go(value, control)),
                "@template" => tokens.extend(template(value, true, control)),
                kind => push(tokens, kind, value, control),
            }
        }
        return Some(offset + matched.end());
    }
    None
}
fn go(source: &str, control: &mut Control<'_>) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut offset = 0;
    while offset < source.len() && control.check() {
        if let Some(end) = match_rule(source, offset, go_rules(), &mut tokens, control) {
            offset = end;
        } else {
            let length = source[offset..].chars().next().unwrap().len_utf8();
            push(
                &mut tokens,
                "Error",
                &source[offset..offset + length],
                control,
            );
            offset += length;
        }
    }
    tokens
}
fn template(source: &str, remap_other: bool, control: &mut Control<'_>) -> Vec<Token> {
    static COMMENT: OnceLock<Regex> = OnceLock::new();
    let comment =
        COMMENT.get_or_init(|| Regex::new(r"\A\{\{(- )?/\*(.|\n)*?\*/( -)?\}\}").unwrap());
    let mut tokens = Vec::new();
    let mut offset = 0;
    let mut stack = vec![0_u8];
    while offset < source.len() && control.check() {
        let tail = &source[offset..];
        let state = *stack.last().unwrap();
        let mut literal = None;
        if state == 0 {
            if let Some(m) = comment.find(tail) {
                literal = Some(("CommentMultiline", m.end()));
            } else if tail.starts_with("{{") {
                let length = if tail.starts_with("{{-") { 3 } else { 2 };
                literal = Some(("CommentPreproc", length));
                stack.push(1);
            } else {
                let length = if tail.starts_with('{') {
                    1
                } else {
                    tail.find('{').unwrap_or(tail.len())
                };
                literal = Some((
                    if remap_other {
                        "LiteralString"
                    } else {
                        "Other"
                    },
                    length,
                ));
            }
        } else if state == 1 && (tail.starts_with("}}") || tail.starts_with("-}}")) {
            literal = Some(("CommentPreproc", if tail.starts_with('-') { 3 } else { 2 }));
            stack.pop();
        } else if state == 2 && tail.starts_with(')') {
            literal = Some(("Operator", 1));
            stack.pop();
        } else if tail.starts_with('(') {
            literal = Some(("Operator", 1));
            stack.push(2);
        }
        if let Some((kind, length)) = literal {
            push(&mut tokens, kind, &tail[..length], control);
            offset += length;
        } else if let Some(end) =
            match_rule(source, offset, expression_rules(), &mut tokens, control)
        {
            offset = end;
        } else {
            let length = tail.chars().next().unwrap().len_utf8();
            push(&mut tokens, "Error", &tail[..length], control);
            offset += length;
            if tail.starts_with('\n') {
                stack.clear();
                stack.push(0);
            }
        }
    }
    tokens
}
/// Returns None for registry lexers whose native translation is still open.
/// Callers must not treat this as the Source unknown-language fallback.
pub fn tokenize(language: &str, source: &str) -> Option<Vec<Token>> {
    tokenize_inner(language, source, &mut Control::unlimited())
}
/// Collects exact token streams with a shared document budget, stopping before a rejected token is retained.
pub fn tokenize_bounded(
    language: &str,
    source: &str,
    budget: &mut CodeBudget,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Option<Vec<Token>>, CodeWorkError> {
    let mut control = Control::bounded(budget, cancel);
    let tokens = if control.check() {
        tokenize_inner(language, source, &mut control)
    } else {
        None
    };
    control.check();
    if let Some(error) = control.error {
        Err(error)
    } else {
        Ok(tokens)
    }
}
/// Bounded original plaintext fallback for a registry name that does not exist.
pub fn plain_bounded(
    source: &str,
    budget: &mut CodeBudget,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Vec<Token>, CodeWorkError> {
    let mut control = Control::bounded(budget, cancel);
    let normalized = source.replace("\r\n", "\n").replace('\r', "\n");
    let tokens = plain_controlled(&normalized, &mut control);
    control.check();
    if let Some(error) = control.error {
        Err(error)
    } else {
        Ok(tokens)
    }
}
/// Names/aliases in the pinned registry; filename lookup remains a separate qualification gap.
pub fn registered(language: &str) -> bool {
    registry::get(language).is_some()
}
/// Original registry name/alias/extension/filename resolution.
pub fn lexer_name(language: &str) -> Option<&'static str> {
    registry::get(language)
}
fn tokenize_inner(language: &str, source: &str, control: &mut Control<'_>) -> Option<Vec<Token>> {
    let source = source.replace("\r\n", "\n").replace('\r', "\n");
    match registry::get(language)? {
        "markdown" => markdown::tokenize(&source, control),
        "YAML" => markdown::yaml(&source, control),
        "Python" => Some(python::tokenize(&source, control)),
        "Go" => Some(go(&source, control)),
        "Go Template" | "Go Text Template" => Some(template(&source, false, control)),
        // Chroma Registry.Get also resolves the *.txt plaintext extension.
        "plaintext" => Some(plain_controlled(&source, control)),
        name => xml::tokenize(&source, name, control),
    }
}
/// Source's plaintext lexer, used when no registry lexer matches a language.
pub fn plain(source: &str) -> Vec<Token> {
    plain_controlled(source, &mut Control::unlimited())
}
fn plain_controlled(source: &str, control: &mut Control<'_>) -> Vec<Token> {
    let mut tokens = Vec::new();
    for line in source.split_inclusive('\n') {
        if !control.check() {
            break;
        }
        let value = line.strip_suffix('\n').unwrap_or(line);
        if !value.is_empty() {
            push(&mut tokens, "Text", value, control);
        }
        if line.ends_with('\n') {
            push(&mut tokens, "Text", "\n", control);
        }
    }
    tokens
}
pub fn split_lines(tokens: Vec<Token>) -> Vec<Vec<Token>> {
    let mut control = Control::unlimited();
    let mut lines = Vec::new();
    let mut line = Vec::new();
    for token in tokens {
        let mut tail = token.value.as_str();
        while let Some(index) = tail.find('\n') {
            push(&mut line, token.kind, &tail[..=index], &mut control);
            lines.push(std::mem::take(&mut line));
            tail = &tail[index + 1..];
        }
        if !tail.is_empty() || token.value.is_empty() {
            push(&mut line, token.kind, tail, &mut control);
        }
    }
    if !line.is_empty() && !(line.len() == 1 && line[0].value.is_empty()) {
        lines.push(line);
    }
    lines
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub color: Option<String>,
    pub background: Option<String>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub no_inherit: bool,
}
fn styles(dark: bool) -> &'static BTreeMap<String, Style> {
    static LIGHT: OnceLock<BTreeMap<String, Style>> = OnceLock::new();
    static DARK: OnceLock<BTreeMap<String, Style>> = OnceLock::new();
    (if dark { &DARK } else { &LIGHT }).get_or_init(|| {
        let source = if dark {
            include_str!("../assets/chroma/catppuccin-mocha.xml")
        } else {
            include_str!("../assets/chroma/github.xml")
        };
        roxmltree::Document::parse(source)
            .expect("pinned code style")
            .descendants()
            .filter(|n| n.has_tag_name("entry"))
            .map(|n| {
                let mut entry = Style::default();
                for part in n.attribute("style").unwrap_or("").split_whitespace() {
                    match part {
                        "bold" => entry.bold = Some(true),
                        "nobold" => entry.bold = Some(false),
                        "italic" => entry.italic = Some(true),
                        "noitalic" => entry.italic = Some(false),
                        "underline" => entry.underline = Some(true),
                        "nounderline" => entry.underline = Some(false),
                        "noinherit" => entry.no_inherit = true,
                        _ if part.starts_with('#') => entry.color = Some(part.into()),
                        _ if part.starts_with("bg:") => entry.background = Some(part[3..].into()),
                        _ => {}
                    }
                }
                (n.attribute("type").unwrap().into(), entry)
            })
            .collect()
    })
}
pub fn style(kind: &str, dark: bool) -> Style {
    let styles = styles(dark);
    let mut entry = styles.get(kind).cloned().unwrap_or_default();
    // Chroma TokenType.Category/SubCategory inherit literal's string/number
    // subcategories before the outer Literal category.
    let category = [
        "Keyword",
        "Name",
        "Literal",
        "Operator",
        "Punctuation",
        "Comment",
        "Generic",
        "Text",
    ]
    .into_iter()
    .find(|prefix| kind.starts_with(prefix))
    .unwrap_or(kind);
    let subcategory = if kind.starts_with("NameBuiltin") {
        "NameBuiltin"
    } else if kind.starts_with("NameVariable") {
        "NameVariable"
    } else if kind.starts_with("NameFunction") {
        "NameFunction"
    } else if kind.starts_with("LiteralString") {
        "LiteralString"
    } else if kind.starts_with("LiteralNumber") {
        "LiteralNumber"
    } else {
        category
    };
    for parent in [subcategory, category, "Text", "Background"] {
        if entry.no_inherit {
            break;
        }
        if let Some(parent) = styles.get(parent) {
            if entry.color.is_none() {
                entry.color = parent.color.clone();
            }
            if entry.background.is_none() {
                entry.background = parent.background.clone();
            }
            if entry.bold.is_none() {
                entry.bold = parent.bold;
            }
            if entry.italic.is_none() {
                entry.italic = parent.italic;
            }
            if entry.underline.is_none() {
                entry.underline = parent.underline;
            }
            entry.no_inherit = parent.no_inherit;
        }
    }
    entry
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn plaintext_names_aliases_and_txt_extension_match_original_token_streams() {
        let controls: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/chroma-plaintext-extension-source-v090.json"
        ))
        .unwrap();
        for control in controls.as_array().unwrap() {
            let tokens = tokenize(
                control["language"].as_str().unwrap(),
                control["source"].as_str().unwrap(),
            )
            .expect("Source plaintext registry lookup");
            let actual = serde_json::Value::Array(
                tokens
                    .iter()
                    .map(|token| serde_json::json!({"kind":token.kind,"value":token.value}))
                    .collect(),
            );
            assert_eq!(
                actual, control["tokens"],
                "complete Source stream {control}"
            );
        }
        // Python now uses the pinned native state-machine translation.
        assert!(tokenize("python", "print(1)").is_some());
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn pinned_go_and_template_tokens_and_theme_entries() {
        let cases: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/chroma-go-template-tokens.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let language = case["language"].as_str().unwrap();
            let source = case["source"].as_str().unwrap();
            let tokens = tokenize(language, source).unwrap();
            let actual = tokens
                .iter()
                .map(|token| serde_json::json!({"kind":token.kind,"value":token.value}))
                .collect::<Vec<_>>();
            assert_eq!(
                serde_json::json!(actual),
                case["tokens"],
                "tokens {language}: {source:?}"
            );
            for (name, dark) in [("github", false), ("catppuccin-mocha", true)] {
                let actual=tokens.iter().map(|token|{let e=style(token.kind,dark);serde_json::json!({"kind":token.kind,"value":token.value,"color":e.color.unwrap_or_default(),"bold":e.bold==Some(true),"italic":e.italic==Some(true),"underline":e.underline==Some(true)})}).collect::<Vec<_>>();
                assert_eq!(
                    serde_json::json!(actual),
                    case["styles"][name],
                    "styles {name}: {source:?}"
                );
            }
        }
    }
    #[test]
    fn token_line_boundaries_keep_empty_groups_and_drop_newline_tails() {
        let tokens = vec![
            Token {
                kind: "LiteralString",
                value: "one\n\ntwo\n".into(),
            },
            Token {
                kind: "TextWhitespace",
                value: "".into(),
            },
        ];
        let lines = split_lines(tokens);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0][0].value, "one\n");
        assert_eq!(lines[1][0].value, "\n");
        assert_eq!(lines[2][0].value, "two\n");
    }
}

#[cfg(test)]
mod measurement_tests {
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_code_block_cursor_bounds_across_font_sizes() {
        let cases: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/chroma-code-measurements.json"
        ))
        .unwrap();
        let font = crate::measure::bundled_source_code_pro_regular().unwrap();
        for case in cases.as_array().unwrap() {
            let size = case["size"].as_f64().unwrap();
            let source = case["source"].as_str().unwrap();
            let metrics = font.measure_with_dot_and_fallback(source, size, &font);
            let height =
                (metrics.height + size * 0.3 * metrics.lines.saturating_sub(1) as f64).ceil();
            assert_eq!(
                metrics.width.ceil(),
                case["width"].as_f64().unwrap(),
                "width {size}: {source:?}"
            );
            assert_eq!(
                height,
                case["height"].as_f64().unwrap(),
                "height {size}: {source:?}"
            );
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod python_source_contract {
    use super::*;
    #[test]
    fn complete_python_state_streams_and_both_source_styles() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/python-tokens-v227.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 1105);
        for row in rows.as_array().unwrap() {
            let tokens = tokenize(
                row["language"].as_str().unwrap(),
                row["source"].as_str().unwrap(),
            )
            .unwrap();
            let actual: Vec<serde_json::Value> = tokens
                .iter()
                .map(|t| serde_json::json!({"kind":t.kind,"value":t.value}))
                .collect();
            assert_eq!(
                serde_json::json!(actual),
                row["tokens"],
                "{}",
                row["source"]
            );
            for (name, dark) in [("github", false), ("catppuccin-mocha", true)] {
                let actual: Vec<serde_json::Value> = tokens.iter().map(|t| {
                    let s=style(t.kind,dark);
                    serde_json::json!({"kind":t.kind,"value":t.value,"color":s.color.unwrap_or_default(),"bold":s.bold==Some(true),"italic":s.italic==Some(true),"underline":s.underline==Some(true)})
                }).collect();
                assert_eq!(
                    serde_json::json!(actual),
                    row["styles"][name],
                    "{name}: {}",
                    row["source"]
                );
            }
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod markdown_yaml_source_contract {
    use super::*;
    #[test]
    fn original_markdown_yaml_tokens_and_both_theme_styles() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/markdown-yaml-tokens-v227.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 1038);
        for row in rows.as_array().unwrap() {
            let tokens = tokenize(
                row["language"].as_str().unwrap(),
                row["source"].as_str().unwrap(),
            )
            .unwrap();
            let actual: Vec<serde_json::Value> = tokens
                .iter()
                .map(|t| serde_json::json!({"kind":t.kind,"value":t.value}))
                .collect();
            assert_eq!(
                serde_json::json!(actual),
                row["tokens"],
                "{}",
                row["source"]
            );
            for (name, dark) in [("github", false), ("catppuccin-mocha", true)] {
                let actual:Vec<serde_json::Value>=tokens.iter().map(|t| {let s=style(t.kind,dark);serde_json::json!({"kind":t.kind,"value":t.value,"color":s.color.unwrap_or_default(),"bold":s.bold==Some(true),"italic":s.italic==Some(true),"underline":s.underline==Some(true)})}).collect();
                assert_eq!(
                    serde_json::json!(actual),
                    row["styles"][name],
                    "{name}: {}",
                    row["source"]
                );
            }
        }
        assert!(
            tokenize("markdown", "```raku\nmy $x=1;\n```").is_some(),
            "Raku fences use the native custom lexer"
        );
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod javascript_json_source_contract {
    use super::*;
    #[test]
    fn original_javascript_json_tokens_and_both_theme_styles() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/javascript-json-tokens-v227.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 1038);
        for row in rows.as_array().unwrap() {
            let tokens = tokenize(
                row["language"].as_str().unwrap(),
                row["source"].as_str().unwrap(),
            )
            .unwrap();
            let actual: Vec<serde_json::Value> = tokens
                .iter()
                .map(|t| serde_json::json!({"kind":t.kind,"value":t.value}))
                .collect();
            assert_eq!(
                serde_json::json!(actual),
                row["tokens"],
                "{}",
                row["source"]
            );
            for (name, dark) in [("github", false), ("catppuccin-mocha", true)] {
                let actual:Vec<serde_json::Value>=tokens.iter().map(|t| {let s=style(t.kind,dark);serde_json::json!({"kind":t.kind,"value":t.value,"color":s.color.unwrap_or_default(),"bold":s.bold==Some(true),"italic":s.italic==Some(true),"underline":s.underline==Some(true)})}).collect();
                assert_eq!(
                    serde_json::json!(actual),
                    row["styles"][name],
                    "{name}: {}",
                    row["source"]
                );
            }
        }
        assert!(
            tokenize("markdown", "```raku\nmy $x=1;\n```").is_some(),
            "Raku fences use the native custom lexer"
        );
    }
}

#[cfg(test)]
mod bash_svg_style_source_contract {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_bash_tokens_registry_names_and_d2svg_style_maps() {
        // tools/chroma-code-oracle against pinned Chroma v2.27.0.
        let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/chroma-bash-styles-v227.json"
        ))
        .unwrap();
        for row in fixture["cases"].as_array().unwrap() {
            let language = row["language"].as_str().unwrap();
            let source = row["source"].as_str().unwrap();
            let expected_lexer = row["lexer"].as_str().unwrap();
            let lexer = lexer_name(language);
            if expected_lexer == "fallback" {
                assert_eq!(lexer, None, "{language}");
                continue;
            }
            assert_eq!(lexer, Some(expected_lexer), "{language}");
            if expected_lexer != "Bash" {
                continue;
            }
            let tokens = tokenize(language, source).unwrap();
            let actual: Vec<serde_json::Value> = tokens
                .iter()
                .map(|t| serde_json::json!({"kind":t.kind,"value":t.value}))
                .collect();
            assert_eq!(serde_json::json!(actual), row["tokens"], "{source:?}");
        }
        for (name, dark) in [("github", false), ("catppuccin-mocha", true)] {
            let expected = fixture[name]["svg"].as_object().unwrap();
            let actual = chroma_style::svg_styles(dark);
            assert_eq!(actual.len(), expected.len(), "{name}");
            for (kind, svg) in expected {
                let t = chroma_style::token_type(kind).unwrap();
                assert_eq!(actual[&t], svg.as_str().unwrap(), "{name} {kind}");
            }
            assert_eq!(
                chroma_style::background(dark),
                fixture[name]["background"],
                "{name}"
            );
        }
    }
}
