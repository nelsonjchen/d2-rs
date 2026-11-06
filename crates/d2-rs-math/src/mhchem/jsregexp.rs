// Copyright (c) 2015-2021 Martin Hensel
// Copyright (c) 2026 luo-studio
// Copyright (c) 2026 erweixin (upstream RaTeX)
// SPDX-License-Identifier: Apache-2.0 AND MIT
// Source: mathjax-go v0.1.0 internal/mhchem/jsregexp.go.
use regex::Regex;
use std::sync::LazyLock;

const WS: &str = r"\x{0009}-\x{000D}\x{0020}\x{00A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";
const GREEK: &[&str] = &[
    "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
    "lambda", "mu", "nu", "xi", "omicron", "pi", "rho", "sigma", "tau", "upsilon", "phi", "chi",
    "psi", "omega", "Gamma", "Delta", "Theta", "Lambda", "Xi", "Pi", "Sigma", "Upsilon", "Phi",
    "Psi", "Omega",
];
static AMOUNT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\([+-]?[0-9]+/[0-9]+\)|[+-]?([0-9]+|\$[a-z]\$|[a-z])/[0-9]+|[+-]?[0-9]+[.,][0-9]+|[+-]?\.[0-9]+|[+-]?[0-9]+)").unwrap()
});
static PAREN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\([a-z]+\)").unwrap());

