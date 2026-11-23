// SPDX-License-Identifier: MPL-2.0
//! Upstream 8558856dc internal/fontface/glyph_path.go. Closed font outlines
//! in scene coordinates, shared by SVG import and native SVG surfaces.
use crate::{ParsedFace, sfnt::SegmentOp};
#[derive(Clone, Copy, Debug, Default)]
pub struct GlyphPathCommand {
    pub kind: u8,
    pub points: [[f64; 2]; 3],
}
pub fn glyph_path(
    face: &ParsedFace,
    id: u32,
    ppem: i32,
    origin: [f64; 2],
) -> Result<Vec<GlyphPathCommand>, String> {
    if id == 0 || id > u16::MAX as u32 || id as usize >= face.outline().num_glyphs() {
        return Err(format!("native SVG has invalid glyph ID {id}"));
    }
    let segments = face
        .outline()
        .load_glyph(id as u16, ppem)
        .map_err(|e| format!("native SVG glyph {id} outline: {e}"))?;
    let mut commands = Vec::new();
    let mut open = false;
    let close = || GlyphPathCommand {
        kind: 5,
        ..Default::default()
    };
    for s in segments {
        let points = std::array::from_fn(|n| {
            [
                origin[0] + s.args[n].x as f64 / 64.,
                origin[1] + s.args[n].y as f64 / 64.,
            ]
        });
        let kind = match s.op {
            SegmentOp::MoveTo => {
                if open {
                    commands.push(close());
                }
                open = true;
                0
            }
            SegmentOp::LineTo => 1,
            SegmentOp::QuadTo => 2,
            SegmentOp::CubeTo => 3,
        };
        // Unused points in d2scene commands are zero, independently of origin.
        let mut points = points;
        for p in &mut points[match kind {
            0 | 1 => 1,
            2 => 2,
            _ => 3,
        }..]
        {
            *p = [0.; 2];
        }
        commands.push(GlyphPathCommand { kind, points });
    }
    if open {
        commands.push(close());
    }
    Ok(commands)
}
