// SPDX-License-Identifier: MPL-2.0
use d2_rs_tala::ast::*;
use serde_json::json;
fn masks(n: usize) -> Vec<usize> {
    let mut out = vec![0, (1 << n) - 1];
    for i in 0..n {
        out.push(1 << i);
        for j in i + 1..n {
            out.push((1 << i) | (1 << j));
        }
    }
    out
}
fn main() {
    let mut rows = Vec::new();
    for mask in masks(5) {
        let mut b = MapNodeBox::default();
        if mask & (1 << 0) != 0 {
            b.comment = Some(Shared::new(Comment::default()));
        }
        if mask & (1 << 1) != 0 {
            b.block_comment = Some(Shared::new(BlockComment::default()));
        }
        if mask & (1 << 2) != 0 {
            b.substitution = Some(Shared::new(Substitution::default()));
        }
        if mask & (1 << 3) != 0 {
            b.import = Some(Shared::new(Import::default()));
        }
        if mask & (1 << 4) != 0 {
            b.map_key = Some(Shared::new(Key::default()));
        }
        let mut row = json!({"box":b,"mask":mask,"kind":"MapNodeBox","selected":b.unbox().map(|c|c.node_type().into_owned())});
        row["board"] = json!(b.is_board_node());
        rows.push(row);
    }
    for mask in masks(13) {
        let mut b = ArrayNodeBox::default();
        if mask & (1 << 0) != 0 {
            b.comment = Some(Shared::new(Comment::default()));
        }
        if mask & (1 << 1) != 0 {
            b.block_comment = Some(Shared::new(BlockComment::default()));
        }
        if mask & (1 << 2) != 0 {
            b.substitution = Some(Shared::new(Substitution::default()));
        }
        if mask & (1 << 3) != 0 {
            b.import = Some(Shared::new(Import::default()));
        }
        if mask & (1 << 4) != 0 {
            b.null = Some(Shared::new(Null::default()));
        }
        if mask & (1 << 5) != 0 {
            b.boolean = Some(Shared::new(Boolean::default()));
        }
        if mask & (1 << 6) != 0 {
            b.number = Some(Shared::new(Number::default()));
        }
        if mask & (1 << 7) != 0 {
            b.unquoted_string = Some(Shared::new(UnquotedString::default()));
        }
        if mask & (1 << 8) != 0 {
            b.double_quoted_string = Some(Shared::new(DoubleQuotedString::default()));
        }
        if mask & (1 << 9) != 0 {
            b.single_quoted_string = Some(Shared::new(SingleQuotedString::default()));
        }
        if mask & (1 << 10) != 0 {
            b.block_string = Some(Shared::new(BlockString::default()));
        }
        if mask & (1 << 11) != 0 {
            b.array = Some(Shared::new(Array::default()));
        }
        if mask & (1 << 12) != 0 {
            b.map = Some(Shared::new(Map::default()));
        }
        let row = json!({"box":b,"mask":mask,"kind":"ArrayNodeBox","selected":b.unbox().map(|c|c.node_type().into_owned())});
        rows.push(row);
    }
    for mask in masks(11) {
        let mut b = ValueBox::default();
        if mask & (1 << 0) != 0 {
            b.null = Some(Shared::new(Null::default()));
        }
        if mask & (1 << 1) != 0 {
            b.suspension = Some(Shared::new(Suspension::default()));
        }
        if mask & (1 << 2) != 0 {
            b.boolean = Some(Shared::new(Boolean::default()));
        }
        if mask & (1 << 3) != 0 {
            b.number = Some(Shared::new(Number::default()));
        }
        if mask & (1 << 4) != 0 {
            b.unquoted_string = Some(Shared::new(UnquotedString::default()));
        }
        if mask & (1 << 5) != 0 {
            b.double_quoted_string = Some(Shared::new(DoubleQuotedString::default()));
        }
        if mask & (1 << 6) != 0 {
            b.single_quoted_string = Some(Shared::new(SingleQuotedString::default()));
        }
        if mask & (1 << 7) != 0 {
            b.block_string = Some(Shared::new(BlockString::default()));
        }
        if mask & (1 << 8) != 0 {
            b.array = Some(Shared::new(Array::default()));
        }
        if mask & (1 << 9) != 0 {
            b.map = Some(Shared::new(Map::default()));
        }
        if mask & (1 << 10) != 0 {
            b.import = Some(Shared::new(Import::default()));
        }
        let mut row = json!({"box":b,"mask":mask,"kind":"ValueBox","selected":b.unbox().map(|c|c.node_type().into_owned())});
        row["scalar_box"] = json!(b.scalar_box());
        row["string_box"] = json!(b.string_box());
        rows.push(row);
    }
    for mask in masks(8) {
        let mut b = ScalarBox::default();
        if mask & (1 << 0) != 0 {
            b.null = Some(Shared::new(Null::default()));
        }
        if mask & (1 << 1) != 0 {
            b.suspension = Some(Shared::new(Suspension::default()));
        }
        if mask & (1 << 2) != 0 {
            b.boolean = Some(Shared::new(Boolean::default()));
        }
        if mask & (1 << 3) != 0 {
            b.number = Some(Shared::new(Number::default()));
        }
        if mask & (1 << 4) != 0 {
            b.unquoted_string = Some(Shared::new(UnquotedString::default()));
        }
        if mask & (1 << 5) != 0 {
            b.double_quoted_string = Some(Shared::new(DoubleQuotedString::default()));
        }
        if mask & (1 << 6) != 0 {
            b.single_quoted_string = Some(Shared::new(SingleQuotedString::default()));
        }
        if mask & (1 << 7) != 0 {
            b.block_string = Some(Shared::new(BlockString::default()));
        }
        let mut row = json!({"box":b,"mask":mask,"kind":"ScalarBox","selected":b.unbox().map(|c|c.node_type().into_owned())});
        if b.unbox().is_some() {
            row["text"] = json!(b.scalar_string());
        }
        rows.push(row);
    }
    rows.push(json!({"suspension_array":ArrayNodeBox::from_child(Some(Child::Suspension(Shared::new(Suspension {value:true,..Default::default()}))))}));
    for board in ["layers", "scenarios", "steps", "Layers", "ordinary"] {
        let k = Shared::new(Key {
            key: Some(Shared::new(KeyPath::from_strings([board]))),
            ampersand: true,
            edges: Some(vec![None, Some(Shared::new(Edge::default()))].into()),
            edge_index: Some(Shared::new(EdgeIndex::default())),
            edge_key: Some(Shared::new(KeyPath::from_strings(["label"]))),
            primary: ScalarBox {
                boolean: Some(Shared::new(Boolean {
                    value: true,
                    ..Default::default()
                })),
                ..Default::default()
            },
            value: ValueBox {
                array: Some(Shared::new(Array {
                    nodes: Some(
                        vec![
                            ArrayNodeBox::default(),
                            ArrayNodeBox {
                                null: Some(Shared::new(Null::default())),
                                ..Default::default()
                            },
                        ]
                        .into(),
                    ),
                    ..Default::default()
                })),
                ..Default::default()
            },
            ..Default::default()
        });
        let b = MapNodeBox {
            comment: Some(Shared::new(Comment::default())),
            map_key: Some(k.clone()),
            ..Default::default()
        };
        let m = Map {
            nodes: Some(
                vec![
                    MapNodeBox::default(),
                    b.clone(),
                    MapNodeBox::from_child(Some(Child::Key(k.clone()))),
                ]
                .into(),
            ),
            ..Default::default()
        };
        let mut types = Vec::new();
        walk(Some(&m), &mut |n| {
            types.push(n.node_type().into_owned());
            true
        });
        let before = json!(m);
        k.write().set_scalar(ScalarBox {
            number: Some(Shared::new(Number {
                raw: "replacement".into(),
                ..Default::default()
            })),
            ..Default::default()
        });
        rows.push(json!({"board_name":board,"board":b.is_board_node(),"filter":m.has_filter(),"file":m.is_file_map(),"before":before,"walk":types,"after":m}));
    }
    println!("{}", serde_json::to_string(&rows).unwrap());
}
