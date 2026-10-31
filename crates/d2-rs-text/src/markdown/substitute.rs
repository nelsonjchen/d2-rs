// SPDX-License-Identifier: MPL-2.0
//! Markdown text-segment substitution used by D2 block strings.
//!
//! D2 substitutes variables in Markdown text nodes while leaving code spans
//! and code blocks untouched. Replacements are collected from source offsets
//! and applied from right to left so one replacement cannot shift the next.

use super::{
    arena::NodeId,
    bytes::Bytes,
    context::{Context, NodeKind},
    text::Segment,
};
use d2_rs_core::LayoutError;
use std::collections::{BTreeMap, VecDeque};

/// A failed substitution never publishes partial text or a partial work count.
#[derive(Clone, Debug)]
pub enum SubstitutionError {
    NegativeLimit,
    ExpansionLimit,
    ResultTooLarge,
    Context(LayoutError),
}
impl std::fmt::Display for SubstitutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NegativeLimit => "negative Markdown substitution expansion limit",
            Self::ExpansionLimit => "Markdown substitution expansion limit exceeded",
            Self::ResultTooLarge => "Markdown substitution result is too large",
            Self::Context(error) => return std::fmt::Display::fmt(error, f),
        })
    }
}
impl std::error::Error for SubstitutionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Context(error) => Some(error),
            _ => None,
        }
    }
}
type Check<'a> = dyn FnMut() -> Option<LayoutError> + 'a;
#[cfg(test)]
mod sparse_tests;
fn check_context(check: &mut Check<'_>) -> Result<(), SubstitutionError> {
    check().map_or(Ok(()), |error| Err(SubstitutionError::Context(error)))
}

#[derive(Default)]
struct State {
    next: BTreeMap<u8, usize>,
    fail: usize,
    output: Option<usize>,
    terminal: Option<(usize, String)>,
}
struct Matcher {
    states: Vec<State>,
}
impl Matcher {
    // d2/lib/textmeasure/substitutions.go: byte-oriented Aho-Corasick over
    // complete patterns. Quoted variable names can contain closing braces.
    fn new(
        variables: &BTreeMap<String, String>,
        check: &mut Check<'_>,
    ) -> Result<Self, SubstitutionError> {
        check_context(check)?;
        let mut states = vec![State::default()];
        for (name, value) in variables {
            check_context(check)?;
            let pattern = format!("${{{name}}}");
            let mut state = 0;
            for (i, b) in pattern.bytes().enumerate() {
                if i & 1023 == 0 {
                    check_context(check)?;
                }
                state = if let Some(next) = states[state].next.get(&b) {
                    *next
                } else {
                    let next = states.len();
                    states[state].next.insert(b, next);
                    states.push(State::default());
                    next
                };
            }
            states[state].terminal = Some((pattern.len(), value.clone()));
        }
        let mut queue: VecDeque<_> = states[0].next.values().copied().collect();
        while let Some(state) = queue.pop_front() {
            check_context(check)?;
            let next: Vec<_> = states[state].next.iter().map(|(b, s)| (*b, *s)).collect();
            for (b, next) in next {
                let mut failure = states[state].fail;
                while failure != 0 && !states[failure].next.contains_key(&b) {
                    failure = states[failure].fail;
                }
                if let Some(candidate) = states[failure].next.get(&b) {
                    states[next].fail = *candidate;
                }
                let failure = states[next].fail;
                states[next].output = if states[failure].terminal.is_some() {
                    Some(failure)
                } else {
                    states[failure].output
                };
                queue.push_back(next);
            }
        }
        Ok(Self { states })
    }
    fn replace(
        &self,
        text: &str,
        limit: u64,
        check: &mut Check<'_>,
    ) -> Result<(String, u64), SubstitutionError> {
        check_context(check)?;
        if text.is_empty() {
            return Ok((text.to_owned(), 0));
        }
        let mut best = BTreeMap::<usize, (usize, &str)>::new();
        let mut used = 0;
        let mut state = 0;
        for (i, b) in text.bytes().enumerate() {
            if i & 1023 == 0 {
                check_context(check)?;
            }
            while state != 0 && !self.states[state].next.contains_key(&b) {
                state = self.states[state].fail;
            }
            state = self.states[state].next.get(&b).copied().unwrap_or(0);
            let mut candidate = Some(state);
            while let Some(index) = candidate {
                let matched = &self.states[index];
                if let Some((len, value)) = &matched.terminal {
                    if used >= limit {
                        return Err(SubstitutionError::ExpansionLimit);
                    }
                    used += 1;
                    if used & 1023 == 0 {
                        check_context(check)?;
                    }
                    let start = i + 1 - len;
                    let entry = best.entry(start).or_insert((0, ""));
                    if *len > entry.0 {
                        *entry = (*len, value);
                    }
                }
                candidate = matched.output;
            }
        }
        let mut replacements = Vec::new();
        // Go scans original byte offsets, jumping over accepted matches. The
        // periodic context checks follow those offsets even without matches.
        let mut start = 0;
        while start < text.len() {
            if start & 1023 == 0 {
                check_context(check)?;
            }
            let Some(&(len, value)) = best.get(&start) else {
                start += 1;
                continue;
            };
            if value.len() as u64 > limit - used {
                return Err(SubstitutionError::ExpansionLimit);
            }
            used += value.len() as u64;
            let stop = start + len;
            replacements.push((start, stop, value));
            start = stop;
        }
        if replacements.is_empty() {
            return Ok((text.to_owned(), 0));
        }
        let mut result_len = text.len();
        for &(start, stop, value) in &replacements {
            result_len = result_len
                .checked_sub(stop - start)
                .and_then(|len| len.checked_add(value.len()))
                .filter(|len| *len <= isize::MAX as usize)
                .ok_or(SubstitutionError::ResultTooLarge)?;
        }
        let mut output = String::with_capacity(result_len);
        let mut written = 0;
        for (start, stop, value) in replacements {
            check_context(check)?;
            output.push_str(&text[written..start]);
            output.push_str(value);
            written = stop;
        }
        output.push_str(&text[written..]);
        Ok((output, used))
    }
}

