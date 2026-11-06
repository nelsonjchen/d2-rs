// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Key-path records and helpers translated from pinned d2ast/d2ast.go.
use super::*;
use serde::Serialize;
use std::borrow::Cow;

#[derive(Debug, Default, Serialize)]
pub struct KeyPath {
    pub range: Range,
    pub path: Option<SharedSlice<Option<Shared<StringBox>>>>,
}

/// Source KeyPath.Copy duplicates path slots but retains the StringBox pointers.
impl Clone for KeyPath {
    fn clone(&self) -> Self {
        Self {
            range: self.range.clone(),
            path: self
                .path
                .as_ref()
                .filter(|path| !path.is_empty())
                .map(|path| {
                    path.iter()
                        .map(|slot| slot.read().clone())
                        .collect::<Vec<_>>()
                        .into()
                }),
        }
    }
}
impl Node for KeyPath {
    fn node_type(&self) -> Cow<'_, str> {
        "key path".into()
    }
    fn range(&self) -> Range {
        self.range.clone()
    }
    fn children(&self) -> Vec<Child> {
        self.path
            .iter()
            .flat_map(SharedSlice::iter)
            .filter_map(|slot| slot.read().as_ref().and_then(|boxed| boxed.read().unbox()))
            .collect()
    }
}
impl KeyPath {
    /// Rebuild path spelling from string-node scalar values, discarding the
    /// original quote forms and ranges as upstream MakeKeyPathString does.
    pub fn from_string_nodes<'a>(values: impl IntoIterator<Item = &'a dyn StringNode>) -> Self {
        Self::from_strings(
            values
                .into_iter()
                .map(|value| value.scalar_bytes().into_owned()),
        )
    }
    pub fn from_strings(values: impl IntoIterator<Item = impl Into<super::ByteString>>) -> Self {
        let path = values
            .into_iter()
            .map(|value| Some(Shared::new(raw_string_box(value, true))))
            .collect::<Vec<_>>();
        Self {
            path: (!path.is_empty()).then(|| path.into()),
            ..Default::default()
        }
    }

    /// Preserve nil output for empty paths. Empty boxes yield nil entries;
    /// nil box pointers are invalid, as in the upstream IDA helper.
    pub fn ida(&self) -> Option<Vec<Option<Child>>> {
        self.path
            .as_ref()
            .filter(|path| !path.is_empty())
            .map(|path| {
                path.iter()
                    .map(|slot| {
                        slot.read()
                            .as_ref()
                            .expect("nil key-path StringBox")
                            .read()
                            .unbox()
                    })
                    .collect()
            })
    }
    pub fn string_ida(&self) -> Option<Vec<String>> {
        self.path
            .as_ref()
            .filter(|path| !path.is_empty())
            .map(|path| {
                path.iter()
                    .map(|slot| {
                        slot.read()
                            .as_ref()
                            .expect("nil key-path StringBox")
                            .read()
                            .scalar_string()
                    })
                    .collect()
            })
    }
    pub fn last(&self) -> Option<Shared<StringBox>> {
        let path = self.path.as_ref().expect("empty key path");
        path.get(path.len().checked_sub(1).expect("empty key path"))
            .unwrap()
            .read()
            .clone()
    }
    pub fn first_glob(&self) -> isize {
        self.path
            .iter()
            .flat_map(SharedSlice::iter)
            .enumerate()
            .find_map(|(index, slot)| {
                let boxed = slot.read();
                let boxed = boxed.as_ref().expect("nil key-path StringBox").read();
                boxed
                    .unquoted_string
                    .as_ref()
                    .filter(|node| {
                        node.read()
                            .pattern
                            .as_ref()
                            .is_some_and(|pattern| !pattern.is_empty())
                    })
                    .map(|_| index as isize)
            })
            .unwrap_or(-1)
    }
    pub fn has_glob(&self) -> bool {
        self.first_glob() != -1
    }
    fn has_pattern(&self, predicate: impl Fn(&[super::ByteString]) -> bool) -> bool {
        self.path.iter().flat_map(SharedSlice::iter).any(|slot| {
            let boxed = slot.read();
            let boxed = boxed.as_ref().expect("nil key-path StringBox").read();
            boxed.unquoted_string.as_ref().is_some_and(|node| {
                let pattern = node
                    .read()
                    .pattern
                    .as_ref()
                    .map(|pattern| {
                        pattern
                            .iter()
                            .map(|slot| slot.read().clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                predicate(&pattern)
            })
        })
    }
    pub fn has_triple_glob(&self) -> bool {
        self.has_pattern(is_triple_glob)
    }
    pub fn has_multi_glob(&self) -> bool {
        self.has_pattern(|pattern| is_double_glob(pattern) || is_triple_glob(pattern))
    }
    pub fn equals(&self, other: &Self) -> bool {
        if self.path.as_ref().map_or(0, SharedSlice::len)
            != other.path.as_ref().map_or(0, SharedSlice::len)
        {
            return false;
        }
        self.path
            .iter()
            .flat_map(SharedSlice::iter)
            .zip(other.path.iter().flat_map(SharedSlice::iter))
            .all(|(a, b)| {
                // Drop each read guard before looking up the other side: it can
                // point to the same box, and shared record access stays bounded.
                let a = a
                    .read()
                    .as_ref()
                    .expect("nil key-path StringBox")
                    .read()
                    .scalar_bytes();
                let b = b
                    .read()
                    .as_ref()
                    .expect("nil key-path StringBox")
                    .read()
                    .scalar_bytes();
                a == b
            })
    }
}

pub fn is_double_glob<T: AsRef<[u8]>>(pattern: &[T]) -> bool {
    pattern.len() == 3
        && pattern
            .iter()
            .zip([b"*".as_slice(), b"", b"*"])
            .all(|(a, b)| a.as_ref() == b)
}
pub fn is_triple_glob<T: AsRef<[u8]>>(pattern: &[T]) -> bool {
    pattern.len() == 5
        && pattern
            .iter()
            .zip([b"*".as_slice(), b"", b"*", b"", b"*"])
            .all(|(a, b)| a.as_ref() == b)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn key_path_copy_detaches_slots_but_shares_boxes() {
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
        assert_eq!(original.string_ida().unwrap(), ["a", "b"]);
        copied
            .last()
            .unwrap()
            .write()
            .unquoted_string
            .as_ref()
            .unwrap()
            .write()
            .set_string("shared".into());
        assert_eq!(original.string_ida().unwrap(), ["a", "shared"]);
        assert_eq!(copied.string_ida().unwrap(), ["c", "shared"]);
    }
    #[test]
    fn glob_queries_use_patterns_not_literal_asterisks() {
        let path = KeyPath::from_strings(["a", "*"]);
        assert!(!path.has_glob());
        let boxed = path.last().unwrap();
        boxed.write().unquoted_string = Some(Shared::new(UnquotedString {
            pattern: Some(vec!["*".into(), "".into(), "*".into()].into()),
            ..Default::default()
        }));
        assert_eq!(path.first_glob(), 1);
        assert!(path.has_multi_glob());
        assert!(!path.has_triple_glob());
    }
}
