// SPDX-License-Identifier: MIT
use d2_rs_text::code;
use serde_json::{Value, json};
fn main() {
    let requests: Vec<Value> = serde_json::from_reader(std::io::stdin()).unwrap();
    let results: Vec<Value> = requests.iter().map(|request| {
        let tokens = code::tokenize(request["language"].as_str().unwrap(), request["source"].as_str().unwrap()).unwrap();
        let rows: Vec<Value> = tokens.iter().map(|t| json!({"kind":t.kind,"value":t.value})).collect();
        let styles: serde_json::Map<String, Value> = [("github",false),("catppuccin-mocha",true)].into_iter().map(|(name,dark)| {
            let rows: Vec<Value> = tokens.iter().map(|t| {let s=code::style(t.kind,dark);json!({"kind":t.kind,"value":t.value,"color":s.color.unwrap_or_default(),"bold":s.bold==Some(true),"italic":s.italic==Some(true),"underline":s.underline==Some(true)})}).collect();
            (name.to_owned(),json!(rows))
        }).collect();
        json!({"language":request["language"],"source":request["source"],"tokens":rows,"styles":styles})
    }).collect();
    println!("{}", json!(results));
}
