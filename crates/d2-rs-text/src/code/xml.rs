// SPDX-License-Identifier: MIT
// Native execution of pinned Chroma v2.27.0 JavaScript/JSON/Bash RegexLexer rules.
// Source: regexp.go LexerState.Iterator/Tokenise, mutators.go Push/Pop/Include
// and emitters.go ByGroups.
use super::{Control, Token, push};
use fancy_regex::{Regex, RegexBuilder};
use std::{collections::BTreeMap, sync::OnceLock};
fn kind(name: &str) -> &'static str {
    super::chroma_style::token_name(name).unwrap_or_else(|| panic!("untranslated XML token {name}"))
}
/// Pinned XML lexers executed by this engine.
#[derive(Clone, Copy)]
pub(super) enum Lexer {
    JavaScript,
    Json,
    Bash,
}
/// Rewrites regexp2 (.NET-syntax) patterns into fancy-regex syntax.
/// Escape- and class-aware: an escaped backslash is copied verbatim, `\b`
/// inside a class stays a backspace, and characters that the Rust class
/// parser treats as nesting/set operators (`[`, `&`, `~`, a leading `]`)
/// are escaped inside classes where .NET reads them literally.
fn translate(pattern: &str) -> String {
    let word = r"[\p{L}\p{Mn}\p{Nd}\p{Pc}\u{200c}\u{200d}]";
    let not_word = r"[^\p{L}\p{Mn}\p{Nd}\p{Pc}\u{200c}\u{200d}]";
    let boundary = format!("(?:(?<={word})(?!{word})|(?<!{word})(?={word}))");
    let nonboundary = format!("(?:(?<={word})(?={word})|(?<!{word})(?!{word}))");
    let mut out = String::with_capacity(pattern.len());
    let chars: Vec<char> = pattern.chars().collect();
    let mut in_class = false;
    let mut class_start = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        let at_start = class_start;
        class_start = false;
        if c == '\\' {
            let Some(&next) = chars.get(i) else {
                out.push('\\');
                break;
            };
            i += 1;
            match next {
                // regexp2's Unicode word class: L, Mn, Nd, Pc and the joiners.
                'w' => out.push_str(word),
                'W' => out.push_str(not_word),
                'd' => out.push_str(r"\p{Nd}"),
                'b' if in_class => out.push_str(r"\x08"),
                'b' => out.push_str(&boundary),
                'B' => out.push_str(&nonboundary),
                other => {
                    out.push('\\');
                    out.push(other);
                }
            }
            continue;
        }
        if in_class {
            match c {
                ']' if at_start => out.push_str(r"\]"),
                ']' => {
                    in_class = false;
                    out.push(']');
                }
                '^' if at_start && out.ends_with('[') => {
                    out.push('^');
                    class_start = true;
                }
                '[' | '&' | '~' => {
                    out.push('\\');
                    out.push(c);
                }
                _ => out.push(c),
            }
        } else {
            if c == '[' {
                in_class = true;
                class_start = true;
            }
            out.push(c);
        }
    }
    out
}
struct Rule {
    regex: Regex,
    /// A single token type, or ByGroups emitters (one per capture group).
    kinds: Vec<&'static str>,
    by_groups: bool,
    pop: usize,
    push: Vec<String>,
}
struct Compiled {
    states: BTreeMap<String, Vec<Rule>>,
    ensure_nl: bool,
}
fn compile(source: &str) -> Compiled {
    // Chroma's Go XML codec represents ordered Push states using repeated
    // state attributes. Preserve every attribute in order while converting
    // that source encoding into children accepted by the strict XML parser.
    let pushes = regex::Regex::new(r#"<push((?:\s+state="[^"]*")*)\s*/>"#).unwrap();
    let attributes = regex::Regex::new(r#"state="([^"]*)""#).unwrap();
    let xml = pushes.replace_all(source, |caps: &regex::Captures<'_>| {
        let mut result = String::from("<push>");
        for attr in attributes.captures_iter(&caps[1]) {
            result.push_str("<state name=\"");
            result.push_str(&attr[1]);
            result.push_str("\"/>");
        }
        result.push_str("</push>");
        result
    });
    let doc = roxmltree::Document::parse(&xml).expect("pinned Chroma XML");
    let enabled = |name| {
        doc.descendants()
            .any(|n| n.has_tag_name(name) && n.text() == Some("true"))
    };
    let flags = format!(
        "{}{}",
        if enabled("not_multiline") { "" } else { "m" },
        if enabled("dot_all") { "s" } else { "" }
    );
    fn expand<'a>(
        doc: &'a roxmltree::Document<'a>,
        state: &str,
        rules: &mut Vec<roxmltree::Node<'a, 'a>>,
    ) {
        let node = doc
            .descendants()
            .find(|n| {
                n.has_tag_name("state")
                    && n.attribute("name") == Some(state)
                    && n.parent().is_some_and(|p| p.has_tag_name("rules"))
            })
            .unwrap();
        for rule in node.children().filter(|n| n.has_tag_name("rule")) {
            if let Some(include) = rule.children().find(|n| n.has_tag_name("include")) {
                expand(doc, include.attribute("state").unwrap(), rules);
            } else {
                rules.push(rule);
            }
        }
    }
    let mut states = BTreeMap::new();
    for state in doc
        .descendants()
        .filter(|n| n.has_tag_name("state") && n.parent().is_some_and(|p| p.has_tag_name("rules")))
    {
        let name = state.attribute("name").unwrap();
        let mut rows = Vec::new();
        expand(&doc, name, &mut rows);
        let rules = rows
            .into_iter()
            .map(|row| {
                let pattern = translate(row.attribute("pattern").unwrap_or(""));
                let pattern = format!(r"(?{flags})\G(?:{pattern})");
                let regex = RegexBuilder::new(&pattern)
                    .build()
                    .unwrap_or_else(|e| panic!("Chroma {name} {pattern}: {e}"));
                let groups = row.children().find(|n| n.has_tag_name("bygroups"));
                let kinds = groups
                    .unwrap_or(row)
                    .children()
                    .filter(|n| n.has_tag_name("token"))
                    .map(|n| kind(n.attribute("type").unwrap()))
                    .collect();
                let pop = row
                    .children()
                    .find(|n| n.has_tag_name("pop"))
                    .map_or(0, |n| n.attribute("depth").unwrap().parse().unwrap());
                let push =
                    row.children()
                        .find(|n| n.has_tag_name("push"))
                        .map_or_else(Vec::new, |n| {
                            n.children()
                                .filter(|n| n.has_tag_name("state"))
                                .map(|n| n.attribute("name").unwrap().to_owned())
                                .collect()
                        });
                Rule {
                    regex,
                    kinds,
                    by_groups: groups.is_some(),
                    pop,
                    push,
                }
            })
            .collect();
        states.insert(name.to_owned(), rules);
    }
    Compiled {
        states,
        ensure_nl: enabled("ensure_nl"),
    }
}
pub(super) fn tokenize(
    source: &str,
    which: Lexer,
    control: &mut Control<'_>,
) -> Option<Vec<Token>> {
    static JS: OnceLock<Compiled> = OnceLock::new();
    static JSON: OnceLock<Compiled> = OnceLock::new();
    static BASH: OnceLock<Compiled> = OnceLock::new();
    let lexer = match which {
        Lexer::JavaScript => {
            JS.get_or_init(|| compile(include_str!("../../assets/chroma/javascript.xml")))
        }
        Lexer::Json => JSON.get_or_init(|| compile(include_str!("../../assets/chroma/json.xml"))),
        Lexer::Bash => BASH.get_or_init(|| compile(include_str!("../../assets/chroma/bash.xml"))),
    };
    let end = source.len();
    let text = if lexer.ensure_nl && !source.ends_with('\n') {
        format!("{source}\n")
    } else {
        source.to_owned()
    };
    let mut offset = 0;
    let mut stack = vec!["root".to_owned()];
    let mut tokens = Vec::new();
    while offset < end && !stack.is_empty() && control.check() {
        let state = stack.last().unwrap();
        let mut selected = None;
        for rule in &lexer.states[state] {
            if !control.check() {
                return None;
            }
            if let Some(captures) = rule.regex.captures_from_pos(&text, offset).ok()? {
                if captures.get(0).unwrap().start() == offset {
                    selected = Some((rule, captures));
                    break;
                }
            }
        }
        if let Some((rule, captures)) = selected {
            let matched = captures.get(0).unwrap();
            offset = matched.end();
            stack.truncate(stack.len().checked_sub(rule.pop)?);
            for state in &rule.push {
                if state == "#pop" {
                    stack.pop()?;
                } else {
                    stack.push(state.clone());
                }
            }
            if rule.by_groups {
                // emitters.go ByGroups: one token per group, "" for groups
                // that did not participate; a count mismatch emits Error.
                if rule.kinds.len() + 1 != captures.len() {
                    push(&mut tokens, "Error", matched.as_str(), control);
                } else {
                    for (index, kind) in rule.kinds.iter().enumerate() {
                        let value = captures.get(index + 1).map_or("", |g| g.as_str());
                        push(&mut tokens, kind, value, control);
                    }
                }
            } else if let Some(kind) = rule.kinds.first() {
                push(&mut tokens, kind, matched.as_str(), control);
            }
        } else {
            let ch = text[offset..].chars().next().unwrap();
            if ch == '\n' && state != "root" {
                stack = vec!["root".to_owned()];
                continue;
            }
            let next = offset + ch.len_utf8();
            push(&mut tokens, "Error", &text[offset..next], control);
            offset = next;
        }
    }
    if stack.is_empty() && offset != text.len() {
        push(&mut tokens, "Error", &text[offset..], control);
    }
    Some(tokens)
}