fn collect_text_nodes(
    context: &Context,
    node: NodeId,
    source: &[u8],
    matcher: &Matcher,
    replacements: &mut Vec<(Segment, String)>,
    remaining: &mut u64,
    check: &mut Check<'_>,
) -> Result<(), SubstitutionError> {
    check_context(check)?;
    match &context.arena.node(node).data {
        NodeKind::FencedCodeBlock { .. } | NodeKind::CodeBlock { .. } | NodeKind::CodeSpan => {
            return check_context(check);
        }
        NodeKind::Text(text) => {
            let source_value = text.segment.value(source);
            let original = String::from_utf8_lossy(&source_value);
            let (replaced, used) = matcher.replace(&original, *remaining, check)?;
            *remaining -= used;
            if original != replaced {
                replacements.push((text.segment, replaced));
            }
        }
        _ => {}
    }
    let mut child = context.arena.node(node).first;
    while let Some(id) = child {
        collect_text_nodes(context, id, source, matcher, replacements, remaining, check)?;
        child = context.arena.node(id).next;
    }
    // ast.Walk also visits each non-skipped node when exiting.
    check_context(check)
}

/// Replaces D2 variables in visible Markdown text segments.
pub fn replace_substitutions(source: &[u8], variables: &BTreeMap<String, String>) -> String {
    replace_substitutions_bounded(source, variables, u64::MAX)
        .map(|(value, _)| value)
        .unwrap_or_else(|_| String::from_utf8_lossy(source).into_owned())
}

/// Bounded master API. One unit per candidate match and per replacement byte;
/// no replacement output is allocated until all of that text node is reserved.
pub fn replace_substitutions_bounded(
    source: &[u8],
    variables: &BTreeMap<String, String>,
    max_expansion: u64,
) -> Result<(String, u64), &'static str> {
    replace_bounded(source, variables, max_expansion, &mut || None).map_err(|error| match error {
        SubstitutionError::NegativeLimit => "negative Markdown substitution expansion limit",
        SubstitutionError::ExpansionLimit => "Markdown substitution expansion limit exceeded",
        SubstitutionError::ResultTooLarge => "Markdown substitution result is too large",
        SubstitutionError::Context(_) => unreachable!("background context"),
    })
}

