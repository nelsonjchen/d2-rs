// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
// Go-text table layout/fixed decoding Copyright 2021 The go-text authors;
// Rust translation Copyright 2026 Nelson Chen.
//! Source d2renderers/internal/fontface/colrv1.go: the trusted COLRv1 plan
//! compiler for D2's bundled Noto Color Emoji. Table decoding follows
//! go-text/typesetting v0.3.4 (Paint*, ColorLine, ClipBoxFormat1 layouts).

use crate::colr::{ColorRecord, Colr, be16, be24, be32};
use crate::error::Error;
use crate::face::{Nrgba, ParsedFace};
use d2_rs_render::go_float::go_fmsub;

const MAX_TRUSTED_COLRV1_PAINT_NODES: usize = 8_192;
const MAX_TRUSTED_COLRV1_DEPTH: usize = 32;
const MAX_TRUSTED_COLRV1_LAYERS: usize = 256;
const MAX_TRUSTED_COLRV1_GRADIENT_STOPS: usize = 32;

/// `trustedCOLRv1Limits`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TrustedCOLRv1Limits {
    pub max_paint_nodes: usize,
    pub max_depth: usize,
    pub max_layers: usize,
    pub max_gradient_stops: usize,
}

/// `bundledNotoColorEmojiCOLRv1Limits`.
pub(crate) fn bundled_noto_color_emoji_colrv1_limits() -> TrustedCOLRv1Limits {
    TrustedCOLRv1Limits {
        max_paint_nodes: MAX_TRUSTED_COLRV1_PAINT_NODES,
        max_depth: MAX_TRUSTED_COLRV1_DEPTH,
        max_layers: MAX_TRUSTED_COLRV1_LAYERS,
        max_gradient_stops: MAX_TRUSTED_COLRV1_GRADIENT_STOPS,
    }
}

impl TrustedCOLRv1Limits {
    fn validate(&self) -> Result<(), Error> {
        if self.max_paint_nodes == 0
            || self.max_depth == 0
            || self.max_layers == 0
            || self.max_gradient_stops == 0
        {
            return Err(Error::msg("d2fonts: every COLRv1 limit must be positive"));
        }
        Ok(())
    }
}

/// `COLRv1Plan`: a renderer-neutral, palette-resolved paint graph in font
/// design units.
#[derive(Clone, Debug, PartialEq)]
pub struct COLRv1Plan {
    pub glyph_id: u32,
    pub root: COLRv1Paint,
    pub clip: Option<COLRv1ClipBox>,
    pub usage: COLRv1PlanUsage,
}

/// `COLRv1PlanUsage`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct COLRv1PlanUsage {
    pub paint_nodes: usize,
    pub max_depth: usize,
    pub max_layers: usize,
    pub max_gradient_stops: usize,
}

/// `COLRv1Paint`: the closed set of paints (Go's sealed interface and its
/// `COLRv1Layers`, `COLRv1Solid`, ... implementations).
#[derive(Clone, Debug, PartialEq)]
pub enum COLRv1Paint {
    Layers(COLRv1Layers),
    Solid(COLRv1Solid),
    LinearGradient(COLRv1LinearGradient),
    RadialGradient(COLRv1RadialGradient),
    Glyph(COLRv1Glyph),
    Transform(COLRv1Transform),
    Composite(COLRv1Composite),
}

