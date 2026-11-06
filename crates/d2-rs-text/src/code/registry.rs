// SPDX-License-Identifier: MIT
// Chroma v2.27.0 LexerRegistry.Register/Get/Match/PrioritisedLexers.
// Copyright 2017 Alec Thomas. Rust translation Copyright 2026 Nelson Chen.
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
    primary: Vec<String>,
    secondary: Vec<String>,
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
// Go path/filepath.Match with Unix separators (match.go: Match, scanChunk,
// matchChunk, getEsc), translated directly. Chroma's registry calls
// filepath.Match on every lookup, so no per-process glob compilation is
// needed. Chroma panics on ErrBadPattern; the pinned patterns are all well
// formed, and a malformed one is reported here as no match.
fn decode_rune(s: &[u8]) -> (char, usize) {
    // utf8.DecodeRuneInString: RuneError with width 1 on invalid input.
    let prefix = &s[..s.len().min(4)];
    let valid = match std::str::from_utf8(prefix) {
        Ok(text) => text,
        Err(error) => std::str::from_utf8(&prefix[..error.valid_up_to()]).unwrap_or_default(),
    };
    match valid.chars().next() {
        Some(c) => (c, c.len_utf8()),
        None => ('\u{fffd}', 1),
    }
}
fn scan_chunk(pattern: &[u8]) -> (bool, &[u8], &[u8]) {
    let mut pattern = pattern;
    let mut star = false;
    while pattern.first() == Some(&b'*') {
        pattern = &pattern[1..];
        star = true;
    }
    let mut in_range = false;
    let mut i = 0;
    while i < pattern.len() {
        match pattern[i] {
            b'\\' => {
                if i + 1 < pattern.len() {
                    i += 1;
                }
            }
            b'[' => in_range = true,
            b']' => in_range = false,
            b'*' if !in_range => break,
            _ => {}
        }
        i += 1;
    }
    (star, &pattern[..i], &pattern[i..])
}
fn get_esc(chunk: &[u8]) -> Option<(char, &[u8])> {
    let mut chunk = chunk;
    if chunk.is_empty() || chunk[0] == b'-' || chunk[0] == b']' {
        return None;
    }
    if chunk[0] == b'\\' {
        chunk = &chunk[1..];
        if chunk.is_empty() {
            return None;
        }
    }
    let (r, n) = decode_rune(chunk);
    if r == '\u{fffd}' && n == 1 {
        return None;
    }
    let rest = &chunk[n..];
    if rest.is_empty() {
        return None;
    }
    Some((r, rest))
}
/// `Ok(Some(rest))` when `chunk` matches the start of `s`, `Ok(None)` when
/// it does not, and `Err(())` for ErrBadPattern.
fn match_chunk<'a>(chunk: &[u8], s: &'a [u8]) -> Result<Option<&'a [u8]>, ()> {
    let mut chunk = chunk;
    let mut s = s;
    let mut failed = false;
    while !chunk.is_empty() {
        if !failed && s.is_empty() {
            failed = true;
        }
        match chunk[0] {
            b'[' => {
                let mut r = '\0';
                if !failed {
                    let (decoded, n) = decode_rune(s);
                    r = decoded;
                    s = &s[n..];
                }
                chunk = &chunk[1..];
                let mut negated = false;
                if chunk.first() == Some(&b'^') {
                    negated = true;
                    chunk = &chunk[1..];
                }
                let mut matched = false;
                let mut ranges = 0;
                loop {
                    if chunk.first() == Some(&b']') && ranges > 0 {
                        chunk = &chunk[1..];
                        break;
                    }
                    let (lo, rest) = get_esc(chunk).ok_or(())?;
                    chunk = rest;
                    let mut hi = lo;
                    if chunk[0] == b'-' {
                        let (high, rest) = get_esc(&chunk[1..]).ok_or(())?;
                        hi = high;
                        chunk = rest;
                    }
                    if lo <= r && r <= hi {
                        matched = true;
                    }
                    ranges += 1;
                }
                if matched == negated {
                    failed = true;
                }
            }
            b'?' => {
                if !failed {
                    if s[0] == b'/' {
                        failed = true;
                    }
                    let (_, n) = decode_rune(s);
                    s = &s[n..];
                }
                chunk = &chunk[1..];
            }
            first => {
                let mut literal = first;
                if first == b'\\' {
                    chunk = &chunk[1..];
                    if chunk.is_empty() {
                        return Err(());
                    }
                    literal = chunk[0];
                }
                if !failed {
                    if literal != s[0] {
                        failed = true;
                    }
                    s = &s[1..];
                }
                chunk = &chunk[1..];
            }
        }
    }
    Ok(if failed { None } else { Some(s) })
}
fn glob_match(pattern: &str, name: &str) -> bool {
    let mut pattern = pattern.as_bytes();
    let mut name = name.as_bytes();
    'pattern: while !pattern.is_empty() {
        let (star, chunk, rest) = scan_chunk(pattern);
        pattern = rest;
        if star && chunk.is_empty() {
            // Trailing * matches the rest of the name unless it has a /.
            return !name.contains(&b'/');
        }
        let result = match_chunk(chunk, name);
        if let Ok(Some(t)) = result {
            if t.is_empty() || !pattern.is_empty() {
                name = t;
                continue;
            }
        }
        if result.is_err() {
            return false;
        }
        if star {
            let mut i = 0;
            while i < name.len() && name[i] != b'/' {
                match match_chunk(chunk, &name[i + 1..]) {
                    Ok(Some(t)) => {
                        if pattern.is_empty() && !t.is_empty() {
                            i += 1;
                            continue;
                        }
                        name = t;
                        continue 'pattern;
                    }
                    Ok(None) => {}
                    Err(()) => return false,
                }
                i += 1;
            }
        }
        return false;
    }
    name.is_empty()
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
                primary: c.filenames.unwrap_or_default(),
                secondary: c.alias_filenames.unwrap_or_default(),
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
// registry.go MatchMimeType: exact MIME spelling and priority, including
// duplicate MIME entries in the pinned registry. Reproduced from its configs
// by scripts/vendor-chroma-lexers.py; no token output is stored in this table.
pub(super) fn match_mime_type(mime: &str) -> Option<&'static str> {
    static MIME_TYPES: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    let names = MIME_TYPES.get_or_init(|| {
        serde_json::from_str(include_str!("../../assets/chroma/mime-types.json")).unwrap()
    });
    get(names.get(mime)?)
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
                if glob_match(pattern, basename)
                    || SUFFIXES.iter().any(|suffix| {
                        basename
                            .strip_suffix(suffix)
                            .is_some_and(|base| glob_match(pattern, base))
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
    if let Some(i) = [
        r.names.get(name),
        r.aliases.get(name),
        r.names.get(&lower(name)),
        r.aliases.get(&lower(name)),
    ]
    .into_iter()
    .flatten()
    .next()
    {
        return Some(&r.entries[*i].name);
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
    #[test]
    fn glob_match_follows_go_filepath_match() {
        // Cases from Go path/filepath match_test.go (Unix rows).
        for (pattern, name, want) in [
            ("abc", "abc", true),
            ("*", "abc", true),
            ("*c", "abc", true),
            ("a*", "a", true),
            ("a*", "abc", true),
            ("a*", "ab/c", false),
            ("a*/b", "abc/b", true),
            ("a*/b", "a/c/b", false),
            ("a*b*c*d*e*/f", "axbxcxdxe/f", true),
            ("a*b*c*d*e*/f", "axbxcxdxexxx/f", true),
            ("a*b*c*d*e*/f", "axbxcxdxe/xxx/f", false),
            ("a*b*c*d*e*/f", "axbxcxdxexxx/fff", false),
            ("a*b?c*x", "abxbbxdbxebxczzx", true),
            ("a*b?c*x", "abxbbxdbxebxczzy", false),
            ("ab[c]", "abc", true),
            ("ab[b-d]", "abc", true),
            ("ab[e-g]", "abc", false),
            ("ab[^c]", "abc", false),
            ("ab[^b-d]", "abc", false),
            ("ab[^e-g]", "abc", true),
            ("a\\*b", "a*b", true),
            ("a\\*b", "ab", false),
            ("a?b", "a☺b", true),
            ("a[^a]b", "a☺b", true),
            ("a???b", "a☺b", false),
            ("a[^a][^a][^a]b", "a☺b", false),
            ("[a-ζ]*", "α", true),
            ("*[a-ζ]", "A", false),
            ("a?b", "a/b", false),
            ("a*b", "a/b", false),
            ("[\\]a]", "]", true),
            ("[\\-]", "-", true),
            ("[x\\-]", "x", true),
            ("[x\\-]", "-", true),
            ("[x\\-]", "z", false),
            ("[\\-x]", "x", true),
            ("[\\-x]", "a", false),
            ("[]a]", "]", false),
            ("[-]", "-", false),
            ("[x-]", "x", false),
            ("[a-b-c]", "a", false),
            ("[", "a", false),
            ("[^", "a", false),
            ("a[", "a", false),
            ("*x", "xxx", true),
            ("*.py", "filename.py", true),
            ("*.🔥", "a.🔥", true),
            ("Dockerfile", "Dockerfile", true),
        ] {
            assert_eq!(super::glob_match(pattern, name), want, "{pattern} {name}");
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn translated_lexer_bodies_resolve_original_names_extensions_and_filenames() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/code-registry-tokens-v227.json"
        ))
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
