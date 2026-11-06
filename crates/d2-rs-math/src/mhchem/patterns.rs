// Copyright (c) 2015-2021 Martin Hensel
// Copyright (c) 2026 luo-studio
// Copyright (c) 2026 erweixin (upstream RaTeX)
// SPDX-License-Identifier: Apache-2.0 AND MIT
// Source: mathjax-go v0.1.0 internal/mhchem/patterns.go.
use super::{Error, jsregexp::JsRegexp};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::LazyLock};
static REGEXES: LazyLock<HashMap<String, JsRegexp>> = LazyLock::new(|| {
    #[derive(Deserialize)]
    struct Table {
        regex: HashMap<String, String>,
    }
    let mut patterns = serde_json::from_str::<Table>(include_str!("data/patterns.json"))
        .unwrap()
        .regex;
    patterns.extend(
        serde_json::from_str::<HashMap<String, String>>(include_str!("data/inline-patterns.json"))
            .unwrap(),
    );
    patterns
        .into_iter()
        .map(|(k, v)| (k, JsRegexp::compile(&v).expect("pinned mhchem pattern")))
        .collect()
});
// Inline expressions are private helpers, not named transition patterns.
static TABLE_NAMES: LazyLock<std::collections::HashSet<String>> = LazyLock::new(|| {
    let table: serde_json::Value =
        serde_json::from_str(include_str!("data/patterns.json")).unwrap();
    table["regex"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect()
});
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(super) enum Token {
    String(String),
    Array(Vec<String>),
}
#[derive(Debug, PartialEq, Eq, Serialize)]
pub(super) struct Hit {
    pub token: Token,
    pub remainder: String,
}
#[derive(Clone, Copy)]
enum Anchor<'a> {
    Literal(&'a str),
    Regex(&'a str),
}
use Anchor::{Literal as L, Regex as R};
impl Anchor<'_> {
    fn prefix(self, input: &str) -> Option<usize> {
        match self {
            L(s) => input.starts_with(s).then_some(s.len()),
            R(name) => REGEXES[name]
                .find(input)
                .filter(|m| m.index == 0)
                .map(|m| m.length),
        }
    }
}
fn extra_close() -> Error {
    Error {
        id: "ExtraCloseMissingOpen".into(),
        message: "Extra close brace or missing open brace".into(),
    }
}
fn scan_end(input: &str, start: usize, end: Anchor<'_>) -> Result<Option<(usize, usize)>, Error> {
    let mut braces = 0;
    for (relative, c) in input[start..].char_indices() {
        let i = start + relative;
        // Empty literal is a missing anchor, rather than a zero-length match.
        let matched = match end {
            L("") => None,
            _ => end.prefix(&input[i..]),
        };
        if let Some(length) = matched {
            if braces == 0 {
                return Ok(Some((i, i + length)));
            }
        }
        if c == '{' {
            braces += 1;
        } else if c == '}' {
            if braces == 0 {
                return Err(extra_close());
            }
            braces -= 1;
        }
    }
    Ok(None)
}
#[derive(Clone, Copy)]
struct Part<'a> {
    exclude: &'a str,
    include: Anchor<'a>,
    end_include: &'a str,
    end_exclude: Anchor<'a>,
}
fn part<'a>(
    exclude: &'a str,
    include: Anchor<'a>,
    end_include: &'a str,
    end_exclude: Anchor<'a>,
) -> Part<'a> {
    Part {
        exclude,
        include,
        end_include,
        end_exclude,
    }
}
fn observe(
    input: &str,
    p: Part<'_>,
    second: Option<Part<'_>>,
    combine: bool,
) -> Result<Option<Hit>, Error> {
    let Some(ex) = L(p.exclude).prefix(input) else {
        return Ok(None);
    };
    let rest = &input[ex..];
    let Some(open) = p.include.prefix(rest) else {
        return Ok(None);
    };
    let end = if p.end_include.is_empty() {
        p.end_exclude
    } else {
        L(p.end_include)
    };
    let Some((start, end)) = scan_end(rest, open, end)? else {
        return Ok(None);
    };
    let m1 = &rest[..if p.end_include.is_empty() { start } else { end }];
    let after = &rest[end..];
    if let Some(p2) = second {
        let Some(hit) = observe(after, p2, None, false)? else {
            return Ok(None);
        };
        let Token::String(m2) = hit.token else {
            return Err(Error {
                id: "MhchemError".into(),
                message: "mhchem: nested Fog pair".into(),
            });
        };
        let token = if combine {
            Token::String(format!("{m1}{m2}"))
        } else {
            Token::Array(vec![m1.into(), m2])
        };
        return Ok(Some(Hit {
            token,
            remainder: hit.remainder,
        }));
    }
    Ok(Some(Hit {
        token: Token::String(m1.into()),
        remainder: after.into(),
    }))
}
fn simple(
    input: &str,
    exclude: &str,
    include: Anchor<'_>,
    end_include: &str,
    end_exclude: Anchor<'_>,
) -> Result<Option<Hit>, Error> {
    observe(
        input,
        part(exclude, include, end_include, end_exclude),
        None,
        false,
    )
}
fn whole(name: &str, input: &str) -> Option<Hit> {
    let m = REGEXES[name].find(input)?;
    Some(Hit {
        token: Token::String(m.text),
        remainder: input[m.index + m.length..].into(),
    })
}
pub(super) fn match_pattern(name: &str, input: &str) -> Result<Option<Hit>, Error> {
    match name {
        "(-)(9)^(-9)" | "(-)(9.,9)(e)(99)" => {
            let key = if name == "(-)(9)^(-9)" {
                "reNegPow"
            } else {
                "reSci"
            };
            let Some(m) = REGEXES[key].find(input) else {
                return Ok(None);
            };
            if key == "reSci" && m.text.is_empty() {
                return Ok(None);
            }
            Ok(Some(Hit {
                token: Token::Array(m.groups.into_iter().skip(1).collect()),
                remainder: input[m.index + m.length..].into(),
            }))
        }
        "state of aggregation $" => {
            if let Some(hit) = simple(input, "", R("reAggOpen"), ")", L(""))? {
                if R("reSoaRemainder").prefix(&hit.remainder).is_some() {
                    return Ok(Some(hit));
                }
            }
            Ok(whole("reSoaAlt", input))
        }
        "amount" | "amount2" => {
            if let Some(hit) = whole("reAmountMain", input) {
                return Ok(Some(hit));
            }
            let Some(hit) = simple(input, "", L("$"), "$", L(""))? else {
                return Ok(None);
            };
            match &hit.token {
                Token::String(s) if REGEXES["reAmountDollar"].find(s).is_some() => Ok(Some(hit)),
                _ => Ok(None),
            }
        }
        "formula$" => {
            if REGEXES["reFormulaParenOnly"].find(input).is_some() {
                Ok(None)
            } else {
                Ok(whole("reFormulaMain", input))
            }
        }
        "^{(...)}" => simple(input, "^{", L(""), "", L("}")),
        "^($...$)" => simple(input, "^", L("$"), "$", L("")),
        r"^\x{}{}" | r"_\x{}{}" | r"\x{}{}" => observe(
            input,
            part(
                if name.starts_with('^') {
                    "^"
                } else if name.starts_with('_') {
                    "_"
                } else {
                    ""
                },
                R("reCmdBrace"),
                "}",
                L(""),
            ),
            Some(part("", L("{"), "}", L(""))),
            true,
        ),
        r"^\x{}" | r"_\x{}" | r"\x{}" => simple(
            input,
            if name.starts_with('^') {
                "^"
            } else if name.starts_with('_') {
                "_"
            } else {
                ""
            },
            R("reCmdBrace"),
            "}",
            L(""),
        ),
        "_{(...)}" => simple(input, "_{", L(""), "", L("}")),
        "_($...$)" => simple(input, "_", L("$"), "$", L("")),
        "{...}" => simple(input, "", L("{"), "}", L("")),
        "{(...)}" => simple(input, "{", L(""), "", L("}")),
        "$...$" => simple(input, "", L("$"), "$", L("")),
        "${(...)}$__$(...)$" => {
            let hit = simple(input, "${", L(""), "", L("}$"))?;
            if hit.is_some() {
                Ok(hit)
            } else {
                simple(input, "$", L(""), "", L("$"))
            }
        }
        r"\bond{(...)}" => simple(input, r"\bond{", L(""), "", L("}")),
        "[(...)]" => simple(input, "[", L(""), "", L("]")),
        r"\frac{(...)}" | r"\overset{(...)}" | r"\underset{(...)}" | r"\underbrace{(...)}" => {
            let exclude = name.split("(...)").next().unwrap();
            observe(
                input,
                part(
                    exclude,
                    L(""),
                    "",
                    L(if name == r"\underbrace{(...)}" {
                        "}_"
                    } else {
                        "}"
                    }),
                ),
                Some(part("{", L(""), "", L("}"))),
                false,
            )
        }
        r"\color{(...)}" => simple(input, r"\color{", L(""), "", L("}")),
        r"\color{(...)}{(...)}" => {
            let hit = observe(
                input,
                part(r"\color{", L(""), "", L("}")),
                Some(part("{", L(""), "", L("}"))),
                false,
            )?;
            if hit.is_some() {
                Ok(hit)
            } else {
                observe(
                    input,
                    part(r"\color", L(r"\"), "", R("reBeforeBrace")),
                    Some(part("{", L(""), "", L("}"))),
                    false,
                )
            }
        }
        r"\ce{(...)}" => simple(input, r"\ce{", L(""), "", L("}")),
        r"\pu{(...)}" => simple(input, r"\pu{", L(""), "", L("}")),
        _ => {
            let re = REGEXES
                .get(name)
                .filter(|_| TABLE_NAMES.contains(name))
                .ok_or_else(|| Error {
                    id: "MhchemBugP".into(),
                    message: format!("mhchem bug P. Please report. ({name})"),
                })?;
            let Some(m) = re.find(input) else {
                return Ok(None);
            };
            let token = if m.groups.len() > 2 {
                Token::Array(m.groups.into_iter().skip(1).collect())
            } else if m.groups.len() > 1 && !m.groups[1].is_empty() {
                Token::String(m.groups[1].clone())
            } else {
                Token::String(m.text)
            };
            Ok(Some(Hit {
                token,
                remainder: input[m.index + m.length..].into(),
            }))
        }
    }
}
#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[test]
    fn original_pattern_tokens_remainders_and_errors() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/mhchem-pattern-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 25688);
        for row in rows {
            let actual = match_pattern(
                row["name"].as_str().unwrap(),
                row["input"].as_str().unwrap(),
            );
            match actual {
                Ok(hit) => {
                    assert_eq!(row["error"], serde_json::Value::Null);
                    assert_eq!(
                        serde_json::to_value(hit).unwrap(),
                        row["hit"],
                        "{} {:?}",
                        row["name"],
                        row["input"]
                    );
                }
                Err(e) => {
                    assert_eq!(
                        e.id,
                        row["error"]["id"].as_str().unwrap(),
                        "{} {:?}",
                        row["name"],
                        row["input"]
                    );
                    assert_eq!(e.message, row["error"]["message"].as_str().unwrap());
                }
            }
        }
    }
}
