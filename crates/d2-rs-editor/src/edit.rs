// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Position-aware source edits.

use serde::{Deserialize, Serialize};
use std::cmp::{Ordering, Reverse};
use thiserror::Error;

/// Whether a source position column counts UTF-8 bytes or UTF-16 code units.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PositionEncoding {
    /// Count Unicode scalar bytes as D2's native parser does.
    #[default]
    Utf8,
    /// Count UTF-16 code units as browser/LSP clients do.
    Utf16,
}

/// A zero-based line and column in a source document.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourcePosition {
    /// Zero-based line number.
    pub line: usize,
    /// Column in the selected [`PositionEncoding`].
    pub column: usize,
}

/// A replacement over a half-open source range.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceEdit {
    /// Inclusive start position.
    pub start: SourcePosition,
    /// Exclusive end position.
    pub end: SourcePosition,
    /// Replacement UTF-8 text.
    pub replacement: String,
}

/// Failure while converting or applying a source edit.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum EditError {
    /// A line number is outside the source document.
    #[error("source line {line} is out of range")]
    LineOutOfRange { line: usize },
    /// A column is outside its line in the selected encoding.
    #[error("source column {column} is out of range on line {line}")]
    ColumnOutOfRange { line: usize, column: usize },
    /// The edit range runs backwards.
    #[error("source edit end precedes start")]
    ReversedRange,
    /// Two edits overlap after conversion to byte offsets.
    #[error("source edits overlap")]
    OverlappingEdits,
    /// A converted byte offset is not a UTF-8 scalar boundary.
    #[error("source edit splits a UTF-8 scalar")]
    InvalidBoundary,
}

fn line_bounds(source: &str, line: usize) -> Result<(usize, usize), EditError> {
    if source.is_empty() && line == 0 {
        return Ok((0, 0));
    }
    let mut start = 0;
    for (index, segment) in source.split_inclusive('\n').enumerate() {
        let end = start + segment.strip_suffix('\n').map_or(segment.len(), str::len);
        if index == line {
            return Ok((start, end));
        }
        start += segment.len();
    }
    // A trailing newline creates a final empty line, which split_inclusive
    // does not return as a segment.
    if line == source.lines().count() && source.ends_with('\n') {
        return Ok((source.len(), source.len()));
    }
    Err(EditError::LineOutOfRange { line })
}

fn column_byte_offset(
    source: &str,
    position: SourcePosition,
    encoding: PositionEncoding,
) -> Result<usize, EditError> {
    let (line_start, line_end) = line_bounds(source, position.line)?;
    let line = &source[line_start..line_end];
    if matches!(encoding, PositionEncoding::Utf8) {
        if position.column > line.len() || !line.is_char_boundary(position.column) {
            return Err(EditError::ColumnOutOfRange {
                line: position.line,
                column: position.column,
            });
        }
        return Ok(line_start + position.column);
    }
    let mut units = 0;
    for (byte, rune) in line.char_indices() {
        if units == position.column {
            return Ok(line_start + byte);
        }
        let width = rune.len_utf16();
        if units + width > position.column {
            return Err(EditError::ColumnOutOfRange {
                line: position.line,
                column: position.column,
            });
        }
        units += width;
    }
    if units == position.column {
        Ok(line_end)
    } else {
        Err(EditError::ColumnOutOfRange {
            line: position.line,
            column: position.column,
        })
    }
}

fn byte_range(
    source: &str,
    edit: &SourceEdit,
    encoding: PositionEncoding,
) -> Result<(usize, usize), EditError> {
    let start = column_byte_offset(source, edit.start, encoding)?;
    let end = column_byte_offset(source, edit.end, encoding)?;
    match end.cmp(&start) {
        Ordering::Less => Err(EditError::ReversedRange),
        Ordering::Equal | Ordering::Greater => Ok((start, end)),
    }
}

/// Applies non-overlapping edits and returns the resulting UTF-8 document.
///
/// Edits are interpreted against the original source and applied from the end
/// of the document toward the beginning, so callers may safely submit ranges
/// from a parser or editor snapshot without manually rebasing them.
pub fn apply_edits(
    source: &str,
    edits: &[SourceEdit],
    encoding: PositionEncoding,
) -> Result<String, EditError> {
    let mut ranges = edits
        .iter()
        .map(|edit| byte_range(source, edit, encoding).map(|range| (range, edit)))
        .collect::<Result<Vec<_>, _>>()?;
    ranges.sort_by_key(|(range, _)| Reverse(*range));
    for pair in ranges.windows(2) {
        if pair[0].0.0 < pair[1].0.1 {
            return Err(EditError::OverlappingEdits);
        }
    }
    let mut output = source.to_owned();
    for ((start, end), edit) in ranges {
        if !output.is_char_boundary(start) || !output.is_char_boundary(end) {
            return Err(EditError::InvalidBoundary);
        }
        output.replace_range(start..end, &edit.replacement);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::{EditError, PositionEncoding, SourceEdit, SourcePosition, apply_edits};

    fn edit(start: usize, end: usize, replacement: &str) -> SourceEdit {
        SourceEdit {
            start: SourcePosition {
                line: 0,
                column: start,
            },
            end: SourcePosition {
                line: 0,
                column: end,
            },
            replacement: replacement.to_owned(),
        }
    }

    #[test]
    fn applies_multiple_utf8_edits_backwards() {
        let result = apply_edits(
            "alpha beta",
            &[edit(0, 5, "a"), edit(6, 10, "b")],
            PositionEncoding::Utf8,
        )
        .unwrap();
        assert_eq!(result, "a b");
    }

    #[test]
    fn converts_utf16_without_splitting_a_surrogate_pair() {
        let edit = SourceEdit {
            start: SourcePosition { line: 0, column: 1 },
            end: SourcePosition { line: 0, column: 3 },
            replacement: "X".into(),
        };
        assert_eq!(
            apply_edits("a😀b", &[edit], PositionEncoding::Utf16).unwrap(),
            "aXb"
        );
        let invalid = SourceEdit {
            start: SourcePosition { line: 0, column: 2 },
            end: SourcePosition { line: 0, column: 2 },
            replacement: "X".into(),
        };
        assert!(matches!(
            apply_edits("a😀b", &[invalid], PositionEncoding::Utf16),
            Err(EditError::ColumnOutOfRange { .. })
        ));
    }

    #[test]
    fn rejects_overlapping_ranges() {
        let result = apply_edits(
            "abcd",
            &[edit(0, 2, "x"), edit(1, 3, "y")],
            PositionEncoding::Utf8,
        );
        assert_eq!(result, Err(EditError::OverlappingEdits));
    }
}
