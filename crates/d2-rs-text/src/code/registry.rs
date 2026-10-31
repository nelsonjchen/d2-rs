// SPDX-License-Identifier: MIT
// Chroma v2.27.0 LexerRegistry.Register/Get/Match/PrioritisedLexers.
// Copyright 2017 Alec Thomas. Rust translation Copyright 2026 Nelson Chen.
use regex::Regex;
use serde::Deserialize;
use std::{collections::BTreeMap, sync::OnceLock};
#[derive(Deserialize)]
struct Config {
    name: String,
    aliases: Option<Vec<String>>,
    filenames: Option<Vec<String>>,
    alias_filenames: Option<Vec<String>>,
    priority: f64,
}
struct Entry {
    name: String,
    primary: Vec<Regex>,
    secondary: Vec<Regex>,
    priority: f64,
}
struct Registry {
    entries: Vec<Entry>,
    names: BTreeMap<String, usize>,
    aliases: BTreeMap<String, usize>,
}
const SUFFIXES: [&str; 13] = [
    "~",
    ".bak",
    ".old",
    ".orig",
    ".dpkg-dist",
    ".dpkg-old",
    ".ucf-dist",
    ".ucf-new",
    ".ucf-old",
    ".rpmnew",
    ".rpmorig",
    ".rpmsave",
    ".in",
];
fn glob(pattern: &str) -> Regex {
    // Pinned filename patterns use Go filepath.Match Unix syntax. Compile
    // once; matching remains case-sensitive and excludes / for * and ?.
    let mut out = String::from(r"(?s)\A");
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => out.push_str("[^/]*"),
            '?' => out.push_str("[^/]"),
            '\\' => out.push_str(&regex::escape(
                &chars.next().expect("pinned glob escape").to_string(),
            )),
            '[' => {
                out.push('[');
                if chars.peek() == Some(&'^') {
                    out.push('^');
                    chars.next();
                }
                loop {
                    let c = chars.next().expect("pinned glob class");
                    out.push(c);
                    if c == ']' {
                        break;
                    }
                }
            }
            c => out.push_str(&regex::escape(&c.to_string())),
        }
    }
    out.push_str(r"\z");
    Regex::new(&out).expect("pinned filepath glob")
}
fn lower(s: &str) -> String {
    // Go strings.ToLower uses a simple one-rune mapping. Rust's full Unicode
    // mapping expands U+0130; its Go simple mapping is the single ASCII i.
    s.chars()
        .map(|c| {
            if c == '\u{130}' {
                'i'
            } else {
                c.to_lowercase().next().unwrap()
            }
        })
        .collect()
}
fn registry() -> &'static Registry {
    static VALUE: OnceLock<Registry> = OnceLock::new();
    VALUE.get_or_init(|| {
        let configs: Vec<Config> =
            serde_json::from_str(include_str!("../../assets/chroma/registry-config.json")).unwrap();
        let mut entries = Vec::new();
        let mut names = BTreeMap::new();
        let mut aliases = BTreeMap::new();
        for c in configs {
            let i = entries.len();
            names.insert(c.name.clone(), i);
            names.insert(lower(&c.name), i);
            for alias in c.aliases.unwrap_or_default() {
                aliases.insert(alias.clone(), i);
                aliases.insert(lower(&alias), i);
            }
            entries.push(Entry {
                name: c.name,
                primary: c
                    .filenames
                    .unwrap_or_default()
                    .iter()
                    .map(|s| glob(s))
                    .collect(),
                secondary: c
                    .alias_filenames
                    .unwrap_or_default()
                    .iter()
                    .map(|s| glob(s))
                    .collect(),
                priority: if c.priority == 0.0 { 1.0 } else { c.priority },
            });
        }
        Registry {
            entries,
            names,
            aliases,
        }
    })
}
fn prioritized(r: &Registry, candidates: &mut [usize]) -> Option<usize> {
    d2_rs_core::go_sort::sort_by(candidates, |a, b| {
        r.entries[*a].priority > r.entries[*b].priority
    });
    candidates.first().copied()
}
fn match_filename(r: &Registry, name: &str) -> Option<usize> {
    let trimmed = name.trim_end_matches('/');
    let basename = if trimmed.is_empty() {
        if name.is_empty() { "." } else { "/" }
    } else {
        trimmed.rsplit('/').next().unwrap()
    };
    for secondary in [false, true] {
        let mut matches = Vec::new();
        for (i, entry) in r.entries.iter().enumerate() {
            let patterns = if secondary {
                &entry.secondary
            } else {
                &entry.primary
            };
            for pattern in patterns {
                if pattern.is_match(basename)
                    || SUFFIXES.iter().any(|suffix| {
                        basename
                            .strip_suffix(suffix)
                            .is_some_and(|base| pattern.is_match(base))
                    })
                {
                    matches.push(i);
                }
            }
        }
        if !matches.is_empty() {
            return prioritized(r, &mut matches);
        }
    }
    None
}
pub(super) fn get(name: &str) -> Option<&'static str> {
    let r = registry();
    for found in [
        r.names.get(name),
        r.aliases.get(name),
        r.names.get(&lower(name)),
        r.aliases.get(&lower(name)),
    ] {
        if let Some(i) = found {
            return Some(&r.entries[*i].name);
        }
    }
    let mut candidates = Vec::new();
    if let Some(i) = match_filename(r, &format!("filename.{name}")) {
        candidates.push(i);
    }
    if let Some(i) = match_filename(r, name) {
        candidates.push(i);
    }
    prioritized(r, &mut candidates).map(|i| r.entries[i].name.as_str())
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn translated_lexer_bodies_resolve_original_names_extensions_and_filenames() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/code-registry-tokens-v227.json"))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 20);
        for row in rows.as_array().unwrap() {
            let tokens = super::super::tokenize(
                row["language"].as_str().unwrap(),
                row["source"].as_str().unwrap(),
            )
            .unwrap();
            let actual: Vec<_> = tokens
                .iter()
                .map(|t| serde_json::json!({"kind":t.kind,"value":t.value}))
                .collect();
            assert_eq!(
                serde_json::json!(actual),
                row["tokens"],
                "{}",
                row["language"]
            );
            for (name, dark) in [("github", false), ("catppuccin-mocha", true)] {
                let actual:Vec<_>=tokens.iter().map(|t|{let s=super::super::style(t.kind,dark);serde_json::json!({"kind":t.kind,"value":t.value,"color":s.color.unwrap_or_default(),"bold":s.bold==Some(true),"italic":s.italic==Some(true),"underline":s.underline==Some(true)})}).collect();
                assert_eq!(
                    serde_json::json!(actual),
                    row["styles"][name],
                    "{}",
                    row["language"]
                );
            }
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_original_registry_get_projection() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/code-registry-get-v227.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 5164);
        let mut mismatches = Vec::new();
        for row in rows.as_array().unwrap() {
            let input = row["input"].as_str().unwrap();
            let actual = get(input).unwrap_or_default();
            if actual != row["name"].as_str().unwrap() {
                mismatches.push((input.to_owned(), actual.to_owned(), row["name"].clone()));
            }
        }
        assert!(
            mismatches.is_empty(),
            "registry differences: {mismatches:?}"
        );
    }
}
