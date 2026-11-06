// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Interpolated strings and shared AST records from pinned d2ast/d2ast.go.
use super::*;
use serde::{Serialize, Serializer};
use std::borrow::Cow;
use std::cell::{Ref, RefCell, RefMut};
use std::rc::Rc;

/// Shared AST record. Cloning preserves identity; replacing a handle detaches it.
#[derive(Debug)]
pub struct Shared<T>(Rc<RefCell<T>>);
impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}
impl<T> Shared<T> {
    /// Allocation identity for internal pointer-keyed memoization, never a
    /// serialized AST identity or a substitute for semantic equality.
    pub(crate) fn identity(&self) -> usize {
        Rc::as_ptr(&self.0) as usize
    }
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
    pub fn new(value: T) -> Self {
        Self(Rc::new(RefCell::new(value)))
    }
    pub fn read(&self) -> Ref<'_, T> {
        self.0.borrow()
    }
    pub fn write(&self) -> RefMut<'_, T> {
        self.0.borrow_mut()
    }
}
impl<T: Serialize> Serialize for Shared<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.read().serialize(serializer)
    }
}

/// Fixed-length slice header with shared element slots. Replacing an element
/// is visible through copies; replacing the whole slice is not.
#[derive(Debug)]
pub struct SharedSlice<T>(Vec<Shared<T>>);
impl<T> Clone for SharedSlice<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T> From<Vec<T>> for SharedSlice<T> {
    fn from(values: Vec<T>) -> Self {
        Self(values.into_iter().map(Shared::new).collect())
    }
}
impl<T> SharedSlice<T> {
    /// Copy a Go slice header for a suffix, retaining its shared element slots.
    pub fn suffix(&self, start: usize) -> Self {
        Self(self.0[start..].to_vec())
    }
    pub fn get(&self, index: usize) -> Option<Shared<T>> {
        self.0.get(index).cloned()
    }
    pub fn iter(&self) -> impl Iterator<Item = Shared<T>> + '_ {
        self.0.iter().cloned()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
}
impl<T: Serialize> Serialize for SharedSlice<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

/// Owned traversal handle, valid after the parent's storage borrow ends.
#[derive(Clone, Debug)]
pub enum Child {
    Comment(Shared<Comment>),
    BlockComment(Shared<BlockComment>),
    Null(Shared<Null>),
    Suspension(Shared<Suspension>),
    Boolean(Shared<Boolean>),
    Number(Shared<Number>),
    Array(Shared<Array>),
    Map(Shared<Map>),
    Key(Shared<Key>),

    Import(Shared<Import>),
    KeyPath(Shared<KeyPath>),
    Edge(Shared<Edge>),
    EdgeIndex(Shared<EdgeIndex>),
    Unquoted(Shared<UnquotedString>),
    DoubleQuoted(Shared<DoubleQuotedString>),
    SingleQuoted(Shared<SingleQuotedString>),
    Block(Shared<BlockString>),
    Substitution(Shared<Substitution>),
}
macro_rules! dispatch_child {
    ($self:expr, $node:ident, $body:expr) => {
        match $self {
            Child::Comment(value) => {
                let $node = value.read();
                $body
            }
            Child::BlockComment(value) => {
                let $node = value.read();
                $body
            }
            Child::Null(value) => {
                let $node = value.read();
                $body
            }
            Child::Suspension(value) => {
                let $node = value.read();
                $body
            }
            Child::Boolean(value) => {
                let $node = value.read();
                $body
            }
            Child::Number(value) => {
                let $node = value.read();
                $body
            }
            Child::Array(value) => {
                let $node = value.read();
                $body
            }
            Child::Map(value) => {
                let $node = value.read();
                $body
            }
            Child::Key(value) => {
                let $node = value.read();
                $body
            }

            Child::Import(value) => {
                let $node = value.read();
                $body
            }
            Child::KeyPath(value) => {
                let $node = value.read();
                $body
            }
            Child::Edge(value) => {
                let $node = value.read();
                $body
            }
            Child::EdgeIndex(value) => {
                let $node = value.read();
                $body
            }
            Child::Unquoted(value) => {
                let $node = value.read();
                $body
            }
            Child::DoubleQuoted(value) => {
                let $node = value.read();
                $body
            }
            Child::SingleQuoted(value) => {
                let $node = value.read();
                $body
            }
            Child::Block(value) => {
                let $node = value.read();
                $body
            }
            Child::Substitution(value) => {
                let $node = value.read();
                $body
            }
        }
    };
}
impl Serialize for Child {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        dispatch_child!(self, node, node.serialize(serializer))
    }
}

