// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Map keys, edge groups and indices from pinned d2parser/parse.go.
use super::*;
use d2_rs_core::ast::{Edge, EdgeIndex, Key, KeyPath, Shared, SharedSlice};
fn has_key(k: &Key) -> bool {
    k.key.is_some() || k.edges.as_ref().is_some_and(|e| !e.is_empty())
}
impl<R: RuneReader> Parser<R> {
    pub fn parse_map_key(&mut self) -> Option<Key> {
        let mut key = Key {
            range: Range {
                path: self.path.clone(),
                start: self.position,
                ..Default::default()
            },
            ..Default::default()
        };
        self.map_key_contents(&mut key);
        key.range.end = self.position;
        has_key(&key).then_some(key)
    }
    fn map_key_contents(&mut self, key: &mut Key) {
        let Some(r) = self.peek() else {
            return;
        };
        if r == '!' {
            let Some(r) = self.peek() else {
                return;
            };
            if r == '&' {
                self.commit();
                key.not_ampersand = true;
            } else {
                self.rewind();
            }
        } else if r == '&' {
            self.commit();
            key.ampersand = true;
        } else {
            self.rewind();
        }
        let Some(r) = self.peek() else {
            return;
        };
        if r == '(' {
            self.commit();
            self.parse_edge_group(key);
            return;
        }
        self.rewind();
        let path = self.parse_key().map(Shared::new);
        key.key = path.clone();
        let (r, newlines) = self.peek_not_space();
        let Some(r) = r else {
            return;
        };
        if newlines > 0 {
            self.rewind();
            return;
        }
        match r {
            '(' => {
                self.commit();
                self.parse_edge_group(key);
            }
            '<' | '>' | '-' => {
                self.rewind();
                key.key = None;
                self.parse_edges(key, path);
                self.parse_map_key_value(key);
            }
            _ => {
                self.rewind();
                self.parse_map_key_value(key);
            }
        }
    }
    fn parse_map_key_value(&mut self, key: &mut Key) {
        let (r, newlines) = self.peek_not_space();
        let Some(r) = r else {
            return;
        };
        if newlines > 0 {
            self.rewind();
            return;
        }
        match r {
            '{' => {
                self.rewind();
                if !has_key(key) {
                    return;
                }
            }
            ':' => {
                self.commit();
                if !has_key(key) {
                    self.error(key.range.start, self.position, "map value without key");
                }
            }
            _ => {
                self.rewind();
                return;
            }
        }
        key.value = self.parse_value();
        if key.value.unbox().is_none() {
            self.error(
                self.position.subtract(':', self.utf16_positions),
                self.position,
                "missing value after colon",
            );
        }
        let scalar = key.value.scalar_box();
        if scalar.unbox().is_some() {
            let (r, newlines) = self.peek_not_space();
            if newlines > 0 || r != Some('{') {
                self.rewind();
                return;
            }
            self.commit();
            self.replay('{');
            key.primary = scalar;
            key.value = self.parse_value();
        }
    }
    fn parse_edge_group(&mut self, key: &mut Key) {
        self.in_edge_group = true;
        self.edge_group_contents(key);
        self.in_edge_group = false;
    }
    fn edge_group_contents(&mut self, key: &mut Key) {
        let src = self.parse_key().map(Shared::new);
        self.parse_edges(key, src);
        let (r, newlines) = self.peek_not_space();
        if r.is_none() || newlines > 0 {
            self.rewind();
            return;
        }
        if r != Some(')') {
            self.rewind();
            self.error(
                key.range.start,
                self.position,
                "edge groups must be terminated with )",
            );
            return;
        }
        self.commit();
        let (r, newlines) = self.peek_not_space();
        if r.is_none() || newlines > 0 {
            self.rewind();
            return;
        }
        if r == Some('[') {
            self.commit();
            key.edge_index = self.parse_edge_index().map(Shared::new);
        } else {
            self.rewind();
        }
        let (r, newlines) = self.peek_not_space();
        if r.is_none() || newlines > 0 {
            self.rewind();
            return;
        }
        if r == Some('.') {
            self.commit();
            key.edge_key = self.parse_key().map(Shared::new);
        } else {
            self.rewind();
        }
        self.in_edge_group = false;
        self.parse_map_key_value(key);
    }
    pub fn parse_edge_index(&mut self) -> Option<EdgeIndex> {
        let mut index = EdgeIndex {
            range: Range {
                path: self.path.clone(),
                start: self.position.subtract('[', self.utf16_positions),
                ..Default::default()
            },
            ..Default::default()
        };
        let valid = self.edge_index_contents(&mut index);
        index.range.end = self.position;
        valid.then_some(index)
    }
    fn edge_index_contents(&mut self, index: &mut EdgeIndex) -> bool {
        let (r, newlines) = self.peek_not_space();
        let Some(r) = r else {
            self.rewind();
            return false;
        };
        if newlines > 0 {
            self.rewind();
            return false;
        }
        if unicode::is_digit(r) {
            self.commit();
            let mut digits = r.to_string();
            loop {
                let (r, newlines) = self.peek_not_space();
                if r.is_none() || newlines > 0 {
                    self.rewind();
                    self.error(index.range.start, self.position, "unterminated edge index");
                    return false;
                }
                let r = r.unwrap();
                if r == ']' {
                    self.rewind();
                    break;
                }
                self.commit();
                if !unicode::is_digit(r) {
                    self.error(
                        self.position.subtract(r, self.utf16_positions),
                        self.position,
                        "unexpected character in edge index",
                    );
                    continue;
                }
                digits.push(r);
            }
            let value = digits.parse::<i64>().unwrap_or_else(|error| {
                if *error.kind() == std::num::IntErrorKind::PosOverflow {
                    i64::MAX
                } else {
                    0
                }
            });
            index.int = Some(Shared::new(value));
        } else if r == '*' {
            self.commit();
            index.glob = true;
        } else {
            self.error(
                self.position.subtract(r, self.utf16_positions),
                self.position,
                "unexpected character in edge index",
            );
        }
        let (r, newlines) = self.peek_not_space();
        if r != Some(']') || newlines > 0 {
            self.rewind();
            self.error(index.range.start, self.position, "unterminated edge index");
        } else {
            self.commit();
        }
        true
    }
    fn parse_edges(&mut self, key: &mut Key, mut src: Option<Shared<KeyPath>>) {
        let mut edges: Vec<_> = key
            .edges
            .iter()
            .flat_map(SharedSlice::iter)
            .map(|e| e.read().clone())
            .collect();
        loop {
            let mut edge = Edge {
                range: Range {
                    path: self.path.clone(),
                    start: src.as_ref().map_or(self.position, |s| s.read().range.start),
                    ..Default::default()
                },
                src: src.clone(),
                ..Default::default()
            };
            let (r, newlines) = self.peek_not_space();
            let Some(r) = r else {
                break;
            };
            if newlines > 0 {
                self.rewind();
                break;
            }
            if matches!(r, '<' | '*') {
                edge.src_arrow = r.to_string();
            } else if r != '-' {
                self.rewind();
                break;
            }
            if src.is_none() {
                let start = self.lookahead_position.subtract(r, self.utf16_positions);
                self.error(start, self.lookahead_position, "connection missing source");
                edge.range.start = start;
            }
            self.commit();
            let valid = self.edge_contents(&mut edge);
            edge.range.end = self.position;
            if !valid {
                break;
            }
            let dst = self.parse_key().map(Shared::new);
            if let Some(dst) = &dst {
                edge.dst = Some(dst.clone());
                edge.range.end = dst.read().range.end;
            } else {
                self.error(
                    edge.range.start,
                    self.position,
                    "connection missing destination",
                );
            }
            edges.push(Some(Shared::new(edge)));
            src = dst;
        }
        if !edges.is_empty() || key.edges.is_some() {
            key.edges = Some(edges.into());
        }
    }
    fn edge_contents(&mut self, edge: &mut Edge) -> bool {
        loop {
            let Some(r) = self.peek() else {
                self.error(
                    edge.range.start,
                    self.reader_position,
                    "unterminated connection",
                );
                return false;
            };
            match r {
                '>' | '*' => {
                    edge.dst_arrow = r.to_string();
                    self.commit();
                    return true;
                }
                '\\' => {
                    self.commit();
                    let (r, newlines) = self.peek_not_space();
                    let Some(r) = r else {
                        continue;
                    };
                    if newlines == 0 {
                        self.rewind();
                        self.error(
                            edge.range.start,
                            self.reader_position,
                            "only newline escapes are allowed in connections",
                        );
                        return false;
                    }
                    if newlines > 1 {
                        self.rewind();
                        continue;
                    }
                    self.commit();
                    self.replay(r);
                }
                '-' => self.commit(),
                _ => {
                    self.rewind();
                    return true;
                }
            }
        }
    }
}
