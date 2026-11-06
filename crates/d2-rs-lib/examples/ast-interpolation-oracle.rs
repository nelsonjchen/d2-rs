// SPDX-License-Identifier: MPL-2.0
use d2_rs_tala::ast::*;
use serde_json::json;

fn main() {
    let sub = Shared::new(Substitution {
        path: Some(
            vec![Some(Shared::new(StringBox {
                single_quoted_string: Some(Shared::new(SingleQuotedString {
                    value: "var".into(),
                    ..Default::default()
                })),
                ..Default::default()
            }))]
            .into(),
        ),
        ..Default::default()
    });
    let substitution = || InterpolationBox {
        substitution: Some(sub.clone()),
        ..Default::default()
    };
    let cases: Vec<Option<SharedSlice<InterpolationBox>>> = vec![
        None,
        Some(vec![].into()),
        Some(
            vec![
                InterpolationBox::literal("a"),
                InterpolationBox::literal("b"),
            ]
            .into(),
        ),
        Some(
            vec![
                InterpolationBox {
                    raw_string: Some(Shared::new("raw a".into())),
                    ..InterpolationBox::literal("a")
                },
                substitution(),
                InterpolationBox::literal("tail"),
            ]
            .into(),
        ),
        Some(vec![substitution(), InterpolationBox::literal("tail")].into()),
        Some(
            vec![
                InterpolationBox {
                    substitution: Some(sub.clone()),
                    ..InterpolationBox::literal("a")
                },
                InterpolationBox::literal("b"),
            ]
            .into(),
        ),
    ];
    let mut rows = Vec::new();
    for (index, parts) in cases.into_iter().enumerate() {
        macro_rules! record {
            ($node:expr, $quoted:expr) => {{
                let mut node = $node;
                let before = json!(&node);
                let scalar = node.scalar_string().into_owned();
                let children = node.children().len();
                let mut copied = node.clone();
                copied.set_string("replacement".into());
                node.coalesce();
                rows.push(json!({"case": index, "quoted": $quoted, "before": before,
                    "scalar_before": scalar, "children_before": children,
                    "coalesced": node, "edited_copy": copied}));
            }};
        }
        record!(
            UnquotedString {
                value: parts.clone(),
                pattern: Some(vec!["*".into()].into()),
                ..Default::default()
            },
            false
        );
        record!(
            DoubleQuotedString {
                value: parts,
                ..Default::default()
            },
            true
        );
    }
    let original = UnquotedString::flat("original");
    let copied = original.clone();
    copied
        .value
        .as_ref()
        .unwrap()
        .get(0)
        .unwrap()
        .write()
        .string = Some(Shared::new("shared mutation".into()));
    rows.push(json!({"copy_alias_original": original, "copy_alias_copy": copied}));
    println!("{}", serde_json::to_string(&rows).unwrap());
}