impl Node for Child {
    fn node_type(&self) -> Cow<'_, str> {
        dispatch_child!(self, node, Cow::Owned(node.node_type().into_owned()))
    }
    fn range(&self) -> Range {
        dispatch_child!(self, node, node.range())
    }
    fn children(&self) -> Vec<Child> {
        dispatch_child!(self, node, node.children())
    }
}

/// JSON box fields remain independent; Unbox uses upstream precedence.
#[derive(Clone, Debug, Default, Serialize)]
pub struct StringBox {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unquoted_string: Option<Shared<UnquotedString>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub double_quoted_string: Option<Shared<DoubleQuotedString>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub single_quoted_string: Option<Shared<SingleQuotedString>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_string: Option<Shared<BlockString>>,
}
impl StringBox {
    /// Inspect the selected scalar without copying its shared storage.
    /// The callback cannot retain a borrow beyond the AST read guards.
    pub fn with_scalar_bytes<R>(&self, inspect: impl FnOnce(&[u8]) -> R) -> R {
        macro_rules! interpolated {
            ($node:expr) => {{
                let node = $node.read();
                if let Some(slot) = node.value.as_ref().and_then(|parts| parts.get(0)) {
                    let slot = slot.read();
                    if let Some(text) = &slot.string {
                        return inspect(text.read().as_bytes());
                    }
                }
                return inspect(&[]);
            }};
        }
        if let Some(node) = &self.unquoted_string {
            interpolated!(node);
        }
        if let Some(node) = &self.double_quoted_string {
            interpolated!(node);
        }
        if let Some(node) = &self.single_quoted_string {
            return inspect(node.read().value.as_bytes());
        }
        if let Some(node) = &self.block_string {
            return inspect(node.read().value.as_bytes());
        }
        panic!("empty AST StringBox")
    }
    pub fn scalar_bytes(&self) -> Vec<u8> {
        self.with_scalar_bytes(<[u8]>::to_vec)
    }
    /// Scalar text of the selected string; an empty box is invalid here.
    pub fn scalar_string(&self) -> String {
        // d2ast.StringBox.ScalarString reads the selected string's storage.
        // In particular, interpolated keys need no temporary scalar_bytes
        // copy. Preserve ByteString's per-invalid-byte presentation on the
        // uncommon invalid UTF-8 path rather than using from_utf8_lossy.
        self.with_scalar_bytes(|bytes| match std::str::from_utf8(bytes) {
            Ok(text) => text.to_owned(),
            Err(_) => ByteString::from(bytes).to_text().into_owned(),
        })
    }
    pub fn unbox(&self) -> Option<Child> {
        self.unquoted_string
            .clone()
            .map(Child::Unquoted)
            .or_else(|| self.double_quoted_string.clone().map(Child::DoubleQuoted))
            .or_else(|| self.single_quoted_string.clone().map(Child::SingleQuoted))
            .or_else(|| self.block_string.clone().map(Child::Block))
    }
}

/// Pinned default special-character sets for unquoted source.
pub const UNQUOTED_KEY_SPECIALS: &str = "#;\n\\{}[]'\"|:.-<>*&()@&";
pub const UNQUOTED_VALUE_SPECIALS: &str = "#;\n\\{}[]'\"|$@";

