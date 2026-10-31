// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// Translated from d2ast/d2ast.go at d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda.
//! Source positions shared by the parser and editor APIs.

use std::fmt;

pub mod keywords;
mod nodes;
pub use nodes::*;
mod interpolation;
pub use interpolation::*;
mod key_path;
pub use key_path::*;
mod edges;
pub use edges::*;
mod imports;
pub use imports::*;
mod number;
pub use number::*;

impl serde::Serialize for Position {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.debug())
    }
}

impl serde::Serialize for Range {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.marshal_text())
    }
}

impl<'de> serde::Deserialize<'de> for Range {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        let mut range = Self::default();
        range
            .unmarshal_text(&value)
            .map_err(serde::de::Error::custom)?;
        Ok(range)
    }
}

/// Zero-based source position. `byte == -1` means an unknown absolute offset.
/// In UTF-16 mode both `column` and `byte` count code units, matching upstream.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Position {
    pub line: i64,
    pub column: i64,
    pub byte: i64,
}

impl Position {
    /// Advance over a Unicode scalar with upstream newline semantics.
    pub fn advance(mut self, rune: char, by_utf16: bool) -> Self {
        let size = if by_utf16 {
            rune.len_utf16()
        } else {
            rune.len_utf8()
        } as i64;
        if rune == '\n' {
            self.line += 1;
            self.column = 0;
        } else {
            self.column += size;
        }
        self.byte += size;
        self
    }

    /// Reverse a scalar advance within a line.
    ///
    /// # Panics
    /// As in upstream D2, subtracting a newline is undefined and panics.
    pub fn subtract(mut self, rune: char, by_utf16: bool) -> Self {
        assert_ne!(rune, '\n', "d2ast: cannot subtract newline from Position");
        let size = if by_utf16 {
            rune.len_utf16()
        } else {
            rune.len_utf8()
        } as i64;
        self.column -= size;
        self.byte -= size;
        self
    }

    pub fn advance_string(self, text: &str, by_utf16: bool) -> Self {
        text.chars()
            .fold(self, |position, rune| position.advance(rune, by_utf16))
    }

    pub fn subtract_string(self, text: &str, by_utf16: bool) -> Self {
        text.chars()
            .fold(self, |position, rune| position.subtract(rune, by_utf16))
    }

    /// Absolute offsets win unless absent or equal; then compare line/column.
    pub fn before(self, other: Self) -> bool {
        if self.byte != other.byte && self.byte != -1 && other.byte != -1 {
            return self.byte < other.byte;
        }
        (self.line, self.column) < (other.line, other.column)
    }

    /// Upstream's zero-based diagnostic/text representation.
    pub fn debug(self) -> String {
        format!("{}:{}:{}", self.line, self.column, self.byte)
    }
}

impl fmt::Display for Position {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(output, "{}:{}", self.line + 1, self.column + 1)
    }
}

/// Source range serialized with upstream's text representation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Range {
    pub path: String,
    pub start: Position,
    pub end: Position,
}

impl Range {
    pub fn one_line(&self) -> bool {
        self.start.line == self.end.line
    }

    pub fn before(&self, other: &Self) -> bool {
        self.start.before(other.start)
    }

    pub fn marshal_text(&self) -> String {
        format!("{},{}-{}", self.path, self.start.debug(), self.end.debug())
    }

    /// Parses D2's compact `path,line:column:byte-line:column:byte` form.
    pub fn unmarshal_text(&mut self, value: &str) -> Result<(), String> {
        let (prefix, end) = value
            .rsplit_once('-')
            .ok_or_else(|| "missing End field".to_string())?;
        let (path, start) = prefix
            .rsplit_once(',')
            .ok_or_else(|| "missing Start field".to_string())?;
        self.path = path.to_string();
        self.start = parse_position(start)?;
        self.end = parse_position(end)?;
        Ok(())
    }
}

