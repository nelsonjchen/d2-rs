// SPDX-License-Identifier: MPL-2.0
use d2_tala_rs::ast::*;
use serde_json::json;
fn main() {
    let rows = [("a", "/b"), ("../a", "../../b"), ("/a", "../../b"), ("", ""), ("", "a\\b"), ("a//", "./b/../c")].into_iter().map(|(pre,file)| {
        let import = Import { pre: pre.into(), path: KeyPath::from_strings([file, "identifier"]).path, ..Default::default() };
        json!({"record": import, "path": import.path_with_pre(), "dir": import.dir(), "ids": import.ida().unwrap().len(), "children": import.children().len()})
    }).collect::<Vec<_>>();
    println!("{}", json!(rows));
}