/// Construct the source-preferred string representation for a key or value.
/// This translates d2ast.RawStringBox, including its trailing-hyphen behavior.
pub fn raw_string_box(value: impl Into<ByteString>, in_key: bool) -> StringBox {
    raw_string_box_with_specials(
        value,
        in_key,
        UNQUOTED_KEY_SPECIALS,
        UNQUOTED_VALUE_SPECIALS,
    )
}
/// Explicit equivalents of upstream's caller-customizable specials sets.
pub fn raw_string_box_with_specials(
    value: impl Into<ByteString>,
    in_key: bool,
    key_specials: &str,
    value_specials: &str,
) -> StringBox {
    let value = value.into();
    let double = |value| StringBox {
        double_quoted_string: Some(Shared::new(DoubleQuotedString::flat(value))),
        ..Default::default()
    };
    let single = |value| StringBox {
        single_quoted_string: Some(Shared::new(SingleQuotedString {
            value,
            ..Default::default()
        })),
        ..Default::default()
    };
    let text = value.to_text();
    if text.is_empty() {
        return double(value);
    }
    if in_key {
        for (index, rune) in text.char_indices() {
            if rune == '-'
                && text
                    .as_bytes()
                    .get(index + 1)
                    .is_some_and(|next| *next != b'-')
            {
                continue;
            }
            if key_specials.contains(rune) {
                return if !text.contains('"') || text.contains('\n') {
                    double(value)
                } else {
                    single(value)
                };
            }
        }
    } else if matches!(text.as_ref(), "null" | "suspend" | "unsuspend")
        || text.chars().any(|rune| value_specials.contains(rune))
    {
        return if (!text.contains('"') && !text.contains('$')) || text.contains('\n') {
            double(value)
        } else {
            single(value)
        };
    }
    // Go unicode.IsSpace's Latin-1 and White_Space ranges. Do not substitute
    // trim rules that may include additional zero-width or formatting characters.
    let is_space = |rune| matches!(rune, '\u{9}'..='\u{d}' | ' ' | '\u{85}' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}'..='\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}');
    if text.chars().next().is_some_and(is_space) || text.chars().next_back().is_some_and(is_space) {
        return double(value);
    }
    StringBox {
        unquoted_string: Some(Shared::new(UnquotedString::flat(value))),
        ..Default::default()
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Substitution {
    pub range: Range,
    pub spread: bool,
    pub path: Option<SharedSlice<Option<Shared<StringBox>>>>,
}
impl Node for Substitution {
    fn node_type(&self) -> Cow<'_, str> {
        "substitution".into()
    }
    fn range(&self) -> Range {
        self.range.clone()
    }
    fn children(&self) -> Vec<Child> {
        self.path
            .iter()
            .flat_map(SharedSlice::iter)
            .filter_map(|slot| slot.read().as_ref().and_then(|value| value.read().unbox()))
            .collect()
    }
}
impl MapNode for Substitution {}
impl ArrayNode for Substitution {}

#[derive(Clone, Debug, Default, Serialize)]
pub struct InterpolationBox {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub string: Option<Shared<ByteString>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_string: Option<Shared<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substitution: Option<Shared<Substitution>>,
}
impl InterpolationBox {
    pub fn literal(value: impl Into<ByteString>) -> Self {
        Self {
            string: Some(Shared::new(value.into())),
            ..Default::default()
        }
    }
}
fn pattern_empty(value: &Option<SharedSlice<ByteString>>) -> bool {
    value.as_ref().is_none_or(SharedSlice::is_empty)
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct UnquotedString {
    pub range: Range,
    pub value: Option<SharedSlice<InterpolationBox>>,
    #[serde(skip_serializing_if = "pattern_empty")]
    pub pattern: Option<SharedSlice<ByteString>>,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct DoubleQuotedString {
    pub range: Range,
    pub value: Option<SharedSlice<InterpolationBox>>,
}

macro_rules! interpolated_string {
    ($kind:ty, $name:literal, $unquoted:literal) => {
        impl Node for $kind {
            fn node_type(&self) -> Cow<'_, str> {
                $name.into()
            }
            fn range(&self) -> Range {
                self.range.clone()
            }
            fn children(&self) -> Vec<Child> {
                self.value
                    .iter()
                    .flat_map(SharedSlice::iter)
                    .filter_map(|slot| slot.read().substitution.clone().map(Child::Substitution))
                    .collect()
            }
        }
        impl ArrayNode for $kind {}
        impl Value for $kind {}
        impl Scalar for $kind {
            fn scalar_bytes(&self) -> Cow<'_, [u8]> {
                self.value
                    .as_ref()
                    .and_then(|parts| parts.get(0))
                    .and_then(|slot| {
                        slot.read()
                            .string
                            .as_ref()
                            .map(|text| text.read().as_bytes().to_vec())
                    })
                    .unwrap_or_default()
                    .into()
            }
            fn scalar_string(&self) -> Cow<'_, str> {
                ByteString::from(self.scalar_bytes().into_owned())
                    .to_text()
                    .into_owned()
                    .into()
            }
        }
        impl StringNode for $kind {
            fn set_string(&mut self, value: String) {
                self.value = Some(vec![InterpolationBox::literal(value)].into());
            }
            fn is_unquoted(&self) -> bool {
                $unquoted
            }
        }
        impl $kind {
            pub fn flat(value: impl Into<ByteString>) -> Self {
                Self {
                    value: Some(vec![InterpolationBox::literal(value)].into()),
                    ..Default::default()
                }
            }
            pub fn coalesce(&mut self) {
                let mut text = Vec::new();
                for slot in self.value.iter().flat_map(SharedSlice::iter) {
                    let part = slot.read();
                    let Some(literal) = &part.string else { break };
                    text.extend_from_slice(literal.read().as_bytes());
                }
                self.value = Some(vec![InterpolationBox::literal(text)].into());
            }
        }
    };
}
interpolated_string!(UnquotedString, "unquoted string", true);
interpolated_string!(DoubleQuotedString, "double quoted string", false);

