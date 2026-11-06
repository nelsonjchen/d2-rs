// SPDX-License-Identifier: MPL-2.0
//! d2scene/scene.go, primitive.go and animation.go.

use std::collections::BTreeMap;

use d2_rs_render::go_float::gomath;
use serde::{Deserialize, Serialize};

use crate::geometry::{Bounds, Box, Matrix, Point, finite, finite_point};
use crate::paint::{Nrgba, Paint, Stroke};
use crate::path::Path;

pub type AssetId = String;

/// Fully resolved, network-free scene resources. Data is base64 in JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Asset {
    FontAsset {
        #[serde(rename = "MIMEType")]
        mime_type: String,
        #[serde(rename = "Data", with = "crate::bytes", default)]
        data: Vec<u8>,
        /// Probe interchange only: a content-addressed file holding `data`,
        /// loaded by Document::load_asset_files.
        #[serde(rename = "DataFile", default, skip_serializing_if = "Option::is_none")]
        data_file: Option<String>,
        #[serde(rename = "FaceIndex", default)]
        face_index: u16,
    },
    RasterAsset {
        #[serde(rename = "MIMEType")]
        mime_type: String,
        #[serde(rename = "Data", with = "crate::bytes")]
        data: Vec<u8>,
        #[serde(rename = "PixelWidth")]
        pixel_width: i64,
        #[serde(rename = "PixelHeight")]
        pixel_height: i64,
        #[serde(rename = "DecodedBytes", default)]
        decoded_bytes: i64,
    },
    VectorAsset {
        #[serde(rename = "ViewBox")]
        view_box: Box,
        #[serde(rename = "Root")]
        root: Option<std::boxed::Box<Node>>,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct LinkRegion {
    pub r#box: Box,
    #[serde(rename = "URL")]
    pub url: String,
    pub tooltip: String,
    pub target: String,
}

crate::go_enum!(BlendMode { NORMAL = 0, MULTIPLY = 1, DARKEN = 2, COLOR_BURN = 3, OVERLAY = 4, LIGHTEN = 5 });
crate::go_enum!(MaskType { ALPHA = 0, LUMINANCE = 1 });

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Clip {
    pub path: Path,
    pub transform: Matrix,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Mask {
    pub r#type: MaskType,
    // d2scenebuild shares the diagram-wide connection mask by pointer.
    pub root: Option<std::sync::Arc<Node>>,
    pub transform: Matrix,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct DropShadow {
    pub offset_x: f64,
    pub offset_y: f64,
    pub sigma_x: f64,
    pub sigma_y: f64,
    pub color: Nrgba,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Filter {
    GaussianBlur {
        #[serde(rename = "SigmaX")]
        sigma_x: f64,
        #[serde(rename = "SigmaY")]
        sigma_y: f64,
    },
    DropShadow(DropShadow),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Rect {
    pub r#box: Box,
    pub radius_x: f64,
    pub radius_y: f64,
    pub fill: Option<Paint>,
    pub stroke: Option<Stroke>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Ellipse {
    pub center: Point,
    pub radius_x: f64,
    pub radius_y: f64,
    pub fill: Option<Paint>,
    pub stroke: Option<Stroke>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Font {
    pub family: String,
    pub style: String,
    pub weight: i64,
    #[serde(with = "crate::float_bits")]
    pub size: u64,
    pub asset: AssetId,
}

impl Font {
    pub fn size(&self) -> f64 {
        f64::from_bits(self.size)
    }
}

crate::go_enum!(TextAnchor { START = 0, MIDDLE = 1, END = 2 });

/// A shaper's exact glyph placement; ink is relative to origin + position.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Glyph {
    #[serde(rename = "ID")]
    pub id: u32,
    pub empty: bool,
    pub asset: AssetId,
    pub position: Point,
    pub advance: f64,
    pub ink: Bounds,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct TextRun {
    pub text: String,
    pub origin: Point,
    pub anchor: TextAnchor,
    pub font: Font,
    pub fallbacks: Option<Vec<AssetId>>,
    pub fill: Option<Paint>,
    pub stroke: Option<Stroke>,
    pub underline: bool,
    pub strike: bool,
    pub glyphs: Option<Vec<Glyph>>,
    pub ink: Bounds,
}

crate::go_enum!(AspectAlign {
    NONE = 0, X_MIN_Y_MIN = 1, X_MID_Y_MIN = 2, X_MAX_Y_MIN = 3, X_MIN_Y_MID = 4,
    X_MID_Y_MID = 5, X_MAX_Y_MID = 6, X_MIN_Y_MAX = 7, X_MID_Y_MAX = 8, X_MAX_Y_MAX = 9
});
crate::go_enum!(AspectFit { MEET = 0, SLICE = 1 });

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct AspectRatio {
    pub align: AspectAlign,
    pub fit: AspectFit,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Image {
    pub asset: AssetId,
    pub r#box: Box,
    pub aspect: AspectRatio,
}

/// The closed set of paintable leaves; groups have no primitive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Primitive {
    Rect(Rect),
    Ellipse(Ellipse),
    Path(Path),
    TextRun(TextRun),
    Image(Image),
}

/// One scene layer; transform maps node-local coordinates into the parent's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Node {
    #[serde(rename = "ID")]
    pub id: String,
    pub classes: Option<Vec<String>>,
    pub transform: Matrix,
    pub opacity: f64,
    pub blend: BlendMode,
    pub clip: Option<Clip>,
    pub mask: Option<Mask>,
    pub filters: Option<Vec<Filter>>,
    pub primitive: Option<Primitive>,
    pub children: Option<Vec<Node>>,
    pub animations: Option<Vec<Track>>,
}

impl Default for Node {
    /// d2scene.NewNode(nil): identity transform and full opacity.
    fn default() -> Self {
        Self::new(None)
    }
}

impl Node {
    pub fn new(primitive: Option<Primitive>) -> Self {
        Self {
            id: String::new(),
            classes: None,
            transform: Matrix::identity(),
            opacity: 1.0,
            blend: BlendMode::NORMAL,
            clip: None,
            mask: None,
            filters: None,
            primitive,
            children: None,
            animations: None,
        }
    }

    pub fn children(&self) -> &[Node] {
        self.children.as_deref().unwrap_or_default()
    }

    pub fn push(&mut self, child: Node) {
        self.children.get_or_insert_with(Vec::new).push(child);
    }
}

crate::go_enum!(ViewportFit { STRETCH = 0, MEET = 1 });
crate::go_enum!(ViewportAlign { X_MIN_Y_MIN = 0, X_MID_Y_MID = 1 });

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Document {
    pub view_box: Box,
    pub logical_width: f64,
    pub logical_height: f64,
    pub viewport_fit: ViewportFit,
    pub viewport_align: ViewportAlign,
    pub root: Option<std::boxed::Box<Node>>,
    pub assets: BTreeMap<AssetId, Asset>,
    pub links: Option<Vec<LinkRegion>>,
}

impl Document {
    /// Resolves interchange `DataFile` references into asset bytes.
    pub fn load_asset_files(&mut self) -> std::io::Result<()> {
        for asset in self.assets.values_mut() {
            if let Asset::FontAsset {
                data, data_file, ..
            } = asset
            {
                if let Some(path) = data_file.take() {
                    *data = std::fs::read(path)?;
                }
            }
        }
        Ok(())
    }

    pub fn new(view_box: Box, root: Node) -> Self {
        Self {
            view_box,
            logical_width: view_box.width,
            logical_height: view_box.height,
            root: Some(std::boxed::Box::new(root)),
            ..Default::default()
        }
    }
}

// ----------------------------------------------------------- animation.go

crate::go_enum!(AnimationProperty {
    OPACITY = 0, TRANSFORM = 1, STROKE_DASH_OFFSET = 2, FILL_COLOR = 3, STROKE_COLOR = 4, DROP_SHADOW = 5
});
crate::go_enum!(AnimationValueKind { NUMBER = 0, TRANSFORM = 1, COLOR = 2, SHADOW = 3 });
crate::go_enum!(EasingKind { LINEAR = 0, CUBIC_BEZIER = 1, STEP_START = 2, STEP_END = 3 });

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct AnimationValue {
    pub kind: AnimationValueKind,
    pub number: f64,
    pub transform: Matrix,
    pub color: Nrgba,
    pub shadow: DropShadow,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Easing {
    pub kind: EasingKind,
    #[serde(rename = "X1")]
    pub x1: f64,
    #[serde(rename = "Y1")]
    pub y1: f64,
    #[serde(rename = "X2")]
    pub x2: f64,
    #[serde(rename = "Y2")]
    pub y2: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Keyframe {
    pub offset: f64,
    pub value: AnimationValue,
    pub easing: Easing,
}

/// One typed animation; durations are nanoseconds (time.Duration).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Track {
    pub property: AnimationProperty,
    pub target_index: i64,
    pub delay: i64,
    pub duration: i64,
    pub repeat: bool,
    pub keyframes: Option<Vec<Keyframe>>,
}

// ----------------------------------------------------------- primitive.go

/// PrimitiveBounds: conservative painted bounds after applying m.
pub fn primitive_bounds(primitive: Option<&Primitive>, m: Matrix) -> Result<Bounds, String> {
    let Some(primitive) = primitive else {
        return Ok(Bounds::default());
    };
    if !m.is_finite() {
        return Err("d2scene: non-finite primitive transform".into());
    }
    match primitive {
        Primitive::Rect(rect) => {
            validate_box(rect.r#box).map_err(|e| format!("d2scene: rectangle: {e}"))?;
            if rect.radius_x < 0.0
                || rect.radius_y < 0.0
                || !finite(rect.radius_x)
                || !finite(rect.radius_y)
            {
                return Err("d2scene: rectangle: invalid corner radius".into());
            }
            if rect.fill.is_none() && !Stroke::visible(rect.stroke.as_ref()) {
                return Ok(Bounds::default());
            }
            Ok(expand_stroke(
                rect.r#box.bounds().transform(m),
                rect.stroke.as_ref(),
                m,
            ))
        }
        Primitive::Ellipse(ellipse) => {
            if !finite_point(ellipse.center)
                || !finite(ellipse.radius_x)
                || !finite(ellipse.radius_y)
                || ellipse.radius_x < 0.0
                || ellipse.radius_y < 0.0
            {
                return Err("d2scene: ellipse: invalid geometry".into());
            }
            if ellipse.fill.is_none() && !Stroke::visible(ellipse.stroke.as_ref()) {
                return Ok(Bounds::default());
            }
            let center = m.point(ellipse.center);
            let x_radius = gomath::hypot(m.a * ellipse.radius_x, m.c * ellipse.radius_y);
            let y_radius = gomath::hypot(m.b * ellipse.radius_x, m.d * ellipse.radius_y);
            let bounds = Bounds::new(
                center.x - x_radius,
                center.y - y_radius,
                center.x + x_radius,
                center.y + y_radius,
            );
            Ok(expand_stroke(bounds, ellipse.stroke.as_ref(), m))
        }
        Primitive::Path(path) => {
            if path.fill.is_none() && !Stroke::visible(path.stroke.as_ref()) {
                return Ok(Bounds::default());
            }
            let bounds = path.transformed_geometry_bounds(m)?;
            Ok(expand_stroke(bounds, path.stroke.as_ref(), m))
        }
        Primitive::TextRun(text) => {
            if text.fill.is_none() && !Stroke::visible(text.stroke.as_ref()) {
                return Ok(Bounds::default());
            }
            if text.font.size() <= 0.0 || !finite(text.font.size()) || !finite_point(text.origin) {
                return Err("d2scene: text: invalid font size or origin".into());
            }
            let mut bounds = text.ink;
            if !bounds.valid {
                for glyph in text.glyphs.iter().flatten() {
                    if !finite_point(glyph.position)
                        || !finite(glyph.advance)
                        || !glyph.ink.is_finite()
                    {
                        return Err("d2scene: text: invalid glyph geometry".into());
                    }
                    bounds = bounds.union(glyph.ink.translate(Point::new(
                        text.origin.x + glyph.position.x,
                        text.origin.y + glyph.position.y,
                    )));
                }
            }
            if !bounds.is_finite() {
                return Err("d2scene: text: invalid ink bounds".into());
            }
            if !bounds.valid {
                return Err("d2scene: text: missing ink or glyph bounds".into());
            }
            Ok(expand_stroke(bounds.transform(m), text.stroke.as_ref(), m))
        }
        Primitive::Image(image) => {
            if image.asset.is_empty() {
                return Err("d2scene: image: empty asset ID".into());
            }
            validate_box(image.r#box).map_err(|e| format!("d2scene: image: {e}"))?;
            Ok(image.r#box.bounds().transform(m))
        }
    }
}

fn expand_stroke(bounds: Bounds, stroke: Option<&Stroke>, m: Matrix) -> Bounds {
    if !bounds.valid || !Stroke::visible(stroke) {
        return bounds;
    }
    let expansion = Stroke::expansion(stroke) * m.max_scale();
    bounds.expand(expansion, expansion)
}

pub(crate) fn validate_box(b: Box) -> Result<(), String> {
    if !finite(b.x) || !finite(b.y) || !finite(b.width) || !finite(b.height) {
        return Err("non-finite box".into());
    }
    if b.width < 0.0 || b.height < 0.0 {
        return Err("negative box size".into());
    }
    Ok(())
}
