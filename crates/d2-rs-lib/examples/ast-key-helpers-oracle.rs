// SPDX-License-Identifier: MPL-2.0
use d2_tala_rs::ast::*;
use serde_json::json;
fn main() {
    let mut rows = Vec::new();
    let c = Child::Comment(Shared::new(Comment {
        value: "cursor".into(),
        ..Default::default()
    }));
    let missing = Child::Comment(Shared::new(Comment {
        value: "cursor".into(),
        ..Default::default()
    }));
    let added = Child::Comment(Shared::new(Comment {
        value: "new".into(),
        ..Default::default()
    }));
    for after in [false, true] {
        for cursor in [None, Some(&c), Some(&missing)] {
            let mut m = Map {
                nodes: Some(
                    vec![
                        MapNodeBox::default(),
                        MapNodeBox::from_child(Some(c.clone())),
                        MapNodeBox::default(),
                        MapNodeBox::from_child(Some(c.clone())),
                    ]
                    .into(),
                ),
                ..Default::default()
            };
            let old = m.clone();
            if after {
                m.insert_after(cursor, Some(added.clone()));
            } else {
                m.insert_before(cursor, Some(added.clone()));
            }
            rows.push(json!({"after":after,"map":m,"old":old}));
        }
    }
    let edge = |dst| {
        Some(
            vec![Some(Shared::new(Edge {
                src: Some(Shared::new(KeyPath::from_strings(["a"]))),
                dst: Some(Shared::new(KeyPath::from_strings([dst]))),
                ..Default::default()
            }))]
            .into(),
        )
    };
    let keys = vec![
        Key::default(),
        Key {
            ampersand: true,
            ..Default::default()
        },
        Key {
            not_ampersand: true,
            ..Default::default()
        },
        Key {
            key: Some(Shared::new(KeyPath::from_strings(["a"]))),
            ..Default::default()
        },
        Key {
            primary: ScalarBox {
                boolean: Some(Shared::new(Boolean {
                    value: true,
                    ..Default::default()
                })),
                ..Default::default()
            },
            ..Default::default()
        },
        Key {
            value: ValueBox {
                null: Some(Shared::new(Null::default())),
                ..Default::default()
            },
            ..Default::default()
        },
        Key {
            value: ValueBox {
                suspension: Some(Shared::new(Suspension::default())),
                ..Default::default()
            },
            ..Default::default()
        },
        Key {
            value: ValueBox {
                map: Some(Shared::new(Map::default())),
                ..Default::default()
            },
            ..Default::default()
        },
        Key {
            value: ValueBox {
                array: Some(Shared::new(Array::default())),
                ..Default::default()
            },
            ..Default::default()
        },
        Key {
            edge_index: Some(Shared::new(EdgeIndex {
                int: Some(Shared::new(3)),
                ..Default::default()
            })),
            ..Default::default()
        },
        Key {
            edge_index: Some(Shared::new(EdgeIndex {
                int: Some(Shared::new(3)),
                ..Default::default()
            })),
            ..Default::default()
        },
        Key {
            edges: edge("b"),
            ..Default::default()
        },
        Key {
            edges: edge("c"),
            ..Default::default()
        },
        Key {
            value: ValueBox {
                map: Some(Shared::new(Map {
                    nodes: Some(
                        vec![MapNodeBox {
                            map_key: Some(Shared::new(Key {
                                ampersand: true,
                                ..Default::default()
                            })),
                            ..Default::default()
                        }]
                        .into(),
                    ),
                    ..Default::default()
                })),
                ..Default::default()
            },
            ..Default::default()
        },
        Key {
            value: ValueBox {
                map: Some(Shared::new(Map {
                    nodes: Some(
                        vec![MapNodeBox {
                            comment: Some(Shared::new(Comment::default())),
                            ..Default::default()
                        }]
                        .into(),
                    ),
                    ..Default::default()
                })),
                ..Default::default()
            },
            ..Default::default()
        },
    ];
    for a in &keys {
        rows.push(json!({"equals":keys.iter().map(|b|a.equals(b)).collect::<Vec<_>>(),"oracle":keys.iter().map(|b|a.d2_oracle_equals(b)).collect::<Vec<_>>()}));
    }
    for pattern in [
        None,
        Some(vec!["*"]),
        Some(vec!["*", "", "*"]),
        Some(vec!["*", "", "*", "", "*"]),
    ] {
        let path = Shared::new(KeyPath {
            path: Some(
                vec![Some(Shared::new(StringBox {
                    unquoted_string: Some(Shared::new(UnquotedString {
                        pattern: pattern.map(|p| {
                            p.into_iter()
                                .map(ByteString::from)
                                .collect::<Vec<_>>()
                                .into()
                        }),
                        ..Default::default()
                    })),
                    ..Default::default()
                }))]
                .into(),
            ),
            ..Default::default()
        });
        for location in 0..5 {
            let mut k = Key::default();
            match location {
                0 => k.key = Some(path.clone()),
                1 => {
                    k.edges = Some(
                        vec![Some(Shared::new(Edge {
                            src: Some(path.clone()),
                            ..Default::default()
                        }))]
                        .into(),
                    )
                }
                2 => {
                    k.edges = Some(
                        vec![Some(Shared::new(Edge {
                            dst: Some(path.clone()),
                            ..Default::default()
                        }))]
                        .into(),
                    )
                }
                3 => k.edge_key = Some(path.clone()),
                _ => {
                    k.edge_index = Some(Shared::new(EdgeIndex {
                        glob: true,
                        ..Default::default()
                    }))
                }
            }
            rows.push(json!({"glob":k.has_glob(),"multi":k.has_multi_glob(),"triple":k.has_triple_glob(),"filters":k.supports_glob_filters()}));
        }
    }
    println!("{}", serde_json::to_string(&rows).unwrap());
}