impl Child {
    /// Identity of the underlying node, independent of its contents.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Comment(a), Self::Comment(b)) => a.ptr_eq(b),
            (Self::BlockComment(a), Self::BlockComment(b)) => a.ptr_eq(b),
            (Self::Null(a), Self::Null(b)) => a.ptr_eq(b),
            (Self::Suspension(a), Self::Suspension(b)) => a.ptr_eq(b),
            (Self::Boolean(a), Self::Boolean(b)) => a.ptr_eq(b),
            (Self::Number(a), Self::Number(b)) => a.ptr_eq(b),
            (Self::Array(a), Self::Array(b)) => a.ptr_eq(b),
            (Self::Map(a), Self::Map(b)) => a.ptr_eq(b),
            (Self::Key(a), Self::Key(b)) => a.ptr_eq(b),
            (Self::Import(a), Self::Import(b)) => a.ptr_eq(b),
            (Self::KeyPath(a), Self::KeyPath(b)) => a.ptr_eq(b),
            (Self::Edge(a), Self::Edge(b)) => a.ptr_eq(b),
            (Self::EdgeIndex(a), Self::EdgeIndex(b)) => a.ptr_eq(b),
            (Self::Unquoted(a), Self::Unquoted(b)) => a.ptr_eq(b),
            (Self::DoubleQuoted(a), Self::DoubleQuoted(b)) => a.ptr_eq(b),
            (Self::SingleQuoted(a), Self::SingleQuoted(b)) => a.ptr_eq(b),
            (Self::Block(a), Self::Block(b)) => a.ptr_eq(b),
            (Self::Substitution(a), Self::Substitution(b)) => a.ptr_eq(b),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_text_preserves_raw_bytes_precedence_and_live_shared_slots() {
        for bytes in [
            b"shape".as_slice(),
            "\u{212a}\u{17f}\u{fffd}".as_bytes(),
            b"\xff\xfe",
            b"\xe2\x82",
            b"a\xf0\x9f\x92z",
            b"",
        ] {
            let text = ByteString::from(bytes);
            let expected = text.to_text();
            let unquoted = StringBox {
                unquoted_string: Some(Shared::new(UnquotedString::flat(bytes))),
                ..Default::default()
            };
            let quoted = StringBox {
                double_quoted_string: Some(Shared::new(DoubleQuotedString::flat(bytes))),
                ..Default::default()
            };
            let single = StringBox {
                single_quoted_string: Some(Shared::new(SingleQuotedString {
                    value: bytes.into(),
                    ..Default::default()
                })),
                ..Default::default()
            };
            let block = StringBox {
                block_string: Some(Shared::new(BlockString {
                    value: bytes.into(),
                    ..Default::default()
                })),
                ..Default::default()
            };
            for boxed in [&unquoted, &quoted, &single, &block] {
                assert_eq!(boxed.scalar_string(), expected);
                assert_eq!(boxed.scalar_bytes(), bytes);
            }
        }

        let shared = Shared::new(UnquotedString {
            value: Some(
                vec![
                    InterpolationBox::literal("first"),
                    InterpolationBox::literal("second"),
                ]
                .into(),
            ),
            ..Default::default()
        });
        let boxed = StringBox {
            unquoted_string: Some(shared.clone()),
            single_quoted_string: Some(Shared::new(SingleQuotedString {
                value: "other variant".into(),
                ..Default::default()
            })),
            ..Default::default()
        };
        assert_eq!(boxed.scalar_string(), "first");
        let slot = shared.read().value.as_ref().unwrap().get(0).unwrap();
        *slot.write().string.as_ref().unwrap().write() = "changed".into();
        assert_eq!(boxed.scalar_string(), "changed");
        slot.write().string = None;
        assert_eq!(boxed.scalar_string(), "");
        shared.write().value = None;
        assert_eq!(boxed.scalar_string(), "");
    }