fn parse_position(value: &str) -> Result<Position, String> {
    let mut fields = value.split(':');
    let line = fields
        .next()
        .ok_or_else(|| "expected three fields".to_string())?
        .parse::<i64>()
        .map_err(|error| error.to_string())?;
    let column = fields
        .next()
        .ok_or_else(|| "expected three fields".to_string())?
        .parse::<i64>()
        .map_err(|error| error.to_string())?;
    let byte = fields
        .next()
        .ok_or_else(|| "expected three fields".to_string())?
        .parse::<i64>()
        .map_err(|error| error.to_string())?;
    if fields.next().is_some() {
        return Err("expected three fields".into());
    }
    Ok(Position { line, column, byte })
}

impl fmt::Display for Range {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.path.is_empty() {
            write!(output, "{}:", self.path)?;
        }
        self.start.fmt(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // d2ast/d2ast_test.go TestRange/Advance, including the non-BMP witness.
    #[test]
    fn upstream_utf8_positions() {
        let position = Position::default().advance_string("a\nè𐀀", false);
        assert_eq!(
            position,
            Position {
                line: 1,
                column: 6,
                byte: 8
            }
        );
        assert_eq!(
            position.subtract_string("𐀀è", false),
            Position {
                line: 1,
                column: 0,
                byte: 2
            }
        );
    }

    #[test]
    fn upstream_utf16_positions() {
        let position = Position::default().advance_string("a\nè𐀀", true);
        assert_eq!(
            position,
            Position {
                line: 1,
                column: 3,
                byte: 5
            }
        );
        assert_eq!(
            position.subtract_string("𐀀è", true),
            Position {
                line: 1,
                column: 0,
                byte: 2
            }
        );
    }

    #[test]
    fn missing_offset_uses_line_and_column() {
        let a = Position {
            line: 3,
            column: 5,
            byte: -1,
        };
        let b = Position {
            line: 3,
            column: 6,
            byte: 0,
        };
        assert!(a.before(b));
        assert!(!b.before(a));
        let a = Position { byte: 100, ..a };
        assert!(!a.before(b));
        let b = Position { byte: 100, ..b };
        assert!(a.before(b));
    }

    #[test]
    fn upstream_range_strings() {
        let mut range = Range {
            path: "/src/example.go".into(),
            start: Position {
                line: 10,
                column: 5,
                byte: 100,
            },
            end: Position {
                line: 10,
                column: 7,
                byte: 101,
            },
        };
        assert_eq!(range.to_string(), "/src/example.go:11:6");
        assert_eq!(range.marshal_text(), "/src/example.go,10:5:100-10:7:101");
        assert!(range.one_line());
        range.path.clear();
        assert_eq!(range.to_string(), "11:6");
    }

    #[test]
    fn upstream_range_unmarshal_preserves_path_and_reports_shape_errors() {
        let mut range = Range::default();
        range.unmarshal_text("json_test.d2,1:1:0-5:1:50").unwrap();
        assert_eq!(range.path, "json_test.d2");
        assert_eq!(
            range.start,
            Position {
                line: 1,
                column: 1,
                byte: 0
            }
        );
        assert_eq!(
            range.end,
            Position {
                line: 5,
                column: 1,
                byte: 50
            }
        );
        range.unmarshal_text("dir/a-b,c.d2,1:1:0-5:1:50").unwrap();
        assert_eq!(range.path, "dir/a-b,c.d2");
        assert_eq!(
            range.unmarshal_text("json_test.d2").unwrap_err(),
            "missing End field"
        );
        assert_eq!(
            range.unmarshal_text("json_test.d2-5:1:50").unwrap_err(),
            "missing Start field"
        );
        assert_eq!(
            range
                .unmarshal_text("json_test.d2,1:10-5:1:50")
                .unwrap_err(),
            "expected three fields"
        );
    }

    #[test]
    #[should_panic(expected = "d2ast: cannot subtract newline from Position")]
    fn newline_cannot_be_subtracted() {
        Position::default().subtract('\n', false);
    }
}

mod composite;
pub use composite::*;
mod key_helpers;
pub use key_helpers::*;
mod byte_string;
pub use byte_string::*;
