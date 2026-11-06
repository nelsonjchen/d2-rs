// SPDX-License-Identifier: MPL-2.0
//! d2scene/aspect.go. Products are explicitly rounded in Go (float64(...)),
//! so no contraction applies.

use crate::geometry::{Box, Matrix, finite};
use crate::scene::{AspectAlign, AspectFit, AspectRatio, validate_box};

/// AspectRatioMatrix: SVG preserveAspectRatio mapping of source into
/// destination.
pub fn aspect_ratio_matrix(
    source: Box,
    destination: Box,
    aspect: AspectRatio,
) -> Result<Matrix, String> {
    if validate_box(source).is_err() || source.width == 0.0 || source.height == 0.0 {
        return Err("d2scene: aspect-ratio source box must have finite positive dimensions".into());
    }
    if validate_box(destination).is_err() {
        return Err(
            "d2scene: aspect-ratio destination box must have finite non-negative dimensions".into(),
        );
    }
    if aspect.align.0 > AspectAlign::X_MAX_Y_MAX.0 || aspect.fit.0 > AspectFit::SLICE.0 {
        return Err(format!(
            "d2scene: invalid aspect-ratio policy align={} fit={}",
            aspect.align.0, aspect.fit.0
        ));
    }
    let mut scale_x = destination.width / source.width;
    let mut scale_y = destination.height / source.height;
    if !finite(scale_x) || !finite(scale_y) {
        return Err("d2scene: aspect-ratio scale is non-finite".into());
    }
    let (mut offset_x, mut offset_y) = (destination.x, destination.y);
    if aspect.align != AspectAlign::NONE {
        let mut scale = scale_x;
        if scale_y < scale {
            scale = scale_y;
        }
        if aspect.fit == AspectFit::SLICE && scale_y > scale_x {
            scale = scale_y;
        } else if aspect.fit == AspectFit::SLICE {
            scale = scale_x;
        }
        scale_x = scale;
        scale_y = scale;
        let (align_x, align_y) = alignment_fractions(aspect.align);
        let extra_x = destination.width - source.width * scale;
        let extra_y = destination.height - source.height * scale;
        offset_x = destination.x + extra_x * align_x;
        offset_y = destination.y + extra_y * align_y;
    }
    let matrix = Matrix {
        a: scale_x,
        b: 0.0,
        c: 0.0,
        d: scale_y,
        e: offset_x - source.x * scale_x,
        f: offset_y - source.y * scale_y,
    };
    if !matrix.is_finite() {
        return Err("d2scene: aspect-ratio transform is non-finite".into());
    }
    Ok(matrix)
}

fn alignment_fractions(align: AspectAlign) -> (f64, f64) {
    match align {
        AspectAlign::X_MID_Y_MIN => (0.5, 0.0),
        AspectAlign::X_MAX_Y_MIN => (1.0, 0.0),
        AspectAlign::X_MIN_Y_MID => (0.0, 0.5),
        AspectAlign::X_MID_Y_MID => (0.5, 0.5),
        AspectAlign::X_MAX_Y_MID => (1.0, 0.5),
        AspectAlign::X_MIN_Y_MAX => (0.0, 1.0),
        AspectAlign::X_MID_Y_MAX => (0.5, 1.0),
        AspectAlign::X_MAX_Y_MAX => (1.0, 1.0),
        _ => (0.0, 0.0),
    }
}