#[derive(Clone, Debug, PartialEq)]
pub struct COLRv1Layers {
    pub paints: Vec<COLRv1Paint>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct COLRv1Solid {
    pub color: Nrgba,
}

#[derive(Clone, Debug, PartialEq)]
pub struct COLRv1LinearGradient {
    pub color_line: COLRv1ColorLine,
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct COLRv1RadialGradient {
    pub color_line: COLRv1ColorLine,
    pub x0: f64,
    pub y0: f64,
    pub radius0: f64,
    pub x1: f64,
    pub y1: f64,
    pub radius1: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct COLRv1Glyph {
    pub glyph_id: u32,
    pub paint: Box<COLRv1Paint>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct COLRv1Transform {
    pub matrix: COLRv1Affine,
    pub paint: Box<COLRv1Paint>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct COLRv1Composite {
    pub source: Box<COLRv1Paint>,
    pub backdrop: Box<COLRv1Paint>,
    pub mode: COLRv1CompositeMode,
}

/// `COLRv1Affine{Xx, Yx, Xy, Yy, Dx, Dy}`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct COLRv1Affine {
    pub xx: f64,
    pub yx: f64,
    pub xy: f64,
    pub yy: f64,
    pub dx: f64,
    pub dy: f64,
}

/// `COLRv1ClipBox` in Y-up design units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct COLRv1ClipBox {
    pub x_min: f64,
    pub y_min: f64,
    pub x_max: f64,
    pub y_max: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct COLRv1ColorStop {
    pub offset: f64,
    pub color: Nrgba,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct COLRv1ColorLine {
    pub stops: Vec<COLRv1ColorStop>,
}

/// `COLRv1CompositeMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum COLRv1CompositeMode {
    SrcIn = 0,
    SoftLight = 1,
}

impl ParsedFace {
    /// `CompileBundledNotoColorEmojiCOLRv1Plan`: Ok(None) when the glyph has
    /// no COLR paint. Only the parser-authenticated bundled face is accepted.
    pub fn compile_bundled_noto_color_emoji_colrv1_plan(
        &self,
        glyph_id: u32,
    ) -> Result<Option<COLRv1Plan>, Error> {
        self.compile_bundled_noto_color_emoji_colrv1_plan_with(
            glyph_id,
            bundled_noto_color_emoji_colrv1_limits(),
        )
    }

    pub(crate) fn compile_bundled_noto_color_emoji_colrv1_plan_with(
        &self,
        glyph_id: u32,
        limits: TrustedCOLRv1Limits,
    ) -> Result<Option<COLRv1Plan>, Error> {
        limits.validate()?;
        if !self.has_parsed_source(&crate::noto_color_emoji::BUNDLED_NOTO_COLOR_EMOJI_COLRV1_SHA256)
        {
            return Err(Error::COLRv1UntrustedFont);
        }
        crate::noto_color_emoji::compile_private_bundled_noto_color_emoji_colrv1_plan(
            glyph_id, limits,
        )
    }
}

/// `compileCOLRv1Plan`, run only on the package-private bundled parse.
pub(crate) fn compile_colrv1_plan(
    face: &ParsedFace,
    glyph_id: u32,
    limits: TrustedCOLRv1Limits,
) -> Result<Option<COLRv1Plan>, Error> {
    let resolution_panic = || {
        Error::msg(format!(
            "d2fonts: COLRv1 glyph {glyph_id} table resolution panicked"
        ))
    };
    if glyph_id == 0
        || glyph_id > u32::from(u16::MAX)
        || glyph_id as usize >= face.outline().num_glyphs()
    {
        return Err(Error::msg(format!(
            "d2fonts: COLRv1 glyph ID {glyph_id} is out of range"
        )));
    }
    let shaping = face.shaping().font();
    let Some(colr) = shaping.colr() else {
        return Ok(None);
    };
    let paint = match colr.search(glyph_id as u16) {
        Ok(paint) => paint,
        Err(error) => {
            return Err(Error::wrap(
                format!("d2fonts: COLRv1 glyph {glyph_id} lookup"),
                error,
            ));
        }
    };
    let Some(paint) = paint else {
        return Ok(None);
    };
    let mut builder = Builder {
        colr: &colr,
        palette: shaping.cpal_palette0(),
        num_glyphs: face.outline().num_glyphs(),
        limits,
        usage: COLRv1PlanUsage::default(),
    };
    let wrap = |error: Error| Error::wrap(format!("d2fonts: COLRv1 glyph {glyph_id}"), error);
    let root = match paint {
        crate::colr::ColrPaint::V1(at) => builder.paint(Node::V1(at), 1),
        crate::colr::ColrPaint::V0Layers { .. } => builder.paint(Node::V0, 1),
    };
    let root = match root {
        Ok(root) => root,
        Err(BuildError::Table) => return Err(resolution_panic()),
        Err(BuildError::Fontface(error)) => return Err(wrap(error)),
    };
    let clip = match builder.clip(glyph_id as u16) {
        Ok(clip) => clip,
        Err(BuildError::Table) => return Err(resolution_panic()),
        Err(BuildError::Fontface(error)) => return Err(wrap(error)),
    };
    Ok(Some(COLRv1Plan {
        glyph_id,
        root,
        clip,
        usage: builder.usage,
    }))
}

/// An out-of-bounds table read (Go: a recovered parser panic) or a typed
/// compiler error.
enum BuildError {
    Table,
    Fontface(Error),
}

impl From<Error> for BuildError {
    fn from(error: Error) -> Self {
        BuildError::Fontface(error)
    }
}

#[derive(Clone, Copy)]
enum Node {
    V1(usize),
    /// A COLRv0 layer list is a `tables.PaintColrLayersResolved` paint.
    V0,
}

struct Builder<'a, 'b> {
    colr: &'b Colr<'a>,
    palette: Option<Vec<ColorRecord>>,
    num_glyphs: usize,
    limits: TrustedCOLRv1Limits,
    usage: COLRv1PlanUsage,
}

fn limit_error(limit: &'static str, value: usize, maximum: usize) -> BuildError {
    BuildError::Fontface(Error::COLRv1Limit {
        limit,
        value,
        maximum,
    })
}

/// `fixed214`.
fn fixed214(value: i16) -> f64 {
    f64::from(value) / f64::from(1 << 14)
}

impl Builder<'_, '_> {
    fn u8(&self, at: usize) -> Result<u8, BuildError> {
        self.colr.data.get(at).copied().ok_or(BuildError::Table)
    }
    fn u16(&self, at: usize) -> Result<u16, BuildError> {
        be16(self.colr.data, at).ok_or(BuildError::Table)
    }
    fn i16(&self, at: usize) -> Result<i16, BuildError> {
        Ok(self.u16(at)? as i16)
    }
    fn u24(&self, at: usize) -> Result<usize, BuildError> {
        be24(self.colr.data, at).ok_or(BuildError::Table)
    }
    fn u32(&self, at: usize) -> Result<u32, BuildError> {
        be32(self.colr.data, at).ok_or(BuildError::Table)
    }
    /// `tables.Float1616` (float32) widened to float64.
    fn f1616(&self, at: usize) -> Result<f64, BuildError> {
        Ok(f64::from(self.u32(at)? as i32 as f32 / 65536.0))
    }

    fn paint(&mut self, node: Node, depth: usize) -> Result<COLRv1Paint, BuildError> {
        if depth > self.limits.max_depth {
            return Err(limit_error("paint depth", depth, self.limits.max_depth));
        }
        self.usage.paint_nodes += 1;
        if self.usage.paint_nodes > self.limits.max_paint_nodes {
            return Err(limit_error(
                "paint node count",
                self.usage.paint_nodes,
                self.limits.max_paint_nodes,
            ));
        }
        self.usage.max_depth = self.usage.max_depth.max(depth);
        let at = match node {
            Node::V1(at) => at,
            Node::V0 => {
                return Err(BuildError::Fontface(Error::COLRv1UnsupportedPaint {
                    type_name: "tables.PaintColrLayersResolved".into(),
                }));
            }
        };
        let format = self.u8(at)?;
        Ok(match format {
            1 => {
                let num_layers = usize::from(self.u8(at + 1)?);
                if num_layers > self.limits.max_layers {
                    return Err(limit_error(
                        "layer count",
                        num_layers,
                        self.limits.max_layers,
                    ));
                }
                let first = self.u32(at + 2)? as usize;
                let list = self.colr.layer_list();
                let count = match list {
                    Some(list) => self.u32(list)? as usize,
                    None => 0,
                };
                let last = (first as u32).wrapping_add(num_layers as u32) as usize;
                if last > count {
                    return Err(BuildError::Fontface(Error::wrap(
                        "d2fonts: COLRv1 layer range",
                        Error::msg(format!(
                            "out of bounds PaintColrLayers: expected {last}, got {count}"
                        )),
                    )));
                }
                self.usage.max_layers = self.usage.max_layers.max(last - first);
                let mut paints = Vec::with_capacity(last - first);
                if let Some(list) = list {
                    for index in first..last {
                        let child = list + self.u32(list + 4 + index * 4)? as usize;
                        paints.push(self.paint(Node::V1(child), depth + 1)?);
                    }
                }
                COLRv1Paint::Layers(COLRv1Layers { paints })
            }
            2 => COLRv1Paint::Solid(COLRv1Solid {
                color: self.color(self.u16(at + 1)?, self.i16(at + 3)?)?,
            }),
            4 => {
                let color_line = self.color_line(at + self.u24(at + 1)?)?;
                COLRv1Paint::LinearGradient(COLRv1LinearGradient {
                    color_line,
                    x0: f64::from(self.i16(at + 4)?),
                    y0: f64::from(self.i16(at + 6)?),
                    x1: f64::from(self.i16(at + 8)?),
                    y1: f64::from(self.i16(at + 10)?),
                    x2: f64::from(self.i16(at + 12)?),
                    y2: f64::from(self.i16(at + 14)?),
                })
            }
            6 => {
                let color_line = self.color_line(at + self.u24(at + 1)?)?;
                COLRv1Paint::RadialGradient(COLRv1RadialGradient {
                    color_line,
                    x0: f64::from(self.i16(at + 4)?),
                    y0: f64::from(self.i16(at + 6)?),
                    radius0: f64::from(self.u16(at + 8)?),
                    x1: f64::from(self.i16(at + 10)?),
                    y1: f64::from(self.i16(at + 12)?),
                    radius1: f64::from(self.u16(at + 14)?),
                })
            }
            10 => {
                let glyph_id = self.u16(at + 4)?;
                if usize::from(glyph_id) >= self.num_glyphs {
                    return Err(BuildError::Fontface(Error::msg(format!(
                        "d2fonts: COLRv1 paint glyph ID {glyph_id} is out of range"
                    ))));
                }
                let child = self.paint(Node::V1(at + self.u24(at + 1)?), depth + 1)?;
                COLRv1Paint::Glyph(COLRv1Glyph {
                    glyph_id: u32::from(glyph_id),
                    paint: Box::new(child),
                })
            }
            12 => {
                let child = self.paint(Node::V1(at + self.u24(at + 1)?), depth + 1)?;
                let m = at + self.u24(at + 4)?;
                COLRv1Paint::Transform(COLRv1Transform {
                    matrix: COLRv1Affine {
                        xx: self.f1616(m)?,
                        yx: self.f1616(m + 4)?,
                        xy: self.f1616(m + 8)?,
                        yy: self.f1616(m + 12)?,
                        dx: self.f1616(m + 16)?,
                        dy: self.f1616(m + 20)?,
                    },
                    paint: Box::new(child),
                })
            }
            14 => {
                let child = self.paint(Node::V1(at + self.u24(at + 1)?), depth + 1)?;
                COLRv1Paint::Transform(COLRv1Transform {
                    matrix: COLRv1Affine {
                        xx: 1.0,
                        yy: 1.0,
                        dx: f64::from(self.i16(at + 4)?),
                        dy: f64::from(self.i16(at + 6)?),
                        ..COLRv1Affine::default()
                    },
                    paint: Box::new(child),
                })
            }
            16 => {
                let child = self.paint(Node::V1(at + self.u24(at + 1)?), depth + 1)?;
                COLRv1Paint::Transform(COLRv1Transform {
                    matrix: COLRv1Affine {
                        xx: fixed214(self.i16(at + 4)?),
                        yy: fixed214(self.i16(at + 6)?),
                        ..COLRv1Affine::default()
                    },
                    paint: Box::new(child),
                })
            }
            18 => {
                let child = self.paint(Node::V1(at + self.u24(at + 1)?), depth + 1)?;
                let (raw_x, raw_y) = (self.i16(at + 4)?, self.i16(at + 6)?);
                let (sx, sy) = (fixed214(raw_x), fixed214(raw_y));
                // colrv1.go:306 (FMSUBDx2): `1 - sx` is fused with the
                // inlined fixed214 product float64(ScaleX) * 2^-14.
                let one_minus = |raw: i16| go_fmsub(f64::from(raw), 1.0 / f64::from(1 << 14), 1.0);
                COLRv1Paint::Transform(COLRv1Transform {
                    matrix: COLRv1Affine {
                        xx: sx,
                        yy: sy,
                        dx: f64::from(self.i16(at + 8)?) * one_minus(raw_x),
                        dy: f64::from(self.i16(at + 10)?) * one_minus(raw_y),
                        ..COLRv1Affine::default()
                    },
                    paint: Box::new(child),
                })
            }
            32 => {
                let raw = self.u8(at + 4)?;
                let mode = match raw {
                    5 => COLRv1CompositeMode::SrcIn,
                    20 => COLRv1CompositeMode::SoftLight,
                    _ => {
                        return Err(BuildError::Fontface(Error::COLRv1UnsupportedComposite {
                            mode: raw,
                        }));
                    }
                };
                let backdrop = self.paint(Node::V1(at + self.u24(at + 5)?), depth + 1)?;
                let source = self.paint(Node::V1(at + self.u24(at + 1)?), depth + 1)?;
                COLRv1Paint::Composite(COLRv1Composite {
                    source: Box::new(source),
                    backdrop: Box::new(backdrop),
                    mode,
                })
            }
            _ => {
                return Err(BuildError::Fontface(Error::COLRv1UnsupportedPaint {
                    type_name: format!("tables.{}", paint_format_name(format)),
                }));
            }
        })
    }

    fn color_line(&mut self, at: usize) -> Result<COLRv1ColorLine, BuildError> {
        let extend = self.u8(at)?;
        let n = usize::from(self.u16(at + 1)?);
        if n == 0 {
            return Err(BuildError::Fontface(Error::msg(
                "d2fonts: COLRv1 gradient has no stops",
            )));
        }
        if extend != 0 {
            return Err(BuildError::Fontface(Error::COLRv1UnsupportedExtend {
                extend,
            }));
        }
        if n > self.limits.max_gradient_stops {
            return Err(limit_error(
                "gradient stop count",
                n,
                self.limits.max_gradient_stops,
            ));
        }
        self.usage.max_gradient_stops = self.usage.max_gradient_stops.max(n);
        let mut stops = Vec::with_capacity(n);
        for index in 0..n {
            let p = at + 3 + index * 6;
            let offset = fixed214(self.i16(p)?);
            let color = self.color(self.u16(p + 2)?, self.i16(p + 4)?)?;
            stops.push(COLRv1ColorStop { offset, color });
        }
        Ok(COLRv1ColorLine { stops })
    }

    fn color(&self, index: u16, alpha: i16) -> Result<Nrgba, BuildError> {
        let Some(palette) = &self.palette else {
            return Err(BuildError::Fontface(Error::msg(
                "d2fonts: COLRv1 has no CPAL palette",
            )));
        };
        let Some(entry) = palette.get(usize::from(index)) else {
            return Err(BuildError::Fontface(Error::msg(format!(
                "d2fonts: COLRv1 palette index {index} is out of range"
            ))));
        };
        let a = fixed214(alpha);
        if !(0.0..=1.0).contains(&a) {
            return Err(BuildError::Fontface(Error::msg(format!(
                "d2fonts: COLRv1 alpha {} is outside [0,1]",
                crate::face::go_format_g(a)
            ))));
        }
        Ok(Nrgba {
            r: entry.red,
            g: entry.green,
            b: entry.blue,
            a: (f64::from(entry.alpha) * a).round() as u8,
        })
    }

    fn clip(&self, glyph: u16) -> Result<Option<COLRv1ClipBox>, BuildError> {
        let Some(at) = self.colr.clip(glyph) else {
            return Ok(None);
        };
        let format = self.u8(at)?;
        if format != 1 {
            return Err(BuildError::Fontface(Error::COLRv1UnsupportedPaint {
                type_name: format!("tables.ClipBoxFormat{format} clip box"),
            }));
        }
        let (x_min, y_min, x_max, y_max) = (
            self.i16(at + 1)?,
            self.i16(at + 3)?,
            self.i16(at + 5)?,
            self.i16(at + 7)?,
        );
        if x_min > x_max || y_min > y_max {
            return Err(BuildError::Fontface(Error::msg(
                "d2fonts: COLRv1 invalid clip box",
            )));
        }
        Ok(Some(COLRv1ClipBox {
            x_min: f64::from(x_min),
            y_min: f64::from(y_min),
            x_max: f64::from(x_max),
            y_max: f64::from(y_max),
        }))
    }
}

/// go-text's Paint table type names (`%T` without the package path).
fn paint_format_name(value: u8) -> &'static str {
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
