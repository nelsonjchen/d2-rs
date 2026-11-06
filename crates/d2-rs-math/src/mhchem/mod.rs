// Copyright (c) 2015-2021 Martin Hensel
// Copyright (c) 2026 luo-studio
// Copyright (c) 2026 erweixin (upstream RaTeX)
// SPDX-License-Identifier: Apache-2.0 AND MIT
// Translated from pinned mathjax-go v0.1.0 internal/mhchem/texify.go.
// Native expansion inserted into the owning TeX parser; wider host gaps tracked.
mod actions;
mod engine;
pub(super) fn to_tex(input: &str, mode: &str) -> Result<String, Error> {
    engine::to_tex(input, mode)
}
mod jsregexp;
mod patterns;

use serde_json::Value;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Error {
    pub id: String,
    pub message: String,
}
fn bug() -> Error {
    Error {
        id: "MhchemBugT".into(),
        message: "mhchem bug T. Please report.".into(),
    }
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
fn inner(v: &Value, key: &str) -> Result<String, Error> {
    match v.get(key).and_then(Value::as_array) {
        Some(a) => texify(a, false),
        None => Ok(String::new()),
    }
}
pub(super) fn texify(input: &[Value], outer: bool) -> Result<String, Error> {
    let mut output = String::new();
    let mut escape = false;
    for value in input {
        if let Some(s) = value.as_str() {
            output.push_str(s);
            continue;
        }
        escape |= text(value, "type_") == "1st-level escape";
        output.push_str(&node(value)?);
    }
    if outer && !escape && !output.is_empty() {
        output = format!("{{{output}}}");
    }
    Ok(output)
}
fn node(v: &Value) -> Result<String, Error> {
    let p = text(v, "p1");
    let kind = text(v, "type_");
    let output = match kind {
        "chemfive" => return chemfive(v),
        "rm" | "roman numeral" => format!(r"\mathrm{{{p}}}"),
        "text" => {
            if p.contains(['^', '_']) {
                format!(
                    r"\mathrm{{{}}}",
                    p.replacen(' ', "~", 1).replacen('-', r"\text{-}", 1)
                )
            } else {
                format!(r"\text{{{p}}}")
            }
        }
        "state of aggregation" => format!("\\mskip2mu {}", inner(v, "p1")?),
        "state of aggregation subscript" => format!("\\mskip1mu {}", inner(v, "p1")?),
        "bond" => match bond(text(v, "kind_")) {
            Some(s) => s.into(),
            None => {
                return Err(Error {
                    id: "MhchemErrorBond".into(),
                    message: format!("mhchem Error. Unknown bond type ({})", text(v, "kind_")),
                });
            }
        },
        "frac" | "pu-frac" => {
            let (a, b) = if kind == "frac" {
                (p.to_owned(), text(v, "p2").to_owned())
            } else {
                (inner(v, "p1")?, inner(v, "p2")?)
            };
            let fraction = format!(r"\frac{{{a}}}{{{b}}}");
            format!(
                r"\mathchoice{{\textstyle{fraction}}}{{{fraction}}}{{{fraction}}}{{{fraction}}}"
            )
        }
        "tex-math" | "1st-level escape" => format!("{p} "),
        "frac-ce" | "overset" | "underset" => {
            let command = if kind == "frac-ce" { "frac" } else { kind };
            format!("\\{command}{{{}}}{{{}}}", inner(v, "p1")?, inner(v, "p2")?)
        }
        "underbrace" => format!(
            r"\underbrace{{{}}}_{{{}}}",
            inner(v, "p1")?,
            inner(v, "p2")?
        ),
        "color" => format!(
            r"{{\color{{{}}}{{{}}}}}",
            text(v, "color1"),
            inner(v, "color2")?
        ),
        "color0" => format!(r"\color{{{}}}", text(v, "color")),
        "arrow" => return arrow(v),
        "operator" => operator(text(v, "kind_")).ok_or_else(bug)?.into(),
        "space" => " ".into(),
        "tinySkip" => r"\mkern2mu".into(),
        "entitySkip" | "pu-space-1" => "~".into(),
        "pu-space-2" => "\\mkern3mu ".into(),
        "1000 separator" => "\\mkern2mu ".into(),
        "commaDecimal" => "{,}".into(),
        "comma enumeration L" | "comma enumeration M" | "comma enumeration S" => {
            let n = match kind {
                "comma enumeration L" => 6,
                "comma enumeration M" => 3,
                _ => 1,
            };
            format!("{{{p}}}\\mkern{n}mu ")
        }
        "hyphen" => r"\text{-}".into(),
        "addition compound" => r"\,{\cdot}\,".into(),
        "electron dot" => "\\mkern1mu \\bullet\\mkern1mu ".into(),
        "KV x" => r"{\times}".into(),
        "prime" => "\\prime ".into(),
        "cdot" => "\\cdot ".into(),
        "tight cdot" => "\\mkern1mu{\\cdot}\\mkern1mu ".into(),
        "times" => "\\times ".into(),
        "circa" => r"{\sim}".into(),
        "^" => "uparrow".into(),
        "v" => "downarrow".into(),
        "ellipsis" => "\\ldots ".into(),
        "/" => "/".into(),
        " / " => r"\,/\,".into(),
        _ => return Err(bug()),
    };
    Ok(output)
}
fn chemfive(v: &Value) -> Result<String, Error> {
    let a = inner(v, "a")?;
    let b = inner(v, "b")?;
    let p = inner(v, "p")?;
    let o = inner(v, "o")?;
    let q = inner(v, "q")?;
    let d = inner(v, "d")?;
    let signed = |s: &str| {
        if s.starts_with(['+', '-']) {
            format!("{{{s}}}")
        } else {
            s.to_owned()
        }
    };
    let mut output = String::new();
    if !a.is_empty() {
        output.push_str(&signed(&a));
        output.push_str(r"\,");
    }
    if !b.is_empty() || !p.is_empty() {
        output.push_str(r"{\vphantom{A}}");
        output.push_str(&format!(r"^{{\hphantom{{{b}}}}}_{{\hphantom{{{p}}}}}"));
        output.push_str(r"\mkern-1.5mu{\vphantom{A}}");
        output.push_str(&format!(r"^{{\smash[t]{{\vphantom{{2}}}}\llap{{{b}}}}}"));
        output.push_str(&format!(r"_{{\vphantom{{2}}\llap{{\smash[t]{{{p}}}}}}}"));
    }
    if !o.is_empty() {
        output.push_str(&signed(&o));
    }
    match text(v, "dType") {
        "kv" => {
            if !d.is_empty() || !q.is_empty() {
                output.push_str(r"{\vphantom{A}}");
            }
            if !d.is_empty() {
                output.push_str(&format!("^{{{d}}}"));
            }
            if !q.is_empty() {
                output.push_str(&format!(r"_{{\smash[t]{{{q}}}}}"));
            }
        }
        "oxidation" => {
            if !d.is_empty() {
                output.push_str(&format!(r"{{\vphantom{{A}}}}^{{{d}}}"));
            }
            if !q.is_empty() {
                output.push_str(&format!(r"{{\vphantom{{A}}}}_{{\smash[t]{{{q}}}}}"));
            }
        }
        _ => {
            if !q.is_empty() {
                output.push_str(&format!(r"{{\vphantom{{A}}}}_{{\smash[t]{{{q}}}}}"));
            }
            if !d.is_empty() {
                output.push_str(&format!(r"{{\vphantom{{A}}}}^{{{d}}}"));
            }
        }
    }
    Ok(output)
}
fn arrow(v: &Value) -> Result<String, Error> {
    let rd = inner(v, "rd")?;
    let rq = inner(v, "rq")?;
    let r = text(v, "r");
    let mut a = match r {
        "->" | "→" | "⟶" => "rightarrow",
        "<-" => "leftarrow",
        "<->" => "leftrightarrow",
        "<-->" => "leftrightarrows",
        "<=>" | "⇌" => "rightleftharpoons",
        "<=>>" => "Rightleftharpoons",
        "<<=>" => "Leftrightharpoons",
        _ => return Err(bug()),
    }
    .to_owned();
    if !rd.is_empty() || !rq.is_empty() {
        if matches!(r, "<=>" | "<=>>" | "<<=>" | "<-->") {
            a = format!(r"\long{a}");
            if !rd.is_empty() {
                a = format!(r"\overset{{{rd}}}{{{a}}}");
            }
            if !rq.is_empty() {
                let n = if r == "<-->" { 2 } else { 6 };
                a = format!(r"\underset{{\lower{n}mu{{{rq}}}}}{{{a}}}");
            }
            a = format!(r" {{}}\mathrel{{{a}}}{{}} ");
        } else {
            if !rq.is_empty() {
                a.push_str(&format!("[{{{rq}}}]"));
            }
            a.push_str(&format!("{{{rd}}}"));
            a = format!(r" {{}}\mathrel{{\x{a}}}{{}} ");
        }
    } else {
        a = format!(r" {{}}\mathrel{{\long{a}}}{{}} ");
    }
    Ok(a)
}
fn bond(k: &str) -> Option<&'static str> {
    Some(match k {
        "-" | "1" => "{-}",
        "=" | "2" => "{=}",
        "#" | "3" => r"{\equiv}",
        "~" => r"{\tripledash}",
        "~-" => r"{\rlap{\lower.1em{-}}\raise.1em{\tripledash}}",
        "~=" | "~--" => r"{\rlap{\lower.2em{-}}\rlap{\raise.2em{\tripledash}}-}",
        "-~-" => r"{\rlap{\lower.2em{-}}\rlap{\raise.2em{-}}\tripledash}",
        "..." => r"{{\cdot}{\cdot}{\cdot}}",
        "...." => r"{{\cdot}{\cdot}{\cdot}{\cdot}}",
        "->" => r"{\rightarrow}",
        "<-" => r"{\leftarrow}",
        "<" => "{<}",
        ">" => "{>}",
        _ => return None,
    })
}
fn operator(k: &str) -> Option<&'static str> {
    Some(match k {
        "+" => " {}+{} ",
        "-" => " {}-{} ",
        "=" => " {}={} ",
        "<" => " {}<{} ",
        ">" => " {}>{} ",
        "<<" => r" {}\ll{} ",
        ">>" => r" {}\gg{} ",
        r"\pm" => r" {}\pm{} ",
        r"\approx" | r"$\approx$" => r" {}\approx{} ",
        "v" | "(v)" => r" \downarrow{} ",
        "^" | "(^)" => r" \uparrow{} ",
        _ => return None,
    })
}
#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[test]
    fn complete_original_texifier_nodes_and_errors() {
        let rows: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/mhchem-texifier-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 718);
        for row in rows {
            let actual = texify(
                row["values"].as_array().unwrap(),
                row["outer"].as_bool().unwrap(),
            );
            match actual {
                Ok(value) => {
                    assert_eq!(row["error"], Value::Null);
                    assert_eq!(value, row["tex"].as_str().unwrap(), "{}", row["name"]);
                }
                Err(error) => {
                    assert_eq!(
                        error.id,
                        row["error"]["id"].as_str().unwrap(),
                        "{}",
                        row["name"]
                    );
                    assert_eq!(
                        error.message,
                        row["error"]["message"].as_str().unwrap(),
                        "{}",
                        row["name"]
                    );
                }
            }
        }
    }
}
