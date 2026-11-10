// SPDX-License-Identifier: MIT
// Chroma v2.27.0 lexers/raku.go: dynamic rules, delimiters and quote/POD emitters.
// Copyright 2017 Alec Thomas. Rust translation Copyright 2026 Nelson Chen.
// See assets/chroma/COPYING and the root THIRD_PARTY_NOTICES.md.
use super::*;
use d2_rs_core::stable_sort::StableSort;
use std::{cell::RefCell, rc::Rc};

#[derive(Clone)]
pub(super) struct Replacement {
    delimiter: String,
    pattern: String,
    token: &'static str,
    state: String,
    bottom: bool,
    pop_state: bool,
    push_state: bool,
    restore_bottom: bool,
    substitution: bool,
}
impl Replacement {
    pub(super) fn from_xml(n: roxmltree::Node<'_, '_>) -> Self {
        Self {
            delimiter: n.attribute("delimiter").unwrap().into(),
            token: kind(n.attribute("token").unwrap()),
            pop_state: n.attribute("pop") == Some("true"),
            push_state: n.attribute("push") == Some("true"),
            ..Self::new(n.attribute("state").unwrap())
        }
    }
    fn new(state: &str) -> Self {
        Self {
            delimiter: String::new(),
            pattern: r"\A\z".into(),
            token: "Ignore",
            state: state.into(),
            bottom: false,
            pop_state: false,
            push_state: false,
            restore_bottom: false,
            substitution: false,
        }
    }
    fn closing(state: &str, delimiter: String) -> Self {
        Self {
            delimiter,
            token: "Punctuation",
            push_state: true,
            ..Self::new(state)
        }
    }
    fn rule(&self) -> Rule {
        // makeRule compiles dynamic patterns with regexp2.None, without the
        // lexer's multiline/dotall flags.
        let pattern = if self.delimiter.is_empty() {
            self.pattern.clone()
        } else {
            format!(r"(?<!(?<!\\)\\){}", regex::escape(&self.delimiter))
        };
        let emitter = if self.substitution {
            Emitter::Groups(vec![
                Emitter::Token("Punctuation"),
                Emitter::SelfState("qq".into()),
                Emitter::Token("Punctuation"),
            ])
        } else {
            Emitter::Token(self.token)
        };
        Rule {
            pattern: format!(r"\G(?:{})", translate(&pattern)),
            regex: OnceLock::new(),
            emitter: Some(emitter),
            mutators: Vec::new(),
        }
    }
}
// Go's RegexLexer shares its compiled rule table across Tokenise calls,
// including nested emitters, but gives each call fresh replacement stacks.
// Retain the serial Go behavior independently on each Rust caller thread;
// unrelated concurrent calls must not replace one another's delimiters.
// Ordinary lexers stay immutable.
type Slots = BTreeMap<(String, bool), (Rc<Rule>, Replacement)>;
thread_local! {
    static SLOTS: RefCell<Slots> = const { RefCell::new(BTreeMap::new()) };
}
#[derive(Default)]
struct Runtime {
    stack: Vec<String>,
    replacements: BTreeMap<(String, bool), Vec<Replacement>>,
}
impl Runtime {
    fn replace(&mut self, rule: Replacement, restoring: bool) -> Option<()> {
        let key = (rule.state.clone(), rule.bottom);
        if !restoring {
            self.replacements.entry(key.clone()).or_insert_with(|| {
                vec![Replacement {
                    bottom: rule.bottom,
                    ..Replacement::new(&rule.state)
                }]
            });
        }
        SLOTS.with_borrow_mut(|slots| {
            slots.insert(key.clone(), (Rc::new(rule.rule()), rule.clone()));
        });
        if rule.pop_state {
            self.stack.pop()?;
        }
        if rule.push_state {
            self.stack.push(rule.state.clone());
        }
        if !restoring {
            self.replacements.get_mut(&key)?.push(rule);
        }
        Some(())
    }
    fn pop_rule(&mut self, state: &str, bottom: bool) -> Option<()> {
        if let Some(stack) = self
            .replacements
            .get_mut(&(state.into(), bottom))
            .filter(|s| !s.is_empty())
        {
            stack.pop();
            let mut previous = stack.last()?.clone();
            previous.pop_state = false;
            previous.push_state = false;
            self.replace(previous, true)?;
        }
        Some(())
    }
}
fn names(rule: &Rule) -> Vec<String> {
    let mut names = Vec::new();
    let mut append = |regex: &Regex| {
        names.extend(regex.capture_names().skip(1).map(|n| n.map(str::to_owned)));
    };
    match rule.regex() {
        Matcher::Direct(regex) => append(regex),
        Matcher::Behind { assertions, rest } => {
            for (regex, _) in assertions {
                append(regex);
            }
            append(rest);
        }
    }
    // regexp2 numbers unnamed captures first, then named captures in source
    // order. fancy-regex numbers both in source order. Keep the latter here
    // to associate values; reorder only when exposing Go's group inventory.
    let mut number = 0;
    names
        .into_iter()
        .map(|n| {
            n.unwrap_or_else(|| {
                number += 1;
                number.to_string()
            })
        })
        .collect()
}
pub(super) fn run(
    source: &str,
    lexer: &Compiled,
    initial: &str,
    _nested: bool,
    control: &mut Control<'_>,
) -> Option<Vec<Token>> {
    let mut runtime = Runtime {
        stack: vec![initial.into()],
        ..Runtime::default()
    };
    let reversed = std::cell::OnceCell::new();
    let mut offset = 0;
    let mut tokens = Vec::new();
    while offset < source.len() && !runtime.stack.is_empty() && control.check() {
        let state = runtime.stack.last()?.clone();
        let rules = lexer.states.get(&state)?;
        let overrides = SLOTS.with_borrow(|slots| {
            [
                slots.get(&(state.clone(), false)).cloned(),
                slots.get(&(state.clone(), true)).cloned(),
            ]
        });
        let mut selected = None;
        for (index, base) in rules.iter().enumerate() {
            if !control.check() {
                return None;
            }
            let replacement = if index == 0 {
                overrides[0].as_ref()
            } else if index + 1 == rules.len() {
                overrides[1].as_ref()
            } else {
                None
            };
            let rule = replacement.map_or(base, |r| r.0.as_ref());
            if let Some((end, groups)) = rule.regex().captures(source, offset, &reversed) {
                selected = Some((rule, replacement.map(|r| &r.1), end, groups));
                break;
            }
        }
        if let Some((rule, replacement, end, captures)) = selected {
            offset = end;
            let mut groups: Vec<String> = captures.iter().map(|g| g.to_string()).collect();
            let names = names(rule);
            let mut named: BTreeMap<String, String> = names
                .iter()
                .cloned()
                .zip(groups.iter().skip(1).cloned())
                .collect();
            let mut go_names = names.clone();
            go_names
                .stable_sort_by_key(|name| name.parse::<usize>().ok().map_or((1, 0), |n| (0, n)));
            let numbered = names.iter().any(|n| n.parse::<usize>().is_ok());
            if numbered && names.iter().any(|n| n.parse::<usize>().is_err()) {
                let mut reordered = vec![groups[0].clone()];
                reordered.extend(go_names.iter().map(|n| named[n].clone()));
                groups = reordered;
            }
            if let Some(replacement) = replacement {
                if !replacement.bottom {
                    runtime.stack.pop()?;
                    runtime.pop_rule(&replacement.state, false)?;
                    if replacement.restore_bottom {
                        runtime.pop_rule(&replacement.state, true)?;
                    }
                }
            }
            for mutator in &rule.mutators {
                match mutator {
                    Mutator::Pop(depth) => runtime
                        .stack
                        .truncate(runtime.stack.len().checked_sub(*depth)?),
                    Mutator::Push(states) if states.is_empty() => runtime.stack.push(state.clone()),
                    Mutator::Push(states) => {
                        for name in states {
                            if name == "#pop" {
                                runtime.stack.pop()?;
                            } else {
                                runtime.stack.push(name.clone());
                            }
                        }
                    }
                    Mutator::RakuReplace(config) => {
                        runtime.replace(config.as_ref().clone(), false)?
                    }
                    Mutator::RakuBrackets(class) => find_brackets(
                        *class,
                        source,
                        &mut offset,
                        &mut groups,
                        &mut named,
                        &mut runtime,
                        control,
                    )?,
                    Mutator::HaxePreproc => unreachable!(),
                }
            }
            if let Some(emitter) = &rule.emitter {
                let groups: Vec<_> = groups.iter().map(String::as_str).collect();
                emit(
                    emitter,
                    &groups,
                    &go_names,
                    &named,
                    lexer,
                    &mut tokens,
                    control,
                )?;
            }
        } else {
            let ch = source[offset..].chars().next()?;
            if ch == '\n' && state != initial {
                runtime.stack = vec![initial.into()];
                continue;
            }
            let next = offset + ch.len_utf8();
            push(&mut tokens, "Error", &source[offset..next], control);
            offset = next;
        }
    }
    if runtime.stack.is_empty() && offset < source.len() {
        push(&mut tokens, "Error", &source[offset..], control);
    }
    Some(tokens)
}
// raku.go indexAt checks only the two immediately preceding backslashes.
// Preserve its rune-index/byte-count conversion on recursive escaped searches.
fn index_at(
    text: &[char],
    delimiter: &[char],
    pos: usize,
    control: &mut Control<'_>,
) -> Option<usize> {
    if delimiter.is_empty() {
        return Some(pos);
    }
    if !control.check() {
        return None;
    }
    let tail = text.get(pos..)?;
    let mut index = tail.windows(delimiter.len()).position(|w| w == delimiter)?;
    if (index > 1 && tail[index - 1] == '\\' && tail[index - 2] != '\\')
        || (index == 1 && tail[0] == '\\')
    {
        index = index_at(tail, delimiter, index + 1, control)?;
        // Go utf8.RuneCountInString(text[:idx]) also counts an incomplete rune.
        let prefix: String = tail.iter().collect();
        let bytes = &prefix.as_bytes()[..index];
        index = match std::str::from_utf8(bytes) {
            Ok(prefix) => prefix.chars().count(),
            Err(error) => {
                // RuneCountInString consumes each incomplete UTF-8 byte as
                // one RuneError, unlike lossy UTF-8's single replacement.
                std::str::from_utf8(&bytes[..error.valid_up_to()])
                    .unwrap()
                    .chars()
                    .count()
                    + bytes.len()
                    - error.valid_up_to()
            }
        };
    }
    Some(pos + index)
}
fn find_brackets(
    class: u8,
    source: &str,
    offset: &mut usize,
    groups: &mut Vec<String>,
    named: &mut BTreeMap<String, String>,
    runtime: &mut Runtime,
    control: &mut Control<'_>,
) -> Option<()> {
    static BRACKETS: OnceLock<BTreeMap<char, char>> = OnceLock::new();
    let brackets = BRACKETS.get_or_init(|| {
        serde_json::from_str(include_str!("../../assets/chroma/raku-brackets.json")).unwrap()
    });
    let get = |name: &str| named.get(name).cloned().unwrap_or_default();
    let opening: Vec<char> = if class == 2 {
        groups.get(1..5)?.concat().chars().collect()
    } else {
        get("opening_delimiters").chars().collect()
    };
    let first = *opening.first()?;
    let closing_char = brackets.get(&first).copied();
    let mirrored = class == 2 || closing_char.is_some();
    let mut count = opening.len();
    if class == 3 {
        let formatter = match get("keyword").as_str() {
            "B" => "GenericStrong",
            "I" => "GenericEmph",
            "U" => "GenericUnderline",
            _ => "LiteralStringOther",
        };
        runtime.replace(
            Replacement {
                bottom: true,
                pattern: ".+?".into(),
                token: formatter,
                ..Replacement::new("pod-formatter")
            },
            false,
        )?;
        let mut rule = Replacement::closing(
            "pod-formatter",
            std::iter::repeat_n(closing_char?, count).collect(),
        );
        rule.restore_bottom = true;
        return runtime.replace(rule, false);
    }
    if class == 6 {
        let delimiter = closing_char.map_or_else(|| opening.iter().collect(), |c| c.to_string());
        let mut rule = Replacement::closing("regex", delimiter);
        rule.pop_state = true;
        return runtime.replace(rule, false);
    }
    if class == 7 {
        let delimiter = regex::escape(&opening.iter().collect::<String>());
        return runtime.replace(
            Replacement {
                pattern: format!(r"({delimiter})((?:\\\\|\\/|.)*?)({delimiter})"),
                token: "Punctuation",
                substitution: true,
                pop_state: true,
                push_state: true,
                ..Replacement::new("regex")
            },
            false,
        );
    }
    let text: Vec<char> = source.chars().collect();
    let pos = source[..*offset].chars().count();
    let mut closing: Vec<char> = Vec::new();
    let mut end;
    if !mirrored {
        closing = opening.clone();
        end = index_at(&text, &closing, pos, control).unwrap_or(text.len());
    } else {
        let pod = if class == 2 {
            Some(
                Regex::new(&format!(
                    r"{}=end\s+{}",
                    get("ws"),
                    regex::escape(&get("name"))
                ))
                .ok()?,
            )
        } else {
            closing = std::iter::repeat_n(closing_char?, count).collect();
            None
        };
        let mut nesting = 1;
        let mut search = pos.checked_sub(count)?;
        end = text.len();
        while nesting > 0 && control.check() {
            let next_close = if let Some(pod) = &pod {
                let start_byte: usize = text[..(search + count).min(text.len())]
                    .iter()
                    .map(|c| c.len_utf8())
                    .sum();
                let matched = pod.find_from_pos(source, start_byte).ok().flatten()?;
                closing = matched.as_str().chars().collect();
                Some(source[..matched.start()].chars().count())
            } else {
                index_at(&text, &closing, search + count, control)
            };
            let next_open = index_at(&text, &opening, search + count, control);
            match (next_close, next_open) {
                (None, _) => {
                    end = text.len();
                    nesting = 0;
                }
                (Some(close), Some(open)) if open < close => {
                    nesting += 1;
                    count = opening.len();
                    search = open;
                }
                (Some(close), _) => {
                    nesting -= 1;
                    count = closing.len();
                    search = close;
                    end = close;
                }
            }
        }
    }
    let adverbs = get("adverbs");
    static HEREDOC: OnceLock<regex::Regex> = OnceLock::new();
    let mut terminator = Vec::new();
    let mut found_heredoc = false;
    if HEREDOC
        .get_or_init(|| regex::Regex::new(r":to\b|:heredoc\b").unwrap())
        .is_match(&adverbs)
    {
        if end != text.len() {
            terminator = text[pos..end].to_vec();
            count = terminator.len();
        } else {
            end = pos + 1;
            count = 0;
        }
        if count > 0 {
            if let Some(next) = index_at(&text[end..], &terminator, 0, control) {
                end += next;
                found_heredoc = true;
            } else {
                end = text.len();
            }
        }
    }
    let between: String = text.get(pos..end)?.iter().collect();
    match class {
        1 | 2 | 4 => {
            named.insert("value".into(), between);
            named.insert("closing_delimiters".into(), closing.iter().collect());
        }
        0 => {
            if !terminator.is_empty() {
                let first_punctuation = count + opening.len() + 1;
                named.insert(
                    "opening_delimiters".into(),
                    opening
                        .iter()
                        .chain(text.get(pos..pos + first_punctuation)?)
                        .collect(),
                );
                named.insert(
                    "value".into(),
                    text.get(pos + first_punctuation..end)?.iter().collect(),
                );
                if found_heredoc {
                    named.insert("closing_delimiters".into(), terminator.iter().collect());
                }
            } else {
                named.insert("value".into(), between);
                if count > 0 {
                    named.insert("closing_delimiters".into(), closing.iter().collect());
                }
            }
        }
        _ => {
            *groups = vec![format!(
                "{}{}",
                groups[0],
                text.get(pos..end + count)?.iter().collect::<String>()
            )];
        }
    }
    // Go advances in runes, including a delimiter length past EOF on an
    // unterminated quote. No further iteration occurs in that case.
    *offset = text[..(end + count).min(text.len())]
        .iter()
        .map(|c| c.len_utf8())
        .sum();
    Some(())
}

