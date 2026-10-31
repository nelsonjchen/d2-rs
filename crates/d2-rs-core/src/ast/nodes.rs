// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Initial leaf nodes from pinned d2ast/d2ast.go and the shared d2ast.Error.

use super::{Child, Range};
use serde::Serialize;

/// Common AST operations, separate from measured graph nodes.
pub trait Node {
    fn node_type(&self) -> std::borrow::Cow<'_, str>;
    fn range(&self) -> Range;
    fn children(&self) -> Vec<Child>;
}

/// A node permitted directly inside a map.
pub trait MapNode: Node {}
/// A node permitted directly inside an array.
pub trait ArrayNode: Node {}
/// A node permitted as a key's value.
pub trait Value: ArrayNode {}
/// A scalar value and its language string representation.
pub trait Scalar: Value {
    /// Authoritative Go scalar bytes; use these for semantic comparisons.
    fn scalar_bytes(&self) -> std::borrow::Cow<'_, [u8]> {
        self.scalar_string().into_owned().into_bytes().into()
    }
    /// Text presentation. May replace invalid bytes; scalar_bytes preserves them.
    fn scalar_string(&self) -> std::borrow::Cow<'_, str>;
}

/// Operations shared by language string nodes. Rust Clone supplies copying.
pub trait StringNode: Scalar {
    fn set_string(&mut self, value: String);
    fn is_unquoted(&self) -> bool;
}

/// Single-quoted source spelling and decoded contents are independent.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct SingleQuotedString {
    pub range: Range,
    pub raw: String,
    pub value: super::ByteString,
}

/// Block string delimiter, tag and decoded contents.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct BlockString {
    pub range: Range,
    pub quote: String,
    pub tag: String,
    pub value: super::ByteString,
}

impl Node for BlockString {
    fn node_type(&self) -> std::borrow::Cow<'_, str> {
        format!("{} block string", self.tag).into()
    }
    fn range(&self) -> Range {
        self.range.clone()
    }
    fn children(&self) -> Vec<Child> {
        Vec::new()
    }
}
impl ArrayNode for BlockString {}
impl Value for BlockString {}
impl Scalar for BlockString {
    fn scalar_bytes(&self) -> std::borrow::Cow<'_, [u8]> {
        self.value.as_bytes().into()
    }
    fn scalar_string(&self) -> std::borrow::Cow<'_, str> {
        self.value.to_text()
    }
}
impl StringNode for BlockString {
    fn set_string(&mut self, value: String) {
        self.value = value.into();
    }
    fn is_unquoted(&self) -> bool {
        false
    }
}
impl Value for SingleQuotedString {}
impl Scalar for SingleQuotedString {
    fn scalar_bytes(&self) -> std::borrow::Cow<'_, [u8]> {
        self.value.as_bytes().into()
    }
    fn scalar_string(&self) -> std::borrow::Cow<'_, str> {
        self.value.to_text()
    }
}
impl StringNode for SingleQuotedString {
    fn set_string(&mut self, value: String) {
        self.raw.clear();
        self.value = value.into();
    }
    fn is_unquoted(&self) -> bool {
        false
    }
}

/// Line comment text and source location.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Comment {
    pub range: Range,
    pub value: String,
}

/// Block comment text and source location.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct BlockComment {
    pub range: Range,
    pub value: super::ByteString,
}

/// The null scalar; it has no separate value field.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Null {
    pub range: Range,
}

/// Suspension state. Its scalar string is empty for either boolean value.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Suspension {
    pub range: Range,
    pub value: bool,
}

/// A parsed boolean scalar.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Boolean {
    pub range: Range,
    pub value: bool,
}

macro_rules! leaf_node {
    ($node:ty, $name:literal) => {
        impl Node for $node {
            fn node_type(&self) -> std::borrow::Cow<'_, str> {
                $name.into()
            }
            fn range(&self) -> Range {
                self.range.clone()
            }
            fn children(&self) -> Vec<Child> {
                Vec::new()
            }
        }
        impl ArrayNode for $node {}
    };
}

leaf_node!(Comment, "comment");
leaf_node!(BlockComment, "block comment");
leaf_node!(Null, "null");
leaf_node!(Suspension, "suspension");
leaf_node!(Boolean, "boolean");
leaf_node!(SingleQuotedString, "single quoted string");
impl MapNode for Comment {}
impl MapNode for BlockComment {}
impl Value for Null {}
impl Value for Suspension {}
impl Value for Boolean {}
impl Scalar for Null {
    fn scalar_string(&self) -> std::borrow::Cow<'_, str> {
        "".into()
    }
}
impl Scalar for Suspension {
    fn scalar_string(&self) -> std::borrow::Cow<'_, str> {
        "".into()
    }
}
impl Scalar for Boolean {
    fn scalar_string(&self) -> std::borrow::Cow<'_, str> {
        (if self.value { "true" } else { "false" }).into()
    }
}

/// Shared parser/compiler diagnostic. Display is the message alone; callers
/// decide whether to prefix source locations, matching upstream d2ast.Error.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Error {
    pub range: Range,
    #[serde(rename = "errmsg")]
    pub message: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_edits_preserve_block_delimiters_but_invalidate_single_quote_spelling() {
        let original = SingleQuotedString {
            raw: "old spelling".into(),
            value: "old".into(),
            ..Default::default()
        };
        let mut edited = original.clone();
        edited.set_string("new".into());
        assert!(edited.raw.is_empty());
        assert_eq!(edited.scalar_string(), "new");
        assert_eq!(original.raw, "old spelling");
        let mut block = BlockString {
            quote: "||||".into(),
            tag: "md".into(),
            value: "old".into(),
            ..Default::default()
        };
        block.set_string("new".into());
        assert_eq!(block.quote, "||||");
        assert_eq!(block.node_type(), "md block string");
        assert_eq!(block.scalar_string(), "new");
        assert!(!block.is_unquoted());
        assert!(!edited.is_unquoted());
        assert_eq!(BlockString::default().node_type(), " block string");
    }

    #[test]
    fn scalar_text_does_not_conflate_suspension_with_boolean() {
        for value in [false, true] {
            let suspension = Suspension {
                value,
                ..Default::default()
            };
            let boolean = Boolean {
                value,
                ..Default::default()
            };
            assert_eq!(suspension.scalar_string(), "");
            assert_eq!(boolean.scalar_string(), value.to_string());
            assert!(suspension.children().is_empty());
            assert!(boolean.children().is_empty());
        }
        assert_eq!(Null::default().scalar_string(), "");
    }

    #[test]
    fn diagnostic_display_does_not_add_a_location() {
        let error = Error {
            range: Range {
                path: "a.d2".into(),
                ..Default::default()
            },
            message: "bad key".into(),
        };
        assert_eq!(error.to_string(), "bad key");
        assert_eq!(error.range.path, "a.d2");
    }
}
