// SPDX-License-Identifier: MIT
// Native translation of Chroma v2.27.0 Python RegexLexer state execution.
use super::{Control, Token, push};
use regex::Regex;
use std::{collections::BTreeMap, sync::OnceLock};
#[path = "python_rules.rs"]
mod rules;
pub(super) struct Definition {
    pattern: &'static str,
    kinds: &'static [&'static str],
    push: &'static [&'static str],
    pop: usize,
    not_dot: bool,
    boundary: bool,
    anchor: u8,
}
struct Compiled {
    definition: &'static Definition,
    regex: Regex,
}
fn states() -> &'static BTreeMap<&'static str, Vec<Compiled>> {
    static STATES: OnceLock<BTreeMap<&str, Vec<Compiled>>> = OnceLock::new();
    STATES.get_or_init(|| rules::STATES.iter().map(|(name, definitions)| {
        (*name, definitions.iter().map(|definition| {
            let pattern = definition.pattern.replace(r"\w", r"[\p{L}\p{Mn}\p{Nd}\p{Pc}\u{200c}\u{200d}]");
            let pattern = if definition.boundary {
                format!(r"\A(?m:({pattern})(?:\z|[^\p{{L}}\p{{Mn}}\p{{Nd}}\p{{Pc}}\u{{200c}}\u{{200d}}]))")
            } else { format!(r"\A(?m:{pattern})") };
            Compiled { definition, regex: Regex::new(&pattern).unwrap_or_else(|error| panic!("Python {name}: {pattern}: {error}")) }
        }).collect())
    }).collect())
}
pub(super) fn tokenize(source: &str, control: &mut Control<'_>) -> Vec<Token> {
    let states = states();
    let mut stack = vec![vec!["root"]];
    let mut offset = 0;
    let mut tokens = Vec::new();
    while offset < source.len() && !stack.is_empty() && control.check() {
        let mut selected = None;
        'rules: for state in stack.last().unwrap() {
            for rule in &states[state] {
                if !control.check() {
                    return tokens;
                }
                if rule.definition.not_dot && source[..offset].ends_with('.') {
                    continue;
                }
                if rule.definition.anchor == 2 && offset != 0 {
                    continue;
                }
                if rule.definition.anchor == 1 && offset != 0 && !source[..offset].ends_with('\n') {
                    continue;
                }
                let Some(captures) = rule.regex.captures(&source[offset..]) else {
                    continue;
                };
                selected = Some((rule, captures));
                break 'rules;
            }
        }
        if let Some((rule, captures)) = selected {
            let definition = rule.definition;
            let matched = captures.get(usize::from(definition.boundary)).unwrap();
            for (index, kind) in definition.kinds.iter().enumerate() {
                let value = if definition.kinds.len() == 1 {
                    matched.as_str()
                } else {
                    captures
                        .get(index + 1 + usize::from(definition.boundary))
                        .map_or("", |group| group.as_str())
                };
                push(&mut tokens, kind, value, control);
            }
            offset += matched.end();
            if definition.pop > 0 {
                stack.truncate(stack.len() - definition.pop);
            }
            if !definition.push.is_empty() {
                stack.push(definition.push.to_vec());
            }
        } else {
            let character = source[offset..].chars().next().unwrap();
            if character == '\n' && stack != [vec!["root"]] {
                stack = vec![vec!["root"]];
                continue;
            }
            let end = offset + character.len_utf8();
            push(&mut tokens, "Error", &source[offset..end], control);
            offset = end;
        }
    }
    tokens
}
