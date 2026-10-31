// SPDX-License-Identifier: MPL-2.0
use d2_tala_rs::ast::*;
use serde::Serialize;
use serde_json::{Value as JsonValue, json};

fn row<T: Node + Serialize>(node: T, scalar: Option<&str>) -> JsonValue {
    json!({"node": node, "type": node.node_type(), "range": node.range().to_string(),
           "children_count": node.children().len(), "scalar": scalar})
}

fn main() {
    let mut rows = Vec::new();
    for range in [
        Range::default(),
        Range {
            path: "folder,a-é.d2".into(),
            start: Position {
                line: 1,
                column: 2,
                byte: 3,
            },
            end: Position {
                line: 4,
                column: 5,
                byte: 6,
            },
        },
    ] {
        rows.push(row(
            Comment {
                range: range.clone(),
                value: "text\n# é".into(),
            },
            None,
        ));
        rows.push(row(
            BlockComment {
                range: range.clone(),
                value: "text\n# é".into(),
            },
            None,
        ));
        let null = Null {
            range: range.clone(),
        };
        let text = null.scalar_string().into_owned();
        rows.push(row(null, Some(&text)));
        for value in [false, true] {
            let node = Suspension {
                range: range.clone(),
                value,
            };
            let text = node.scalar_string().into_owned();
            rows.push(row(node, Some(&text)));
        }
        for value in [false, true] {
            let node = Boolean {
                range: range.clone(),
                value,
            };
            let text = node.scalar_string().into_owned();
            rows.push(row(node, Some(&text)));
        }
        let single = SingleQuotedString {
            range: range.clone(),
            raw: "old spelling".into(),
            value: "old".into(),
        };
        let mut edited_single = single.clone();
        edited_single.set_string("new\né".into());
        for node in [single, edited_single] {
            let text = node.scalar_string().into_owned();
            rows.push(row(node, Some(&text)));
        }
        let block = BlockString {
            range: range.clone(),
            quote: "||||".into(),
            tag: "md".into(),
            value: "old".into(),
        };
        let mut edited_block = block.clone();
        edited_block.set_string("new\né".into());
        for node in [
            block,
            edited_block,
            BlockString {
                range: range.clone(),
                ..Default::default()
            },
        ] {
            let text = node.scalar_string().into_owned();
            rows.push(row(node, Some(&text)));
        }
        let error = Error {
            range,
            message: "bad key\nsecond line".into(),
        };
        rows.push(json!({"display": error.to_string(), "error": error}));
    }
    println!("{}", serde_json::to_string(&rows).unwrap());
}
