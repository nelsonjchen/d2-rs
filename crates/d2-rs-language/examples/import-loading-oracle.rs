// SPDX-License-Identifier: MPL-2.0
use d2_rs_core::ast::{self, Shared, StringNode};
use d2_rs_language::{compiler::imports::ImportState, parser::ParseError};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
#[derive(Default, Deserialize)]
#[serde(default)]
pub struct Request {
    files: BTreeMap<String, String>,
    utf16: bool,
    ops: Vec<Operation>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Operation {
    op: String,
    file: String,
    pre: String,
    located: bool,
    hex: String,
}
fn decode(s: &str) -> Vec<u8> {
    s.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
pub fn evaluate(r: Request) -> Value {
    let mut files: BTreeMap<_, _> = r
        .files
        .into_iter()
        .map(|(name, value)| (name, decode(&value)))
        .collect();
    let mut state = ImportState {
        utf16_positions: r.utf16,
        ..Default::default()
    };
    let mut errors = ParseError::default();
    let mut working: Option<Shared<ast::Map>> = None;
    let mut opens = BTreeMap::<String, usize>::new();
    let mut out = Vec::new();
    for op in r.ops {
        let mut path = None;
        let mut open_error = None;
        let mut ok = true;
        match op.op.as_str() {
            "push" => {
                let import = Shared::new(ast::Import {
                    pre: op.pre,
                    path: ast::KeyPath::from_strings([op.file]).path,
                    range: if op.located {
                        ast::Range {
                            path: "fixture.d2".into(),
                            end: ast::Position {
                                byte: 1,
                                column: 1,
                                ..Default::default()
                            },
                            ..Default::default()
                        }
                    } else {
                        Default::default()
                    },
                    ..Default::default()
                });
                path = state.push(import, &mut errors);
                ok = path.is_some();
            }
            "pop" => state.pop(),
            "load" => {
                (working, open_error) = state.load_ast(&op.file, &mut errors, |path| {
                    *opens.entry(path.into()).or_default() += 1;
                    files
                        .get(path)
                        .cloned()
                        .ok_or_else(|| format!("missing {path}"))
                });
                ok = working.is_some();
            }
            "write" => {
                files.insert(op.file, decode(&op.hex));
            }
            "error" => errors.errors.get_or_insert_default().push(ast::Error {
                message: "existing error".into(),
                ..Default::default()
            }),
            "clear-errors" => errors.errors = None,
            "mutate" => {
                if let Some(map) = &working
                    && let Some(first) = map.read().nodes.as_ref().and_then(|n| n.get(0))
                    && let Some(key) = first.read().map_key.clone()
                {
                    let key = key.read();
                    if let Some(path) = &key.key
                        && let Some(slot) = path.read().path.as_ref().and_then(|p| p.get(0))
                    {
                        *slot.write() = Some(Shared::new(ast::raw_string_box("changed", true)));
                    }
                    if let Some(value) = &key.value.unquoted_string {
                        value.write().set_string("changed".into());
                    }
                }
            }
            _ => panic!("unknown operation"),
        }
        let mut cache: Vec<_> = state.parsed.keys().cloned().collect();
        cache.sort();
        out.push(json!({"path":path,"ok":ok,"open_error":open_error,"stack":state.stack,"cache":cache,"errors":errors,"working":working,"opens":opens}));
    }
    json!(out)
}
fn main() {
    let requests: Vec<Request> = serde_json::from_reader(std::io::stdin()).unwrap();
    serde_json::to_writer(
        std::io::stdout(),
        &requests.into_iter().map(evaluate).collect::<Vec<_>>(),
    )
    .unwrap();
}
