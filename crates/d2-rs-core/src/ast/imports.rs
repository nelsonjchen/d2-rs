// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Import and substitution helpers from pinned d2ast/d2ast.go.
use super::*;
use serde::Serialize;
use std::borrow::Cow;

#[derive(Clone, Debug, Default, Serialize)]
pub struct Import {
    pub range: Range,
    pub spread: bool,
    pub pre: String,
    pub path: Option<SharedSlice<Option<Shared<StringBox>>>>,
}
impl Node for Import {
    fn node_type(&self) -> Cow<'_, str> {
        "import".into()
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
impl MapNode for Import {}
impl ArrayNode for Import {}
impl Value for Import {}

impl Substitution {
    pub fn ida(&self) -> Option<Vec<String>> {
        self.path
            .as_ref()
            .filter(|path| !path.is_empty())
            .map(|path| {
                path.iter()
                    .map(|slot| {
                        slot.read()
                            .as_ref()
                            .expect("nil substitution StringBox")
                            .read()
                            .scalar_string()
                    })
                    .collect()
            })
    }
}
impl Import {
    /// Identifier components following the file path. An empty path is invalid.
    pub fn ida(&self) -> Option<Vec<Option<Child>>> {
        let path = self.path.as_ref().expect("empty import path");
        assert!(!path.is_empty(), "empty import path");
        (path.len() > 1).then(|| {
            path.iter()
                .skip(1)
                .map(|slot| {
                    slot.read()
                        .as_ref()
                        .expect("nil import StringBox")
                        .read()
                        .unbox()
                })
                .collect()
        })
    }
    pub fn path_with_pre(&self) -> String {
        let Some(first) = self.path.as_ref().and_then(|path| path.get(0)) else {
            return String::new();
        };
        let file = first
            .read()
            .as_ref()
            .expect("nil import StringBox")
            .read()
            .scalar_string();
        if self.pre.is_empty() && file.is_empty() {
            return String::new();
        }
        let joined = if self.pre.is_empty() {
            file
        } else if file.is_empty() {
            self.pre.clone()
        } else {
            format!("{}/{file}", self.pre)
        };
        clean_path(&joined)
    }
    pub fn dir(&self) -> String {
        let path = self.path_with_pre();
        clean_path(path.rfind('/').map_or("", |index| &path[..=index]))
    }
}

// Go path.Clean semantics: slash-separated on every host, with lexical dot
// reduction and no filesystem/symlink lookup. A later '/' does not reset Join.
fn clean_path(path: &str) -> String {
    let rooted = path.starts_with('/');
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|last| *last != "..") => {
                parts.pop();
            }
            ".." if rooted => {}
            _ => parts.push(part),
        }
    }
    let result = parts.join("/");
    if rooted {
        format!("/{result}")
    } else if result.is_empty() {
        ".".into()
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn import(pre: &str, file: &str) -> Import {
        Import {
            pre: pre.into(),
            path: KeyPath::from_strings([file, "identifier"]).path,
            ..Default::default()
        }
    }
    #[test]
    fn import_paths_use_slash_join_not_platform_path_replacement() {
        for (pre, file, path, dir) in [
            ("a", "/b", "a/b", "a"),
            ("../a", "../../b", "../../b", "../.."),
            ("/a", "../../b", "/b", "/"),
            ("", "", "", "."),
            ("", "a\\b", "a\\b", "."),
            ("a//", "./b/../c", "a/c", "a"),
        ] {
            let value = import(pre, file);
            assert_eq!(value.path_with_pre(), path);
            assert_eq!(value.dir(), dir);
            assert_eq!(value.ida().unwrap().len(), 1);
        }
        assert_eq!(
            Import {
                pre: "ignored".into(),
                ..Default::default()
            }
            .path_with_pre(),
            ""
        );
    }
}
