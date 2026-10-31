// SPDX-License-Identifier: MPL-2.0
//! d2scene/paint.go.

use serde::{Deserialize, Serialize};

use crate::geometry::{Box, Matrix, Point};
use crate::scene::Node;

/// image/color.NRGBA (non-premultiplied 8-bit RGBA).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Nrgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Nrgba {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct GradientStop {
    pub offset: f64,
    pub color: Nrgba,
}

crate::go_enum!(PaintUnits { OBJECT_BOUNDING_BOX = 0, USER_SPACE_ON_USE = 1 });
crate::go_enum!(SpreadMethod { PAD = 0, REFLECT = 1, REPEAT = 2 });
crate::go_enum!(LineCap { BUTT = 0, ROUND = 1, SQUARE = 2 });
crate::go_enum!(LineJoin { MITER = 0, ROUND = 1, BEVEL = 2 });

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct LinearGradient {
    pub start: Point,
    pub end: Point,
    pub stops: Vec<GradientStop>,
    pub units: PaintUnits,
    pub transform: Matrix,
    pub spread: SpreadMethod,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct RadialGradient {
    pub center: Point,
    pub radius: f64,
    pub focal: Point,
    pub focal_radius: f64,
    pub stops: Vec<GradientStop>,
    pub units: PaintUnits,
    pub transform: Matrix,
    pub spread: SpreadMethod,
}

/// Repeats `root` over `tile`; root is in tile-local coordinates.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PatternPaint {
    pub tile: Box,
    pub root: Option<std::boxed::Box<Node>>,
    pub units: PaintUnits,
    pub transform: Matrix,
}

/// The closed set of renderer-neutral paints; `None` (Go nil) is no paint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Paint {
    SolidPaint {
        #[serde(rename = "Color")]
        color: Nrgba,
    },
    LinearGradient(LinearGradient),
    RadialGradient(RadialGradient),
    PatternPaint(PatternPaint),
}

impl Paint {
    pub const fn solid(color: Nrgba) -> Self {
        Self::SolidPaint { color }
    }
}

/// Path stroking in logical coordinates; dashes alternate on/off lengths.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Stroke {
    pub paint: Option<Paint>,
    pub width: f64,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f64,
    pub dashes: Option<Vec<f64>>,
    pub dash_offset: f64,
}

impl Stroke {
    pub(crate) fn visible(stroke: Option<&Stroke>) -> bool {
        stroke.is_some_and(|s| s.paint.is_some() && s.width > 0.0)
    }

    /// Conservative local-coordinate expansion around centerline geometry.
    pub(crate) fn expansion(stroke: Option<&Stroke>) -> f64 {
        let Some(s) = stroke.filter(|s| Self::visible(Some(s))) else {
            return 0.0;
        };
        let half = s.width / 2.0;
        let mut factor = 1.0;
        if s.cap == LineCap::SQUARE {
            factor = std::f64::consts::SQRT_2;
        }
        if s.join == LineJoin::MITER {
            let mut miter_limit = s.miter_limit;
            if miter_limit <= 0.0 {
                miter_limit = 4.0;
            }
            if miter_limit > factor {
                factor = miter_limit;
            }
        }
        half * factor
    }
}
