// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Composite records and JSON boxes from pinned d2ast/d2ast.go.
use super::*;
use serde::Serialize;

// A box is a record with independent nullable fields, not a tagged union.
// Declaration order below is also upstream's Unbox precedence.
macro_rules! ast_box {
    ($name:ident { $($field:ident: $ty:ident => $variant:ident),+ $(,)? }) => {
        #[derive(Clone, Debug, Default, Serialize)]
        pub struct $name {
            $(#[serde(skip_serializing_if = "Option::is_none")]
            pub $field: Option<Shared<$ty>>,)+
        }
        impl $name {
            pub fn unbox(&self) -> Option<Child> {
                $(if let Some(value) = &self.$field {
                    return Some(Child::$variant(value.clone()));
                })+
                None
            }
            /// Unrepresented node types produce an empty box, as in Go's switch.
            pub fn from_child(child: Option<Child>) -> Self {
                match child {
                    $(Some(Child::$variant(value)) => Self { $field: Some(value), ..Default::default() },)+
                    _ => Self::default(),
                }
            }
        }
    };
}
ast_box!(MapNodeBox {
    comment: Comment => Comment, block_comment: BlockComment => BlockComment,
    substitution: Substitution => Substitution, import: Import => Import, map_key: Key => Key,
});
// Suspension intentionally has no slot in upstream ArrayNodeBox.
ast_box!(ArrayNodeBox {
    comment: Comment => Comment, block_comment: BlockComment => BlockComment,
    substitution: Substitution => Substitution, import: Import => Import,
    null: Null => Null, boolean: Boolean => Boolean, number: Number => Number,
    unquoted_string: UnquotedString => Unquoted, double_quoted_string: DoubleQuotedString => DoubleQuoted,
    single_quoted_string: SingleQuotedString => SingleQuoted, block_string: BlockString => Block,
    array: Array => Array, map: Map => Map,
});
ast_box!(ValueBox {
    null: Null => Null, suspension: Suspension => Suspension, boolean: Boolean => Boolean,
    number: Number => Number, unquoted_string: UnquotedString => Unquoted,
    double_quoted_string: DoubleQuotedString => DoubleQuoted, single_quoted_string: SingleQuotedString => SingleQuoted,
    block_string: BlockString => Block, array: Array => Array, map: Map => Map, import: Import => Import,
});
ast_box!(ScalarBox {
    null: Null => Null, suspension: Suspension => Suspension, boolean: Boolean => Boolean,
    number: Number => Number, unquoted_string: UnquotedString => Unquoted,
    double_quoted_string: DoubleQuotedString => DoubleQuoted, single_quoted_string: SingleQuotedString => SingleQuoted,
    block_string: BlockString => Block,
});

impl ValueBox {
    pub fn scalar_box(&self) -> ScalarBox {
        ScalarBox {
            null: self.null.clone(),
            suspension: self.suspension.clone(),
            boolean: self.boolean.clone(),
            number: self.number.clone(),
            unquoted_string: self.unquoted_string.clone(),
            double_quoted_string: self.double_quoted_string.clone(),
            single_quoted_string: self.single_quoted_string.clone(),
            block_string: self.block_string.clone(),
        }
    }
    pub fn string_box(&self) -> StringBox {
        StringBox {
            unquoted_string: self.unquoted_string.clone(),
            double_quoted_string: self.double_quoted_string.clone(),
            single_quoted_string: self.single_quoted_string.clone(),
            block_string: self.block_string.clone(),
        }
    }
}
impl ScalarBox {
    pub fn scalar_bytes(&self) -> Vec<u8> {
        macro_rules! text {
            ($value:expr) => {
                $value.read().scalar_bytes().into_owned()
            };
        }
        match self.unbox().expect("empty AST ScalarBox") {
            Child::Null(v) => text!(v),
            Child::Suspension(v) => text!(v),
            Child::Boolean(v) => text!(v),
            Child::Number(v) => text!(v),
            Child::Unquoted(v) => text!(v),
            Child::DoubleQuoted(v) => text!(v),
            Child::SingleQuoted(v) => text!(v),
            Child::Block(v) => text!(v),
            _ => unreachable!("ScalarBox contains only scalars"),
        }
    }
    /// Inspect scalar presentation under its source read guards. Interpolated
    /// strings retain first-slot semantics and Go's one-invalid-byte decoder.
    pub fn with_scalar_text<R>(&self, inspect: impl FnOnce(&str) -> R) -> R {
        macro_rules! scalar {
            ($field:ident) => {
                if let Some(value) = &self.$field {
                    return inspect(value.read().scalar_string().as_ref());
                }
            };
        }
        // Keep Unbox precedence even for caller-assembled multi-slot boxes.
        scalar!(null);
        scalar!(suspension);
        scalar!(boolean);
        scalar!(number);
        macro_rules! interpolated {
            ($field:ident) => {
                if let Some(value) = &self.$field {
                    let node = value.read();
                    if let Some(slot) = node.value.as_ref().and_then(|parts| parts.get(0)) {
                        let slot = slot.read();
                        if let Some(text) = &slot.string {
                            return inspect(text.read().to_text().as_ref());
                        }
                    }
                    return inspect("");
                }
            };
        }
        interpolated!(unquoted_string);
        interpolated!(double_quoted_string);
        scalar!(single_quoted_string);
        scalar!(block_string);
        panic!("empty AST ScalarBox")
    }
    pub fn scalar_string(&self) -> String {
        self.with_scalar_text(str::to_owned)
    }
}

