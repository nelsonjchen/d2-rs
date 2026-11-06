// Copyright (c) 2015-2021 Martin Hensel
// Copyright (c) 2026 luo-studio
// Copyright (c) 2026 erweixin (upstream RaTeX)
// SPDX-License-Identifier: Apache-2.0 AND MIT
// Source: mathjax-go v0.1.0 internal/mhchem/actions.go.
use super::{
    Error,
    engine::{Action, Buffer, error, run, slot},
    jsregexp::is_whitespace,
    patterns::{Token, match_pattern},
};
use regex::Regex;
use serde_json::{Value, json};
use std::sync::LazyLock;
static DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[1-9][0-9]*$").unwrap());
static CELSIUS_C: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"°C|\^oC|\^\{o\}C").unwrap());
static CELSIUS_F: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"°F|\^oF|\^\{o\}F").unwrap());
static HALF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([0-9]+|\$[a-z]\$|[a-z])/([0-9]+)(\$[a-z]\$|[a-z])?$").unwrap());
fn text(t: &Token) -> String {
    match t {
        Token::String(s) => s.clone(),
        Token::Array(a) => a.concat(),
    }
}
fn first(t: &Token) -> &str {
    match t {
        Token::String(s) => s,
        Token::Array(a) => a.first().map(String::as_str).unwrap_or(""),
    }
}
fn pair<'a>(t: &'a Token, message: &str) -> Result<(&'a str, &'a str), Error> {
    match t {
        Token::Array(a) if a.len() >= 2 => Ok((&a[0], &a[1])),
        _ => Err(error(message)),
    }
}
fn cat(s: &mut Option<String>, t: &Token) {
    s.get_or_insert_default().push_str(&text(t));
}
fn node(kind: &str) -> Value {
    json!({"type_":kind})
}
fn field(kind: &str, p: &str) -> Value {
    json!({"type_":kind,"p1":p})
}
fn kind_option<'a>(a: &'a Action, t: &'a Token) -> &'a str {
    let s = a.option.as_str().unwrap_or("");
    if s.is_empty() { first(t) } else { s }
}
pub(super) fn apply(
    machine: &str,
    b: &mut Buffer,
    t: &Token,
    a: &Action,
) -> Result<Vec<Value>, Error> {
    let k = a.kind.as_str();
    match (machine, k) {
        ("ce", "output") => return ce_output(b, &a.option),
        ("ce", "o after d") => {
            let mut out = if b.d.as_ref().is_some_and(|s| DIGITS.is_match(s)) {
                let d = b.d.take().unwrap();
                let mut out = ce_output(b, &Value::Null)?;
                out.push(node("tinySkip"));
                b.b = Some(d);
                out
            } else {
                ce_output(b, &Value::Null)?
            };
            cat(&mut b.o, t);
            return Ok(std::mem::take(&mut out));
        }
        ("ce", "d= kv") => {
            b.d = Some(text(t));
            b.d_type = Some("kv".into());
            return Ok(vec![]);
        }
        ("ce", "charge or bond") => {
            if b.begins_with_bond {
                let mut out = ce_output(b, &Value::Null)?;
                out.push(json!({"type_":"bond","kind_":"-"}));
                return Ok(out);
            }
            b.d = Some(text(t));
            return Ok(vec![]);
        }
        ("ce", "- after o/d") => return after_od(b, t, a.option.as_bool().unwrap_or(false)),
        ("ce", "a to o") => {
            b.o = b.a.take();
            return Ok(vec![]);
        }
        ("ce", "sb=true" | "sb=false") => {
            b.sb = k == "sb=true";
            return Ok(vec![]);
        }
        ("ce", "beginsWithBond=true" | "beginsWithBond=false") => {
            b.begins_with_bond = k == "beginsWithBond=true";
            return Ok(vec![]);
        }
        ("ce", "parenthesisLevel++" | "parenthesisLevel--") => {
            b.parenthesis_level += if k.ends_with("++") { 1 } else { -1 };
            return Ok(vec![]);
        }
        ("ce", "state of aggregation") | ("pq", "state of aggregation") => {
            return Ok(vec![
                json!({"type_":if machine=="ce"{"state of aggregation"}else{"state of aggregation subscript"},"p1":run(&text(t),"o")?}),
            ]);
        }
        ("ce", "comma") => {
            let raw = text(t);
            let trimmed = raw.trim_end_matches(is_whitespace);
            return Ok(vec![field(
                if trimmed != raw && b.parenthesis_level == 0 {
                    "comma enumeration L"
                } else {
                    "comma enumeration M"
                },
                trimmed,
            )]);
        }
        ("ce", "oxidation-output") => {
            let mut out = vec![json!("{")];
            out.extend(run(&text(t), "oxidation")?);
            out.push(json!("}"));
            return Ok(out);
        }
        ("ce", "frac-output" | "overset-output" | "underset-output" | "underbrace-output") => {
            let (x, y) = pair(
                t,
                if k == "frac-output" {
                    "frac-output"
                } else {
                    "two-arg output"
                },
            )?;
            let kind = if k == "frac-output" {
                "frac-ce"
            } else {
                k.strip_suffix("-output").unwrap()
            };
            return Ok(vec![
                json!({"type_":kind,"p1":run(x,"ce")?,"p2":run(y,"ce")?}),
            ]);
        }
        ("ce" | "pq" | "bd", "color-output") => {
            let (x, y) = pair(
                t,
                if machine == "ce" {
                    "color-output"
                } else {
                    "color bd/pq"
                },
            )?;
            return Ok(vec![
                json!({"type_":"color","color1":x,"color2":run(y,machine)?}),
            ]);
        }
        ("text", "output") => {
            let out = b
                .text
                .take()
                .map(|s| vec![field("text", &s)])
                .unwrap_or_default();
            if !out.is_empty() {
                b.clear_all();
            }
            return Ok(out);
        }
        ("oxidation", "roman-numeral") => return Ok(vec![field("roman numeral", first(t))]),
        ("tex-math" | "tex-math tight", "output") => {
            let out =
                b.o.take()
                    .map(|s| vec![field("tex-math", &s)])
                    .unwrap_or_default();
            if !out.is_empty() {
                b.clear_all();
            }
            return Ok(out);
        }
        ("tex-math tight", "tight operator") => {
            b.o.get_or_insert_default()
                .push_str(&format!("{{{}}}", first(t)));
            return Ok(vec![]);
        }
        ("9,9" | "pu-9,9", "comma") => return Ok(vec![node("commaDecimal")]),
        ("pu", "enumber") => return pu_number(t, true),
        ("pu", "number^") => return pu_number(t, false),
        ("pu", "space") => return Ok(vec![node("pu-space-1")]),
        ("pu", "output") => return pu_output(b),
        ("pu-2", "cdot") => return Ok(vec![node("tight cdot")]),
        ("pu-2", "^(-1)") => {
            b.rm.get_or_insert_default()
                .push_str(&format!("^{{{}}}", first(t)));
            return Ok(vec![]);
        }
        ("pu-2", "space") => return Ok(vec![node("pu-space-2")]),
        ("pu-2", "output") => return pu2_output(b),
        ("pu-9,9", "output-0" | "output-o") => return Ok(pu99(b, k == "output-0")),
        _ => {}
    }
    let append_slot = match k {
        "a=" => Some(&mut b.a),
        "b=" => Some(&mut b.b),
        "p=" => Some(&mut b.p),
        "o=" => Some(&mut b.o),
        "q=" => Some(&mut b.q),
        "d=" => Some(&mut b.d),
        "rm=" => Some(&mut b.rm),
        "text=" => Some(&mut b.text),
        _ => None,
    };
    if let Some(s) = append_slot {
        cat(s, t);
        return Ok(vec![]);
    }
    let set_slot = match k {
        "r=" => Some(&mut b.r),
        "rdt=" => Some(&mut b.rdt),
        "rqt=" => Some(&mut b.rqt),
        "rd=" => Some(&mut b.rd),
        "rq=" => Some(&mut b.rq),
        _ => None,
    };
    if let Some(s) = set_slot {
        *s = Some(text(t));
        return Ok(vec![]);
    }
    let opt = a.option.as_str().unwrap_or("");
    Ok(match k {
        "insert" => vec![node(opt)],
        "insert+p1" => vec![field(opt, first(t))],
        "insert+p1+p2" => {
            let (x, y) = pair(t, "insert+p1+p2")?;
            vec![json!({"type_":opt,"p1":x,"p2":y})]
        }
        "copy" => vec![json!(text(t))],
        "write" => vec![json!(opt)],
        "rm" => vec![field("rm", first(t))],
        "text" | "tex-math" | "tex-math tight" | "ce" | "pu" | "9,9" => run(&text(t), k)?,
        "{text}" => {
            let mut out = vec![json!("{")];
            out.extend(run(&text(t), "text")?);
            out.push(json!("}"));
            out
        }
        "bond" | "operator" => vec![json!({"type_":k,"kind_":kind_option(a,t)})],
        "color0-output" => vec![json!({"type_":"color0","color":first(t)})],
        "1/2" => half(t),
        _ => {
            return Err(Error {
                id: "MhchemBugA".into(),
                message: format!("mhchem bug A. Please report. ({k})"),
            });
        }
    })
}
fn after_od(b: &mut Buffer, t: &Token, after_d: bool) -> Result<Vec<Value>, Error> {
    let o = slot(&b.o);
    let c1 = match_pattern("orbital", o).ok().flatten();
    let c2 = match_pattern("one lowercase greek letter $", o)
        .ok()
        .flatten();
    let c3 = match_pattern("one lowercase latin letter $", o)
        .ok()
        .flatten();
    let c4 = match_pattern("$one lowercase latin letter$ $", o)
        .ok()
        .flatten();
    let hyphen = text(t) == "-"
        && (c1.as_ref().is_some_and(|h| h.remainder.is_empty())
            || c2.is_some()
            || c3.is_some()
            || c4.is_some());
    if hyphen
        && [&b.a, &b.b, &b.p, &b.d, &b.q]
            .into_iter()
            .all(|s| slot(s).is_empty())
        && c1.is_none()
        && c3.is_some()
    {
        b.o = Some(format!("${}$", slot(&b.o)));
    }
    if hyphen {
        let mut out = ce_output(b, &Value::Null)?;
        out.push(node("hyphen"));
        return Ok(out);
    }
    let d_only = match_pattern("digits", slot(&b.d))
        .ok()
        .flatten()
        .is_some_and(|h| h.remainder.is_empty());
    if after_d && d_only {
        cat(&mut b.d, t);
        ce_output(b, &Value::Null)
    } else {
        let mut out = ce_output(b, &Value::Null)?;
        out.push(json!({"type_":"bond","kind_":"-"}));
        Ok(out)
    }
}
fn ce_output(b: &mut Buffer, entity: &Value) -> Result<Vec<Value>, Error> {
    let entity = entity.as_i64();
    if slot(&b.r).is_empty() {
        if [&b.a, &b.b, &b.p, &b.o, &b.q, &b.d]
            .into_iter()
            .all(|s| slot(s).is_empty())
            && entity.is_none()
        {
            b.clear_soft();
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        if b.sb {
            out.push(node("entitySkip"));
        }
        let mut d_type = b.d_type.clone();
        if [&b.o, &b.q, &b.d, &b.b, &b.p]
            .into_iter()
            .all(|s| slot(s).is_empty())
            && entity != Some(2)
        {
            b.o = b.a.take();
        } else if [&b.o, &b.q, &b.d].into_iter().all(|s| slot(s).is_empty())
            && (!slot(&b.b).is_empty() || !slot(&b.p).is_empty())
        {
            b.o = b.a.take();
            b.d = b.b.take();
            b.q = b.p.take();
        } else if !slot(&b.o).is_empty() && d_type.as_deref() == Some("kv") {
            if match_pattern("d-oxidation$", slot(&b.d))
                .ok()
                .flatten()
                .is_some()
            {
                d_type = Some("oxidation".into());
            } else if slot(&b.q).is_empty() {
                d_type = None;
            }
        }
        let mut chem = json!({"type_":"chemfive","a":run(slot(&b.a),"a")?,"b":run(slot(&b.b),"bd")?,"p":run(slot(&b.p),"pq")?,"o":run(slot(&b.o),"o")?,"q":run(slot(&b.q),"pq")?,"d":run(slot(&b.d),if d_type.as_deref()==Some("oxidation"){"oxidation"}else{"bd"})?});
        if let Some(kind) = d_type {
            chem["dType"] = json!(kind);
        }
        out.push(chem);
        b.clear_soft();
        return Ok(out);
    }
    fn label(kind: &str, source: &str) -> Result<Vec<Value>, Error> {
        match kind {
            "M" => run(source, "tex-math"),
            "T" => Ok(vec![field("text", source)]),
            _ => run(source, "ce"),
        }
    }
    let arrow = json!({"type_":"arrow","r":slot(&b.r),"rd":label(slot(&b.rdt),slot(&b.rd))?,"rq":label(slot(&b.rqt),slot(&b.rq))?});
    b.clear_soft();
    Ok(vec![arrow])
}
fn pu_number(t: &Token, scientific: bool) -> Result<Vec<Value>, Error> {
    let Token::Array(parts) = t else {
        return Ok(vec![]);
    };
    let get = |i| parts.get(i).map(String::as_str).unwrap_or("");
    let mut out = Vec::new();
    match get(0) {
        "+-" | "+/-" => out.push(json!(r"\pm ")),
        "" => {}
        s => out.push(json!(s)),
    }
    if scientific {
        if !get(1).is_empty() {
            out.extend(run(get(1), "pu-9,9")?);
            if !get(2).is_empty() {
                if get(2).contains([',', '.']) {
                    out.extend(run(get(2), "pu-9,9")?);
                } else {
                    out.push(json!(get(2)));
                }
            }
            let star = get(4).trim_matches(go_space);
            let e = get(3).trim_matches(go_space);
            let mult = if star.is_empty() { e } else { star };
            if !mult.is_empty() {
                out.push(node(if mult == "e" || mult.starts_with('*') {
                    "cdot"
                } else {
                    "times"
                }));
            }
        }
        if !get(5).is_empty() {
            out.push(json!(format!("10^{{{}}}", get(5))));
        }
    } else {
        if !get(1).is_empty() || parts.len() > 1 {
            out.extend(run(get(1), "pu-9,9")?);
        }
        if !get(2).is_empty() || parts.len() > 2 {
            out.push(json!(format!("^{{{}}}", get(2))));
        }
    }
    Ok(out)
}
// strings.TrimSpace follows Go's Unicode whitespace, unlike ECMAScript \s.
fn go_space(c: char) -> bool {
    matches!(c,'\u{9}'..='\u{D}'|' '| '\u{85}'|'\u{A0}'|'\u{1680}'|'\u{2000}'..='\u{200A}'|'\u{2028}'|'\u{2029}'|'\u{202F}'|'\u{205F}'|'\u{3000}')
}
fn unwrap_group(source: &str) -> String {
    if let Ok(Some(h)) = match_pattern("{(...)}", source) {
        if h.remainder.is_empty() {
            if let Token::String(s) = h.token {
                return s;
            }
        }
    }
    source.into()
}
fn pu_output(b: &mut Buffer) -> Result<Vec<Value>, Error> {
    let replace = |s: &str| {
        let c = CELSIUS_C.replace_all(s, "{}^{\\circ}C");
        CELSIUS_F.replace_all(&c, "{}^{\\circ}F").into_owned()
    };
    let d = replace(&unwrap_group(slot(&b.d)));
    let q = replace(&unwrap_group(slot(&b.q)));
    let out = if !q.is_empty() {
        let mut d_ast = run(&d, "pu")?;
        let q_ast = run(&q, "pu")?;
        if slot(&b.o) == "//" {
            vec![json!({"type_":"pu-frac","p1":d_ast,"p2":q_ast})]
        } else {
            d_ast.push(node(if d_ast.len() > 1 || q_ast.len() > 1 {
                " / "
            } else {
                "/"
            }));
            d_ast.extend(q_ast);
            d_ast
        }
    } else {
        run(&d, "pu-2")?
    };
    b.clear_all();
    Ok(out)
}
fn pu2_output(b: &mut Buffer) -> Result<Vec<Value>, Error> {
    let out = if let Some(rm) = b.rm.take() {
        if let Ok(Some(h)) = match_pattern("{(...)}", &rm) {
            if h.remainder.is_empty() {
                match h.token {
                    Token::String(s) => run(&s, "pu")?,
                    _ => vec![],
                }
            } else {
                vec![field("rm", &rm)]
            }
        } else {
            vec![field("rm", &rm)]
        }
    } else {
        vec![]
    };
    b.clear_all();
    Ok(out)
}
fn pu99(b: &mut Buffer, reverse: bool) -> Vec<Value> {
    let t = b.text.take().unwrap_or_default();
    let mut out = Vec::new();
    if t.len() > 4 {
        if reverse {
            let a = if t.len().is_multiple_of(3) {
                3
            } else {
                t.len() % 3
            };
            let mut i = t.len() - 3;
            while i > 0 {
                out.push(json!(&t[i..i + 3]));
                out.push(node("1000 separator"));
                if i < 3 {
                    break;
                }
                i -= 3;
            }
            out.push(json!(&t[..a]));
            out.reverse();
        } else {
            let a = t.len() - 3;
            let mut i = 0;
            while i < a {
                out.push(json!(&t[i..i + 3]));
                out.push(node("1000 separator"));
                i += 3;
            }
            out.push(json!(&t[i..]));
        }
    } else {
        out.push(json!(t));
    }
    b.clear_all();
    out
}
fn half(t: &Token) -> Vec<Value> {
    let mut s = text(t);
    let mut out = Vec::new();
    if s.starts_with(['+', '-']) {
        out.push(json!(&s[..1]));
        s = s[1..].into();
    }
    let Some(m) = HALF.captures(&s) else {
        return vec![];
    };
    out.push(json!({"type_":"frac","p1":m[1].replace('$',""),"p2":&m[2]}));
    if let Some(tail) = m.get(3).filter(|m| !m.is_empty()) {
        out.push(field("tex-math", &tail.as_str().replace('$', "")));
    }
    out
}
