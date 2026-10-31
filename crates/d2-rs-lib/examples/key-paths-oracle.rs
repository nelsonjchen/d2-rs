// SPDX-License-Identifier: MPL-2.0
use d2_tala_rs::ast::*;
use serde_json::json;
fn main() {
    let mut paths = [
        vec![],
        vec!["a", "b"],
        vec!["a.b", "*", "$v", ""],
        vec!["null", "a-", "é"],
    ]
    .into_iter()
    .map(KeyPath::from_strings)
    .collect::<Vec<_>>();
    paths.push(KeyPath {
        path: Some(vec![].into()),
        ..Default::default()
    });
    let single = SingleQuotedString {
        value: "a.b".into(),
        ..Default::default()
    };
    let double = DoubleQuotedString {
        value: Some(
            vec![
                InterpolationBox::literal("a"),
                InterpolationBox::literal("b"),
            ]
            .into(),
        ),
        ..Default::default()
    };
    paths.push(KeyPath::from_string_nodes([
        &single as &dyn StringNode,
        &double,
    ]));
    for pattern in [
        vec!["*"],
        vec!["*", "", "*"],
        vec!["*", "", "*", "", "*"],
        vec!["*", "x", "*"],
    ] {
        let kp = KeyPath::from_strings(["a", "*"]);
        kp.last().unwrap().write().unquoted_string = Some(Shared::new(UnquotedString {
            pattern: Some(
                pattern
                    .into_iter()
                    .map(ByteString::from)
                    .collect::<Vec<_>>()
                    .into(),
            ),
            ..Default::default()
        }));
        paths.push(kp);
    }
    let mut rows = paths.iter().map(|kp| json!({"path": kp, "copy": kp.clone(), "ids": kp.string_ida(), "children": kp.children().len(), "first_glob": kp.first_glob(), "has_glob": kp.has_glob(), "multi": kp.has_multi_glob(), "triple": kp.has_triple_glob(), "equals_copy": kp.equals(&kp.clone())})).collect::<Vec<_>>();
    let original = KeyPath::from_strings(["a", "b"]);
    let copied = original.clone();
    copied
        .path
        .as_ref()
        .unwrap()
        .get(0)
        .unwrap()
        .write()
        .replace(Shared::new(raw_string_box("c", true)));
    copied
        .last()
        .unwrap()
        .write()
        .unquoted_string
        .as_ref()
        .unwrap()
        .write()
        .set_string("shared".into());
    rows.push(
        json!({"original": original, "edited_copy": copied, "equals": original.equals(&copied)}),
    );
    println!("{}", serde_json::to_string(&rows).unwrap());
}
