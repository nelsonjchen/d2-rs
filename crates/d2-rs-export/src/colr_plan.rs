// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
// Go-text table layout/fixed decoding Copyright 2021 The go-text authors;
// Rust translation Copyright 2026 Nelson Chen.
//! Immutable authenticated bundled COLRv1 graph: fontface/colrv1.go.
//! Painting, cumulative scene work and cancellation are separate consumers.
use sha2::{Digest, Sha256};
use ttf_parser::{Face, Tag};

#[derive(Debug, PartialEq)]
#[cfg_attr(test, derive(serde::Deserialize))]
pub(super) struct Plan {
    pub glyph_id: u32,
    pub root: Paint,
    pub clip: Option<[f64; 4]>,
    pub usage: Usage,
}
#[derive(Debug, Default, PartialEq)]
#[cfg_attr(test, derive(serde::Deserialize))]
pub(super) struct Usage {
    pub paint_nodes: usize,
    pub max_depth: usize,
    pub max_layers: usize,
    pub max_gradient_stops: usize,
}
#[derive(Debug, PartialEq)]
#[cfg_attr(test, derive(serde::Deserialize))]
#[cfg_attr(test, serde(tag = "kind", rename_all = "snake_case"))]
pub(super) enum Paint {
    Layers {
        paints: Vec<Paint>,
    },
    Solid {
        color: [u8; 4],
    },
    Linear {
        stops: Vec<Stop>,
        points: [f64; 6],
    },
    Radial {
        stops: Vec<Stop>,
        circles: [f64; 6],
    },
    Glyph {
        glyph_id: u32,
        paint: Box<Paint>,
    },
    Transform {
        matrix: [f64; 6],
        paint: Box<Paint>,
    },
    Composite {
        source: Box<Paint>,
        backdrop: Box<Paint>,
        mode: u8,
    },
}
#[derive(Debug, PartialEq)]
#[cfg_attr(test, derive(serde::Deserialize))]
pub(super) struct Stop {
    pub offset: f64,
    pub color: [u8; 4],
}