/// Master's signed-limit API with cooperative cancellation/deadline checks.
/// The original typed cause is returned before any partial result is published.
pub fn replace_substitutions_bounded_with_context(
    source: &[u8],
    variables: &BTreeMap<String, String>,
    max_expansion: i64,
    check: &mut Check<'_>,
) -> Result<(String, u64), SubstitutionError> {
    if max_expansion < 0 {
        return Err(SubstitutionError::NegativeLimit);
    }
    replace_bounded(source, variables, max_expansion as u64, check)
}

fn replace_bounded(
    source: &[u8],
    variables: &BTreeMap<String, String>,
    max_expansion: u64,
    check: &mut Check<'_>,
) -> Result<(String, u64), SubstitutionError> {
    check_context(check)?;
    if variables.is_empty() || !source.windows(2).any(|window| window == b"${") {
        return Ok((String::from_utf8_lossy(source).into_owned(), 0));
    }
    let matcher = Matcher::new(variables, check)?;
    let (context, root) = super::document::parse_document(source);
    let source_bytes = Bytes::new(source);
    let source_view = source_bytes.bytes();
    let mut replacements = Vec::new();
    let mut remaining = max_expansion;
    collect_text_nodes(
        &context,
        root,
        &source_view,
        &matcher,
        &mut replacements,
        &mut remaining,
        check,
    )?;
    replacements.sort_by_key(|(segment, _)| std::cmp::Reverse(segment.start));
    let mut output = source.to_vec();
    for (segment, replacement) in replacements {
        let start = usize::try_from(segment.start).expect("negative Markdown segment start");
        let stop = usize::try_from(segment.stop).expect("negative Markdown segment stop");
        output.splice(start..stop, replacement.into_bytes());
    }
    Ok((
        String::from_utf8_lossy(&output).into_owned(),
        max_expansion - remaining,
    ))
}

#[cfg(test)]
mod tests {
    use super::{replace_substitutions, replace_substitutions_bounded};
    use std::collections::BTreeMap;

    fn variables(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn replaces_visible_text_and_preserves_markdown_source() {
        assert_eq!(
            replace_substitutions(
                b"**hello ${name}** [link](https://example.com)",
                &variables(&[("name", "world")]),
            ),
            "**hello world** [link](https://example.com)"
        );
    }

    #[test]
    fn skips_inline_and_fenced_code() {
        assert_eq!(
            replace_substitutions(
                b"`ignore ${name}` visible ${name}\n\n```\n${name}\n```",
                &variables(&[("name", "world")]),
            ),
            "`ignore ${name}` visible world\n\n```\n${name}\n```"
        );
    }

    #[test]
    fn skips_indented_code_at_end_of_source() {
        assert_eq!(
            replace_substitutions(
                b"    ignore ${name}\nvisible ${name}",
                &variables(&[("name", "world")])
            ),
            "    ignore ${name}\nvisible world"
        );
    }

    #[test]
    fn bounded_replacements_charge_candidates_and_utf8_bytes_before_output() {
        let vars = variables(&[("x", "é")]);
        assert!(replace_substitutions_bounded(b"${x}${x}", &vars, 5).is_err());
        assert_eq!(
            replace_substitutions_bounded(b"${x}${x}", &vars, 6).unwrap(),
            ("éé".into(), 6)
        );
        assert_eq!(
            replace_substitutions_bounded(b"`${x}`", &vars, 0).unwrap(),
            ("`${x}`".into(), 0)
        );
    }

    #[test]
    fn complete_patterns_choose_longest_original_match_without_cascading() {
        let vars = variables(&[("a", "short"), ("a}b", "${z}"), ("z", "expanded")]);
        assert_eq!(
            replace_substitutions_bounded(b"${a}b}", &vars, 6).unwrap(),
            ("${z}".into(), 6)
        );
        assert!(replace_substitutions_bounded(b"${a}b}", &vars, 5).is_err());
    }
}
