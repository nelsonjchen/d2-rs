// SPDX-License-Identifier: MPL-2.0
//! Explicit grid capacity admission from d2graph/grid_diagram.go.

pub const MAX_GRID_DIMENSION: i64 = 10_000;
pub const MAX_GRID_CELLS: i64 = 1_000_000;

/// Validate axes before multiplication, including empty and overflowing grids.
pub fn grid_capacity(rows: i64, columns: i64) -> Result<i64, String> {
    if rows <= 0 || columns <= 0 {
        return Err(format!(
            "grid dimensions must be positive, got {rows} by {columns}"
        ));
    }
    if rows > MAX_GRID_DIMENSION || columns > MAX_GRID_DIMENSION {
        return Err(format!(
            "grid dimensions {rows} by {columns} exceed the maximum of {MAX_GRID_DIMENSION} rows or columns"
        ));
    }
    if rows > MAX_GRID_CELLS / columns {
        return Err(format!(
            "grid dimensions {rows} by {columns} exceed the limit of {MAX_GRID_CELLS} cells"
        ));
    }
    Ok(rows * columns)
}