pub(super) struct Compiler<'a> {
    face: Face<'a>,
    colr: &'a [u8],
    cpal: &'a [u8],
}
impl<'a> Compiler<'a> {
    pub fn new(bytes: &'a [u8]) -> Result<Self, String> {
        let hash = Sha256::digest(bytes);
        if hash.as_slice() != crate::bundled_emoji::DECODED_HASH {
            return Err(
                "d2fonts: COLRv1 font is not the parser-authenticated bundled resource".into(),
            );
        }
        let face = Face::parse(bytes, 0).map_err(|_| "d2fonts: nil parsed font face")?;
        let colr = face
            .raw_face()
            .table(Tag::from_bytes(b"COLR"))
            .ok_or("d2fonts: missing bundled COLR table")?;
        let cpal = face
            .raw_face()
            .table(Tag::from_bytes(b"CPAL"))
            .ok_or("d2fonts: COLRv1 has no CPAL palette")?;
        Ok(Self { face, colr, cpal })
    }
    pub fn compile(&self, glyph: u32) -> Result<Option<Plan>, String> {
        if glyph == 0
            || glyph > u32::from(u16::MAX)
            || glyph >= u32::from(self.face.number_of_glyphs())
        {
            return Err(format!("d2fonts: COLRv1 glyph ID {glyph} is out of range"));
        }
        let base = self.u32(14)? as usize;
        let n = self.u32(base)? as usize;
        // tables.baseGlyphList.paintForGlyph uses a first-match lower bound.
        let (mut first, mut end) = (0, n);
        while first < end {
            let middle = first + (end - first) / 2;
            if u32::from(self.u16(base + 4 + middle * 6)?) < glyph {
                first = middle + 1;
            } else {
                end = middle;
            }
        }
        if first == n || u32::from(self.u16(base + 4 + first * 6)?) != glyph {
            return Ok(None);
        }
        let root = base + self.u32(base + 6 + first * 6)? as usize;
        let mut usage = Usage::default();
        let root = self
            .paint(root, 1, &mut usage)
            .map_err(|e| format!("d2fonts: COLRv1 glyph {glyph}: {e}"))?;
        let clip = self
            .clip(glyph)
            .map_err(|e| format!("d2fonts: COLRv1 glyph {glyph}: {e}"))?;
        Ok(Some(Plan {
            glyph_id: glyph,
            root,
            clip,
            usage,
        }))
    }
    fn bytes(&self, at: usize, n: usize) -> Result<&[u8], String> {
        self.colr
            .get(at..at.checked_add(n).ok_or("COLR offset overflow")?)
            .ok_or_else(|| "d2fonts: COLRv1 table resolution out of bounds".into())
    }
    fn u8(&self, at: usize) -> Result<u8, String> {
        Ok(self.bytes(at, 1)?[0])
    }
    fn u16(&self, at: usize) -> Result<u16, String> {
        Ok(u16::from_be_bytes(self.bytes(at, 2)?.try_into().unwrap()))
    }
    fn i16(&self, at: usize) -> Result<f64, String> {
        Ok(f64::from(self.u16(at)? as i16))
    }
    fn u24(&self, at: usize) -> Result<usize, String> {
        let b = self.bytes(at, 3)?;
        Ok((usize::from(b[0]) << 16) | (usize::from(b[1]) << 8) | usize::from(b[2]))
    }
    fn u32(&self, at: usize) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.bytes(at, 4)?.try_into().unwrap()))
    }
    fn f16(&self, at: usize) -> Result<f64, String> {
        Ok(f64::from(self.u32(at)? as i32 as f32 / 65536.0))
    }
    fn f2(&self, at: usize) -> Result<f64, String> {
        Ok(self.i16(at)? / 16384.0)
    }
    fn color(&self, index: u16, alpha: f64) -> Result<[u8; 4], String> {
        let read = |at: usize| -> Result<u16, String> {
            Ok(u16::from_be_bytes(
                self.cpal
                    .get(at..at + 2)
                    .ok_or("d2fonts: COLRv1 CPAL out of bounds")?
                    .try_into()
                    .unwrap(),
            ))
        };
        if read(4)? == 0 {
            return Err("d2fonts: COLRv1 has no CPAL palette".into());
        }
        if index >= read(2)? {
            return Err(format!(
                "d2fonts: COLRv1 palette index {index} is out of range"
            ));
        }
        if !(0.0..=1.0).contains(&alpha) {
            return Err(format!("d2fonts: COLRv1 alpha {alpha} is outside [0,1]"));
        }
        let offset = u32::from_be_bytes(self.cpal[8..12].try_into().unwrap()) as usize;
        let at = offset + (usize::from(read(12)?) + usize::from(index)) * 4;
        let b = self
            .cpal
            .get(at..at + 4)
            .ok_or("d2fonts: COLRv1 CPAL out of bounds")?;
        Ok([b[2], b[1], b[0], (f64::from(b[3]) * alpha).round() as u8])
    }
    fn stops(&self, at: usize, usage: &mut Usage) -> Result<Vec<Stop>, String> {
        let n = usize::from(self.u16(at + 1)?);
        if n == 0 {
            return Err("d2fonts: COLRv1 gradient has no stops".into());
        }
        let extend = self.u8(at)?;
        if extend != 0 {
            return Err(format!(
                "d2fonts: unsupported COLRv1 gradient extend {extend}"
            ));
        }
        limit("gradient stop count", n, 32)?;
        usage.max_gradient_stops = usage.max_gradient_stops.max(n);
        (0..n)
            .map(|i| {
                let p = at + 3 + i * 6;
                Ok(Stop {
                    offset: self.f2(p)?,
                    color: self.color(self.u16(p + 2)?, self.f2(p + 4)?)?,
                })
            })
            .collect()
    }
    fn paint(&self, at: usize, depth: usize, usage: &mut Usage) -> Result<Paint, String> {
        limit("paint depth", depth, 32)?;
        usage.paint_nodes += 1;
        limit("paint node count", usage.paint_nodes, 8192)?;
        usage.max_depth = usage.max_depth.max(depth);
        let format = self.u8(at)?;
        Ok(match format {
            1 => {
                let n = usize::from(self.u8(at + 1)?);
                limit("layer count", n, 256)?;
                let list = self.u32(18)? as usize;
                let count = self.u32(list)? as usize;
                let first = self.u32(at + 2)? as usize;
                if first + n > count {
                    return Err(format!(
                        "d2fonts: COLRv1 layer range: out of bounds PaintColrLayers: expected {}, got {count}",
                        first + n
                    ));
                }
                usage.max_layers = usage.max_layers.max(n);
                let paints = (first..first + n)
                    .map(|i| {
                        self.paint(
                            list + self.u32(list + 4 + i * 4)? as usize,
                            depth + 1,
                            usage,
                        )
                    })
                    .collect::<Result<_, _>>()?;
                Paint::Layers { paints }
            }
            2 => Paint::Solid {
                color: self.color(self.u16(at + 1)?, self.f2(at + 3)?)?,
            },
            4 | 6 => {
                let stops = self.stops(at + self.u24(at + 1)?, usage)?;
                let mut values = [0.0; 6];
                for (i, v) in values.iter_mut().enumerate() {
                    *v = if format == 6 && (i == 2 || i == 5) {
                        f64::from(self.u16(at + 4 + i * 2)?)
                    } else {
                        self.i16(at + 4 + i * 2)?
                    };
                }
                if format == 4 {
                    Paint::Linear {
                        stops,
                        points: values,
                    }
                } else {
                    Paint::Radial {
                        stops,
                        circles: values,
                    }
                }
            }
            10 => {
                let glyph_id = u32::from(self.u16(at + 4)?);
                if glyph_id >= u32::from(self.face.number_of_glyphs()) {
                    return Err(format!(
                        "d2fonts: COLRv1 paint glyph ID {glyph_id} is out of range"
                    ));
                }
                let paint = Box::new(self.paint(at + self.u24(at + 1)?, depth + 1, usage)?);
                Paint::Glyph { glyph_id, paint }
            }
            12 | 14 | 16 | 18 => {
                let paint = Box::new(self.paint(at + self.u24(at + 1)?, depth + 1, usage)?);
                let mut matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
                match format {
                    12 => {
                        let m = at + self.u24(at + 4)?;
                        for (i, v) in matrix.iter_mut().enumerate() {
                            *v = self.f16(m + i * 4)?;
                        }
                    }
                    14 => {
                        matrix[4] = self.i16(at + 4)?;
                        matrix[5] = self.i16(at + 6)?;
                    }
                    _ => {
                        matrix[0] = self.f2(at + 4)?;
                        matrix[3] = self.f2(at + 6)?;
                        if format == 18 {
                            matrix[4] = self.i16(at + 8)? * (1.0 - matrix[0]);
                            matrix[5] = self.i16(at + 10)? * (1.0 - matrix[3]);
                        }
                    }
                }
                Paint::Transform { matrix, paint }
            }
            32 => {
                let raw = self.u8(at + 4)?;
                let mode = match raw {
                    5 => 0,
                    20 => 1,
                    _ => return Err(format!("d2fonts: unsupported COLRv1 composite mode {raw}")),
                };
                let backdrop = Box::new(self.paint(at + self.u24(at + 5)?, depth + 1, usage)?);
                let source = Box::new(self.paint(at + self.u24(at + 1)?, depth + 1, usage)?);
                Paint::Composite {
                    source,
                    backdrop,
                    mode,
                }
            }
            _ => {
                return Err(format!(
                    "d2fonts: unsupported COLRv1 paint tables.{}",
                    format_name(format)
                ));
            }
        })
    }
    fn clip(&self, glyph: u32) -> Result<Option<[f64; 4]>, String> {
        let list = self.u32(22)? as usize;
        if list == 0 {
            return Ok(None);
        }
        let count = self.u32(list + 1)? as usize;
        for i in 0..count {
            let p = list + 5 + i * 7;
            if u32::from(self.u16(p)?) <= glyph && glyph <= u32::from(self.u16(p + 2)?) {
                let b = list + self.u24(p + 4)?;
                if self.u8(b)? != 1 {
                    return Err(
                        "d2fonts: unsupported COLRv1 paint tables.ClipBoxFormat2 clip box".into(),
                    );
                }
                let result = [
                    self.i16(b + 1)?,
                    self.i16(b + 3)?,
                    self.i16(b + 5)?,
                    self.i16(b + 7)?,
                ];
                if result[0] > result[2] || result[1] > result[3] {
                    return Err("d2fonts: COLRv1 invalid clip box".into());
                }
                return Ok(Some(result));
            }
        }
        Ok(None)
    }
}
fn limit(name: &str, value: usize, maximum: usize) -> Result<(), String> {
    if value > maximum {
        Err(format!(
            "d2fonts: COLRv1 {name} {value} exceeds limit {maximum}"
        ))
    } else {
        Ok(())
    }
}
fn format_name(value: u8) -> &'static str {
    const NAMES: [&str; 32] = [
        "PaintColrLayers",
        "PaintSolid",
        "PaintVarSolid",
        "PaintLinearGradient",
        "PaintVarLinearGradient",
        "PaintRadialGradient",
        "PaintVarRadialGradient",
        "PaintSweepGradient",
        "PaintVarSweepGradient",
        "PaintGlyph",
        "PaintColrGlyph",
        "PaintTransform",
        "PaintVarTransform",
        "PaintTranslate",
        "PaintVarTranslate",
        "PaintScale",
        "PaintVarScale",
        "PaintScaleAroundCenter",
        "PaintVarScaleAroundCenter",
        "PaintScaleUniform",
        "PaintVarScaleUniform",
        "PaintScaleUniformAroundCenter",
        "PaintVarScaleUniformAroundCenter",
        "PaintRotate",
        "PaintVarRotate",
        "PaintRotateAroundCenter",
        "PaintVarRotateAroundCenter",
        "PaintSkew",
        "PaintVarSkew",
        "PaintSkewAroundCenter",
        "PaintVarSkewAroundCenter",
        "PaintComposite",
    ];
    NAMES
        .get(usize::from(value).wrapping_sub(1))
        .copied()
        .unwrap_or("unknown")
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    #[derive(serde::Deserialize)]
    struct Outcome {
        glyph_id: u32,
        found: bool,
        error: String,
        plan: Option<Plan>,
    }
    fn bits(p: &Paint, out: &mut Vec<u64>) {
        match p {
            Paint::Layers { paints } => {
                for p in paints {
                    bits(p, out)
                }
            }
            Paint::Solid { .. } => {}
            Paint::Glyph { paint, .. } => bits(paint, out),
            Paint::Transform { matrix, paint } => {
                out.extend(matrix.iter().map(|v| v.to_bits()));
                bits(paint, out)
            }
            Paint::Linear { stops, points }
            | Paint::Radial {
                stops,
                circles: points,
            } => {
                out.extend(points.iter().map(|v| v.to_bits()));
                out.extend(stops.iter().map(|v| v.offset.to_bits()));
            }
            Paint::Composite {
                source, backdrop, ..
            } => {
                bits(backdrop, out);
                bits(source, out)
            }
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn every_bundled_glyph_plan_and_error_matches_pristine_source() {
        let mut database = std::sync::Arc::new(resvg::usvg::fontdb::Database::new());
        let id = crate::bundled_emoji::Resolver::default()
            .select('😀', &[], &mut database)
            .unwrap();
        let data = database
            .with_face_data(id, |bytes, _| bytes.to_vec())
            .unwrap();
        let compiler = Compiler::new(&data).unwrap();
        let rows: Vec<Outcome> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/colrv1-plans-source-v090.json"
        ))
        .unwrap();
        assert_eq!(
            rows.len(),
            usize::from(compiler.face.number_of_glyphs()) + 2
        );
        for (i, row) in rows.into_iter().enumerate() {
            assert_eq!(row.glyph_id, i as u32);
            let actual = compiler.compile(row.glyph_id);
            if !row.error.is_empty() {
                assert_eq!(actual.unwrap_err(), row.error, "glyph {}", row.glyph_id);
                continue;
            }
            let actual = actual.unwrap();
            assert_eq!(actual.is_some(), row.found, "glyph {}", row.glyph_id);
            assert_eq!(actual, row.plan, "glyph {}", row.glyph_id);
            if let (Some(a), Some(b)) = (actual, row.plan) {
                let mut aa = Vec::new();
                let mut bb = Vec::new();
                bits(&a.root, &mut aa);
                bits(&b.root, &mut bb);
                aa.extend(a.clip.into_iter().flatten().map(f64::to_bits));
                bb.extend(b.clip.into_iter().flatten().map(f64::to_bits));
                assert_eq!(aa, bb, "all Source float bits, glyph {}", row.glyph_id);
            }
        }
    }
}
