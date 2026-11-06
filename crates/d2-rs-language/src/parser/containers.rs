// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Map/array recovery and value dispatch from pinned d2parser/parse.go.
use super::*;
use d2_rs_core::ast::{
    Array, ArrayNodeBox, Boolean, Child, Map, MapNodeBox, Node, Null, Number, Scalar, Shared,
    Suspension, ValueBox,
};

fn equal_keyword(value: &str, keyword: &str) -> bool {
    value
        .chars()
        .map(|r| {
            if r == 'ſ' {
                's'
            } else {
                r.to_ascii_lowercase()
            }
        })
        .eq(keyword.chars())
}
fn quote_rune(r: char) -> String {
    let body = match r {
        '\u{7}' => "\\a".into(),
        '\u{8}' => "\\b".into(),
        '\u{c}' => "\\f".into(),
        '\n' => "\\n".into(),
        '\r' => "\\r".into(),
        '\t' => "\\t".into(),
        '\u{b}' => "\\v".into(),
        '\\' => "\\\\".into(),
        '\'' => "\\'".into(),
        _ if unicode::is_print(r) => r.to_string(),
        _ if (r as u32) < 0x20 || r == '\u{7f}' => format!("\\x{:02x}", r as u32),
        _ if (r as u32) < 0x10000 => format!("\\u{:04x}", r as u32),
        _ => format!("\\U{:08x}", r as u32),
    };
    format!("'{body}'")
}
impl<R: RuneReader> Parser<R> {
    pub fn parse_value(&mut self) -> ValueBox {
        let (r, newlines) = self.peek_not_space();
        if r.is_none() || newlines > 0 {
            self.rewind();
            return ValueBox::default();
        }
        let r = r.unwrap();
        self.commit();
        match r {
            '[' => {
                return ValueBox {
                    array: Some(Shared::new(self.parse_array())),
                    ..Default::default()
                };
            }
            '{' => {
                return ValueBox {
                    map: Some(Shared::new(self.parse_map(false))),
                    ..Default::default()
                };
            }
            '@' => {
                return ValueBox {
                    import: Some(Shared::new(self.parse_import(false))),
                    ..Default::default()
                };
            }
            _ => {}
        }
        self.replay(r);
        let strings = self.parse_string(false);
        if strings.double_quoted_string.is_some() {
            return ValueBox {
                double_quoted_string: strings.double_quoted_string,
                ..Default::default()
            };
        }
        if strings.single_quoted_string.is_some() {
            return ValueBox {
                single_quoted_string: strings.single_quoted_string,
                ..Default::default()
            };
        }
        if strings.block_string.is_some() {
            return ValueBox {
                block_string: strings.block_string,
                ..Default::default()
            };
        }
        let Some(unquoted) = strings.unquoted_string else {
            return ValueBox::default();
        };
        let text = unquoted.read().scalar_string().into_owned();
        let range = unquoted.read().range.clone();
        if equal_keyword(&text, "null") {
            return ValueBox {
                null: Some(Shared::new(Null { range })),
                ..Default::default()
            };
        }
        if equal_keyword(&text, "suspend") || equal_keyword(&text, "unsuspend") {
            return ValueBox {
                suspension: Some(Shared::new(Suspension {
                    range,
                    value: equal_keyword(&text, "suspend"),
                })),
                ..Default::default()
            };
        }
        if equal_keyword(&text, "true") || equal_keyword(&text, "false") {
            return ValueBox {
                boolean: Some(Shared::new(Boolean {
                    range,
                    value: equal_keyword(&text, "true"),
                })),
                ..Default::default()
            };
        }
        if let Some(value) = numeric::parse_rational(&text) {
            return ValueBox {
                number: Some(Shared::new(Number {
                    range,
                    raw: text,
                    value: Some(Shared::new(value)),
                })),
                ..Default::default()
            };
        }
        ValueBox {
            unquoted_string: Some(unquoted),
            ..Default::default()
        }
    }
    // Some(empty box) is meaningful: a recognized spread may yield nil.
    fn special_node(&mut self, r: char) -> Option<MapNodeBox> {
        match r {
            '#' => {
                return Some(MapNodeBox {
                    comment: Some(Shared::new(self.parse_comment())),
                    ..Default::default()
                });
            }
            '"' => {
                let (text, eof) = self.peek_n(2);
                if eof {
                    return None;
                }
                if text != "\"\"" {
                    self.rewind();
                    return None;
                }
                self.commit();
                return Some(MapNodeBox {
                    block_comment: Some(Shared::new(self.parse_block_comment())),
                    ..Default::default()
                });
            }
            '.' => {
                let (text, eof) = self.peek_n(2);
                if eof {
                    return None;
                }
                if text != ".." {
                    self.rewind();
                    return None;
                }
                let next = self.peek()?;
                if next == '$' {
                    self.commit();
                    return Some(MapNodeBox {
                        substitution: self.parse_substitution(true).map(Shared::new),
                        ..Default::default()
                    });
                }
                if next == '@' {
                    self.commit();
                    return Some(MapNodeBox {
                        import: Some(Shared::new(self.parse_import(true))),
                        ..Default::default()
                    });
                }
                self.rewind();
            }
            _ => {}
        }
        None
    }
    pub fn parse_map(&mut self, file: bool) -> Map {
        let mut start = self.position;
        if !file {
            start = start.subtract('{', self.utf16_positions);
            if !self.enter_nesting(start) {
                return Map {
                    range: Range {
                        path: self.path.clone(),
                        start,
                        end: self.position,
                    },
                    ..Default::default()
                };
            }
            self.depth += 1;
        }
        let mut nodes = Vec::new();
        loop {
            let Some(r) = self.read_not_space() else {
                if !file {
                    self.error(
                        start,
                        self.reader_position,
                        "maps must be terminated with }",
                    );
                }
                break;
            };
            if r == ';' {
                continue;
            }
            if r == '}' {
                if file {
                    self.error(
                        self.position.subtract(r, self.utf16_positions),
                        self.position,
                        "unexpected map termination character } in file map",
                    );
                    continue;
                }
                break;
            }
            let node = if let Some(node) = self.special_node(r) {
                node
            } else {
                self.replay(r);
                MapNodeBox {
                    map_key: self.parse_map_key().map(Shared::new),
                    ..Default::default()
                }
            };
            if node.unbox().is_some() {
                nodes.push(node.clone());
            }
            if node.block_comment.is_some() {
                continue;
            }
            let after = self.position;
            loop {
                let (r, newlines) = self.peek_not_space();
                if r.is_none() || newlines != 0 || matches!(r, Some(';' | '}' | '#')) {
                    self.rewind();
                    break;
                }
                self.commit();
            }
            if after != self.position {
                if let Some(child) = node.unbox() {
                    let value = node.map_key.as_ref().and_then(|k| k.read().value.unbox());
                    let message = if let Some(value) = value {
                        let suffix = if matches!(value, Child::Block(_)) {
                            ". See https://d2lang.com/tour/text#advanced-block-strings."
                        } else {
                            ""
                        };
                        format!("unexpected text after {}{suffix}", value.node_type())
                    } else {
                        format!("unexpected text after {}", child.node_type())
                    };
                    self.error(after, self.position, &message);
                } else {
                    self.error(after, self.position, "invalid text beginning unquoted key");
                }
            }
        }
        if !file {
            self.depth -= 1;
            self.leave_nesting();
        }
        Map {
            range: Range {
                path: self.path.clone(),
                start,
                end: self.position,
            },
            nodes: if nodes.is_empty() {
                None
            } else {
                Some(nodes.into())
            },
        }
    }
    pub fn parse_array(&mut self) -> Array {
        let start = self.position.subtract('[', self.utf16_positions);
        if !self.enter_nesting(start) {
            return Array {
                range: Range {
                    path: self.path.clone(),
                    start,
                    end: self.reader_position,
                },
                ..Default::default()
            };
        }
        self.depth += 1;
        let mut nodes = Vec::new();
        loop {
            let Some(r) = self.read_not_space() else {
                self.error(
                    start,
                    self.reader_position,
                    "arrays must be terminated with ]",
                );
                break;
            };
            if r == ';' {
                continue;
            }
            if r == ']' {
                break;
            }
            let node = if let Some(special) = self.special_node(r) {
                ArrayNodeBox::from_child(special.unbox())
            } else {
                self.replay(r);
                let value = self.parse_value();
                if let Some(unquoted) = &value.unquoted_string {
                    let unquoted = unquoted.read();
                    let substitution = unquoted
                        .value
                        .as_ref()
                        .and_then(|v| v.get(0))
                        .is_some_and(|b| b.read().substitution.is_some());
                    if unquoted.scalar_bytes().is_empty() && !substitution {
                        self.error(
                            self.position,
                            self.position.advance(r, self.utf16_positions),
                            &format!("unquoted strings cannot start on {}", quote_rune(r)),
                        );
                    }
                }
                // Upstream omits Suspension when transferring fields to ArrayNodeBox.
                ArrayNodeBox {
                    null: value.null,
                    boolean: value.boolean,
                    number: value.number,
                    unquoted_string: value.unquoted_string,
                    double_quoted_string: value.double_quoted_string,
                    single_quoted_string: value.single_quoted_string,
                    block_string: value.block_string,
                    array: value.array,
                    map: value.map,
                    import: value.import,
                    ..Default::default()
                }
            };
            if node.unbox().is_some() {
                nodes.push(node.clone());
            }
            if node.block_comment.is_some() {
                continue;
            }
            let after = self.position;
            loop {
                let (r, newlines) = self.peek_not_space();
                if r.is_none() || newlines != 0 || matches!(r, Some(';' | ']' | '#')) {
                    self.rewind();
                    break;
                }
                self.commit();
            }
            if after != self.position {
                let message = node.unbox().map_or_else(
                    || "invalid text beginning unquoted string".into(),
                    |n| format!("unexpected text after {}", n.node_type()),
                );
                self.error(after, self.position, &message);
            }
        }
        self.depth -= 1;
        self.leave_nesting();
        Array {
            range: Range {
                path: self.path.clone(),
                start,
                end: self.reader_position,
            },
            nodes: if nodes.is_empty() {
                None
            } else {
                Some(nodes.into())
            },
        }
    }
}