    #[test]
    fn raw_string_selection_distinguishes_key_and_value_rules() {
        let kind = |text, key| {
            raw_string_box(text, key)
                .unbox()
                .unwrap()
                .node_type()
                .into_owned()
        };
        assert_eq!(kind("a-b", true), "unquoted string");
        assert_eq!(kind("a-", true), "double quoted string");
        assert_eq!(kind("a--b", true), "double quoted string");
        assert_eq!(kind("$x", true), "unquoted string");
        assert_eq!(kind("$x", false), "single quoted string");
        assert_eq!(kind("null", false), "double quoted string");
        assert_eq!(kind("null", true), "unquoted string");
        assert_eq!(kind("\u{85}x", false), "double quoted string");
        assert_eq!(kind("\u{200b}x", false), "unquoted string");
    }

    #[test]
    fn copying_shares_slots_but_set_string_replaces_only_the_copy() {
        let slots: SharedSlice<_> = vec![10, 20, 30].into();
        let suffix = slots.suffix(1);
        *suffix.get(0).unwrap().write() = 40;
        assert_eq!(*slots.get(1).unwrap().read(), 40);
        assert_eq!(slots.len(), 3);
        assert_eq!(suffix.len(), 2);
        assert!(slots.suffix(3).is_empty());
        let original = UnquotedString::flat("original");
        let mut copied = original.clone();
        copied
            .value
            .as_ref()
            .unwrap()
            .get(0)
            .unwrap()
            .write()
            .string = Some(Shared::new("shared".into()));
        assert_eq!(original.scalar_string(), "shared");
        copied.set_string("detached".into());
        assert_eq!(original.scalar_string(), "shared");
        assert_eq!(copied.scalar_string(), "detached");
    }

    #[test]
    fn mixed_box_keeps_child_identity_after_parent_replacement() {
        let substitution = Shared::new(Substitution::default());
        let mut node = UnquotedString {
            value: Some(
                vec![
                    InterpolationBox {
                        substitution: Some(substitution.clone()),
                        ..InterpolationBox::literal("a")
                    },
                    InterpolationBox::literal("b"),
                ]
                .into(),
            ),
            pattern: Some(vec!["*".into()].into()),
            ..Default::default()
        };
        let children = node.children();
        assert_eq!(node.scalar_string(), "a");
        node.coalesce();
        assert_eq!(node.scalar_string(), "ab");
        assert!(node.children().is_empty());
        assert_eq!(node.pattern.as_ref().unwrap().len(), 1);
        drop(node);
        substitution.write().range.path = "changed.d2".into();
        assert_eq!(children[0].range().path, "changed.d2");
    }

    #[test]
    fn substitution_skips_null_boxes_and_preserves_unbox_precedence() {
        let boxed = Shared::new(StringBox {
            unquoted_string: Some(Shared::new(UnquotedString::flat("first"))),
            single_quoted_string: Some(Shared::new(SingleQuotedString::default())),
            ..Default::default()
        });
        let substitution = Substitution {
            path: Some(vec![None, Some(Shared::new(StringBox::default())), Some(boxed)].into()),
            ..Default::default()
        };
        let children = substitution.children();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].node_type(), "unquoted string");
    }
}