pub(super) fn emit(
    emitter: &Emitter,
    groups: &[&str],
    names: &[String],
    named: &BTreeMap<String, String>,
    lexer: &Compiled,
    tokens: &mut Vec<Token>,
    control: &mut Control<'_>,
) -> Option<()> {
    if !control.check() {
        return None;
    }
    let get = |name: &str| named.get(name).map_or("", String::as_str);
    match emitter {
        Emitter::NamedGroups(emitters) => {
            for name in names {
                let value = get(name);
                if let Some(emitter) = emitters.get(name) {
                    emit(
                        emitter,
                        &[value],
                        &[],
                        &BTreeMap::new(),
                        lexer,
                        tokens,
                        control,
                    )?;
                } else {
                    push(tokens, "Error", value, control);
                }
            }
        }
        Emitter::Raku(name) => match name.as_str() {
            "podConfig" => tokens.extend(run(
                groups[0],
                lexer,
                "colon-pair-attribute",
                true,
                control,
            )?),
            "quote" => {
                let keyword = get("keyword");
                let adverbs = get("adverbs");
                for (kind, value) in [
                    ("Keyword", keyword),
                    ("LiteralStringAffix", adverbs),
                    ("Text", get("ws")),
                    ("Punctuation", get("opening_delimiters")),
                ] {
                    push(tokens, kind, value, control);
                }
                let mut states = Vec::new();
                for adverb in adverbs.split(':') {
                    states.push(match adverb {
                        "c" | "closure" => "Q-closure",
                        "qq" => "qq",
                        "ww" => "ww",
                        "s" | "scalar" | "a" | "array" | "h" | "hash" | "f" | "function" => {
                            "Q-variable"
                        }
                        _ => "",
                    });
                }
                let state = if keyword == "qq" || states.contains(&"qq") {
                    "qq"
                } else if adverbs == "ww" || states.contains(&"ww") {
                    "ww"
                } else if states.contains(&"Q-closure") && states.contains(&"Q-variable") {
                    "qq"
                } else if states.contains(&"Q-closure") {
                    "Q-closure"
                } else if states.contains(&"Q-variable") {
                    "Q-variable"
                } else {
                    "Q"
                };
                tokens.extend(run(get("value"), lexer, state, true, control)?);
                push(tokens, "Punctuation", get("closing_delimiters"), control);
            }
            "colonPair-LiteralString" | "colonPair-NameAttribute" => {
                push(tokens, "Punctuation", get("colon"), control);
                let key = get("key");
                if name == "colonPair-NameAttribute" {
                    push(tokens, "NameAttribute", key, control);
                } else {
                    let state = if !key.is_empty() && key.bytes().all(|c| c.is_ascii_digit()) {
                        "common"
                    } else {
                        "Q"
                    };
                    tokens.extend(run(key, lexer, state, true, control)?);
                }
                let opening = get("opening_delimiters");
                push(tokens, "Punctuation", opening, control);
                let state = match opening {
                    "(" | "{" | "[" => Some("root"),
                    "<<" | "«" => Some("ww"),
                    "<" => Some("Q"),
                    _ => None,
                };
                if let Some(state) = state {
                    tokens.extend(run(get("value"), lexer, state, true, control)?);
                }
                push(tokens, "Punctuation", get("closing_delimiters"), control);
            }
            "podCode" => {
                for (kind, value) in [
                    ("Comment", get("ws")),
                    ("Keyword", get("keyword")),
                    ("Keyword", get("ws2")),
                    ("Keyword", get("name")),
                ] {
                    push(tokens, kind, value, control);
                }
                tokens.extend(run(
                    get("config"),
                    lexer,
                    "colon-pair-attribute",
                    true,
                    control,
                )?);
                static LANG: OnceLock<regex::Regex> = OnceLock::new();
                let language = LANG
                    // Go regexp's Perl word classes are ASCII, unlike
                    // regexp2 and Rust regex's default Unicode classes.
                    .get_or_init(|| {
                        regex::Regex::new(r":lang[^A-Za-z0-9_]+([A-Za-z0-9_]+)").unwrap()
                    })
                    .captures(get("config"))
                    .and_then(|c| c.get(1))
                    .map_or("", |m| m.as_str());
                if super::super::registry::get(language).is_some() {
                    tokens.extend(delegate(get("value"), language, false, control)?);
                } else {
                    push(tokens, "LiteralStringDoc", get("value"), control);
                }
                for (kind, value) in [
                    ("Comment", get("ws3")),
                    ("Keyword", get("end_keyword")),
                    ("Keyword", get("ws4")),
                    ("Keyword", get("name")),
                ] {
                    push(tokens, kind, value, control);
                }
            }
            _ => unreachable!("pinned Raku emitter {name}"),
        },
        _ => emitter.emit(groups, lexer, tokens, control)?,
    }
    Some(())
}
