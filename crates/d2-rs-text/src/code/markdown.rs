// SPDX-License-Identifier: MIT
// Chroma v2.27.0 Markdown/YAML rules. Copyright 2017 Alec Thomas.
use super::{Control, Token, push};
use fancy_regex::{Regex, RegexBuilder};
use std::sync::OnceLock;
struct Rule {
    regex: Regex,
    kinds: Vec<&'static str>,
    grouped: bool,
    delegate: u8,
}
fn compile(pattern: &str, kinds: Vec<&'static str>, grouped: bool, delegate: u8) -> Rule {
    let word = r"[\p{L}\p{Mn}\p{Nd}\p{Pc}\u{200c}\u{200d}]";
    let nonword = r"[^\p{L}\p{Mn}\p{Nd}\p{Pc}\u{200c}\u{200d}]";
    let boundary = format!("(?:(?<={word})(?!{word})|(?<!{word})(?={word}))");
    let pattern = pattern
        .replace(r"\w", word)
        .replace(r"\W", nonword)
        .replace(r"\b", &boundary)
        .replace(r"\!", "!")
        .replace(r"\@", "@")
        .replace(r"\:", ":")
        .replace(r"\?", "?");
    let pattern = format!(r"(?m)\G(?:{pattern})");
    let regex = RegexBuilder::new(&pattern)
        .build()
        .unwrap_or_else(|e| panic!("Chroma {pattern}: {e}"));
    Rule {
        regex,
        kinds,
        grouped,
        delegate,
    }
}
fn inline_rules() -> &'static Vec<Rule> {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        vec![
            compile(r"<!--[\w\W]*?-->", vec!["CommentMultiline"], false, 0),
            compile(r"\\.", vec!["Text"], false, 0),
            compile(
                r"(\s)(\*|_)((?:(?!\2).)*)(\2)((?=\W|\n))",
                vec!["Text", "GenericEmph", "GenericEmph", "GenericEmph", "Text"],
                true,
                0,
            ),
            compile(
                r"(\s)((\*\*|__).*?)\3((?=\W|\n))",
                vec!["Text", "GenericStrong", "GenericStrong", "Text"],
                true,
                0,
            ),
            compile(
                r"(\s)(~~[^~]+~~)((?=\W|\n))",
                vec!["Text", "GenericDeleted", "Text"],
                true,
                0,
            ),
            compile(r"`[^`]+`", vec!["LiteralStringBacktick"], false, 0),
            compile(r"[@#][\w/:]+", vec!["NameEntity"], false, 0),
            compile(
                r"(!?\[)([^]]+)(\])(\()([^)]+)(\))",
                vec!["Text", "NameTag", "Text", "Text", "NameAttribute", "Text"],
                true,
                0,
            ),
            compile(r".|\n", vec!["Text"], false, 0),
        ]
    })
}
fn root_rules() -> &'static Vec<Rule> {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        vec![
            compile(r"<!--[\w\W]*?-->", vec!["CommentMultiline"], false, 0),
            compile(r"^(#[^#].+\n)", vec!["GenericHeading"], true, 0),
            compile(r"^(#{2,6}.+\n)", vec!["GenericSubheading"], true, 0),
            compile(
                r"^(\s*)([*-] )(\[[ xX]\])( .+\n)",
                vec!["Text", "Keyword", "Keyword", "Text"],
                true,
                4,
            ),
            compile(
                r"^(\s*)([*-])(\s)(.+\n)",
                vec!["Text", "Keyword", "Text", "Text"],
                true,
                4,
            ),
            compile(
                r"^(\s*)([0-9]+\.)( .+\n)",
                vec!["Text", "Keyword", "Text"],
                true,
                3,
            ),
            compile(r"^(\s*>\s)(.+\n)", vec!["Keyword", "GenericEmph"], true, 0),
            compile(
                "^(```\\n)([\\w\\W]*?)(^```$)",
                vec!["LiteralString", "Text", "LiteralString"],
                true,
                0,
            ),
            compile(
                "^(```)(\\w+)(\\n)([\\w\\W]*?)(^```$)",
                vec![
                    "LiteralString",
                    "LiteralString",
                    "LiteralString",
                    "Text",
                    "LiteralString",
                ],
                true,
                255,
            ),
        ]
    })
}
fn execute(source: &str, inline: bool, control: &mut Control<'_>) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut offset = 0;
    while offset < source.len() && control.check() {
        let mut found = false;
        for rule in (if inline {
            [].iter()
        } else {
            root_rules().iter()
        })
        .chain(inline_rules().iter())
        {
            let Some(c) = rule.regex.captures_from_pos(source, offset).ok()? else {
                continue;
            };
            let m = c.get(0).unwrap();
            if m.start() != offset {
                continue;
            }
            for (i, kind) in rule.kinds.iter().enumerate() {
                let value = if rule.grouped {
                    c.get(i + 1).map_or("", |m| m.as_str())
                } else {
                    m.as_str()
                };
                if rule.delegate == 255 && i == 3 {
                    let language = c.get(2).unwrap().as_str();
                    if let Some(inner) = super::tokenize_inner(language, value, control) {
                        tokens.extend(inner)
                    } else if registered(language) {
                        return None;
                    } else {
                        push(&mut tokens, kind, value, control)
                    }
                } else if rule.delegate != 0
                    && rule.delegate != 255
                    && i + 1 == usize::from(rule.delegate)
                {
                    tokens.extend(execute(value, true, control)?);
                } else {
                    push(&mut tokens, kind, value, control)
                }
            }
            offset = m.end();
            found = true;
            break;
        }
        if !found {
            let ch = source[offset..].chars().next().unwrap();
            let end = offset + ch.len_utf8();
            push(&mut tokens, "Error", &source[offset..end], control);
            offset = end;
        }
    }
    Some(tokens)
}
fn registered(language: &str) -> bool {
    super::registered(language)
}
fn kind(value: &str) -> &'static str {
    match value {
        "NameNamespace" => "NameNamespace",
        "Text" => "Text",
        "Comment" => "Comment",
        "CommentPreproc" => "CommentPreproc",
        "Punctuation" => "Punctuation",
        "LiteralStringDoc" => "LiteralStringDoc",
        "Ignore" => "Ignore",
        "KeywordConstant" => "KeywordConstant",
        "LiteralStringDouble" => "LiteralStringDouble",
        "LiteralStringSingle" => "LiteralStringSingle",
        "LiteralDate" => "LiteralDate",
        "LiteralNumber" => "LiteralNumber",
        "Literal" => "Literal",
        "TextWhitespace" => "TextWhitespace",
        "NameTag" => "NameTag",
        _ => panic!("YAML token {value}"),
    }
}
fn yaml_rules() -> &'static Vec<Rule> {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        let doc = roxmltree::Document::parse(include_str!("../../assets/chroma/yaml.xml")).unwrap();
        fn expand(doc: &roxmltree::Document<'_>, name: &str, out: &mut Vec<Rule>) {
            let state = doc
                .descendants()
                .find(|n| n.has_tag_name("state") && n.attribute("name") == Some(name))
                .unwrap();
            for node in state.children().filter(|n| n.has_tag_name("rule")) {
                if let Some(inc) = node.children().find(|n| n.has_tag_name("include")) {
                    expand(doc, inc.attribute("state").unwrap(), out);
                    continue;
                }
                let group = node.children().find(|n| n.has_tag_name("bygroups"));
                let kinds = if let Some(group) = group {
                    group
                        .children()
                        .filter(|n| n.has_tag_name("token"))
                        .map(|n| kind(n.attribute("type").unwrap()))
                        .collect()
                } else {
                    vec![kind(
                        node.children()
                            .find(|n| n.has_tag_name("token"))
                            .unwrap()
                            .attribute("type")
                            .unwrap(),
                    )]
                };
                out.push(compile(
                    node.attribute("pattern").unwrap(),
                    kinds,
                    group.is_some(),
                    0,
                ));
            }
        }
        let mut out = Vec::new();
        expand(&doc, "root", &mut out);
        out
    })
}
pub(super) fn yaml(source: &str, control: &mut Control<'_>) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut offset = 0;
    while offset < source.len() && control.check() {
        let mut found = false;
        for rule in yaml_rules() {
            let Some(c) = rule.regex.captures_from_pos(source, offset).ok()? else {
                continue;
            };
            let m = c.get(0).unwrap();
            if m.start() != offset {
                continue;
            }
            for (i, kind) in rule.kinds.iter().enumerate() {
                if *kind == "Ignore" {
                    continue;
                }
                let value = if rule.grouped {
                    c.get(i + 1).map_or("", |m| m.as_str())
                } else {
                    m.as_str()
                };
                push(&mut tokens, kind, value, control);
            }
            offset = m.end();
            found = true;
            break;
        }
        if !found {
            let ch = source[offset..].chars().next().unwrap();
            let end = offset + ch.len_utf8();
            push(&mut tokens, "Error", &source[offset..end], control);
            offset = end;
        }
    }
    Some(tokens)
}
pub(super) fn tokenize(source: &str, control: &mut Control<'_>) -> Option<Vec<Token>> {
    if source.starts_with("---\n") {
        let mut offset = 4;
        while let Some(end) = source[offset..].find('\n') {
            if !control.check() {
                return None;
            }
            let next = offset + end + 1;
            if &source[offset..offset + end] == "---" {
                let mut tokens = yaml(&source[..next], control)?;
                tokens.extend(execute(&source[next..], false, control)?);
                return Some(tokens);
            }
            offset = next;
        }
    }
    execute(source, false, control)
}
