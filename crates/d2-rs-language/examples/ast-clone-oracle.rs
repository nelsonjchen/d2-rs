// SPDX-License-Identifier: MPL-2.0
use d2_rs_core::{ast::*, ir::clone_ast};
use serde::Deserialize;
use serde_json::{Value as Json, json};
#[derive(Deserialize)]
pub struct Request {
    hex: String,
    #[serde(default)]
    path: String,
}
pub fn evaluate(request: Request) -> Json {
    let input = request
        .hex
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|h| u8::from_str_radix(std::str::from_utf8(h).unwrap(), 16).unwrap())
        .collect::<Vec<_>>();
    let result = d2_rs_language::parse(&request.path, &input, None);
    let src = Child::Map(Shared::new(result.map));
    let dst = clone_ast(&src);
    observations(&src, &dst)
}
pub fn observations(src: &Child, dst: &Child) -> Json {
    let before = json!(dst);
    let source_before = json!(src);
    let mut aliases = Vec::new();
    fn pair(a: &Child, b: &Child, out: &mut Vec<Json>) {
        let mut row = json!({"node":a.ptr_eq(b)});
        let scalar = ScalarBox::from_child(Some(a.clone()));
        if scalar.unbox().is_some() {
            row["source_scalar_bytes"] = json!(
                scalar
                    .scalar_bytes()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            );
            row["scalar_bytes"] = json!(
                ScalarBox::from_child(Some(b.clone()))
                    .scalar_bytes()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            );
        }
        let mut interpolations = None;
        match (a, b) {
            (Child::Unquoted(a), Child::Unquoted(b)) => {
                let a = a.read();
                let b = b.read();
                interpolations = Some((a.value.clone(), b.value.clone()));
                if let Some(p) = &a.pattern
                    && !p.is_empty()
                {
                    row["pattern"] = json!(
                        p.get(0)
                            .unwrap()
                            .ptr_eq(&b.pattern.as_ref().unwrap().get(0).unwrap())
                    );
                }
            }
            (Child::DoubleQuoted(a), Child::DoubleQuoted(b)) => {
                interpolations = Some((a.read().value.clone(), b.read().value.clone()))
            }
            (Child::Number(a), Child::Number(b)) => {
                if let Some(v) = &a.read().value {
                    row["number"] = json!(v.ptr_eq(b.read().value.as_ref().unwrap()));
                }
            }
            (Child::EdgeIndex(a), Child::EdgeIndex(b)) => {
                if let Some(v) = &a.read().int {
                    row["index"] = json!(v.ptr_eq(b.read().int.as_ref().unwrap()));
                }
            }
            _ => {}
        }
        if let Some((Some(a), Some(b))) = interpolations {
            row["parts"] = json!(
                a.iter()
                    .zip(b.iter())
                    .map(|(a, b)| {
                        let a = a.read();
                        let b = b.read();
                        let mut row = json!({});
                        if let Some(v) = &a.string {
                            row["string"] = json!(v.ptr_eq(b.string.as_ref().unwrap()));
                        }
                        if let Some(v) = &a.raw_string {
                            row["raw"] = json!(v.ptr_eq(b.raw_string.as_ref().unwrap()));
                        }
                        row
                    })
                    .collect::<Vec<_>>()
            );
        }
        out.push(row);
        let aa = a.children();
        let bb = b.children();
        assert_eq!(aa.len(), bb.len());
        for (a, b) in aa.iter().zip(&bb) {
            pair(a, b, out);
        }
    }
    fn mutate(n: &Child) {
        for c in n.children() {
            mutate(&c)
        }
        match n {
            Child::Unquoted(n) => n.write().set_string("changed".into()),
            Child::DoubleQuoted(n) => n.write().set_string("changed".into()),
            Child::SingleQuoted(n) => n.write().set_string("changed".into()),
            Child::Block(n) => n.write().set_string("changed".into()),
            Child::Number(n) => {
                if let Some(v) = &n.read().value {
                    *v.write() = BigRational::from_integer(73.into());
                }
            }
            Child::EdgeIndex(n) => {
                if let Some(v) = &n.read().int {
                    *v.write() = 41;
                }
            }
            _ => {}
        }
    }
    pair(src, dst, &mut aliases);
    mutate(dst);
    json!({"before":before,"after":dst,"source_before":source_before,"source_after":src,"aliases":aliases})
}
fn main() {
    let requests: Vec<Request> = serde_json::from_reader(std::io::stdin()).unwrap();
    println!(
        "{}",
        json!(requests.into_iter().map(evaluate).collect::<Vec<_>>())
    );
}