pub(super) fn is_whitespace(c: char) -> bool {
    matches!(c, '\u{0009}'..='\u{000D}' | '\u{0020}' | '\u{00A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}')
}
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct Match {
    pub index: usize,
    pub length: usize,
    pub text: String,
    pub groups: Vec<String>,
}
pub(super) struct JsRegexp {
    re: Option<Regex>,
    custom: Option<Custom>,
}
#[derive(Clone, Copy)]
enum Custom {
    Any,
    Aggregation,
    Brace,
    Amount,
    Kv,
    OrbitalDash,
    EndDash,
    DigitDash,
    Ellipsis,
    Cmt,
    Ca,
    Greek,
    Script,
    Arrow,
    Letters,
    LowerGreek,
    Operator,
    Orbital,
    SpaceA,
    Entities,
    EmptyScript,
}
impl JsRegexp {
    pub fn compile(source: &str) -> Result<Self, String> {
        let custom = custom_kind(source);
        let re = if custom.is_none() {
            Some(Regex::new(&lower(source)?).map_err(|e| e.to_string())?)
        } else {
            None
        };
        Ok(Self { re, custom })
    }
    pub fn find(&self, input: &str) -> Option<Match> {
        if let Some(kind) = self.custom {
            let end = custom_match(kind, input)?;
            let text = input[..end].to_owned();
            return Some(Match {
                index: 0,
                length: end,
                groups: vec![text.clone()],
                text,
            });
        }
        let captures = self.re.as_ref()?.captures(input)?;
        let whole = captures.get(0)?;
        Some(Match {
            index: whole.start(),
            length: whole.len(),
            text: whole.as_str().into(),
            groups: captures
                .iter()
                .map(|g| g.map_or("", |m| m.as_str()).into())
                .collect(),
        })
    }
}
// Rust supports non-capturing groups directly. Unlike Go's lowering this needs
// no extra capture map, but exposes exactly the same original captures.
fn lower(source: &str) -> Result<String, String> {
    let mut output = String::new();
    let mut chars = source.chars().peekable();
    let mut in_class = false;
    while let Some(c) = chars.next() {
        match c {
            '[' => {
                in_class = true;
                output.push(c);
            }
            ']' if in_class => {
                in_class = false;
                output.push(c);
            }
            '\\' => {
                let next = chars.next().ok_or("dangling escape")?;
                if next == 'u' {
                    let mut probe = chars.clone();
                    let digits: String = probe.by_ref().take(4).collect();
                    if digits.len() == 4 && u16::from_str_radix(&digits, 16).is_ok() {
                        output.push_str(&format!(r"\x{{{digits}}}"));
                        chars = probe;
                        continue;
                    }
                }
                match next {
                    's' => {
                        if !in_class {
                            output.push('[');
                        }
                        output.push_str(WS);
                        if !in_class {
                            output.push(']');
                        }
                    }
                    // Go RE2's \d and \w are ASCII, while Rust defaults to Unicode.
                    'd' => output.push_str(if in_class { "0-9" } else { "[0-9]" }),
                    'w' => output.push_str(if in_class {
                        "A-Za-z0-9_"
                    } else {
                        "[A-Za-z0-9_]"
                    }),
                    '/' | ',' | '_' | ' ' | ';' | ':' => output.push(next),
                    '-' => output.push_str(r"\x{2d}"),
                    _ => {
                        output.push('\\');
                        output.push(next);
                    }
                }
            }
            '(' if !in_class && matches!(chars.peek(), Some('?')) => {
                let tail: String = chars.clone().take(2).collect();
                if tail == "?=" || tail == "?!" {
                    return Err(format!("unsupported look-ahead in {source:?}"));
                }
                output.push(c);
            }
            _ => output.push(c),
        }
    }
    Ok(output)
}
fn custom_kind(s: &str) -> Option<Custom> {
    Some(match s {
        r"^." => Custom::Any,
        r"^\([a-z]{1,3}(?=[\),])" => Custom::Aggregation,
        r"^(?=\{)" => Custom::Brace,
        _ if s.contains(r"[a-z](?=\s*[A-Z])") && s.contains(r"\+(?!\s)") => Custom::Amount,
        r"^(?:[A-Z][a-z]{0,2}|i)(?=,)" => Custom::Kv,
        _ if s.starts_with(r"^-(?=(?:[spd]|sp)") => Custom::OrbitalDash,
        _ if s.starts_with(r"^-(?=[\s_},;") && s.contains(r"\([a-z]+\))") => Custom::EndDash,
        r"^-(?=[0-9])" => Custom::DigitDash,
        r"^\.\.\.(?=$|[^.])" => Custom::Ellipsis,
        r"^[CMT](?=\[)" => Custom::Cmt,
        _ if s.starts_with(r"^\\ca(?:\s+|") => Custom::Ca,
        _ if s.starts_with(r"^\\(?:alpha|beta") && s.contains(r"(?![a-zA-Z])") => Custom::Greek,
        r"^(?:\^(?=_)|\_(?=\^)|[\^_]$)" => Custom::Script,
        _ if s.starts_with(r"^(?:v|\(v\)|\^|\(\^\))(?=") => Custom::Arrow,
        _ if s.starts_with(r"^(?:[a-zA-Z\u03B1-") => Custom::Letters,
        _ if s.starts_with(r"^(?:\$?[\u03B1-") && s.ends_with('$') => Custom::LowerGreek,
        _ if s.starts_with(r"^(?:\+|(?:[\-=<>]") => Custom::Operator,
        _ if s.starts_with(r"^(?:[0-9]{1,2}[spdfgh]") => Custom::Orbital,
        r"^\s(?=[A-Z\\$])" => Custom::SpaceA,
        _ if s.starts_with(r"^(?:pH|pOH|pC|pK|iPr|iBu)(?=") => Custom::Entities,
        r"^\{\}(?=\^)" => Custom::EmptyScript,
        _ => return None,
    })
}
fn skip_space(s: &str, mut i: usize) -> usize {
    while let Some(c) = s[i..].chars().next() {
        if !is_whitespace(c) {
            break;
        }
        i += c.len_utf8();
    }
    i
}
fn boundary(s: &str, end: usize) -> bool {
    s.as_bytes()
        .get(end)
        .is_none_or(|b| !b.is_ascii_alphabetic())
}
fn command(s: &str) -> Option<usize> {
    if !s.starts_with('\\') {
        return None;
    }
    for name in GREEK {
        let prefix = format!("\\{name}");
        if !s.starts_with(&prefix) {
            continue;
        }
        let i = prefix.len();
        if i == s.len() {
            return Some(i);
        }
        if s[i..].starts_with("{}") {
            return Some(i + 2);
        }
        if is_whitespace(s[i..].chars().next().unwrap()) {
            return Some(skip_space(s, i));
        }
        if boundary(s, i) {
            return Some(i);
        }
    }
    None
}
fn lower_greek(s: &str) -> bool {
    let mut i = usize::from(s.starts_with('$'));
    let Some(c) = s[i..].chars().next() else {
        return false;
    };
    if ('α'..='ω').contains(&c) {
        i += c.len_utf8();
    } else {
        let Some(name) = GREEK[..24]
            .iter()
            .find(|name| s[i..].starts_with(&format!("\\{name}")))
        else {
            return false;
        };
        i += name.len() + 1;
        i = skip_space(s, i);
    }
    if s[i..].starts_with('$') {
        i += 1;
    }
    if i == s.len() {
        return true;
    }
    if s[i..].starts_with("{}") {
        i += 2;
    } else {
        i = skip_space(s, i);
    }
    i == s.len()
}
fn upper_after_space(s: &str) -> bool {
    s[skip_space(s, 0)..].starts_with(|c: char| c.is_ascii_uppercase())
}
fn amount(s: &str) -> Option<usize> {
    if let Some(m) = AMOUNT.find(s) {
        let mut end = m.end();
        if s.as_bytes().get(end).is_some_and(u8::is_ascii_lowercase)
            && upper_after_space(&s[end + 1..])
        {
            end += 1;
        }
        return Some(end);
    }
    let i = usize::from(s.starts_with(['+', '-']));
    if s.as_bytes().get(i).is_some_and(u8::is_ascii_lowercase) && upper_after_space(&s[i + 1..]) {
        return Some(i + 1);
    }
    if s.starts_with('+') && (s.len() == 1 || !is_whitespace(s[1..].chars().next().unwrap())) {
        return Some(1);
    }
    None
}
fn custom_match(kind: Custom, s: &str) -> Option<usize> {
    let b = s.as_bytes();
    match kind {
        Custom::Any => s
            .chars()
            .next()
            .filter(|c| !matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}'))
            .map(char::len_utf8),
        Custom::Aggregation => {
            if b.first() != Some(&b'(') {
                return None;
            }
            let mut i = 1;
            while i < s.len() && i <= 3 && b[i].is_ascii_lowercase() {
                i += 1;
            }
            (i > 1 && b.get(i).is_some_and(|c| matches!(c, b',' | b')'))).then_some(i)
        }
        Custom::Brace => s.starts_with('{').then_some(0),
        Custom::Amount => amount(s),
        Custom::Kv => {
            if s.starts_with("i,") {
                return Some(1);
            }
            if !b.first().is_some_and(u8::is_ascii_uppercase) {
                return None;
            }
            let mut i = 1;
            while i < s.len() && i < 3 && b[i].is_ascii_lowercase() {
                i += 1;
            }
            (b.get(i) == Some(&b',')).then_some(i)
        }
        Custom::OrbitalDash => {
            let rest = s.strip_prefix('-')?;
            for orbital in ["sp", "s", "p", "d"] {
                if let Some(tail) = rest.strip_prefix(orbital) {
                    if tail.is_empty()
                        || tail.starts_with(|c: char| is_whitespace(c) || ",;)]}".contains(c))
                    {
                        return Some(1);
                    }
                }
            }
            None
        }
        Custom::EndDash => {
            let rest = s.strip_prefix('-')?;
            (rest.is_empty()
                || rest.starts_with(|c: char| "_},;]/".contains(c) || is_whitespace(c))
                || PAREN.is_match(rest))
            .then_some(1)
        }
        Custom::DigitDash => {
            (s.starts_with('-') && b.get(1).is_some_and(u8::is_ascii_digit)).then_some(1)
        }
        Custom::Ellipsis => (s.starts_with("...") && b.get(3) != Some(&b'.')).then_some(3),
        Custom::Cmt => {
            (b.first().is_some_and(|c| b"CMT".contains(c)) && b.get(1) == Some(&b'[')).then_some(1)
        }
        Custom::Ca => {
            if !s.starts_with(r"\ca") {
                return None;
            }
            let i = skip_space(s, 3);
            (i > 3 || boundary(s, i)).then_some(i)
        }
        Custom::Greek => command(s),
        Custom::Script => {
            if s.starts_with("^_") || s == "^" || s.starts_with("_^") || s == "_" {
                Some(1)
            } else {
                None
            }
        }
        Custom::Arrow => {
            for op in ["(v)", "(^)", "v", "^"] {
                if let Some(tail) = s.strip_prefix(op) {
                    if tail.is_empty()
                        || tail.starts_with(|c: char| is_whitespace(c) || ",;)]}".contains(c))
                    {
                        return Some(op.len());
                    }
                }
            }
            None
        }
        Custom::Letters => {
            let mut i = 0;
            while let Some(c) = s[i..].chars().next() {
                if c.is_ascii_alphabetic()
                    || ('α'..='ω').contains(&c)
                    || ('Α'..='Ω').contains(&c)
                    || matches!(c, '?' | '@')
                {
                    i += c.len_utf8();
                } else if let Some(n) = command(&s[i..]) {
                    i += n;
                } else {
                    break;
                }
            }
            (i > 0).then_some(i)
        }
        Custom::LowerGreek => lower_greek(s).then_some(s.len()),
        Custom::Operator => {
            if s.starts_with('+') {
                return Some(1);
            }
            for op in [r"$\approx$", r"\approx", "<<", ">>", "-", "=", "<", ">"] {
                if let Some(tail) = s.strip_prefix(op) {
                    if tail.is_empty()
                        || tail.starts_with(is_whitespace)
                        || tail.as_bytes().first().is_some_and(u8::is_ascii_digit)
                        || (tail.starts_with('-')
                            && tail.as_bytes().get(1).is_some_and(u8::is_ascii_digit))
                    {
                        return Some(op.len());
                    }
                }
            }
            None
        }
        Custom::Orbital => {
            let digits = b.iter().take(2).take_while(|c| c.is_ascii_digit()).count();
            if s[digits..].starts_with("sp") && boundary(s, digits + 2) {
                return Some(digits + 2);
            }
            if digits > 0
                && b.get(digits).is_some_and(|c| b"spdfgh".contains(c))
                && boundary(s, digits + 1)
            {
                return Some(digits + 1);
            }
            None
        }
        Custom::SpaceA => {
            let c = s.chars().next()?;
            let width = c.len_utf8();
            (is_whitespace(c)
                && b.get(width)
                    .is_some_and(|c| c.is_ascii_uppercase() || matches!(c, b'\\' | b'$')))
            .then_some(width)
        }
        Custom::Entities => ["pOH", "pH", "pC", "pK", "iPr", "iBu"]
            .into_iter()
            .find(|entity| s.starts_with(entity) && boundary(s, entity.len()))
            .map(str::len),
        Custom::EmptyScript => s.starts_with("{}^").then_some(2),
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[test]
    fn original_all_regex_matches_and_captures() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/mhchem-regexp-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 15984);
        for row in rows {
            let re = JsRegexp::compile(row["source"].as_str().unwrap()).unwrap();
            assert_eq!(
                serde_json::to_value(re.find(row["input"].as_str().unwrap())).unwrap(),
                row["match"],
                "pattern {} input {:?}",
                row["name"],
                row["input"]
            );
        }
    }
}