#[cfg(test)]
mod scalar_text_tests {
    use super::*;

    #[test]
    fn borrowed_scalar_text_preserves_bytes_presentation_and_box_precedence() {
        let mut values = vec![
            ScalarBox {
                null: Some(Shared::new(Null::default())),
                ..Default::default()
            },
            ScalarBox {
                suspension: Some(Shared::new(Suspension::default())),
                ..Default::default()
            },
            ScalarBox {
                boolean: Some(Shared::new(Boolean {
                    value: true,
                    ..Default::default()
                })),
                ..Default::default()
            },
            ScalarBox {
                number: Some(Shared::new(Number {
                    raw: "000.20".into(),
                    ..Default::default()
                })),
                ..Default::default()
            },
            ScalarBox {
                unquoted_string: Some(Shared::new(UnquotedString::default())),
                ..Default::default()
            },
        ];
        for bytes in [
            b"plain".to_vec(),
            "ΣKſ".as_bytes().to_vec(),
            vec![0xe2, 0x82],
            vec![0xff, 0xfe],
        ] {
            values.extend([
                ScalarBox {
                    unquoted_string: Some(Shared::new(UnquotedString::flat(bytes.clone()))),
                    ..Default::default()
                },
                ScalarBox {
                    double_quoted_string: Some(Shared::new(DoubleQuotedString::flat(
                        bytes.clone(),
                    ))),
                    ..Default::default()
                },
                ScalarBox {
                    single_quoted_string: Some(Shared::new(SingleQuotedString {
                        value: bytes.clone().into(),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
                ScalarBox {
                    block_string: Some(Shared::new(BlockString {
                        value: bytes.into(),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
            ]);
        }
        let literal = InterpolationBox::literal("first");
        values.push(ScalarBox {
            unquoted_string: Some(Shared::new(UnquotedString {
                value: Some(vec![literal, InterpolationBox::literal("second")].into()),
                ..Default::default()
            })),
            double_quoted_string: Some(Shared::new(DoubleQuotedString::flat("ignored"))),
            ..Default::default()
        });
        values.push(ScalarBox {
            unquoted_string: Some(Shared::new(UnquotedString {
                value: Some(
                    vec![
                        InterpolationBox::default(),
                        InterpolationBox::literal("ignored"),
                    ]
                    .into(),
                ),
                ..Default::default()
            })),
            ..Default::default()
        });
        let mut multi = values.last().unwrap().clone();
        multi.boolean = Some(Shared::new(Boolean {
            value: false,
            ..Default::default()
        }));
        multi.null = Some(Shared::new(Null::default()));
        values.push(multi);
        for value in values {
            let expected = ByteString::from(value.scalar_bytes())
                .to_text()
                .into_owned();
            assert_eq!(value.with_scalar_text(str::to_owned), expected);
            assert_eq!(value.scalar_string(), expected);
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Array {
    pub range: Range,
    pub nodes: Option<SharedSlice<ArrayNodeBox>>,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct Map {
    pub range: Range,
    pub nodes: Option<SharedSlice<MapNodeBox>>,
}
macro_rules! collection_node {
    ($ty:ty, $name:literal) => {
        impl Node for $ty {
            fn node_type(&self) -> std::borrow::Cow<'_, str> {
                $name.into()
            }
            fn range(&self) -> Range {
                self.range.clone()
            }
            fn children(&self) -> Vec<Child> {
                self.nodes
                    .iter()
                    .flat_map(SharedSlice::iter)
                    .filter_map(|b| b.read().unbox())
                    .collect()
            }
        }
        impl ArrayNode for $ty {}
        impl Value for $ty {}
    };
}
collection_node!(Array, "array");
collection_node!(Map, "map");

#[derive(Clone, Debug, Default, Serialize)]
pub struct Key {
    pub range: Range,
    #[serde(skip_serializing_if = "is_false")]
    pub ampersand: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub not_ampersand: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<Shared<KeyPath>>,
    #[serde(skip_serializing_if = "empty_edges")]
    pub edges: Option<SharedSlice<Option<Shared<Edge>>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_index: Option<Shared<EdgeIndex>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_key: Option<Shared<KeyPath>>,
    pub primary: ScalarBox,
    pub value: ValueBox,
}
fn is_false(value: &bool) -> bool {
    !value
}
fn empty_edges(value: &Option<SharedSlice<Option<Shared<Edge>>>>) -> bool {
    value.as_ref().is_none_or(SharedSlice::is_empty)
}
impl Node for Key {
    fn node_type(&self) -> std::borrow::Cow<'_, str> {
        "map key".into()
    }
    fn range(&self) -> Range {
        self.range.clone()
    }
    fn children(&self) -> Vec<Child> {
        let mut out = Vec::new();
        out.extend(self.key.clone().map(Child::KeyPath));
        for edge in self.edges.iter().flat_map(SharedSlice::iter) {
            out.extend(edge.read().clone().map(Child::Edge));
        }
        out.extend(self.edge_index.clone().map(Child::EdgeIndex));
        out.extend(self.edge_key.clone().map(Child::KeyPath));
        out.extend(self.primary.unbox());
        out.extend(self.value.unbox());
        out
    }
}
impl MapNode for Key {}
impl Key {
    pub fn set_scalar(&mut self, scalar: ScalarBox) {
        if self.value.unbox().is_some() && self.value.scalar_box().unbox().is_none() {
            self.primary = scalar;
        } else {
            self.value = ValueBox::from_child(scalar.unbox());
        }
    }
}
impl Map {
    pub fn insert_before(&mut self, cursor: Option<&Child>, node: Option<Child>) {
        self.insert(cursor, node, false);
    }
    pub fn insert_after(&mut self, cursor: Option<&Child>, node: Option<Child>) {
        self.insert(cursor, node, true);
    }
    fn insert(&mut self, cursor: Option<&Child>, node: Option<Child>, after: bool) {
        let mut nodes: Vec<_> = self
            .nodes
            .iter()
            .flat_map(SharedSlice::iter)
            .map(|slot| slot.read().clone())
            .collect();
        let mut index = nodes.len();
        for (i, entry) in nodes.iter().enumerate() {
            let matches = match (entry.unbox().as_ref(), cursor) {
                (None, None) => true,
                (Some(a), Some(b)) => a.ptr_eq(b),
                _ => false,
            };
            if matches {
                index = i + usize::from(after);
            }
        }
        nodes.insert(index, MapNodeBox::from_child(node));
        // Upstream allocates a fresh slice: old copies retain their slots.
        self.nodes = Some(nodes.into());
    }
    pub fn is_file_map(&self) -> bool {
        self.range.start.line == 0 && self.range.start.column == 0
    }
    pub fn has_filter(&self) -> bool {
        self.nodes.iter().flat_map(SharedSlice::iter).any(|b| {
            b.read().map_key.as_ref().is_some_and(|k| {
                let k = k.read();
                k.ampersand || k.not_ampersand
            })
        })
    }
}
impl MapNodeBox {
    pub fn is_board_node(&self) -> bool {
        let Some(key) = &self.map_key else {
            return false;
        };
        let key = key.read();
        let Some(path) = &key.key else {
            return false;
        };
        let path = path.read();
        let Some(parts) = &path.path else {
            return false;
        };
        if parts.len() != 1 {
            return false;
        }
        let slot = parts.get(0).unwrap();
        let part = slot.read();
        let part = part.as_ref().expect("nil board path box").read();
        matches!(
            part.scalar_string().as_str(),
            "layers" | "scenarios" | "steps"
        )
    }
}
