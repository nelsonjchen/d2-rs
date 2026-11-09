//! Float64 path data for the simple embedded SVG images D2 rasterizes through
//! its native scene pipeline.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use base64::Engine as _;
use svgtypes::{PathParser, PathSegment};

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
const XLINK_NAMESPACE: &str = "http://www.w3.org/1999/xlink";
const SVG_DATA_PREFIX: &str = "data:image/svg+xml;base64,";
const SOURCE_ID_PREFIX: &str = "d2-rs-native-raster-svg-source-";
const RAW_PATH_SOURCE_ID_PREFIX: &str = "d2-rs-native-raster-path-source-";

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SvgPoint {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

impl SvgPoint {
    fn finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SvgPathCommand {
    MoveTo(SvgPoint),
    LineTo(SvgPoint),
    QuadraticTo(SvgPoint, SvgPoint),
    CubicTo(SvgPoint, SvgPoint, SvgPoint),
    ArcTo {
        radius_x: f64,
        radius_y: f64,
        rotation: f64,
        large_arc: bool,
        sweep: bool,
        end: SvgPoint,
    },
    Close,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SvgPath {
    pub(crate) intrinsic_clips: Vec<IntrinsicClip>,
    pub(crate) commands: Vec<SvgPathCommand>,
    pub(crate) intrinsic_transforms: Option<Vec<[f64; 6]>>,
    pub(crate) scene_transform: Option<[f64; 6]>,
    pub(crate) intrinsic_viewport: Option<[f64; 2]>,
    pub(crate) image_viewport: Option<([f64; 6], [f64; 2])>,
    pub(crate) paint_opacities: Option<[f64; 2]>,
    pub(crate) raster_image: Option<SceneImagePlacement>,
}

// Original d2svgimport compile.go retains the nested placement/clip node
// separately from its viewport content transform.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct IntrinsicClip {
    pub(crate) id: String,
    pub(crate) transforms: Vec<[f64; 6]>,
    pub(crate) size: [f64; 2],
}

fn intrinsic_viewport(node: roxmltree::Node<'_, '_>) -> Option<([f64; 6], [f64; 2])> {
    let number = |name| {
        node.attribute(name)?
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
    };
    let width = number("width")?;
    let height = number("height")?;
    let values = svgtypes::NumberListParser::from(node.attribute("viewBox")?)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let [x, y, w, h] = values.as_slice() else {
        return None;
    };
    if width <= 0.0 || height <= 0.0 || *w <= 0.0 || *h <= 0.0 || !x.is_finite() || !y.is_finite() {
        return None;
    }
    let mut sx = width / w;
    let mut sy = height / h;
    let mut dx = 0.0;
    let mut dy = 0.0;
    let aspect = node
        .attribute("preserveAspectRatio")
        .unwrap_or("xMidYMid meet");
    let fields: Vec<_> = aspect.split_whitespace().collect();
    if fields.first().copied() != Some("none") {
        let fit = fields.get(1).copied().unwrap_or("meet");
        let scale = match fit {
            "meet" => sx.min(sy),
            "slice" => sx.max(sy),
            _ => return None,
        };
        sx = scale;
        sy = scale;
        let align = fields.first().copied()?;
        let ax = if align.starts_with("xMin") {
            0.0
        } else if align.starts_with("xMid") {
            0.5
        } else if align.starts_with("xMax") {
            1.0
        } else {
            return None;
        };
        let ay = if align.ends_with("YMin") {
            0.0
        } else if align.ends_with("YMid") {
            0.5
        } else if align.ends_with("YMax") {
            1.0
        } else {
            return None;
        };
        dx = (width - w * scale) * ax;
        dy = (height - h * scale) * ay;
    }
    let matrix = source_matrix_mul(
        source_matrix_mul([1.0, 0.0, 0.0, 1.0, dx, dy], [sx, 0.0, 0.0, sy, 0.0, 0.0]),
        [1.0, 0.0, 0.0, 1.0, -x, -y],
    );
    matrix
        .iter()
        .all(|v| v.is_finite())
        .then_some((matrix, [width, height]))
}

fn tag_intrinsic_viewports(source: &str) -> String {
    let Ok(document) = roxmltree::Document::parse(source) else {
        return source.to_owned();
    };
    let root = document.root_element();
    if root.attribute("data-d2-rs-intrinsic-math") != Some("ordinary") {
        return source.to_owned();
    }
    let mut ids: HashSet<_> = document
        .descendants()
        .filter_map(|n| n.attribute("id").map(str::to_owned))
        .collect();
    let mut insertions = Vec::new();
    for node in root
        .descendants()
        .filter(|n| n.has_tag_name("svg") && *n != root && n.attribute("id").is_none())
    {
        let mut suffix = node.range().start;
        let mut id = format!("d2-rs-intrinsic-viewport-{suffix}");
        while ids.contains(&id) {
            suffix += 1;
            id = format!("d2-rs-intrinsic-viewport-{suffix}");
        }
        ids.insert(id.clone());
        if let Some(position) =
            start_tag_insert_position(source, node.range().start, node.range().end)
        {
            insertions.push((position, format!(" id=\"{id}\"")));
        }
    }
    let mut tagged = source.to_owned();
    insertions.sort_by(|a, b| b.0.cmp(&a.0));
    for (position, insertion) in insertions {
        tagged.insert_str(position, &insertion);
    }
    tagged
}

/// Original scene transform and image box, kept separate from texel mapping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SceneImagePlacement {
    pub(crate) matrix: [f64; 6],
    pub(crate) image_box: [f64; 4],
    pub(crate) aspect: crate::png_image::ImageAspect,
}

fn scene_image_placement(node: roxmltree::Node<'_, '_>) -> Option<SceneImagePlacement> {
    let aspect = crate::png_image::ImageAspect::parse(node.attribute("preserveAspectRatio"))?;
    let length = |name, default| {
        node.attribute(name)
            .map_or(Some(default), |value| value.parse::<f64>().ok())
    };
    let image_box = [
        length("x", 0.0)?,
        length("y", 0.0)?,
        length("width", 0.0)?,
        length("height", 0.0)?,
    ];
    if image_box.iter().any(|value| !value.is_finite())
        || image_box[2] <= 0.0
        || image_box[3] <= 0.0
    {
        return None;
    }
    let mut transforms = Vec::new();
    for ancestor in node.ancestors().filter(|ancestor| ancestor.is_element()) {
        match ancestor.tag_name().name() {
            "pattern" => {
                // Pattern resources have their own user-space scene origin.
                // The paint instance handles the pattern transform separately.
                if ancestor
                    .attribute("patternContentUnits")
                    .is_some_and(|units| units != "userSpaceOnUse")
                {
                    return None;
                }
                break;
            }
            "svg" => {
                // Unresolved nested/viewBox viewports must retain the existing
                // path until their exact scene mapping is carried too.
                if ancestor.parent_element().is_some()
                    && !(ancestor.attribute("data-d2-rs-canvas-reference") == Some("ordinary")
                        && ancestor.attribute("class").is_some_and(|classes| {
                            classes.split_whitespace().any(|c| c == "d2-svg")
                        })
                        && native_canvas_geometry(node.document().root_element(), ancestor)
                            .is_some())
                {
                    return None;
                }
                // The generated ordinary scene root has an explicit viewBox.
                // Keep its source f64 viewport instead of falling back to usvg's
                // f32 image/aspect transform at this otherwise supported root.
                let attrs = ancestor
                    .attributes()
                    .map(|a| (a.name().to_owned(), a.value().to_owned()))
                    .collect();
                let viewport =
                    d2_rs_math::svg_viewport::compile("scene", &attrs, false, false, None).ok()?;
                transforms.push(viewport.transform);
                if ancestor.parent_element().is_none() {
                    break;
                }
            }
            "g" | "image" => {
                if ancestor
                    .attribute("style")
                    .is_some_and(|style| style.contains("transform"))
                {
                    return None;
                }
                let transform = ancestor
                    .attribute("transform")
                    .map_or(Some(svgtypes::Transform::default()), |value| {
                        value.parse::<svgtypes::Transform>().ok()
                    })?;
                transforms.push([
                    transform.a,
                    transform.b,
                    transform.c,
                    transform.d,
                    transform.e,
                    transform.f,
                ]);
            }
            _ => return None,
        }
    }
    let mut matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    for right in transforms.into_iter().rev() {
        matrix = d2_rs_math::svg_transform::mul(matrix, right);
    }
    if matrix.iter().any(|value| !value.is_finite()) {
        return None;
    }
    Some(SceneImagePlacement {
        matrix,
        image_box,
        aspect,
    })
}

#[derive(Clone, Copy)]
pub(crate) struct NativeCanvasViewport {
    pub(crate) view_box: [f64; 4],
    pub(crate) center: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct NativeRect {
    pub(crate) box_values: [f64; 4],
    pub(crate) radii: [f64; 2],
}

#[derive(Clone, Debug)]
pub(crate) struct GradientStop64 {
    pub offset: f64,
    pub color: [u8; 4],
}
/// A gradient's source-precision stops and, for linear gradients, its
/// unrounded x1/y1/x2/y2 (usvg retains f32; d2scene keeps f64).
#[derive(Clone, Debug)]
pub(crate) struct RawGradient {
    pub stops: Vec<GradientStop64>,
    pub vector: Option<[f64; 4]>,
    pub object_bounding_box: bool,
    pub source_transform: Option<[f64; 6]>,
    pub native_transform: Option<[f64; 6]>,
}

#[cfg(all(test, feature = "oracle-fixtures"))]
pub(crate) fn gradient_stops64(svg: &str) -> HashMap<String, RawGradient> {
    if !svg.contains("linearGradient") && !svg.contains("radialGradient") {
        return HashMap::new();
    }
    let Ok(document) = roxmltree::Document::parse(svg) else {
        return HashMap::new();
    };
    gradient_stops_in_document(&document)
}

fn gradient_stops_in_document(document: &roxmltree::Document<'_>) -> HashMap<String, RawGradient> {
    let mut result = HashMap::new();
    let mut gradients = document
        .descendants()
        .filter(|n| n.has_tag_name("linearGradient") || n.has_tag_name("radialGradient"))
        .peekable();
    if gradients.peek().is_none() {
        return result;
    }
    // CSS-resolved stop paint remains with the renderer until the importer
    // retains that computed presentation in f64. Do not override it with
    // uncomputed attributes or bind an ambiguous document identity.
    if document
        .descendants()
        .filter(|node| node.has_tag_name("style"))
        .filter_map(|node| node.text())
        .any(|css| {
            let css = css.to_ascii_lowercase();
            css.contains("stop-color") || css.contains("stop-opacity")
        })
    {
        return result;
    }
    let mut counts = HashMap::<&str, usize>::new();
    for id in document
        .descendants()
        .filter_map(|node| node.attribute("id"))
    {
        *counts.entry(id).or_default() += 1;
    }
    for gradient in gradients {
        let Some(id) = gradient.attribute("id") else {
            continue;
        };
        if counts.get(id) != Some(&1) {
            continue;
        }
        let mut stops = Vec::new();
        let mut previous = 0.0_f64;
        let mut supported = true;
        for stop in gradient.children().filter(|n| n.has_tag_name("stop")) {
            if stop.attribute("style").is_some() || stop.attribute("class").is_some() {
                supported = false;
                break;
            }
            let raw = stop.attribute("offset").unwrap_or("0");
            let (raw, divisor) = raw.strip_suffix('%').map_or((raw, 1.0), |v| (v, 100.0));
            let Some(offset) = raw
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .map(|v| (v / divisor).clamp(0.0, 1.0).max(previous))
            else {
                supported = false;
                break;
            };
            previous = offset;
            let Ok(color) = stop
                .attribute("stop-color")
                .unwrap_or("black")
                .parse::<svgtypes::Color>()
            else {
                supported = false;
                break;
            };
            let Some(opacity) = stop
                .attribute("stop-opacity")
                .unwrap_or("1")
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
            else {
                supported = false;
                break;
            };
            stops.push(GradientStop64 {
                offset,
                color: [
                    color.red,
                    color.green,
                    color.blue,
                    (f64::from(color.alpha) * opacity.clamp(0.0, 1.0)).round() as u8,
                ],
            });
        }
        if supported && !stops.is_empty() {
            // Percentages are fractions only in objectBoundingBox units, and
            // a referenced template may supply the vector instead.
            let user_space = gradient.attribute("gradientUnits") == Some("userSpaceOnUse");
            let inherited = gradient
                .attributes()
                .any(|attribute| attribute.name() == "href");
            let vector = (gradient.has_tag_name("linearGradient")
                && !inherited
                && !(user_space
                    && ["x1", "y1", "x2", "y2"]
                        .iter()
                        .any(|name| gradient.attribute(*name).is_some_and(|v| v.ends_with('%')))))
            .then(|| {
                let coordinate = |name: &str, default: f64| {
                    gradient.attribute(name).map_or(Some(default), |raw| {
                        let (raw, divisor) =
                            raw.strip_suffix('%').map_or((raw, 1.0), |v| (v, 100.0));
                        raw.trim()
                            .parse::<f64>()
                            .ok()
                            .filter(|v| v.is_finite())
                            .map(|v| v / divisor)
                    })
                };
                Some([
                    coordinate("x1", 0.0)?,
                    coordinate("y1", 0.0)?,
                    coordinate("x2", 1.0)?,
                    coordinate("y2", 0.0)?,
                ])
            });
            result.insert(
                id.to_owned(),
                RawGradient {
                    stops,
                    vector: vector.flatten(),
                    object_bounding_box: !user_space && !inherited,
                    source_transform: gradient
                        .attribute("gradientTransform")
                        .unwrap_or("")
                        .parse::<svgtypes::Transform>()
                        .ok()
                        .map(|m| [m.a, m.b, m.c, m.d, m.e, m.f]),
                    native_transform: None,
                },
            );
        }
    }
    result
}

/// Exact generated scene filter operands, independent of SVG filter units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NativeDropShadow {
    pub offset: [f64; 2],
    pub sigma: [f64; 2],
    pub color: [u8; 4],
}

pub(crate) struct NativeConnectionMask {
    pub viewport: NativeCanvasViewport,
    pub boxes: Vec<[f64; 4]>,
}

pub(crate) struct NativeCodeToken {
    pub text: String,
    pub origin: SvgPoint,
    pub parsed_origin: SvgPoint,
    pub prototype: usize,
}

#[derive(Default)]
pub(crate) struct SvgImagePathSources {
    pattern_document: Option<String>,
    native_ids: HashMap<String, String>,
    native_cache: Option<d2_rs_math::svg_asset_cache::AssetCache>,
    prepared_jpegs: std::collections::BTreeMap<String, Vec<u8>>,
    gradient_stops: HashMap<String, RawGradient>,
    native_gradient_ids: HashMap<String, [Option<String>; 2]>,
    images: HashMap<String, Vec<SvgPath>>,
    image_paths_by_id: HashMap<String, HashMap<String, SvgPath>>,
    raw_paths: HashMap<String, SvgPath>,
    native_image_clips: HashMap<String, SvgPath>,
    native_connection_masks: HashMap<String, NativeConnectionMask>,
    native_mask_transforms: HashMap<String, [f64; 6]>,
    image_group_opacities: HashMap<String, HashMap<String, f64>>,
    text_origins: Vec<SvgTextOrigin>,
    text_origins_by_id: HashMap<String, SvgPoint>,
    text_contents_by_id: HashMap<String, String>,
    code_lines: HashMap<String, Vec<NativeCodeToken>>,
    text_stroke_widths_by_id: HashMap<String, f64>,
    text_font_sizes_by_id: HashMap<String, f64>,
    native_canvas_viewports: HashSet<String>,
    native_canvas_geometry: HashMap<String, NativeCanvasViewport>,
    native_markdown_viewports: HashMap<String, [f64; 4]>,
    native_group_transforms: HashMap<String, [f64; 6]>,
    native_shape_shadows: HashSet<String>,
    native_legend_shadows: HashSet<String>,
    native_legend_shape_shadows: HashSet<String>,
    native_rects: HashMap<String, NativeRect>,
    effect_opacities_by_id: HashMap<String, f64>,
    scene_images_by_id: HashMap<String, SceneImagePlacement>,
    logical_size: Option<[f64; 2]>,
}

#[derive(Clone, Debug, PartialEq)]
struct SvgTextOrigin {
    text: String,
    x: f64,
    y: f64,
}

pub(crate) struct NativeImage<'a> {
    pub vector: &'a d2_rs_math::svg_asset_cache::Vector,
    pub jpeg_pngs: &'a std::collections::BTreeMap<String, Vec<u8>>,
    pub resources:
        &'a std::collections::BTreeMap<String, Option<d2_rs_math::svg_image::RasterAsset>>,
}
impl SvgImagePathSources {
    pub(crate) fn native_gradient_id(&self, path: &str, role: usize) -> Option<&str> {
        self.native_gradient_ids.get(path)?[role].as_deref()
    }

    fn retain_native_gradient_ids(&mut self, svg: &str) {
        if self.gradient_stops.is_empty() {
            return;
        }
        let Ok(document) = roxmltree::Document::parse(svg) else {
            return;
        };
        for path in document.descendants().filter(|n| n.has_tag_name("path")) {
            let Some(id) = path.attribute("id") else {
                continue;
            };
            let ids = ["fill", "stroke"].map(|role| {
                let svgtypes::Paint::FuncIRI(id, _) =
                    svgtypes::Paint::from_str(path.attribute(role)?).ok()?
                else {
                    return None;
                };
                self.gradient_stops.contains_key(id).then(|| id.to_owned())
            });
            if ids.iter().any(Option::is_some) {
                self.native_gradient_ids.insert(id.to_owned(), ids);
            }
        }
    }
    pub(crate) fn native_image_clip(&self, id: &str) -> Option<&SvgPath> {
        self.native_image_clips.get(id)
    }
    pub(crate) fn set_prepared_jpegs(
        &mut self,
        prepared: std::collections::BTreeMap<String, Vec<u8>>,
    ) {
        self.prepared_jpegs = prepared;
    }
    pub(crate) fn native_image(&self, id: &str) -> Option<NativeImage<'_>> {
        let cache = self.native_cache.as_ref()?;
        let id = self.native_ids.get(id)?;
        Some(NativeImage {
            vector: cache.vectors.get(id)?,
            resources: &cache.document.resources,
            jpeg_pngs: &self.prepared_jpegs,
        })
    }

    pub(crate) fn native_markdown_viewport(&self, id: &str) -> Option<[f64; 4]> {
        self.native_markdown_viewports.get(id).copied()
    }
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.images.len()
    }

    pub(crate) fn gradient_stops(&self) -> &HashMap<String, RawGradient> {
        &self.gradient_stops
    }

    pub(crate) fn image_paths(&self, id: &str) -> Option<&[SvgPath]> {
        self.images.get(id).map(Vec::as_slice)
    }

    pub(crate) fn image_paths_by_id(&self, id: &str) -> Option<&HashMap<String, SvgPath>> {
        self.image_paths_by_id.get(id)
    }

    pub(crate) fn image_group_opacities(&self, id: &str) -> Option<&HashMap<String, f64>> {
        self.image_group_opacities.get(id)
    }

    pub(crate) fn raw_path(&self, id: &str) -> Option<&SvgPath> {
        self.raw_paths.get(id)
    }

    pub(crate) fn is_native_legend_shadow(&self, id: &str) -> bool {
        self.native_legend_shadows.contains(id) || self.native_legend_shape_shadows.contains(id)
    }
    pub(crate) fn has_native_legend_panel(&self) -> bool {
        !self.native_legend_shadows.is_empty()
    }
    pub(crate) fn native_legend_shadow(&self, id: &str) -> Option<NativeDropShadow> {
        if self.native_legend_shadows.contains(id) {
            Some(NativeDropShadow {
                offset: [0.0, 2.0],
                sigma: [3.0, 3.0],
                color: [0, 0, 0, 26],
            })
        } else if self.native_legend_shape_shadows.contains(id) {
            Some(NativeDropShadow {
                offset: [3.0, 5.0],
                sigma: [1.7, 1.7],
                color: [61, 69, 116, 102],
            })
        } else {
            None
        }
    }
    pub(crate) fn is_native_shape_shadow(&self, id: &str) -> bool {
        self.native_shape_shadows.contains(id)
    }

    /// Original resource XML is retained before usvg narrows coordinates.
    pub(crate) fn pattern_document(&self) -> Option<&str> {
        self.pattern_document.as_deref()
    }
    pub(crate) fn inherit_pattern_paths(&mut self, parent: &Self) {
        self.raw_paths
            .extend(parent.raw_paths.iter().map(|(k, v)| (k.clone(), v.clone())));
    }

    pub(crate) fn native_rects(&self) -> &HashMap<String, NativeRect> {
        &self.native_rects
    }

    pub(crate) fn is_native_canvas_viewport(&self, id: &str) -> bool {
        self.native_canvas_viewports.contains(id)
    }

    pub(crate) fn logical_size(&self, parsed: resvg::usvg::Size) -> [f64; 2] {
        let resolved = [parsed.width(), parsed.height()];
        self.logical_size
            // CSS or contextual units may resolve to a different viewport.
            // Bind precision to the actual parser result before using it.
            .filter(|size| {
                size.iter()
                    .zip(resolved)
                    .all(|(exact, parsed)| (*exact as f32).to_bits() == parsed.to_bits())
            })
            .unwrap_or_else(|| resolved.map(f64::from))
    }

    pub(crate) fn native_mask_transform(&self, id: &str) -> [f64; 6] {
        self.native_mask_transforms
            .get(id)
            .copied()
            .unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0])
    }

    pub(crate) fn native_connection_mask(&self, id: &str) -> Option<&NativeConnectionMask> {
        self.native_connection_masks.get(id)
    }
    pub(crate) fn native_canvas_geometry(&self, id: &str) -> Option<NativeCanvasViewport> {
        self.native_canvas_geometry.get(id).copied()
    }

    pub(crate) fn scene_image(&self, id: &str) -> Option<&SceneImagePlacement> {
        self.scene_images_by_id.get(id)
    }

    pub(crate) fn exact_effect_opacity(&self, id: &str, parsed: f32) -> f64 {
        self.effect_opacities_by_id
            .get(id)
            .copied()
            // A CSS override or different resolved instance must not acquire
            // the authored scene value merely because it shares an ID.
            .filter(|value| (*value as f32).to_bits() == parsed.to_bits())
            .unwrap_or_else(|| f64::from(parsed))
    }

    pub(crate) fn native_group_transform(&self, id: &str) -> Option<[f64; 6]> {
        self.native_group_transforms.get(id).copied()
    }

    pub(crate) fn exact_text_origin_by_id(&self, id: &str) -> Option<SvgPoint> {
        self.text_origins_by_id.get(id).copied()
    }

    pub(crate) fn exact_text_content_by_id(&self, id: &str) -> Option<&str> {
        self.text_contents_by_id.get(id).map(String::as_str)
    }

    pub(crate) fn code_line(&self, id: &str) -> Option<&[NativeCodeToken]> {
        self.code_lines.get(id).map(Vec::as_slice)
    }

    pub(crate) fn has_native_code_texts(&self) -> bool {
        !self.code_lines.is_empty()
    }

    /// Materialize native code handoffs only if the ordinary SVG renderer is
    /// needed, as with imported raster assets. The prepared native renderer
    /// shapes the retained source instead of usvg's one-glyph prototypes.
    pub(crate) fn materialize_code_texts<'a>(&self, svg: &'a str) -> Cow<'a, str> {
        if self.code_lines.is_empty() {
            return Cow::Borrowed(svg);
        }
        let Ok(document) = roxmltree::Document::parse(svg) else {
            return Cow::Borrowed(svg);
        };
        let mut edits = Vec::new();
        for node in document
            .descendants()
            .filter(|node| node.has_tag_name("text"))
        {
            let Some(tokens) = node.attribute("id").and_then(|id| self.code_line(id)) else {
                continue;
            };
            let prototypes = node
                .children()
                .filter(|node| node.has_tag_name("tspan"))
                .collect::<Vec<_>>();
            let mut replacement = svg[node.range()].split_once('>').unwrap().0.to_owned();
            replacement.push('>');
            for token in tokens {
                let span = prototypes[token.prototype];
                replacement.push_str("<tspan");
                for attribute in span.attributes().filter(|a| !matches!(a.name(), "x" | "y")) {
                    replacement.push(' ');
                    replacement.push_str(&svg[attribute.range()]);
                }
                // Round-trip the original parser coordinates, rather than
                // another token's prototype origin or the private f64 origin.
                replacement.push_str(&format!(
                    " x=\"{}\" y=\"{}\">",
                    token.parsed_origin.x as f32, token.parsed_origin.y as f32,
                ));
                replacement.push_str(
                    &token
                        .text
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;"),
                );
                replacement.push_str("</tspan>");
            }
            replacement.push_str("</text>");
            edits.push((node.range(), replacement));
        }
        edits.sort_unstable_by_key(|(range, _)| range.start);
        let mut output = String::with_capacity(svg.len());
        let mut cursor = 0;
        for (range, replacement) in edits {
            output.push_str(&svg[cursor..range.start]);
            output.push_str(&replacement);
            cursor = range.end;
        }
        output.push_str(&svg[cursor..]);
        Cow::Owned(output)
    }

    /// Native code tokens remain independent shaping chunks, with their own
    /// font, paint and f64 origin. Share their enclosing text node per line
    /// instead of retaining a second text/group wrapper for every token.
    /// d2scenebuild/code_shape.go and d2raster/text.go prepare these runs once.
    fn group_native_code_lines(
        &mut self,
        svg: &str,
        document: &roxmltree::Document<'_>,
        identity_counts: &HashMap<&str, usize>,
    ) -> Vec<(std::ops::Range<usize>, String)> {
        if !svg.contains("d2-rs-native-code-text-") {
            return Vec::new();
        }
        let mut edits = Vec::new();
        fn flush(
            sources: &mut SvgImagePathSources,
            svg: &str,
            run: &mut Vec<(roxmltree::Node<'_, '_>, SvgPoint)>,
            edits: &mut Vec<(std::ops::Range<usize>, String)>,
        ) {
            if run.len() > 1 {
                let first = run[0].0;
                let id = first.attribute_node("id").unwrap();
                let mut replacement = format!("<text {} xml:space=\"preserve\">", &svg[id.range()]);
                let mut prototypes = HashMap::new();
                let mut tokens = Vec::with_capacity(run.len());
                for (node, origin) in run.iter() {
                    // d2raster/text.go retains one prepared font and paint.
                    // usvg needs a prototype for each style, not each token.
                    let style = node
                        .attributes()
                        .filter(|a| {
                            !matches!(
                                a.name(),
                                "id" | "x"
                                    | "y"
                                    | "data-d2-rs-text-x-bits"
                                    | "data-d2-rs-text-y-bits"
                            )
                        })
                        .map(|a| (a.namespace(), a.name(), a.value()))
                        .collect::<Vec<_>>();
                    let next = prototypes.len();
                    let prototype = *prototypes.entry(style).or_insert(next);
                    tokens.push(NativeCodeToken {
                        text: node.text().unwrap().to_owned(),
                        origin: *origin,
                        parsed_origin: SvgPoint {
                            x: f64::from(node.attribute("x").unwrap().parse::<f32>().unwrap()),
                            y: f64::from(node.attribute("y").unwrap().parse::<f32>().unwrap()),
                        },
                        prototype,
                    });
                    if prototype != next {
                        continue;
                    }
                    replacement.push_str("<tspan");
                    // Origins are already retained in code_lines.
                    // d2raster keeps the prepared run, rather than a second
                    // copy of its precision transport in the parser input.
                    for attribute in node.attributes().filter(|a| {
                        !matches!(
                            a.name(),
                            "id" | "data-d2-rs-text-x-bits" | "data-d2-rs-text-y-bits"
                        )
                    }) {
                        replacement.push(' ');
                        replacement.push_str(&svg[attribute.range()]);
                    }
                    replacement.push('>');
                    // d2raster/text.go prepares one native glyph run. usvg
                    // needs only a prototype for the face, paint and layout
                    // axes, not a second outline of every source glyph.
                    // All bundled mono faces cover this simple outline. Its
                    // ink is unused: native glyphs come from the full source.
                    replacement.push('-');
                    replacement.push_str("</tspan>");
                }
                replacement.push_str("</text>");
                sources.code_lines.insert(id.value().to_owned(), tokens);
                sources
                    .text_origins_by_id
                    .insert(id.value().to_owned(), run[0].1);
                edits.push((
                    first.range().start..run.last().unwrap().0.range().end,
                    replacement,
                ));
            }
            run.clear();
        }
        for group in document.descendants().filter(|node| {
            node.has_tag_name("g")
                && matches!(node.attribute("class"), Some("light-code" | "dark-code"))
        }) {
            let mut run: Vec<(roxmltree::Node<'_, '_>, SvgPoint)> = Vec::new();
            for node in group.children().filter(|node| node.is_element()) {
                // Synthetic italic uses a separate transform wrapper. Keep
                // it, raw/control text and decorations on their existing path.
                let eligible = node.has_tag_name("text")
                    && matches!(
                        node.attribute("class"),
                        Some("text-mono" | "text-mono-bold")
                    )
                    && node.attribute("text-anchor") == Some("start")
                    && node.attributes().all(|attribute| {
                        (attribute.namespace() == Some("http://www.w3.org/XML/1998/namespace")
                            && attribute.name() == "space")
                            || (attribute.namespace().is_none()
                                && matches!(
                                    attribute.name(),
                                    "class"
                                        | "x"
                                        | "y"
                                        | "text-anchor"
                                        | "font-size"
                                        | "fill"
                                        | "font-weight"
                                        | "id"
                                        | "data-d2-rs-text-x-bits"
                                        | "data-d2-rs-text-y-bits"
                                ))
                    })
                    && node.attribute("text-decoration").is_none()
                    && node.attribute("font-style").is_none()
                    && node.children().count() == 1
                    && node.first_child().is_some_and(|n| n.is_text())
                    && node.text().is_some_and(|text| {
                        text.bytes().all(|b| (32..=126).contains(&b))
                            && text.bytes().any(|b| b != b' ')
                    })
                    && node.attribute("id").is_some_and(|id| {
                        id.starts_with("d2-rs-native-code-text-")
                            && identity_counts.get(id) == Some(&1)
                    })
                    && node.attribute("data-d2-rs-text-raw") != Some("true");
                let origin = eligible
                    .then(|| {
                        for axis in ["x", "y"] {
                            if !node.attribute(axis)?.parse::<f32>().ok()?.is_finite() {
                                return None;
                            }
                        }
                        let operand = |name| {
                            node.attribute(name)
                                .and_then(|bits| u64::from_str_radix(bits, 16).ok())
                                .map(f64::from_bits)
                        };
                        Some(SvgPoint {
                            x: operand("data-d2-rs-text-x-bits")?,
                            y: operand("data-d2-rs-text-y-bits")?,
                        })
                        .filter(|origin| origin.finite())
                    })
                    .flatten();
                if let Some(origin) = origin {
                    if run
                        .last()
                        .is_some_and(|(_, previous)| previous.y.to_bits() != origin.y.to_bits())
                    {
                        flush(self, svg, &mut run, &mut edits);
                    }
                    run.push((node, origin));
                } else {
                    flush(self, svg, &mut run, &mut edits);
                }
            }
            flush(self, svg, &mut run, &mut edits);
        }
        edits
    }

    pub(crate) fn text_stroke_width(&self, id: &str, parsed: f32) -> f64 {
        self.text_stroke_widths_by_id
            .get(id)
            .copied()
            .filter(|width| (*width as f32).to_bits() == parsed.to_bits())
            .unwrap_or_else(|| f64::from(parsed))
    }

    pub(crate) fn text_font_size(&self, id: &str, parsed: f32) -> f64 {
        self.text_font_sizes_by_id
            .get(id)
            .copied()
            .filter(|size| (*size as f32).to_bits() == parsed.to_bits())
            .unwrap_or_else(|| f64::from(parsed))
    }

    pub(crate) fn exact_text_origin(
        &self,
        text: &str,
        near_x: f64,
        near_y: f64,
    ) -> Option<SvgPoint> {
        self.text_origins
            .iter()
            .filter(|origin| origin.text == text)
            .min_by(|left, right| {
                let left_distance = (left.x - near_x).powi(2) + (left.y - near_y).powi(2);
                let right_distance = (right.x - near_x).powi(2) + (right.y - near_y).powi(2);
                left_distance.total_cmp(&right_distance)
            })
            .map(|origin| SvgPoint {
                x: origin.x,
                y: origin.y,
            })
    }
}

fn point(x: f64, y: f64, absolute: bool, current: SvgPoint) -> SvgPoint {
    if absolute {
        SvgPoint { x, y }
    } else {
        SvgPoint {
            x: current.x + x,
            y: current.y + y,
        }
    }
}

fn reflected(control: SvgPoint, around: SvgPoint) -> SvgPoint {
    SvgPoint {
        x: 2.0 * around.x - control.x,
        y: 2.0 * around.y - control.y,
    }
}

/// Expand all relative commands before any coordinate is converted to f32.
/// D2's SVG importer stores absolute float64 scene commands, then applies the
/// image transform and flattens the curves in scene space.
fn parse_path(data: &str) -> Option<SvgPath> {
    let mut commands = Vec::new();
    let mut current = SvgPoint::default();
    let mut subpath_start = SvgPoint::default();
    let mut previous = ' ';
    let mut last_cubic_control = None;
    let mut last_quadratic_control = None;

    for segment in PathParser::from(data) {
        let segment = segment.ok()?;
        match segment {
            PathSegment::MoveTo { abs, x, y } => {
                let end = point(x, y, abs, current);
                commands.push(SvgPathCommand::MoveTo(end));
                current = end;
                subpath_start = end;
                previous = 'M';
                last_cubic_control = None;
                last_quadratic_control = None;
            }
            PathSegment::LineTo { abs, x, y } => {
                let end = point(x, y, abs, current);
                commands.push(SvgPathCommand::LineTo(end));
                current = end;
                previous = 'L';
                last_cubic_control = None;
                last_quadratic_control = None;
            }
            PathSegment::HorizontalLineTo { abs, x } => {
                let end = SvgPoint {
                    x: if abs { x } else { current.x + x },
                    y: current.y,
                };
                commands.push(SvgPathCommand::LineTo(end));
                current = end;
                previous = 'L';
                last_cubic_control = None;
                last_quadratic_control = None;
            }
            PathSegment::VerticalLineTo { abs, y } => {
                let end = SvgPoint {
                    x: current.x,
                    y: if abs { y } else { current.y + y },
                };
                commands.push(SvgPathCommand::LineTo(end));
                current = end;
                previous = 'L';
                last_cubic_control = None;
                last_quadratic_control = None;
            }
            PathSegment::CurveTo {
                abs,
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                let control1 = point(x1, y1, abs, current);
                let control2 = point(x2, y2, abs, current);
                let end = point(x, y, abs, current);
                commands.push(SvgPathCommand::CubicTo(control1, control2, end));
                current = end;
                previous = 'C';
                last_cubic_control = Some(control2);
                last_quadratic_control = None;
            }
            PathSegment::SmoothCurveTo { abs, x2, y2, x, y } => {
                let control1 = if matches!(previous, 'C' | 'S') {
                    reflected(last_cubic_control?, current)
                } else {
                    current
                };
                let control2 = point(x2, y2, abs, current);
                let end = point(x, y, abs, current);
                commands.push(SvgPathCommand::CubicTo(control1, control2, end));
                current = end;
                previous = 'S';
                last_cubic_control = Some(control2);
                last_quadratic_control = None;
            }
            PathSegment::Quadratic { abs, x1, y1, x, y } => {
                let control = point(x1, y1, abs, current);
                let end = point(x, y, abs, current);
                commands.push(SvgPathCommand::QuadraticTo(control, end));
                current = end;
                previous = 'Q';
                last_cubic_control = None;
                last_quadratic_control = Some(control);
            }
            PathSegment::SmoothQuadratic { abs, x, y } => {
                let control = if matches!(previous, 'Q' | 'T') {
                    reflected(last_quadratic_control?, current)
                } else {
                    current
                };
                let end = point(x, y, abs, current);
                commands.push(SvgPathCommand::QuadraticTo(control, end));
                current = end;
                previous = 'T';
                last_cubic_control = None;
                last_quadratic_control = Some(control);
            }
            PathSegment::EllipticalArc {
                abs,
                rx,
                ry,
                x_axis_rotation,
                large_arc,
                sweep,
                x,
                y,
            } => {
                if rx < 0.0 || ry < 0.0 {
                    return None;
                }
                let end = point(x, y, abs, current);
                if end != current {
                    if rx == 0.0 || ry == 0.0 {
                        commands.push(SvgPathCommand::LineTo(end));
                    } else {
                        let rotation =
                            x_axis_rotation - (x_axis_rotation / 360.0).round_ties_even() * 360.0;
                        commands.push(SvgPathCommand::ArcTo {
                            radius_x: rx,
                            radius_y: ry,
                            rotation: rotation / 180.0 * std::f64::consts::PI,
                            large_arc,
                            sweep,
                            end,
                        });
                    }
                }
                current = end;
                previous = 'A';
                last_cubic_control = None;
                last_quadratic_control = None;
            }
            PathSegment::ClosePath { .. } => {
                commands.push(SvgPathCommand::Close);
                current = subpath_start;
                previous = 'Z';
                last_cubic_control = None;
                last_quadratic_control = None;
            }
        }
        if !current.finite() {
            return None;
        }
    }

    (!commands.is_empty()).then_some(SvgPath {
        intrinsic_clips: Vec::new(),
        commands,
        intrinsic_transforms: None,
        intrinsic_viewport: None,
        image_viewport: None,
        paint_opacities: None,
        raster_image: None,
        scene_transform: None,
    })
}

fn math_scene_transform(
    image: roxmltree::Node<'_, '_>,
    root: roxmltree::Node<'_, '_>,
) -> Option<[f64; 6]> {
    let canvas = image.ancestors().find(|n| {
        n.has_tag_name("svg")
            && n.attribute("data-d2-rs-canvas-reference") == Some("ordinary")
            && n.attribute("id")
                .is_some_and(|id| id.starts_with("d2-rs-native-canvas-viewport-"))
    })?;
    let viewport = native_canvas_geometry(root, canvas)?;
    let size = svg_logical_size(root)?;
    // Match d2scene.AspectRatioMatrix before composing the canvas transform.
    let outer = d2_rs_math::svg_viewport::aspect_matrix(
        [0.0, 0.0, viewport.view_box[2], viewport.view_box[3]],
        [0.0, 0.0, size[0], size[1]],
        [if viewport.center { 5 } else { 1 }, 0],
    )
    .ok()?;
    let mut result = source_matrix_mul(
        outer,
        [
            1.0,
            0.0,
            0.0,
            1.0,
            -viewport.view_box[0],
            -viewport.view_box[1],
        ],
    );
    let ancestors: Vec<_> = image.ancestors().take_while(|n| *n != canvas).collect();
    for node in ancestors.into_iter().rev() {
        if !matches!(node.tag_name().name(), "g" | "image") || node.attribute("style").is_some() {
            return None;
        }
        if let Some(raw) = node.attribute("transform") {
            let m = raw.parse::<svgtypes::Transform>().ok()?;
            result = source_matrix_mul(result, [m.a, m.b, m.c, m.d, m.e, m.f]);
        }
    }
    let x = image.attribute("x")?.parse::<f64>().ok()?;
    let y = image.attribute("y")?.parse::<f64>().ok()?;
    result = source_matrix_mul(result, [1.0, 0.0, 0.0, 1.0, x, y]);
    result.iter().all(|v| v.is_finite()).then_some(result)
}

pub(crate) fn source_matrix_mul(l: [f64; 6], r: [f64; 6]) -> [f64; 6] {
    let sum = |a: f64, b: f64, c: f64, d: f64| {
        let second = c * d;
        if cfg!(target_arch = "aarch64") {
            a.mul_add(b, second)
        } else {
            a * b + second
        }
    };
    [
        sum(l[0], r[0], l[2], r[1]),
        sum(l[1], r[0], l[3], r[1]),
        sum(l[0], r[2], l[2], r[3]),
        sum(l[1], r[2], l[3], r[3]),
        sum(l[0], r[4], l[2], r[5]) + l[4],
        sum(l[1], r[4], l[3], r[5]) + l[5],
    ]
}

fn parse_simple_svg_paths(source: &str) -> Option<Vec<SvgPath>> {
    let document = roxmltree::Document::parse(source).ok()?;
    let root = document.root_element();
    if root.tag_name().name() != "svg"
        || root
            .tag_name()
            .namespace()
            .is_some_and(|namespace| namespace != SVG_NAMESPACE)
    {
        return None;
    }

    let mut paths = Vec::new();
    for child in root.children().filter(roxmltree::Node::is_element) {
        append_simple_svg_path(child, &mut paths)?;
    }
    if root.attribute("data-d2-rs-intrinsic-math") == Some("ordinary") {
        let width = root.attribute("width")?.parse::<f64>().ok()?;
        let height = root.attribute("height")?.parse::<f64>().ok()?;
        let values = svgtypes::NumberListParser::from(root.attribute("viewBox")?)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        let [x, y, w, h] = values.as_slice() else {
            return None;
        };
        if [width, height, *w, *h]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
            || !x.is_finite()
            || !y.is_finite()
            || root.attribute("preserveAspectRatio").is_some()
        {
            return None;
        }
        let scale = (width / w).min(height / h);
        // Keep viewport translate/scale/translate factors in Source order.
        let viewport = vec![
            [
                1.0,
                0.0,
                0.0,
                1.0,
                (width - w * scale) / 2.0,
                (height - h * scale) / 2.0,
            ],
            [scale, 0.0, 0.0, scale, 0.0, 0.0],
            [1.0, 0.0, 0.0, 1.0, -x, -y],
        ];
        let viewport = vec![
            viewport
                .into_iter()
                .fold([1.0, 0.0, 0.0, 1.0, 0.0, 0.0], source_matrix_mul),
        ];
        let nodes: Vec<_> = root
            .descendants()
            .filter(|n| {
                n.is_element()
                    && matches!(
                        n.tag_name().name(),
                        "path" | "rect" | "circle" | "ellipse" | "polygon" | "polyline" | "line"
                    )
                    && !(n.tag_name().name() == "path"
                        && n.attribute("d").is_some_and(|d| d.trim().is_empty()))
                    && !simple_svg_path_is_unpainted(*n)
            })
            .collect();
        if nodes.len() != paths.len() {
            return None;
        }
        for (path, node) in paths.iter_mut().zip(nodes) {
            let mut transforms = viewport.clone();
            let mut clips = Vec::new();
            let ancestors: Vec<_> = node.ancestors().take_while(|n| *n != root).collect();
            for n in ancestors.into_iter().rev() {
                if n.attribute("style").is_some() {
                    return None;
                }
                if n.has_tag_name("svg") {
                    if n.attribute("transform").is_some() || n.attribute("clip-path").is_some() {
                        return None;
                    }
                    let position = |name| {
                        n.attribute(name)
                            .map_or(Some(0.0), |v| v.parse::<f64>().ok())
                            .filter(|v| v.is_finite())
                    };
                    transforms.push([1.0, 0.0, 0.0, 1.0, position("x")?, position("y")?]);
                    let (content, size) = intrinsic_viewport(n)?;
                    clips.push(IntrinsicClip {
                        id: n.attribute("id")?.to_owned(),
                        transforms: transforms.clone(),
                        size,
                    });
                    transforms.push(content);
                }
                if let Some(raw) = n.attribute("transform") {
                    let m = raw.parse::<svgtypes::Transform>().ok()?;
                    let matrix = [m.a, m.b, m.c, m.d, m.e, m.f];
                    if matrix.iter().any(|v| !v.is_finite()) {
                        return None;
                    }
                    transforms.push(matrix);
                }
            }
            path.intrinsic_clips = clips;
            path.intrinsic_transforms = Some(transforms);
            path.intrinsic_viewport = Some([width, height]);
        }
    }
    (!paths.is_empty()).then_some(paths)
}

// Bind geometry to the actual retained SVG path ID rather than assuming
// every XML element produces exactly one rendered path. Source SVG import
// accepts non-rendering editor metadata and drops unpainted primitives.
// The primary tree still resolves visibility, styles, transforms and uses;
// this side channel preserves only coordinates of an unambiguous path.
fn parse_svg_paths_by_id(source: &str) -> HashMap<String, SvgPath> {
    let Ok(document) = roxmltree::Document::parse(source) else {
        return HashMap::new();
    };
    let mut paths = HashMap::new();
    let mut duplicates = HashSet::new();
    let mut identities = HashSet::new();
    for id in document
        .descendants()
        .filter_map(|node| node.attribute("id"))
    {
        if !identities.insert(id) {
            duplicates.insert(id.to_owned());
        }
    }
    for node in document.descendants().filter(|node| {
        node.is_element()
            && matches!(node.tag_name().name(), "path" | "image" | "rect")
            && node
                .tag_name()
                .namespace()
                .is_none_or(|ns| ns == SVG_NAMESPACE)
    }) {
        let Some(id) = node.attribute("id").filter(|id| !id.is_empty()) else {
            continue;
        };
        let path = if node.has_tag_name("image") {
            // A generated usvg aspect clip can be handled by the original PNG
            // box sampler only when no authored image effect/clip is present.
            if ["clip-path", "mask", "filter"]
                .iter()
                .any(|name| node.attribute(*name).is_some())
                || document
                    .descendants()
                    .filter(|n| n.has_tag_name("style"))
                    .any(|n| {
                        n.text().is_some_and(|text| {
                            ["clip-path", "mask", "filter"]
                                .iter()
                                .any(|name| text.contains(name))
                        })
                    })
                || node.attribute("style").is_some_and(|text| {
                    ["clip-path", "mask", "filter"]
                        .iter()
                        .any(|name| text.contains(name))
                })
            {
                continue;
            }
            let href = node
                .attribute("href")
                .or_else(|| node.attribute(("http://www.w3.org/1999/xlink", "href")));
            if !href.is_some_and(|href| href.starts_with("data:image/png;base64,")) {
                continue;
            }
            let length = |name, default| {
                node.attribute(name).map_or(Some(default), |v| {
                    v.trim()
                        .strip_suffix("px")
                        .unwrap_or(v.trim())
                        .parse::<f64>()
                        .ok()
                })
            };
            let image_box = [
                length("x", 0.0),
                length("y", 0.0),
                length("width", 0.0),
                length("height", 0.0),
            ];
            let Some(image_box) = image_box.into_iter().collect::<Option<Vec<_>>>() else {
                continue;
            };
            let image_box: [f64; 4] = image_box.try_into().unwrap();
            if image_box.iter().any(|v| !v.is_finite())
                || image_box[2] <= 0.0
                || image_box[3] <= 0.0
            {
                continue;
            }
            let Some(aspect) =
                crate::png_image::ImageAspect::parse(node.attribute("preserveAspectRatio"))
            else {
                continue;
            };
            SvgPath {
                commands: Vec::new(),
                intrinsic_clips: Vec::new(),
                intrinsic_transforms: None,
                scene_transform: None,
                intrinsic_viewport: None,
                image_viewport: None,
                paint_opacities: None,
                raster_image: Some(SceneImagePlacement {
                    matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                    image_box,
                    aspect,
                }),
            }
        } else if node.has_tag_name("rect") {
            let mut rectangles = Vec::new();
            if append_simple_svg_path(node, &mut rectangles).is_none() || rectangles.len() != 1 {
                continue;
            }
            rectangles.pop().unwrap()
        } else {
            let Some(path) = node.attribute("d").and_then(parse_path) else {
                continue;
            };
            path
        };
        if paths.insert(id.to_owned(), path).is_some() {
            duplicates.insert(id.to_owned());
        }
    }
    for id in duplicates {
        paths.remove(&id);
    }
    paths
}

// Original stylesheet.go accepts only one simple ASCII class selector per
// rule. Internal path identities cannot affect those selectors. Keep general
// CSS and use instances on their prior path rather than changing ID semantics.
fn tag_anonymous_imported_paths(source: &str) -> String {
    let Ok(document) = roxmltree::Document::parse(source) else {
        return source.to_owned();
    };
    let root = document.root_element();
    if root.attribute("data-d2-rs-intrinsic-math").is_some()
        || document.descendants().any(|n| n.has_tag_name("use"))
    {
        return source.to_owned();
    }
    for style in document.descendants().filter(|n| n.has_tag_name("style")) {
        let text: String = style
            .children()
            .filter_map(|n| if n.is_text() { n.text() } else { None })
            .collect();
        let mut remaining = text.trim();
        while !remaining.is_empty() {
            let Some((selector, body)) = remaining.split_once('{') else {
                return source.to_owned();
            };
            let Some(class) = selector.trim().strip_prefix('.') else {
                return source.to_owned();
            };
            if class.is_empty()
                || !class.bytes().enumerate().all(|(index, byte)| {
                    byte.is_ascii_alphabetic()
                        || byte == b'_'
                        || (index > 0 && (byte.is_ascii_digit() || byte == b'-'))
                })
            {
                return source.to_owned();
            }
            let Some((declarations, rest)) = body.split_once('}') else {
                return source.to_owned();
            };
            if declarations.contains('{') {
                return source.to_owned();
            }
            remaining = rest.trim();
        }
    }
    let mut used: HashSet<String> = document
        .descendants()
        .filter_map(|n| n.attribute("id").map(str::to_owned))
        .collect();
    let mut insertions = Vec::new();
    for node in document.descendants().filter(|n| {
        (n.has_tag_name("path") || n.has_tag_name("image") || n.has_tag_name("rect"))
            && n.attribute("id").is_none()
    }) {
        if node
            .tag_name()
            .namespace()
            .is_some_and(|namespace| namespace != SVG_NAMESPACE)
        {
            continue;
        }
        let mut suffix = node.range().start;
        let mut id = format!("d2-rs-imported-path-{suffix}");
        while used.contains(&id) {
            suffix += 1;
            id = format!("d2-rs-imported-path-{suffix}");
        }
        let Some(position) =
            start_tag_insert_position(source, node.range().start, node.range().end)
        else {
            continue;
        };
        used.insert(id.clone());
        insertions.push((position, format!(" id=\"{id}\"")));
    }
    let mut tagged = source.to_owned();
    for (position, insertion) in insertions.into_iter().rev() {
        tagged.insert_str(position, &insertion);
    }
    tagged
}

// Original style.go:declarationsFor/computeStyle inherits fill/stroke opacity,
// applies attributes, then class rules in source order, then inline declarations.
fn imported_paint_opacities(node: roxmltree::Node<'_, '_>) -> Option<[f64; 2]> {
    let mut rules = Vec::new();
    for style in node
        .document()
        .descendants()
        .filter(|n| n.has_tag_name("style"))
    {
        let text: String = style
            .children()
            .filter_map(|n| if n.is_text() { n.text() } else { None })
            .collect();
        if text.contains("/*") || text.contains("*/") {
            return None;
        }
        let mut remaining = text.trim();
        while !remaining.is_empty() {
            let (selector, body) = remaining.split_once('{')?;
            let class = selector.trim().strip_prefix('.')?;
            if class.is_empty()
                || !class.bytes().enumerate().all(|(index, byte)| {
                    byte.is_ascii_alphabetic()
                        || byte == b'_'
                        || (index > 0 && (byte.is_ascii_digit() || byte == b'-'))
                })
            {
                return None;
            }
            let (declarations, rest) = body.split_once('}')?;
            if declarations.contains('{') || declarations.contains('!') {
                return None;
            }
            rules.push((class.to_owned(), declarations.to_owned()));
            remaining = rest.trim();
        }
    }
    let declarations = |raw: &str, values: &mut HashMap<String, String>| -> Option<()> {
        for entry in raw.split(';').filter(|entry| !entry.trim().is_empty()) {
            let (name, value) = entry.split_once(':')?;
            let name = name.trim().to_ascii_lowercase();
            if matches!(name.as_str(), "fill-opacity" | "stroke-opacity") {
                values.insert(name, value.trim().to_owned());
            }
        }
        Some(())
    };
    let mut opacity = [1.0, 1.0];
    for ancestor in node
        .ancestors()
        .filter(|n| n.is_element())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let mut values = HashMap::new();
        for property in ["fill-opacity", "stroke-opacity"] {
            if let Some(value) = ancestor.attribute(property) {
                values.insert(property.to_owned(), value.trim().to_owned());
            }
        }
        for (class, raw) in &rules {
            if ancestor
                .attribute("class")
                .is_some_and(|classes| classes.split_ascii_whitespace().any(|c| c == class))
            {
                declarations(raw, &mut values)?;
            }
        }
        if let Some(raw) = ancestor.attribute("style") {
            declarations(raw, &mut values)?;
        }
        for (index, property) in ["fill-opacity", "stroke-opacity"].into_iter().enumerate() {
            if let Some(raw) = values.get(property) {
                if raw.eq_ignore_ascii_case("inherit") {
                    continue;
                }
                let value = raw.parse::<f64>().ok()?;
                if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return None;
                }
                opacity[index] = value;
            }
        }
    }
    Some(opacity)
}

// Original assets.go retains intrinsic viewport mapping before image.go
// applies the display mapping. Identity-bound paths need this chain even
// when metadata/styles/resources prevent safe ordinal correspondence.
fn qualify_image_path_transform(
    path: &mut SvgPath,
    node: roxmltree::Node<'_, '_>,
    image: roxmltree::Node<'_, '_>,
    outer_root: roxmltree::Node<'_, '_>,
) -> Option<()> {
    let root = node.document().root_element();
    if root.attribute("data-d2-rs-intrinsic-math").is_some()
        || root.attribute("transform").is_some()
        || root.descendants().any(|n| n.has_tag_name("use"))
    {
        return None;
    }
    if root.descendants().any(|n| {
        n.is_element()
            && (n.attribute("style").is_some_and(|s| {
                s.split(';')
                    .filter_map(|p| p.split_once(':'))
                    .any(|(k, _)| k.trim().starts_with("transform"))
            }) || (n.has_tag_name("style")
                && n.text()
                    .is_some_and(|s| s.to_ascii_lowercase().contains("transform"))))
    }) {
        return None;
    }
    let length = |n: roxmltree::Node<'_, '_>, name| {
        n.attribute(name)
            .map(|v| {
                v.trim()
                    .strip_suffix("px")
                    .unwrap_or(v.trim())
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
            })
            .unwrap_or(Some(0.0))
    };
    let viewbox = root.attribute("viewBox").and_then(|v| {
        svgtypes::NumberListParser::from(v)
            .collect::<Result<Vec<_>, _>>()
            .ok()
    });
    let mut width = length(root, "width")?;
    let mut height = length(root, "height")?;
    let viewbox = match viewbox {
        Some(v) if v.len() == 4 && v.iter().all(|v| v.is_finite()) && v[2] > 0.0 && v[3] > 0.0 => {
            [v[0], v[1], v[2], v[3]]
        }
        None if width > 0.0 && height > 0.0 => [0.0, 0.0, width, height],
        _ => return None,
    };
    if root.attribute("width").is_none() && root.attribute("height").is_none() {
        width = viewbox[2];
        height = viewbox[3];
    } else if root.attribute("width").is_none() {
        width = height * viewbox[2] / viewbox[3];
    } else if root.attribute("height").is_none() {
        height = width * viewbox[3] / viewbox[2];
    }
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let aspect = |source: [f64; 4], w: f64, h: f64, raw: Option<&str>| {
        // Use the native aspect implementation, including master's explicit
        // product rounding in d2scene/aspect.go.
        let policy =
            d2_rs_math::svg_viewport::aspect("", raw.unwrap_or("xMidYMid meet"), "image").ok()?;
        d2_rs_math::svg_viewport::aspect_matrix(source, [0.0, 0.0, w, h], policy).ok()
    };
    let display_width = length(image, "width")?;
    let display_height = length(image, "height")?;
    if display_width <= 0.0 || display_height <= 0.0 {
        return None;
    }
    let mut chain = vec![
        aspect(
            [0.0, 0.0, width, height],
            display_width,
            display_height,
            image.attribute("preserveAspectRatio"),
        )?,
        aspect(
            viewbox,
            width,
            height,
            root.attribute("preserveAspectRatio"),
        )?,
    ];
    let ancestors: Vec<_> = node.ancestors().take_while(|n| *n != root).collect();
    for n in ancestors.into_iter().rev() {
        if !matches!(n.tag_name().name(), "g" | "path" | "image" | "rect") {
            return None;
        }
        if let Some(raw) = n.attribute("transform") {
            let m = raw.parse::<svgtypes::Transform>().ok()?;
            let m = [m.a, m.b, m.c, m.d, m.e, m.f];
            if m.iter().any(|v| !v.is_finite()) {
                return None;
            }
            chain.push(m);
        }
    }
    let scene = math_scene_transform(image, outer_root)?;
    path.paint_opacities = imported_paint_opacities(node);
    path.image_viewport = Some((chain[0], [width, height]));
    path.intrinsic_transforms = Some(chain);
    path.scene_transform = Some(scene);
    path.intrinsic_viewport = Some([display_width, display_height]);
    Some(())
}
// Original d2svgimport.geometry.primitive drops hidden and unpainted paths.
// Keep the raw-path ordinal list and intrinsic transform owners in step with
// the converted tree when presentation attributes resolve this unambiguously.
// Paint/visibility CSS remains the primary tree's responsibility.
fn simple_svg_path_is_unpainted(node: roxmltree::Node<'_, '_>) -> bool {
    // Original d2svgimport/compile.go geometryRect emits no primitive for a
    // zero dimension. Keep these nodes out of both retained path ordinals and
    // transform-owner enumeration, without disabling other precise paths.
    if node.has_tag_name("rect") {
        let dimensions = ["width", "height"]
            .map(|name| node.attribute(name).and_then(|v| v.parse::<f64>().ok()));
        if let [Some(w), Some(h)] = dimensions {
            return w.is_finite()
                && h.is_finite()
                && w >= 0.0
                && h >= 0.0
                && (w == 0.0 || h == 0.0);
        }
        return false;
    }
    if !node.has_tag_name("path") {
        return false;
    }
    if node.ancestors().any(|ancestor| {
        ancestor.attribute("style").is_some_and(|style| {
            style
                .split(';')
                .filter_map(|entry| entry.split_once(':'))
                .any(|(name, _)| {
                    matches!(
                        name.trim(),
                        "fill" | "stroke" | "stroke-width" | "visibility" | "display"
                    )
                })
        })
    }) {
        return false;
    }
    let inherited = |name| {
        node.ancestors()
            .find_map(|ancestor| ancestor.attribute(name).filter(|value| *value != "inherit"))
    };
    if matches!(inherited("visibility"), Some("hidden" | "collapse")) {
        return true;
    }
    inherited("fill") == Some("none")
        && (inherited("stroke").is_none_or(|value| value == "none")
            || inherited("stroke-width").and_then(|value| value.parse::<f64>().ok()) == Some(0.0))
}

fn append_simple_svg_path(node: roxmltree::Node<'_, '_>, paths: &mut Vec<SvgPath>) -> Option<()> {
    let tag = node.tag_name();
    if tag
        .namespace()
        .is_some_and(|namespace| namespace != SVG_NAMESPACE)
    {
        return None;
    }
    match tag.name() {
        "title" | "desc" | "metadata" => Some(()),
        "path" => {
            if node.attribute("display") == Some("none") {
                return None;
            }
            let data = node.attribute("d")?;
            // Original SVG import emits no painted primitive for empty path data.
            if data.trim().is_empty() || simple_svg_path_is_unpainted(node) {
                return Some(());
            }
            paths.push(parse_path(data)?);
            Some(())
        }
        "line" => {
            if node.attribute("display") == Some("none") || node.attribute("style").is_some() {
                return None;
            }
            // SVG lines have no painted fill. Preserve ordinal correspondence
            // only when a stroke is explicit on the line or an ancestor;
            // unresolved stylesheet paint keeps the existing usvg fallback.
            let stroke = node.ancestors().find_map(|n| n.attribute("stroke"))?;
            if !matches!(
                svgtypes::Paint::from_str(stroke).ok()?,
                svgtypes::Paint::Color(_) | svgtypes::Paint::CurrentColor
            ) {
                return None;
            }
            let stroke_width = node
                .ancestors()
                .find_map(|n| n.attribute("stroke-width"))
                .map_or(Some(1.0), |v| v.parse::<f64>().ok())?;
            if !stroke_width.is_finite() || stroke_width <= 0.0 {
                return None;
            }
            let number = |name| {
                node.attribute(name)
                    .map_or(Some(0.0), |v| v.parse::<f64>().ok())
                    .filter(|v| v.is_finite())
            };
            let (x1, y1, x2, y2) = (number("x1")?, number("y1")?, number("x2")?, number("y2")?);
            if x1 == x2 && y1 == y2 {
                return None;
            }
            paths.push(SvgPath {
                intrinsic_clips: Vec::new(),
                intrinsic_transforms: None,
                intrinsic_viewport: None,
                image_viewport: None,
                paint_opacities: None,
                raster_image: None,
                scene_transform: None,
                commands: vec![
                    SvgPathCommand::MoveTo(SvgPoint { x: x1, y: y1 }),
                    SvgPathCommand::LineTo(SvgPoint { x: x2, y: y2 }),
                ],
            });
            Some(())
        }
        "rect" => {
            if simple_svg_path_is_unpainted(node) {
                return Some(());
            }
            let number = |name, default| {
                node.attribute(name)
                    .map_or(Some(default), |v| v.parse::<f64>().ok())
                    .filter(|v| v.is_finite())
            };
            let x = number("x", 0.0)?;
            let y = number("y", 0.0)?;
            let w = number("width", 0.0)?;
            let h = number("height", 0.0)?;
            if w <= 0.0
                || h <= 0.0
                || number("rx", 0.0)? != 0.0
                || number("ry", 0.0)? != 0.0
                || !((x + w).is_finite() && (y + h).is_finite())
                || node.attribute("display") == Some("none")
            {
                return None;
            }
            paths.push(SvgPath {
                intrinsic_clips: Vec::new(),
                intrinsic_transforms: None,
                intrinsic_viewport: None,
                image_viewport: None,
                paint_opacities: None,
                raster_image: None,
                scene_transform: None,
                commands: vec![
                    SvgPathCommand::MoveTo(SvgPoint { x, y }),
                    SvgPathCommand::LineTo(SvgPoint { x: x + w, y }),
                    SvgPathCommand::LineTo(SvgPoint { x: x + w, y: y + h }),
                    SvgPathCommand::LineTo(SvgPoint { x, y: y + h }),
                    SvgPathCommand::Close,
                ],
            });
            Some(())
        }
        "circle" | "ellipse" => {
            if node.attribute("display") == Some("none") {
                return None;
            }
            let number = |name, default| {
                node.attribute(name)
                    .map_or(Some(default), |v| v.parse::<f64>().ok())
                    .filter(|v| v.is_finite())
            };
            let cx = number("cx", 0.0)?;
            let cy = number("cy", 0.0)?;
            let (rx, ry) = if tag.name() == "circle" {
                let r = number("r", 0.0)?;
                (r, r)
            } else {
                (number("rx", 0.0)?, number("ry", 0.0)?)
            };
            // Zero/negative shapes do not have a one-to-one retained path.
            if rx <= 0.0 || ry <= 0.0 {
                return None;
            }
            if [cx + rx, cx - rx, cy + ry, cy - ry]
                .iter()
                .any(|v| !v.is_finite())
            {
                return None;
            }
            let k = 0.5522847498307936;
            let p = |x, y| SvgPoint { x, y };
            paths.push(SvgPath {
                intrinsic_clips: Vec::new(),
                intrinsic_transforms: None,
                intrinsic_viewport: None,
                image_viewport: None,
                paint_opacities: None,
                raster_image: None,
                scene_transform: None,
                commands: vec![
                    SvgPathCommand::MoveTo(p(cx + rx, cy)),
                    SvgPathCommand::CubicTo(
                        p(cx + rx, cy + k * ry),
                        p(cx + k * rx, cy + ry),
                        p(cx, cy + ry),
                    ),
                    SvgPathCommand::CubicTo(
                        p(cx - k * rx, cy + ry),
                        p(cx - rx, cy + k * ry),
                        p(cx - rx, cy),
                    ),
                    SvgPathCommand::CubicTo(
                        p(cx - rx, cy - k * ry),
                        p(cx - k * rx, cy - ry),
                        p(cx, cy - ry),
                    ),
                    SvgPathCommand::CubicTo(
                        p(cx + k * rx, cy - ry),
                        p(cx + rx, cy - k * ry),
                        p(cx + rx, cy),
                    ),
                    SvgPathCommand::Close,
                ],
            });
            Some(())
        }
        "polygon" | "polyline" => {
            if node.attribute("display") == Some("none") {
                return None;
            }
            let values = svgtypes::NumberListParser::from(node.attribute("points")?)
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            if values.len() % 2 != 0 {
                return None;
            }
            let points = values
                .chunks_exact(2)
                .map(|pair| (pair[0], pair[1]))
                .collect::<Vec<_>>();
            if points.len() < 2 || points.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()) {
                return None;
            }
            let mut commands = vec![SvgPathCommand::MoveTo(SvgPoint {
                x: points[0].0,
                y: points[0].1,
            })];
            commands.extend(
                points[1..]
                    .iter()
                    .map(|(x, y)| SvgPathCommand::LineTo(SvgPoint { x: *x, y: *y })),
            );
            if tag.name() == "polygon" {
                commands.push(SvgPathCommand::Close);
            }
            paths.push(SvgPath {
                intrinsic_clips: Vec::new(),
                commands,
                intrinsic_transforms: None,
                intrinsic_viewport: None,
                image_viewport: None,
                paint_opacities: None,
                raster_image: None,
                scene_transform: None,
            });
            Some(())
        }
        "g" | "svg" => {
            if tag.name() == "svg" {
                intrinsic_viewport(node)?;
            }
            for child in node.children().filter(roxmltree::Node::is_element) {
                append_simple_svg_path(child, paths)?;
            }
            Some(())
        }
        // usvg supplies the resolved group transform and inherited paint on
        // each path. Definitions, stylesheets, and other elements need their
        // own source ordering or resource resolution, so retain the fallback.
        _ => None,
    }
}

fn attribute_value<'a, 'input: 'a>(attribute: roxmltree::Attribute<'a, 'input>) -> &'a str {
    attribute.value()
}

fn image_href<'a, 'input: 'a>(image: roxmltree::Node<'a, 'input>) -> Option<&'a str> {
    image
        .attributes()
        .find(|attribute| {
            attribute.name() == "href"
                && (attribute.namespace().is_none()
                    || attribute.namespace() == Some(XLINK_NAMESPACE))
        })
        .map(attribute_value)
}

fn start_tag_insert_position(source: &str, start: usize, end: usize) -> Option<usize> {
    let tag = source.get(start..end)?;
    let mut quote = None;
    for (offset, byte) in tag.bytes().enumerate() {
        match (quote, byte) {
            (Some(expected), actual) if expected == actual => quote = None,
            (None, b'\'' | b'"') => quote = Some(byte),
            (None, b'>') => {
                let before_close = start + offset;
                return Some(
                    if source.as_bytes().get(before_close.wrapping_sub(1)) == Some(&b'/') {
                        before_close - 1
                    } else {
                        before_close
                    },
                );
            }
            _ => {}
        }
    }
    None
}

/// Add private IDs to simple embedded SVG images and built-in paper-pattern
/// paths for the native raster pass, retaining their source float64 commands.
/// The public SVG returned to callers is never modified.
// Source d2raster scales float64 logical dimensions before ceil. Keep
// numeric pixel viewports before usvg narrows them; unresolved units use the
// parser's resolved dimensions instead. Nested resource viewports are separate.
fn svg_logical_size(root: roxmltree::Node<'_, '_>) -> Option<[f64; 2]> {
    if root.tag_name().name() != "svg"
        || root
            .tag_name()
            .namespace()
            .is_some_and(|namespace| namespace != SVG_NAMESPACE)
    {
        return None;
    }
    let pixel_length = |value: &str| {
        let value = value.trim();
        value
            .strip_suffix("px")
            .unwrap_or(value)
            .trim()
            .parse::<f64>()
            .ok()
    };
    let size = match (root.attribute("width"), root.attribute("height")) {
        (Some(width), Some(height)) => [pixel_length(width)?, pixel_length(height)?],
        (None, None) => {
            let values = root
                .attribute("viewBox")?
                .split(|c: char| c.is_whitespace() || c == ',')
                .filter(|s| !s.is_empty())
                .map(str::parse::<f64>)
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            if values.len() != 4 || values.iter().any(|value| !value.is_finite()) {
                return None;
            }
            [values[2], values[3]]
        }
        _ => return None,
    };
    size.iter()
        .all(|value| value.is_finite() && *value > 0.0)
        .then_some(size)
}

fn native_canvas_geometry(
    root: roxmltree::Node<'_, '_>,
    canvas: roxmltree::Node<'_, '_>,
) -> Option<NativeCanvasViewport> {
    // This is the renderer's ordinary serialization viewport. Authored
    // viewports and resources keep their independently resolved mapping.
    for node in [root, canvas] {
        if [
            "transform",
            "style",
            "clip-path",
            "mask",
            "filter",
            "opacity",
        ]
        .iter()
        .any(|attribute| node.attribute(*attribute).is_some())
        {
            return None;
        }
    }
    if canvas.attribute("x").is_some() || canvas.attribute("y").is_some() {
        return None;
    }
    let parse_box = |node: roxmltree::Node<'_, '_>| {
        let values = node
            .attribute("viewBox")?
            .split_whitespace()
            .map(str::parse::<f64>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        if values.len() != 4 || values.iter().any(|value| !value.is_finite()) {
            return None;
        }
        Some([values[0], values[1], values[2], values[3]])
    };
    let view_box = parse_box(canvas)?;
    if view_box[2] <= 0.0
        || view_box[3] <= 0.0
        || parse_box(root)? != [0.0, 0.0, view_box[2], view_box[3]]
        || canvas.attribute("width")?.parse::<f64>().ok()? != view_box[2]
        || canvas.attribute("height")?.parse::<f64>().ok()? != view_box[3]
    {
        return None;
    }
    let center = match root.attribute("preserveAspectRatio")? {
        "xMinYMin meet" => false,
        "xMidYMid meet" => true,
        _ => return None,
    };
    Some(NativeCanvasViewport { view_box, center })
}

pub(crate) fn scaled_dimension(logical: f64, scale: u32) -> Option<u32> {
    let scaled = logical * f64::from(scale);
    let rounded = scaled.ceil();
    (scaled.is_finite() && scaled > 0.0 && rounded <= f64::from(u32::MAX)).then_some(rounded as u32)
}

// Preserve authored opacity before usvg narrows it. Only presentation-attribute
// groups without a stylesheet or ancestor style qualify; resolved f32 values
// are checked again at the paint boundary. Identities are local to each asset.
fn tag_imported_group_opacities(source: &str) -> (String, HashMap<String, f64>) {
    let Ok(document) = roxmltree::Document::parse(source) else {
        return (source.to_owned(), HashMap::new());
    };
    if document
        .descendants()
        .any(|node| node.has_tag_name("style"))
    {
        return (source.to_owned(), HashMap::new());
    }
    let mut counts = HashMap::new();
    for id in document
        .descendants()
        .filter_map(|node| node.attribute("id"))
    {
        *counts.entry(id).or_insert(0_usize) += 1;
    }
    let mut values = HashMap::new();
    let mut insertions = Vec::new();
    for node in document.descendants().filter(|node| node.has_tag_name("g")) {
        if node
            .ancestors()
            .any(|ancestor| ancestor.attribute("style").is_some())
        {
            continue;
        }
        let Some(value) = node
            .attribute("opacity")
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        else {
            continue;
        };
        let id = if let Some(id) = node.attribute("id") {
            if counts.get(id) != Some(&1) {
                continue;
            }
            id.to_owned()
        } else {
            let mut suffix = node.range().start;
            let mut id = format!("d2-rs-imported-opacity-{suffix}");
            while counts.contains_key(id.as_str()) {
                suffix += 1;
                id = format!("d2-rs-imported-opacity-{suffix}");
            }
            let Some(position) =
                start_tag_insert_position(source, node.range().start, node.range().end)
            else {
                continue;
            };
            insertions.push((position, format!(" id=\"{id}\"")));
            id
        };
        values.insert(id, value);
    }
    let mut tagged = source.to_owned();
    insertions.sort_by(|a, b| b.0.cmp(&a.0));
    for (position, insertion) in insertions {
        tagged.insert_str(position, &insertion);
    }
    (tagged, values)
}

/// After strict importer admission, adapt only the lexical attribute separator
/// rule to the renderer's XML parser. Go encoding/xml does not require spaces
/// between quoted attributes; resvg's decoder does. Diagnostics use raw assets.
pub(crate) fn separate_embedded_svg_attributes(svg: &str) -> Cow<'_, str> {
    // Only href attributes can contain an embedded SVG. Attribute names are
    // literal XML names, so character references cannot hide this spelling.
    if !svg.contains("href") {
        return Cow::Borrowed(svg);
    }
    let Ok(document) = roxmltree::Document::parse(svg) else {
        return Cow::Borrowed(svg);
    };
    let mut replacements = Vec::new();
    for image in document.descendants().filter(|node| {
        node.is_element()
            && node.tag_name().name() == "image"
            && node
                .tag_name()
                .namespace()
                .is_none_or(|namespace| namespace == SVG_NAMESPACE)
    }) {
        let Some(attribute) = image.attributes().find(|attribute| {
            attribute.name() == "href"
                && matches!(attribute.namespace(), None | Some(XLINK_NAMESPACE))
        }) else {
            continue;
        };
        let Some(bytes) = super::svg_import::svg_data_uri(attribute.value()) else {
            continue;
        };
        let Ok(prepared) =
            super::svg_asset_codec::prepare(&bytes, true, super::svg_asset_codec::CLI_SVG_LIMIT)
        else {
            continue;
        };
        let Ok(source) = std::str::from_utf8(&prepared) else {
            continue;
        };
        let rendered = super::svg_asset_xml::renderer_attribute_separators(source);
        if matches!(prepared, Cow::Owned(_)) || matches!(rendered, Cow::Owned(_)) {
            let payload = base64::engine::general_purpose::STANDARD.encode(rendered.as_bytes());
            replacements.push((
                attribute.range_value(),
                format!("{SVG_DATA_PREFIX}{payload}"),
            ));
        }
    }
    if replacements.is_empty() {
        return Cow::Borrowed(svg);
    }
    let mut rendered = svg.to_owned();
    replacements.sort_by(|a, b| b.0.start.cmp(&a.0.start));
    for (range, value) in replacements {
        rendered.replace_range(range, &value);
    }
    Cow::Owned(rendered)
}

pub(crate) fn tag_embedded_svg_images(svg: &str) -> (Cow<'_, str>, SvgImagePathSources) {
    tag_embedded_svg_images_with_assets(svg, None)
}
pub(crate) fn tag_embedded_svg_images_with_assets(
    svg: &str,
    cache: Option<d2_rs_math::svg_asset_cache::AssetCache>,
) -> (Cow<'_, str>, SvgImagePathSources) {
    let Ok(document) = roxmltree::Document::parse(svg) else {
        return (Cow::Borrowed(svg), SvgImagePathSources::default());
    };

    let mut sources = SvgImagePathSources::default();
    sources.native_cache = cache;
    sources.logical_size = svg_logical_size(document.root_element());
    sources.gradient_stops = gradient_stops_in_document(&document);
    if !svg.contains(SVG_DATA_PREFIX)
        && !svg.contains("paper-")
        && !svg.contains("d2-rs-sketch-streak-")
        && !svg.contains("<text")
        && !svg.contains("data-d2-rs-exact-opacity")
        && !svg.contains("data-d2-rs-scene-image")
        && !svg.contains("data-d2-rs-canvas-reference")
        && !svg.contains("data-d2-rs-markdown-reference")
    {
        return (Cow::Borrowed(svg), sources);
    }
    let mut identity_counts = HashMap::<&str, usize>::new();
    for id in document
        .descendants()
        .filter_map(|node| node.attribute("id"))
    {
        *identity_counts.entry(id).or_default() += 1;
    }
    for node in document.descendants().filter(|node| {
        node.is_element()
            && node.tag_name().name() == "g"
            && node
                .tag_name()
                .namespace()
                .is_none_or(|namespace| namespace == SVG_NAMESPACE)
    }) {
        let Some(id) = node
            .attribute("id")
            .filter(|id| identity_counts.get(id) == Some(&1))
        else {
            continue;
        };
        let Some(value) = node
            .attribute("data-d2-rs-exact-opacity")
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        else {
            continue;
        };
        sources.effect_opacities_by_id.insert(id.to_owned(), value);
    }
    for node in document.descendants().filter(|node| {
        node.is_element()
            && node.tag_name().name() == "image"
            && node.attribute("data-d2-rs-scene-image") == Some("ordinary")
    }) {
        let Some(id) = node
            .attribute("id")
            .filter(|id| identity_counts.get(id) == Some(&1))
        else {
            continue;
        };
        if let Some(placement) = scene_image_placement(node) {
            sources.scene_images_by_id.insert(id.to_owned(), placement);
        }
    }
    // Only the main viewport explicitly marked by the native D2 renderer
    // represents ordinary scene ancestry. Real SVG clip/effect groups and
    // imported resources must not acquire this qualification from geometry.
    for node in document.root_element().children().filter(|node| {
        node.is_element()
            && node.tag_name().name() == "svg"
            && node
                .tag_name()
                .namespace()
                .is_none_or(|namespace| namespace == SVG_NAMESPACE)
            && node.attribute("data-d2-rs-canvas-reference") == Some("ordinary")
            // The renderer-authored wrapper has no explicit clipping or
            // inline styles. Do not qualify a same-ID authored SVG effect.
            && node.attribute("clip-path").is_none()
            && node.attribute("style").is_none()
            && node
                .attribute("class")
                .is_some_and(|classes| classes.split_whitespace().any(|class| class == "d2-svg"))
    }) {
        if let Some(id) = node
            .attribute("id")
            .filter(|id| id.starts_with("d2-rs-native-canvas-viewport-"))
        {
            sources.native_canvas_viewports.insert(id.to_owned());
            if identity_counts.get(id) == Some(&1)
                && let Some(viewport) = native_canvas_geometry(document.root_element(), node)
            {
                sources
                    .native_canvas_geometry
                    .insert(id.to_owned(), viewport);
            }
        }
    }
    // d2scenebuild.buildConnectionMask retains rectangles in scene coordinates.
    // Keep their operands and the canvas mapping before usvg narrows both.
    for mask in document.descendants().filter(|node| {
        node.is_element()
            && node.tag_name().name() == "mask"
            && node.tag_name().namespace() == Some(SVG_NAMESPACE)
            && node.attribute("data-d2-rs-connection-mask") == Some("scene")
            && node.attribute("maskUnits") == Some("userSpaceOnUse")
            && ["transform", "style", "mask", "filter"]
                .iter()
                .all(|key| node.attribute(*key).is_none())
    }) {
        let Some(id) = mask
            .attribute("id")
            .filter(|id| identity_counts.get(id) == Some(&1))
        else {
            continue;
        };
        let Some(viewport) = mask
            .parent_element()
            .and_then(|node| node.attribute("id"))
            .and_then(|id| sources.native_canvas_geometry.get(id))
            .copied()
        else {
            continue;
        };
        let boxes = mask
            .children()
            .filter(|node| node.is_element())
            .map(|node| {
                if node.tag_name().name() != "rect"
                    || node.tag_name().namespace() != Some(SVG_NAMESPACE)
                    || !matches!(
                        node.attribute("fill"),
                        Some("white" | "black" | "rgba(0,0,0,0.75)")
                    )
                    || node
                        .attributes()
                        .any(|a| !matches!(a.name(), "x" | "y" | "width" | "height" | "fill"))
                {
                    return None;
                }
                let values = ["x", "y", "width", "height"]
                    .map(|key| node.attribute(key)?.parse::<f64>().ok());
                let [Some(x), Some(y), Some(w), Some(h)] = values else {
                    return None;
                };
                [x, y, w, h]
                    .iter()
                    .all(|v| v.is_finite())
                    .then_some([x, y, w, h])
                    .filter(|_| w > 0.0 && h > 0.0)
            })
            .collect::<Option<Vec<_>>>();
        if let Some(boxes) = boxes.filter(|boxes| !boxes.is_empty()) {
            sources
                .native_connection_masks
                .insert(id.to_owned(), NativeConnectionMask { viewport, boxes });
        }
    }

    // Native image clips must retain ArcTo operands. usvg converts arcs to
    // f32 cubics before rasterization, unlike d2raster's bounded arc flattener.
    for clip in document.descendants().filter(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(SVG_NAMESPACE)
            && node.tag_name().name() == "clipPath"
            && node.attribute("data-d2-rs-image-radius") == Some("scene")
    }) {
        let Some(id) = clip
            .attribute("id")
            .filter(|id| identity_counts.get(id) == Some(&1))
        else {
            continue;
        };
        if ["transform", "style", "clip-path"]
            .iter()
            .any(|key| clip.attribute(*key).is_some())
            || clip
                .attribute("clipPathUnits")
                .is_some_and(|units| units != "userSpaceOnUse")
            || !clip.ancestors().any(|node| {
                node.attribute("id")
                    .is_some_and(|id| sources.native_canvas_geometry.contains_key(id))
            })
        {
            continue;
        }
        let mut children = clip.children().filter(|node| node.is_element());
        let Some(path) = children.next() else {
            continue;
        };
        if children.next().is_some()
            || !path.has_tag_name((SVG_NAMESPACE, "path"))
            || path
                .attributes()
                .any(|attr| !matches!(attr.name(), "d" | "id"))
        {
            continue;
        }
        if let Some(path) = path.attribute("d").and_then(parse_path) {
            sources.native_image_clips.insert(id.to_owned(), path);
        }
    }

    // Preserve source transform operands only for uniquely identified groups
    // under a qualified native canvas. Resource subroots and CSS transforms
    // must not inherit main-scene precision qualification.
    for node in document.descendants().filter(|n| {
        n.is_element()
            && n.tag_name().namespace() == Some(SVG_NAMESPACE)
            && n.tag_name().name() == "g"
    }) {
        let Some(id) = node
            .attribute("id")
            .filter(|id| identity_counts.get(id) == Some(&1))
        else {
            continue;
        };
        if node.ancestors().any(|n| {
            n.attribute("style").is_some()
                || matches!(
                    n.tag_name().name(),
                    "defs" | "mask" | "clipPath" | "pattern" | "marker" | "symbol"
                )
        }) {
            continue;
        }
        if !node.ancestors().any(|n| {
            n.attribute("id")
                .is_some_and(|id| sources.native_canvas_geometry.contains_key(id))
        }) {
            continue;
        }
        let Some(transform) = node
            .attribute("transform")
            .and_then(|raw| raw.parse::<svgtypes::Transform>().ok())
        else {
            continue;
        };
        let values = [
            transform.a,
            transform.b,
            transform.c,
            transform.d,
            transform.e,
            transform.f,
        ];
        if values.iter().all(|v| v.is_finite()) {
            sources
                .native_group_transforms
                .insert(id.to_owned(), values);
        }
    }
    // usvg starts a rounded <rect> at its top-right corner. Source Rect
    // preparation starts at the left end of the top edge, which determines
    // dash phase. Preserve the primitive operands and identity, rather than
    // rotating arbitrary authored paths that happen to resemble rectangles.
    for node in document.descendants().filter(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(SVG_NAMESPACE)
            && node.tag_name().name() == "rect"
            && node.attribute("stroke-dasharray").is_some()
    }) {
        let Some(id) = node
            .attribute("id")
            .filter(|id| identity_counts.get(id) == Some(&1))
        else {
            continue;
        };
        let Some(owner_node) = node
            .ancestors()
            .find(|parent| parent.attribute("data-d2-rs-dash-owner-id").is_some())
        else {
            continue;
        };
        let owner = owner_node.attribute("data-d2-rs-dash-owner-id").unwrap();
        let operand = |name| {
            owner_node
                .attribute(name)
                .and_then(|bits| u64::from_str_radix(bits, 16).ok())
                .map(f64::from_bits)
                .filter(|value| value.is_finite())
        };
        let valid_dash = operand("data-d2-rs-dash-size-bits")
            .zip(operand("data-d2-rs-dash-gap-bits"))
            .is_some_and(|(dash, gap)| dash >= 0.0 && gap >= 0.0 && dash + gap > 0.0)
            && operand("data-d2-rs-dash-offset-bits").is_some();
        if !valid_dash
            || !owner.contains("-native-raster-shape-")
            || id
                .strip_prefix(owner)
                .and_then(|suffix| suffix.strip_prefix("-primitive-"))
                .and_then(|suffix| suffix.parse::<usize>().ok())
                .is_none()
            || ["style", "transform", "filter", "mask", "clip-path"]
                .iter()
                .any(|name| node.attribute(*name).is_some())
            || !node.ancestors().any(|parent| {
                parent
                    .attribute("id")
                    .is_some_and(|id| sources.native_canvas_geometry.contains_key(id))
            })
        {
            continue;
        }
        let number = |name: &str, default: Option<f64>| match node.attribute(name) {
            Some(value) => value.parse::<f64>().ok().filter(|value| value.is_finite()),
            None => default,
        };
        let Some((x, y, width, height, rx, ry)) = (|| {
            let x = number("x", Some(0.0))?;
            let y = number("y", Some(0.0))?;
            let width = number("width", None)?;
            let height = number("height", None)?;
            let rx = number("rx", number("ry", Some(0.0)))?;
            let ry = number("ry", Some(rx))?;
            (width > 0.0
                && height > 0.0
                && rx >= 0.0
                && ry >= 0.0
                && (x + width).is_finite()
                && (y + height).is_finite())
            .then_some((x, y, width, height, rx, ry))
        })() else {
            continue;
        };
        sources.native_rects.insert(
            id.to_owned(),
            NativeRect {
                box_values: [x, y, width, height],
                radii: [rx, ry],
            },
        );
    }
    // Private structured-shape SVG carries the scene's f64 streak rectangle
    // operands. usvg lowers rectangles to f32 paths; retaining these values
    // keeps d2raster's coverage at fractional row boundaries.
    for node in document.descendants().filter(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(SVG_NAMESPACE)
            && node.tag_name().name() == "rect"
    }) {
        let Some(id) = node.attribute("id").filter(|id| {
            identity_counts.get(id) == Some(&1)
                && (id.contains("-native-class-") || id.contains("-native-sql-"))
                && (id.ends_with("-streak-body") || id.ends_with("-streak-header"))
        }) else {
            continue;
        };
        if node.attribute("stroke") != Some("none")
            || !node
                .attribute("fill")
                .is_some_and(|fill| fill.starts_with("url(#d2-rs-sketch-streak-"))
            || ["style", "transform", "filter", "mask", "clip-path"]
                .iter()
                .any(|name| node.attribute(*name).is_some())
            || !node.ancestors().any(|parent| {
                parent
                    .attribute("id")
                    .is_some_and(|id| sources.native_canvas_geometry.contains_key(id))
            })
        {
            continue;
        }
        let number = |name: &str, default: Option<f64>| match node.attribute(name) {
            Some(value) => value.parse::<f64>().ok().filter(|value| value.is_finite()),
            None => default,
        };
        let Some((x, y, width, height)) = (|| {
            let x = number("x", Some(0.0))?;
            let y = number("y", Some(0.0))?;
            let width = number("width", None)?;
            let height = number("height", None)?;
            (width > 0.0 && height > 0.0 && (x + width).is_finite() && (y + height).is_finite())
                .then_some((x, y, width, height))
        })() else {
            continue;
        };
        sources.native_rects.insert(
            id.to_owned(),
            NativeRect {
                box_values: [x, y, width, height],
                radii: [0.0, 0.0],
            },
        );
    }
    for node in document.descendants().filter(|node| {
        node.is_element()
            && node.tag_name().name() == "svg"
            && node.attribute("data-d2-rs-markdown-reference") == Some("ordinary")
    }) {
        let Some(id) = node.attribute("id").filter(|id| {
            id.starts_with("d2-rs-native-md-viewport-") && identity_counts.get(id) == Some(&1)
        }) else {
            continue;
        };
        let dimensions = node
            .attribute("width")
            .and_then(|v| v.parse::<f64>().ok())
            .zip(node.attribute("height").and_then(|v| v.parse::<f64>().ok()));
        if let Some((width, height)) =
            dimensions.filter(|(w, h)| w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0)
        {
            let viewbox = node.attribute("viewBox").and_then(|value| {
                value
                    .split(|c: char| c.is_ascii_whitespace() || c == ',')
                    .filter(|v| !v.is_empty())
                    .map(str::parse::<f64>)
                    .collect::<Result<Vec<_>, _>>()
                    .ok()
            });
            // Native painters may serialize the same exact viewport with
            // integer or fixed-decimal SVG numbers. Qualification is about
            // coordinate identity, not incidental number formatting.
            if viewbox.as_deref() == Some(&[0.0, 0.0, width, height]) {
                let position = |name| {
                    node.attribute(name)
                        .map_or(Some(0.0), |v| v.parse::<f64>().ok())
                        .filter(|v| v.is_finite())
                };
                if let (Some(x), Some(y)) = (position("x"), position("y")) {
                    sources
                        .native_markdown_viewports
                        .insert(id.to_owned(), [x, y, width, height]);
                }
            }
        }
    }
    for node in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "rect")
    {
        let Some(id) = node.attribute("id").filter(|id| {
            id.starts_with("d2-rs-native-md-rect-") && identity_counts.get(id) == Some(&1)
        }) else {
            continue;
        };
        if !node.ancestors().any(|n| {
            n.attribute("id")
                .is_some_and(|id| sources.native_markdown_viewports.contains_key(id))
        }) {
            continue;
        }
        if ["style", "transform", "filter", "mask", "clip-path"]
            .iter()
            .any(|name| node.attribute(*name).is_some())
        {
            continue;
        }
        let number = |name, default| {
            node.attribute(name)
                .map_or(default, |v| v.parse::<f64>().ok())
                .filter(|v| v.is_finite())
        };
        if let Some((x, y, w, h, rx, ry)) = (|| {
            Some((
                number("x", Some(0.0))?,
                number("y", Some(0.0))?,
                number("width", None)?,
                number("height", None)?,
                number("rx", Some(0.0))?,
                number("ry", number("rx", Some(0.0)))?,
            ))
        })() {
            if w > 0.0 && h > 0.0 && rx >= 0.0 && ry >= 0.0 {
                sources.native_rects.insert(
                    id.to_owned(),
                    NativeRect {
                        box_values: [x, y, w, h],
                        radii: [rx, ry],
                    },
                );
            }
        }
    }
    // Resolve code-line origins before collecting ordinary text metadata.
    // Each grouped token is already addressed by its prepared chunk, so its
    // old string/ID lookup entries would only duplicate that retained run.
    let code_lines = sources.group_native_code_lines(svg, &document, &identity_counts);
    let mut code_line_ranges = code_lines
        .iter()
        .map(|(range, _)| range.clone())
        .collect::<Vec<_>>();
    code_line_ranges.sort_unstable_by_key(|range| range.start);
    for node in document.descendants().filter(|node| {
        node.is_element()
            && node.tag_name().name() == "text"
            && node
                .tag_name()
                .namespace()
                .is_none_or(|namespace| namespace == SVG_NAMESPACE)
            && !node.children().any(|child| child.is_element())
    }) {
        let range = node.range();
        let index = code_line_ranges.partition_point(|code| code.end <= range.start);
        if code_line_ranges
            .get(index)
            .is_some_and(|code| code.start <= range.start && range.end <= code.end)
        {
            continue;
        }
        let Some(text) = node.text() else { continue };
        if let Some(id) = node
            .attribute("id")
            .filter(|id| id.contains("d2rs-md-stroke-") && identity_counts.get(id) == Some(&1))
            && let Some(width) = node
                .attribute("stroke-width")
                .and_then(|width| width.parse::<f64>().ok())
                .filter(|width| width.is_finite() && *width > 0.0)
        {
            sources
                .text_stroke_widths_by_id
                .insert(id.to_owned(), width);
        }
        let private_origin = node
            .attribute("data-d2-rs-text-x-bits")
            .and_then(|bits| u64::from_str_radix(bits, 16).ok())
            .zip(
                node.attribute("data-d2-rs-text-y-bits")
                    .and_then(|bits| u64::from_str_radix(bits, 16).ok()),
            )
            .map(|(x, y)| SvgPoint {
                x: f64::from_bits(x),
                y: f64::from_bits(y),
            })
            .filter(|origin| origin.finite());
        let origin = private_origin.or_else(|| {
            Some(SvgPoint {
                x: node.attribute("x")?.parse().ok()?,
                y: node.attribute("y")?.parse().ok()?,
            })
        });
        let Some(origin) = origin else { continue };
        sources.text_origins.push(SvgTextOrigin {
            text: text.to_owned(),
            x: origin.x,
            y: origin.y,
        });
        if private_origin.is_some()
            && let Some(id) = node
                .attribute("id")
                .filter(|id| !id.is_empty() && identity_counts.get(id) == Some(&1))
        {
            // The ID here is the final XML identity after any native-only
            // synthetic-italic rewrite; usvg retains the same text node ID.
            sources.text_origins_by_id.insert(id.to_owned(), origin);
            // Native scene TextRuns retain raw text, including control runes.
            // usvg applies SVG whitespace processing before shaping; keep the
            // original XML character data beside the private scene origin.
            if node.attribute("data-d2-rs-text-raw") == Some("true") {
                let text = node
                    .attribute("data-d2-rs-text-source")
                    .and_then(|encoded| {
                        base64::engine::general_purpose::STANDARD
                            .decode(encoded)
                            .ok()
                    })
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                    .unwrap_or_else(|| text.to_owned());
                sources.text_contents_by_id.insert(id.to_owned(), text);
            }
            if let Some(size) = node
                .attribute("font-size")
                .and_then(|size| size.parse::<f64>().ok())
                .filter(|size| size.is_finite() && *size > 0.0)
            {
                sources.text_font_sizes_by_id.insert(id.to_owned(), size);
            }
        }
    }
    for node in document.descendants().filter(|node| {
        node.is_element()
            && node.tag_name().name() == "g"
            && node.attribute("data-d2-rs-shape-shadow") == Some("sharp")
    }) {
        let Some(id) = node.attribute("id").filter(|id| {
            id.starts_with("d2-rs-native-shape-shadow-") && identity_counts.get(id) == Some(&1)
        }) else {
            continue;
        };
        if ["style", "mask", "clip-path", "transform", "opacity"]
            .iter()
            .any(|name| node.attribute(*name).is_some())
        {
            continue;
        }
        if !node.ancestors().any(|parent| {
            parent
                .attribute("id")
                .is_some_and(|id| sources.native_canvas_geometry.contains_key(id))
        }) {
            continue;
        }
        let Some(filter_id) = node
            .attribute("filter")
            .and_then(|value| value.strip_prefix("url(#"))
            .and_then(|value| value.strip_suffix(')'))
        else {
            continue;
        };
        if identity_counts.get(filter_id) != Some(&1) {
            continue;
        }
        let Some(filter) = document
            .descendants()
            .find(|n| n.attribute("id") == Some(filter_id) && n.tag_name().name() == "filter")
        else {
            continue;
        };
        // This private carrier represents precisely the renderer's static
        // shadow. Authored filter inputs, units, regions or primitive options
        // must not silently acquire native-scene semantics.
        let only_attributes = |node: roxmltree::Node<'_, '_>, allowed: &[&str]| {
            node.attributes().all(|attribute| {
                attribute.namespace().is_none() && allowed.contains(&attribute.name())
            })
        };
        if !only_attributes(filter, &["id", "x", "y", "width", "height"])
            || filter.attribute("x") != Some("-20%")
            || filter.attribute("y") != Some("-20%")
            || filter.attribute("width") != Some("160%")
            || filter.attribute("height") != Some("160%")
        {
            continue;
        }
        let parts = filter
            .children()
            .filter(|n| n.is_element())
            .collect::<Vec<_>>();
        if parts.len() != 4 {
            continue;
        }
        let number_is = |node: roxmltree::Node<'_, '_>, name, expected| {
            node.attribute(name).and_then(|s| s.parse::<f64>().ok()) == Some(expected)
        };
        if !parts.iter().all(|part| {
            part.tag_name().namespace() == Some("http://www.w3.org/2000/svg")
                && !part.children().any(|child| child.is_element())
        }) || !only_attributes(parts[0], &["flood-color", "flood-opacity", "result"])
            || !only_attributes(parts[1], &["in", "in2", "operator"])
            || !only_attributes(parts[2], &["dx", "dy"])
            || !only_attributes(parts[3], &["in", "mode"])
        {
            continue;
        }
        if parts[0].tag_name().name() == "feFlood"
            && parts[0].attribute("flood-color") == Some("#3d4574")
            && number_is(parts[0], "flood-opacity", 0.4)
            && parts[0].attribute("result") == Some("shadow-color")
            && parts[1].tag_name().name() == "feComposite"
            && parts[1].attribute("in") == Some("shadow-color")
            && parts[1].attribute("in2") == Some("SourceAlpha")
            && parts[1].attribute("operator") == Some("in")
            && parts[2].tag_name().name() == "feOffset"
            && number_is(parts[2], "dx", 3.0)
            && number_is(parts[2], "dy", 5.0)
            && parts[3].tag_name().name() == "feBlend"
            && parts[3].attribute("in") == Some("SourceGraphic")
            && parts[3].attribute("mode") == Some("normal")
        {
            sources.native_shape_shadows.insert(id.to_owned());
        }
    }
    let mut insertions = Vec::new();
    let mut replacements = Vec::new();
    for image in document.descendants().filter(|node| {
        let tag = node.tag_name();
        node.is_element()
            && tag.name() == "image"
            && tag
                .namespace()
                .is_none_or(|namespace| namespace == SVG_NAMESPACE)
    }) {
        if image
            .attribute("id")
            .is_some_and(|id| identity_counts.get(id) != Some(&1))
        {
            continue;
        }
        let Some(href) = image_href(image) else {
            continue;
        };
        let unavailable = sources
            .native_cache
            .as_ref()
            .and_then(|cache| cache.by_source.get(href))
            .is_some_and(|id| id == "image:unavailable");
        // This empty viewport only carries the image placement through usvg.
        // Its actual geometry is the shared native vector, never imported SVG.
        const UNAVAILABLE_VIEWPORT: &[u8] =
            b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"64\" height=\"64\"/>";
        let bytes = if unavailable {
            UNAVAILABLE_VIEWPORT.to_vec()
        } else {
            let Some(payload) = href.strip_prefix(SVG_DATA_PREFIX) else {
                continue;
            };
            let Some(bytes) = base64::engine::general_purpose::STANDARD
                .decode(payload)
                .ok()
            else {
                continue;
            };
            bytes
        };
        let Ok(source) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let native_id = if unavailable {
            Some("image:unavailable".to_owned())
        } else {
            d2_rs_math::svg_asset_cache::content_digest("image/svg+xml", &bytes, None)
                .ok()
                .and_then(|digest| {
                    sources
                        .native_cache
                        .as_ref()?
                        .by_digest
                        .get(&digest)
                        .cloned()
                })
        };
        let viewport_source = tag_intrinsic_viewports(source);
        let identity_source = if math_scene_transform(image, document.root_element()).is_some() {
            tag_anonymous_imported_paths(&viewport_source)
        } else {
            viewport_source.clone()
        };
        let source = identity_source.as_str();
        let mut paths = parse_simple_svg_paths(source);
        if let Some(paths) = paths.as_mut() {
            let scene = math_scene_transform(image, document.root_element());
            for path in paths {
                if path.intrinsic_transforms.is_some() {
                    path.scene_transform = scene;
                }
            }
        }
        let mut paths_by_id = parse_svg_paths_by_id(source);
        if let Ok(imported) = roxmltree::Document::parse(source) {
            for (id, path) in &mut paths_by_id {
                if let Some(node) = imported
                    .descendants()
                    .find(|n| n.attribute("id") == Some(id.as_str()))
                {
                    qualify_image_path_transform(path, node, image, document.root_element());
                }
            }
        }
        if paths.is_none() && paths_by_id.is_empty() && native_id.is_none() {
            continue;
        }

        let id = image.attribute("id").map(str::to_owned).unwrap_or_else(|| {
            let mut suffix = sources.images.len();
            let mut id = format!("{SOURCE_ID_PREFIX}{suffix}");
            while identity_counts.contains_key(id.as_str()) {
                suffix += 1;
                id = format!("{SOURCE_ID_PREFIX}{suffix}");
            }
            id
        });
        let Some(insert_at) =
            start_tag_insert_position(svg, image.range().start, image.range().end)
        else {
            continue;
        };
        if image.attribute("id").is_none() {
            insertions.push((insert_at, format!(" id=\"{id}\"")));
        }
        if let Some(native_id) = native_id {
            sources.native_ids.insert(id.clone(), native_id);
        }
        // The raster resolver's unavailable resource keeps its original URL
        // until this substitution, so it cannot receive the earlier data-URI
        // placement tag. Retain the same admitted scene placement here rather
        // than dropping this vector onto usvg's float32 fallback transform.
        if unavailable {
            if let Some(placement) = scene_image_placement(image) {
                sources.scene_images_by_id.insert(id.clone(), placement);
            }
        }
        let (tagged_source, opacities) = tag_imported_group_opacities(source);
        if unavailable || !opacities.is_empty() || tagged_source.as_bytes() != bytes.as_slice() {
            if let Some(attribute) = image
                .attributes()
                .find(|attribute| attribute.name() == "href")
            {
                let payload = base64::engine::general_purpose::STANDARD.encode(tagged_source);
                replacements.push((
                    attribute.range_value(),
                    format!("{SVG_DATA_PREFIX}{payload}"),
                ));
                sources.image_group_opacities.insert(id.clone(), opacities);
            }
        }
        sources.image_paths_by_id.insert(id.clone(), paths_by_id);
        // Reserve one owner identity even when ordinal binding is unavailable.
        sources.images.insert(id, paths.unwrap_or_default());
    }

    // buildLegendShapeIcon filters ordinary geometry at source sigma 1.7,
    // independently of the card shadow and of ordinary SVG's sharp shadow.
    for node in document.descendants().filter(|n| {
        n.has_tag_name("g") && n.attribute("data-d2-rs-legend-shape-shadow") == Some("scene")
    }) {
        let Some(id) = node
            .attribute("id")
            .filter(|id| identity_counts.get(id) == Some(&1))
        else {
            continue;
        };
        let Some(index) = id
            .strip_prefix("d2-rs-native-legend-shape-shadow-")
            .filter(|s| s.parse::<u64>().is_ok())
        else {
            continue;
        };
        let owner = format!("d2-rs-native-legend-shape-icon-{index}");
        if !node.ancestors().any(|n| {
            n.attribute("id") == Some(owner.as_str())
                && identity_counts.get(owner.as_str()) == Some(&1)
        }) || !node.ancestors().any(|n| {
            n.attribute("id")
                .is_some_and(|id| sources.native_canvas_geometry.contains_key(id))
        }) || node.ancestors().any(|n| {
            matches!(
                n.tag_name().name(),
                "defs" | "mask" | "clipPath" | "pattern" | "marker" | "symbol"
            )
        }) {
            continue;
        }
        let attrs = |n: roxmltree::Node<'_, '_>, allowed: &[&str]| {
            n.attributes()
                .all(|a| a.namespace().is_none() && allowed.contains(&a.name()))
        };
        if !attrs(node, &["id", "data-d2-rs-legend-shape-shadow", "filter"]) {
            continue;
        }
        let Some(filter_id) = node
            .attribute("filter")
            .and_then(|v| v.strip_prefix("url(#"))
            .and_then(|v| v.strip_suffix(')'))
            .filter(|id| identity_counts.get(id) == Some(&1))
        else {
            continue;
        };
        let Some(filter) = document
            .descendants()
            .find(|n| n.has_tag_name("filter") && n.attribute("id") == Some(filter_id))
        else {
            continue;
        };
        if !attrs(
            filter,
            &[
                "id",
                "x",
                "y",
                "width",
                "height",
                "color-interpolation-filters",
            ],
        ) || [
            ("x", "-100%"),
            ("y", "-100%"),
            ("width", "300%"),
            ("height", "300%"),
            ("color-interpolation-filters", "sRGB"),
        ]
        .iter()
        .any(|(k, v)| filter.attribute(*k) != Some(*v))
        {
            continue;
        }
        let parts = filter
            .children()
            .filter(|n| n.is_element())
            .collect::<Vec<_>>();
        if parts.len() != 1
            || !parts[0].has_tag_name("feDropShadow")
            || !attrs(
                parts[0],
                &["dx", "dy", "stdDeviation", "flood-color", "flood-opacity"],
            )
            || [
                ("dx", "3"),
                ("dy", "5"),
                ("stdDeviation", "1.7"),
                ("flood-color", "#3d4574"),
                ("flood-opacity", "0.4"),
            ]
            .iter()
            .any(|(k, v)| parts[0].attribute(*k) != Some(*v))
        {
            continue;
        }
        if !node.children().any(|n| n.is_element()) {
            continue;
        }
        sources.native_legend_shape_shadows.insert(id.to_owned());
    }

    // Retain the complete multi-subpath scene border before usvg narrows it.
    // A single centerline cannot represent the separate 3D depth strokes.
    for node in document
        .descendants()
        .filter(|n| n.has_tag_name("g") && n.attribute("data-d2-rs-legend-shadow") == Some("scene"))
    {
        let Some(id) = node.attribute("id").filter(|id| {
            id.starts_with("d2-rs-native-legend-shadow-") && identity_counts.get(id) == Some(&1)
        }) else {
            continue;
        };
        if !node.ancestors().any(|n| {
            n.attribute("id")
                .is_some_and(|id| sources.native_canvas_geometry.contains_key(id))
        }) {
            continue;
        }
        let attrs = |n: roxmltree::Node<'_, '_>, allowed: &[&str]| {
            n.attributes()
                .all(|a| a.namespace().is_none() && allowed.contains(&a.name()))
        };
        if !attrs(node, &["id", "data-d2-rs-legend-shadow", "filter"]) {
            continue;
        }
        let Some(filter_id) = node
            .attribute("filter")
            .and_then(|v| v.strip_prefix("url(#"))
            .and_then(|v| v.strip_suffix(')'))
            .filter(|id| identity_counts.get(id) == Some(&1))
        else {
            continue;
        };
        let Some(filter) = document
            .descendants()
            .find(|n| n.has_tag_name("filter") && n.attribute("id") == Some(filter_id))
        else {
            continue;
        };
        if !attrs(
            filter,
            &[
                "id",
                "x",
                "y",
                "width",
                "height",
                "color-interpolation-filters",
            ],
        ) || [
            ("x", "-100%"),
            ("y", "-100%"),
            ("width", "300%"),
            ("height", "300%"),
            ("color-interpolation-filters", "sRGB"),
        ]
        .iter()
        .any(|(k, v)| filter.attribute(*k) != Some(*v))
        {
            continue;
        }
        let parts = filter
            .children()
            .filter(|n| n.is_element())
            .collect::<Vec<_>>();
        if parts.len() != 1
            || !parts[0].has_tag_name("feDropShadow")
            || !attrs(
                parts[0],
                &["dx", "dy", "stdDeviation", "flood-color", "flood-opacity"],
            )
            || [
                ("dx", "0"),
                ("dy", "2"),
                ("stdDeviation", "3"),
                ("flood-color", "#000000"),
                ("flood-opacity", "0.10196078431372549"),
            ]
            .iter()
            .any(|(k, v)| parts[0].attribute(*k) != Some(*v))
        {
            continue;
        }
        let children = node
            .children()
            .filter(|n| n.is_element())
            .collect::<Vec<_>>();
        if children.len() != 1 || !children[0].has_tag_name("rect") {
            continue;
        }
        let rect = children[0];
        if !attrs(
            rect,
            &[
                "id",
                "x",
                "y",
                "width",
                "height",
                "rx",
                "fill",
                "stroke",
                "stroke-width",
            ],
        ) || rect.attribute("fill") != Some("#F7F7FA")
            || rect.attribute("stroke") != Some("#DEE1EB")
            || rect.attribute("stroke-width") != Some("1")
            || rect.attribute("rx") != Some("4")
        {
            continue;
        }
        let number = |name| {
            rect.attribute(name)
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|v| v.is_finite())
        };
        let Some((x, y, w, h)) = (|| {
            Some((
                number("x")?,
                number("y")?,
                number("width")?,
                number("height")?,
            ))
        })() else {
            continue;
        };
        if w < 8.0 || h < 8.0 {
            continue;
        }
        sources.native_legend_shadows.insert(id.to_owned());
        if let Some(rid) = rect
            .attribute("id")
            .filter(|rid| identity_counts.get(rid) == Some(&1))
        {
            sources.native_rects.insert(
                rid.to_owned(),
                NativeRect {
                    box_values: [x, y, w, h],
                    radii: [4.0, 4.0],
                },
            );
        }
        // The matching white panel has the same source rounded-box operands.
        for panel in document.descendants().filter(|n| {
            n.has_tag_name("rect")
                && n.attribute("id") == Some("d2-rs-native-legend-panel")
                && identity_counts.get("d2-rs-native-legend-panel") == Some(&1)
        }) {
            if attrs(
                panel,
                &[
                    "id",
                    "x",
                    "y",
                    "width",
                    "height",
                    "rx",
                    "fill",
                    "stroke",
                    "stroke-width",
                ],
            ) && panel.attribute("fill") == Some("#ffffff")
                && panel.attribute("stroke") == Some("#DEE1EB")
                && panel.attribute("stroke-width") == Some("1")
                && panel.attribute("rx") == Some("4")
                && ["x", "y", "width", "height"]
                    .iter()
                    .all(|k| panel.attribute(*k) == rect.attribute(*k))
                && panel.ancestors().any(|n| {
                    n.attribute("id")
                        .is_some_and(|id| sources.native_canvas_geometry.contains_key(id))
                })
            {
                sources.native_rects.insert(
                    "d2-rs-native-legend-panel".into(),
                    NativeRect {
                        box_values: [x, y, w, h],
                        radii: [4.0, 4.0],
                    },
                );
            }
        }
    }
    for node in document.descendants().filter(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(SVG_NAMESPACE)
            && node.tag_name().name() == "path"
            && node.attribute("data-d2-rs-native-3d-border") == Some("scene")
    }) {
        let Some(id) = node.attribute("id").filter(|id| {
            identity_counts.get(id) == Some(&1)
                && id.contains("-native-raster-shape-")
                && id.ends_with("-3d-border")
        }) else {
            continue;
        };
        if ["style", "transform", "filter", "mask", "clip-path"]
            .iter()
            .any(|name| node.attribute(*name).is_some())
            || node.ancestors().any(|parent| {
                matches!(
                    parent.tag_name().name(),
                    "defs" | "mask" | "clipPath" | "pattern" | "marker" | "symbol"
                )
            })
            || !node.ancestors().any(|parent| {
                parent
                    .attribute("id")
                    .is_some_and(|id| sources.native_canvas_geometry.contains_key(id))
            })
        {
            continue;
        }
        if let Some(path) = node.attribute("d").and_then(parse_path) {
            sources.raw_paths.insert(id.to_owned(), path);
        }
    }

    // Retain generated rough scene operations before usvg narrows them.
    // The marker is interpreted only inside the validated native viewport.
    for node in document.descendants().filter(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(SVG_NAMESPACE)
            && node.tag_name().name() == "path"
            && node.attribute("data-d2-rs-sketch-set") == Some("rough")
    }) {
        if !node.ancestors().any(|parent| {
            parent
                .attribute("id")
                .is_some_and(|id| sources.native_canvas_geometry.contains_key(id))
        }) || node.ancestors().any(|parent| {
            matches!(
                parent.tag_name().name(),
                "defs" | "pattern" | "mask" | "clipPath" | "marker" | "symbol"
            )
        }) {
            continue;
        }
        let Some(source) = node.attribute("d").and_then(parse_path) else {
            continue;
        };
        let id = if let Some(id) = node.attribute("id") {
            if identity_counts.get(id) != Some(&1) {
                continue;
            }
            id.to_owned()
        } else {
            let id = format!("{RAW_PATH_SOURCE_ID_PREFIX}{}", sources.raw_paths.len());
            let Some(insert_at) =
                start_tag_insert_position(svg, node.range().start, node.range().end)
            else {
                continue;
            };
            insertions.push((insert_at, format!(" id=\"{id}\"")));
            id
        };
        // d2scenebuild.compileSketchShapeDrawable retains the local node
        // transform separately from the rough path commands. usvg moves a
        // path transform into an anonymous group; give that group an identity
        // so the raster walk can compose the original f64 operands.
        if let Some(attribute) = node.attribute_node("transform")
            && node.attribute("style").is_none()
            && let Ok(transform) = attribute.value().parse::<svgtypes::Transform>()
        {
            let values = [
                transform.a,
                transform.b,
                transform.c,
                transform.d,
                transform.e,
                transform.f,
            ];
            let group_id = format!("{id}-transform");
            if values.iter().all(|value| value.is_finite())
                && !identity_counts.contains_key(group_id.as_str())
                && !sources.native_group_transforms.contains_key(&group_id)
            {
                // d2scenebuild masks each rough node in its local user space
                // (d2raster.prepare.node composes the node transform first).
                // The SVG transport puts the mask on the outer wrapper, so
                // retain the rough node's transform for that native mask too.
                if let Some(parent) = node.parent_element()
                    && parent.has_tag_name((SVG_NAMESPACE, "g"))
                    && parent
                        .attributes()
                        .all(|a| matches!(a.name(), "mask" | "id"))
                    && let Some(mask_id) = parent
                        .attribute("mask")
                        .and_then(|v| v.strip_prefix("url(#"))
                        .and_then(|v| v.strip_suffix(')'))
                    && sources.native_connection_masks.contains_key(mask_id)
                    && parent.children().filter(|n| n.is_element()).count() == 1
                {
                    let mask_group_id = parent
                        .attribute("id")
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("{id}-mask"));
                    if parent
                        .attribute("id")
                        .is_some_and(|id| identity_counts.get(id) == Some(&1))
                        || (!identity_counts.contains_key(mask_group_id.as_str())
                            && !sources.native_mask_transforms.contains_key(&mask_group_id))
                    {
                        if parent.attribute("id").is_none()
                            && let Some(at) = start_tag_insert_position(
                                svg,
                                parent.range().start,
                                parent.range().end,
                            )
                        {
                            insertions.push((at, format!(" id=\"{mask_group_id}\"")));
                        }
                        sources.native_mask_transforms.insert(mask_group_id, values);
                    }
                }
                insertions.push((
                    node.range().start,
                    format!("<g id=\"{group_id}\" transform=\"{}\">", attribute.value()),
                ));
                insertions.push((node.range().end, "</g>".to_owned()));
                replacements.push((attribute.range(), String::new()));
                sources.native_group_transforms.insert(group_id, values);
            }
        }
        sources.raw_paths.insert(id, source);
    }

    // D2's built-in paper texture is a vector asset in the native Go scene
    // renderer. Parse its path coordinates as f64 before usvg converts them to
    // f32, matching the upstream scene import path and avoiding small coverage
    // changes from rounded source coordinates.
    for path in document.descendants().filter(|node| {
        let tag = node.tag_name();
        node.is_element()
            && tag.name() == "path"
            && tag
                .namespace()
                .is_none_or(|namespace| namespace == SVG_NAMESPACE)
    }) {
        let Some(pattern) = path
            .ancestors()
            .find(|ancestor| ancestor.tag_name().name() == "pattern")
        else {
            continue;
        };
        if !pattern
            .attribute("id")
            .is_some_and(|id| id.starts_with("paper-") || id.starts_with("d2-rs-sketch-streak-"))
            || path
                .ancestors()
                .any(|ancestor| ancestor.tag_name().name() == "mask")
            || path.attribute("id").is_some()
        {
            continue;
        }
        let Some(source) = path.attribute("d").and_then(parse_path) else {
            continue;
        };
        let id = format!("{RAW_PATH_SOURCE_ID_PREFIX}{}", sources.raw_paths.len());
        let Some(insert_at) = start_tag_insert_position(svg, path.range().start, path.range().end)
        else {
            continue;
        };
        insertions.push((insert_at, format!(" id=\"{id}\"")));
        sources.raw_paths.insert(id, source);
    }

    replacements.extend(code_lines);
    if insertions.is_empty() && replacements.is_empty() {
        sources.retain_native_gradient_ids(svg);
        if svg.contains("<pattern") {
            sources.pattern_document = Some(svg.to_owned());
        }
        return (Cow::Borrowed(svg), sources);
    }
    replacements.extend(
        insertions
            .into_iter()
            .map(|(position, insertion)| (position..position, insertion)),
    );
    replacements.sort_by(|left, right| right.0.start.cmp(&left.0.start));
    // Paper textures have thousands of paths. Splicing an ID into each one
    // repeatedly shifts the remaining document; copy each source span once.
    // Reverse the original edit order, including ties, so insertions at the
    // same offset retain the ordering of the descending splices.
    let length = replacements
        .iter()
        .fold(svg.len(), |length, (range, text)| {
            length - range.len() + text.len()
        });
    let mut tagged = String::with_capacity(length);
    let mut copied = 0;
    for (range, replacement) in replacements.into_iter().rev() {
        tagged.push_str(&svg[copied..range.start]);
        tagged.push_str(&replacement);
        copied = range.end;
    }
    tagged.push_str(&svg[copied..]);
    sources.retain_native_gradient_ids(&tagged);
    if tagged.contains("<pattern") {
        sources.pattern_document = Some(tagged.clone());
    }
    (Cow::Owned(tagged), sources)
}

#[cfg(test)]
mod tests {
    #[test]
    fn original_zero_rectangles_keep_neighbor_path_ordinals_and_f64_frames() {
        for dimensions in ["width='0' height='60'", "width='25' height='0'"] {
            let svg = format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='20' height='20' viewBox='0 0 1000 1000' data-d2-rs-intrinsic-math='ordinary'><g transform='translate(0.125,0.375)'><path d='M0 0L2 3'/><rect x='853' y='615.3' {dimensions}/><path d='M4 5L6 7'/></g></svg>"
            );
            let paths = super::parse_simple_svg_paths(&svg)
                .expect("zero rect is unpainted, not an unsupported asset");
            assert_eq!(paths.len(), 2);
            assert!(paths.iter().all(|p| p.intrinsic_transforms.is_some()));
            assert!(matches!(
                paths[1].commands[0],
                super::SvgPathCommand::MoveTo(super::SvgPoint { x: 4.0, y: 5.0 })
            ));
        }
    }

    #[test]
    fn embedded_separator_transport_covers_base64_percent_and_xlink_without_changing_original_assets()
     {
        let raw = "<svg width='2'height='2'><rect width='1'height='1'/></svg>";
        for (name, href) in [
            (
                "href",
                format!(
                    "data:image/svg+xml;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(raw)
                ),
            ),
            (
                "xlink:href",
                format!(
                    "data:image/svg+xml,{}",
                    raw.replace('<', "%3C")
                        .replace('>', "%3E")
                        .replace(' ', "%20")
                        .replace('\'', "%27")
                ),
            ),
        ] {
            let outer = format!(
                "<svg xmlns='http://www.w3.org/2000/svg' xmlns:xlink='http://www.w3.org/1999/xlink'><image {name}=\"{href}\"/></svg>"
            );
            let changed = super::separate_embedded_svg_attributes(&outer);
            assert!(matches!(changed, std::borrow::Cow::Owned(_)));
            let document = roxmltree::Document::parse(&changed).unwrap();
            let image = document
                .descendants()
                .find(|node| node.has_tag_name("image"))
                .unwrap();
            let payload = super::image_href(image).unwrap();
            let bytes = crate::svg_import::svg_data_uri(payload).unwrap();
            let inner = std::str::from_utf8(&bytes).unwrap();
            assert_eq!(
                inner,
                "<svg width='2' height='2'><rect width='1' height='1'/></svg>"
            );
            assert_eq!(
                raw,
                "<svg width='2'height='2'><rect width='1'height='1'/></svg>"
            );
            assert_eq!(super::separate_embedded_svg_attributes(&changed), changed);
        }
    }

    #[test]
    fn imported_opacity_identity_and_css_guards_preserve_asset_scope() {
        use base64::Engine as _;
        let asset = r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><g id="authored" opacity="0.9"><path d="M0 0H20V20H0Z"/></g><g id="duplicate" opacity="0.9"/><g id="duplicate" opacity="0.5"/></svg>"#;
        let payload = base64::engine::general_purpose::STANDARD.encode(asset);
        let outer = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><image id="asset" width="20" height="20" href="data:image/svg+xml;base64,{payload}"/></svg>"#
        );
        let (tagged, sources) = super::tag_embedded_svg_images(&outer);
        let document = roxmltree::Document::parse(&tagged).unwrap();
        assert_eq!(
            document
                .descendants()
                .filter(|node| node.attribute("id") == Some("asset"))
                .count(),
            1
        );
        let values = sources.image_group_opacities("asset").unwrap();
        assert_eq!(values.get("authored"), Some(&0.9_f64));
        assert!(!values.contains_key("duplicate"));
        for asset in [
            asset.replace("<g id=", "<g style=\"opacity:0.5\" id="),
            asset.replace("<g id=", "<style>g { opacity:0.5 }</style><g id="),
        ] {
            let (_, values) = super::tag_imported_group_opacities(&asset);
            assert!(values.is_empty());
        }
        let asset = r#"<svg><g id="d2-rs-imported-opacity-47"/><g opacity="0.9"><path d="M0 0H1V1Z"/></g></svg>"#;
        let (tagged, values) = super::tag_imported_group_opacities(asset);
        let document = roxmltree::Document::parse(&tagged).unwrap();
        for id in values.keys() {
            assert_eq!(
                document
                    .descendants()
                    .filter(|node| node.attribute("id") == Some(id.as_str()))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn fractional_native_text_sizes_retain_source_underline_operands() {
        let input = r#"<svg xmlns="http://www.w3.org/2000/svg"><text id="native" x="5.44" y="18" font-size="13.6" data-d2-rs-text-x-bits="4015c28f5c28f5c3" data-d2-rs-text-y-bits="4032000000000000">Agm</text></svg>"#;
        let (_, sources) = tag_embedded_svg_images(input);
        assert_eq!(
            sources.text_font_size("native", 13.6_f32).to_bits(),
            13.6_f64.to_bits()
        );
        assert_eq!(sources.text_font_size("native", 16.0), 16.0);
        assert_eq!(
            sources.text_font_size("missing", 13.6_f32),
            f64::from(13.6_f32)
        );
    }
    #[test]
    fn private_scene_text_retains_control_character_data() {
        let input = r#"<svg xmlns="http://www.w3.org/2000/svg"><text id="native" data-d2-rs-text-raw="true" x="5" y="18" data-d2-rs-text-x-bits="4014000000000000" data-d2-rs-text-y-bits="4032000000000000">b&#xD;&#x9;&#xA;b</text><text id="forbidden" data-d2-rs-text-raw="true" data-d2-rs-text-source="ZS0MIg==" x="6" y="19" data-d2-rs-text-x-bits="4018000000000000" data-d2-rs-text-y-bits="4033000000000000">e-�&#34;</text><text id="ordinary" x="5" y="18">b&#xD;b</text></svg>"#;
        let (_, sources) = tag_embedded_svg_images(input);
        assert_eq!(sources.exact_text_content_by_id("native"), Some("b\r\t\nb"));
        assert_eq!(
            sources.exact_text_content_by_id("forbidden"),
            Some("e-\u{c}\"")
        );
        assert_eq!(sources.exact_text_content_by_id("ordinary"), None);
    }
    #[test]
    fn duplicate_private_text_identities_cannot_alias_precision_metadata() {
        let mut svg = String::from("<svg>");
        for (x, y, size) in [(5.0_f64, 18.0_f64, 13), (9.0, 20.0, 16)] {
            svg.push_str(&format!(r#"<text id="duplicate" data-d2-rs-text-raw="true" x="{x}" y="{y}" font-size="{size}" data-d2-rs-text-x-bits="{:016x}" data-d2-rs-text-y-bits="{:016x}">same</text>"#,x.to_bits(),y.to_bits()));
        }
        svg.push_str("</svg>");
        let (_, sources) = tag_embedded_svg_images(&svg);
        assert!(sources.exact_text_origin_by_id("duplicate").is_none());
        assert!(sources.exact_text_content_by_id("duplicate").is_none());
        assert_eq!(sources.text_font_size("duplicate", 17.0), 17.0);
        let first = sources.exact_text_origin("same", 5.0, 18.0).unwrap();
        let second = sources.exact_text_origin("same", 9.0, 20.0).unwrap();
        assert_eq!((first.x, first.y), (5.0, 18.0));
        assert_eq!((second.x, second.y), (9.0, 20.0));
    }
    #[test]
    fn native_group_transform_preserves_source_operands_and_rejects_ambiguous_subroots() {
        let source = r##"<svg xmlns="http://www.w3.org/2000/svg" width="302" height="282" viewBox="0 0 302 282" preserveAspectRatio="xMinYMin meet"><svg id="d2-rs-native-canvas-viewport-main" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="302" height="282" viewBox="-101 -101 302 282"><g id="effect" transform="translate(13.125 17.25) matrix(1.0000000000000002 0 0 1 0 0)"><path d="M0 0L1 1"/></g></svg></svg>"##;
        let (_, retained) = tag_embedded_svg_images(source);
        assert_eq!(
            retained.native_group_transform("effect"),
            Some([1.0000000000000002, 0.0, 0.0, 1.0, 13.125, 17.25])
        );
        let rough = source.replace(
            "<path d=\"M0 0L1 1\"/>",
            "<path id=\"rough\" data-d2-rs-sketch-set=\"rough\" transform=\"translate(12 148)\" d=\"M0 0L1 1\"/>",
        );
        let (tagged, retained) = tag_embedded_svg_images(&rough);
        assert_eq!(
            retained.native_group_transform("rough-transform"),
            Some([1.0, 0.0, 0.0, 1.0, 12.0, 148.0])
        );
        let parsed = roxmltree::Document::parse(&tagged).unwrap();
        let path = parsed
            .descendants()
            .find(|n| n.attribute("id") == Some("rough"))
            .unwrap();
        assert!(path.attribute("transform").is_none());
        assert_eq!(
            path.parent_element().unwrap().attribute("id"),
            Some("rough-transform")
        );
        assert_eq!(retained.raw_path("rough").unwrap().commands.len(), 2);
        for altered in [
            source.replace(
                "data-d2-rs-canvas-reference=\"ordinary\"",
                "data-d2-rs-canvas-reference=\"resource\"",
            ),
            source.replace(
                "<g id=\"effect\"",
                "<g style=\"transform:translate(1px)\" id=\"effect\"",
            ),
            source.replace("<g id=\"effect\"", "<g xmlns=\"urn:foreign\" id=\"effect\""),
            source.replace("<g id=\"effect\"", "<g id=\"effect\"/><g id=\"effect\""),
            source
                .replace("<g id=\"effect\"", "<defs><g id=\"effect\"")
                .replace("</g></svg>", "</g></defs></svg>"),
            source.replace(
                "translate(13.125 17.25) matrix(1.0000000000000002 0 0 1 0 0)",
                "matrix(1e309 0 0 1 0 0)",
            ),
            source.replace(
                "translate(13.125 17.25) matrix(1.0000000000000002 0 0 1 0 0)",
                "translate(bad)",
            ),
        ] {
            let (_, retained) = tag_embedded_svg_images(&altered);
            assert!(
                retained.native_group_transform("effect").is_none(),
                "{altered}"
            );
        }
    }

    #[test]
    fn native_three_dee_border_retains_all_subpaths_only_under_qualified_canvas() {
        let source = r##"<svg xmlns="http://www.w3.org/2000/svg" width="302" height="282" viewBox="0 0 302 282" preserveAspectRatio="xMinYMin meet"><svg id="d2-rs-native-canvas-viewport-main" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="302" height="282" viewBox="-101 -101 302 282"><path id="control-native-raster-shape-0-3d-border" data-d2-rs-native-3d-border="scene" d="M4.25 7.125 L15.5 19.75 M15.5 19.75 L30.125 4.25" stroke="black" fill="none"/></svg></svg>"##;
        let id = "control-native-raster-shape-0-3d-border";
        let (_, retained) = tag_embedded_svg_images(source);
        assert_eq!(retained.raw_path(id).unwrap().commands.len(), 4);
        for altered in [
            source.replace("data-d2-rs-native-3d-border=\"scene\"", ""),
            source.replace("control-native-raster-shape-0-3d-border", "authored-border"),
            source.replace(
                "data-d2-rs-canvas-reference=\"ordinary\"",
                "data-d2-rs-canvas-reference=\"resource\"",
            ),
            source.replace("<path ", "<path style=\"stroke:red\" "),
            source.replace("<path ", "<path transform=\"translate(1)\" "),
            source.replace("<path ", "<path clip-path=\"url(#clip)\" "),
            source.replace("<path ", "<path mask=\"url(#mask)\" "),
            source.replace("<path ", "<path filter=\"url(#filter)\" "),
            source.replace("<path ", "<path xmlns=\"urn:foreign\" "),
            source
                .replace("<path ", "<defs><path ")
                .replace("/></svg>", "/></defs></svg>"),
            source.replace(
                "<path ",
                "<g id=\"control-native-raster-shape-0-3d-border\"/><path ",
            ),
        ] {
            let (_, retained) = tag_embedded_svg_images(&altered);
            assert!(retained.raw_path(id).is_none(), "{altered}");
        }
    }

    #[test]
    fn native_rect_contour_requires_unique_primitive_and_native_dash_owner() {
        let source = r##"<svg xmlns="http://www.w3.org/2000/svg" width="302" height="282" viewBox="0 0 302 282" preserveAspectRatio="xMinYMin meet"><svg id="d2-rs-native-canvas-viewport-main" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="302" height="282" viewBox="-101 -101 302 282"><g data-d2-rs-dash-owner-id="control-native-raster-shape-0" data-d2-rs-dash-size-bits="4024000000000000" data-d2-rs-dash-gap-bits="4023bb3511f45fb3" data-d2-rs-dash-offset-bits="0000000000000000"><rect id="control-native-raster-shape-0-primitive-0" x="4.25" y="7.125" width="140.5" height="80.75" rx="7.75" stroke-dasharray="10,9.865639"/></g></svg></svg>"##;
        let (_, sources) = super::tag_embedded_svg_images(source);
        let rect = sources
            .native_rects()
            .get("control-native-raster-shape-0-primitive-0")
            .unwrap();
        assert_eq!(rect.box_values, [4.25, 7.125, 140.5, 80.75]);
        assert_eq!(rect.radii, [7.75, 7.75]);
        for altered in [
            source.replace("data-d2-rs-dash-owner-id", "authored-dash-owner"),
            source.replace("data-d2-rs-dash-gap-bits", "authored-gap"),
            source.replace(
                "data-d2-rs-canvas-reference=\"ordinary\"",
                "data-d2-rs-canvas-reference=\"resource\"",
            ),
            source.replace("<rect id=", "<rect style=\"rx:1\" id="),
            source.replace("<rect id=", "<rect transform=\"translate(1)\" id="),
            source.replace("<rect id=", "<rect xmlns=\"urn:authored\" id="),
            source.replace("rx=\"7.75\"", "rx=\"-1\""),
            source.replace("width=\"140.5\"", "width=\"50%\""),
            source.replace("height=\"80.75\"", "height=\"0\""),
            source.replace("x=\"4.25\"", "x=\"NaN\""),
            source
                .replace("x=\"4.25\"", "x=\"1e308\"")
                .replace("width=\"140.5\"", "width=\"1e308\""),
            source.replace("-primitive-0", "-primitive-authored"),
            source.replace(
                "</g>",
                "<path id=\"control-native-raster-shape-0-primitive-0\"/></g>",
            ),
        ] {
            let (_, sources) = super::tag_embedded_svg_images(&altered);
            assert!(sources.native_rects().is_empty(), "{altered}");
        }
    }

    #[test]
    fn native_sharp_shadow_requires_exact_filter_and_native_owner() {
        let source = r##"<svg xmlns="http://www.w3.org/2000/svg" width="342" height="282" viewBox="0 0 342 282" preserveAspectRatio="xMinYMin meet"><defs><filter id="shadow" x="-20%" y="-20%" width="160%" height="160%"><feFlood flood-color="#3d4574" flood-opacity="0.4" result="shadow-color"/><feComposite in="shadow-color" in2="SourceAlpha" operator="in"/><feOffset dx="3" dy="5"/><feBlend in="SourceGraphic" mode="normal"/></filter></defs><svg id="d2-rs-native-canvas-viewport-control" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="342" height="282" viewBox="-101 -101 342 282"><g id="d2-rs-native-shape-shadow-control" data-d2-rs-shape-shadow="sharp" filter="url(#shadow)"><rect width="140" height="80"/></g></svg></svg>"##;
        let id = "d2-rs-native-shape-shadow-control";
        let (_, sources) = super::tag_embedded_svg_images(source);
        assert!(sources.is_native_shape_shadow(id));
        for altered in [
            source.replace(
                "data-d2-rs-canvas-reference=\"ordinary\"",
                "data-d2-rs-canvas-reference=\"resource\"",
            ),
            source.replace("<filter id=", "<filter filterUnits=\"userSpaceOnUse\" id="),
            source.replace("<feBlend in=", "<feBlend in2=\"SourceAlpha\" in="),
            source.replace("<feOffset dx=", "<feOffset result=\"changed\" dx="),
            source.replace("dy=\"5\"", "dy=\"6\""),
            source.replace("160%", "150%"),
            source.replace("<feComposite in=", "<feComposite style=\"opacity:0.5\" in="),
            source.replace("<g id=", "<g transform=\"translate(1)\" id="),
            source.replace("</defs>", "<filter id=\"shadow\"/></defs>"),
            source.replace("</g>", "<g id=\"d2-rs-native-shape-shadow-control\"/></g>"),
        ] {
            let (_, sources) = super::tag_embedded_svg_images(&altered);
            assert!(!sources.is_native_shape_shadow(id), "{altered}");
        }
    }

    #[test]
    fn native_canvas_geometry_requires_unique_ordinary_root_viewport_contract() {
        let source = r#"<svg xmlns="http://www.w3.org/2000/svg" width="101" height="94" viewBox="0 0 302 282" preserveAspectRatio="xMidYMid meet"><svg id="d2-rs-native-canvas-viewport-main" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="302" height="282" viewBox="-101 -101 302 282"/></svg>"#;
        let (_, sources) = super::tag_embedded_svg_images(source);
        let viewport = sources
            .native_canvas_geometry("d2-rs-native-canvas-viewport-main")
            .unwrap();
        assert_eq!(viewport.view_box, [-101.0, -101.0, 302.0, 282.0]);
        assert!(viewport.center);
        let mask_source = source.replace("/></svg>", r#"><mask id="labels" data-d2-rs-connection-mask="scene" maskUnits="userSpaceOnUse"><rect x="-101" y="-101" width="302" height="282" fill="white"/><rect x="12.00000001" y="148" width="43" height="21" fill="black"/></mask></svg></svg>"#);
        let (_, retained) = super::tag_embedded_svg_images(&mask_source);
        let mask = retained.native_connection_mask("labels").unwrap();
        assert_eq!(mask.viewport.view_box, viewport.view_box);
        assert_eq!(mask.boxes[1], [12.00000001, 148.0, 43.0, 21.0]);
        // d2raster/raster.go composes the masked rough node's transform
        // before preparing its shared connection mask.
        let rough_mask = mask_source.replace("</mask>", r##"</mask><g id="rough-mask" mask="url(#labels)"><path id="rough" data-d2-rs-sketch-set="rough" transform="translate(12.00000001 148)" d="M0 0L1 1"/></g>"##);
        let (_, retained) = super::tag_embedded_svg_images(&rough_mask);
        assert_eq!(
            retained.native_mask_transform("rough-mask"),
            [1.0, 0.0, 0.0, 1.0, 12.00000001, 148.0]
        );
        for altered in [
            rough_mask.replace("data-d2-rs-sketch-set=\"rough\"", ""),
            rough_mask.replace("data-d2-rs-connection-mask=\"scene\"", ""),
            rough_mask.replace(
                "<g id=\"rough-mask\"",
                "<g style=\"opacity:1\" id=\"rough-mask\"",
            ),
        ] {
            let (_, retained) = super::tag_embedded_svg_images(&altered);
            assert_eq!(
                retained.native_mask_transform("rough-mask"),
                [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                "{altered}"
            );
        }
        for altered in [
            mask_source.replace("maskUnits=", "transform=\"translate(1)\" maskUnits="),
            mask_source.replace("height=\"21\"", "height=\"0\""),
            mask_source.replace("<rect x=\"12", "<rect rx=\"1\" x=\"12"),
            mask_source.replace("data-d2-rs-connection-mask=\"scene\"", ""),
        ] {
            let (_, retained) = super::tag_embedded_svg_images(&altered);
            assert!(
                retained.native_connection_mask("labels").is_none(),
                "{altered}"
            );
        }
        let authored = [
            source.replace("<svg id=", "<svg x=\"1\" id="),
            source.replace("<svg id=", "<svg style=\"opacity:0.5\" id="),
            source.replace("<svg id=", "<svg transform=\"translate(1)\" id="),
            source.replace("<svg id=", "<svg mask=\"url(#mask)\" id="),
            source.replace("viewBox=\"0 0 302 282\"", "viewBox=\"0 0 303 282\""),
            source.replace("width=\"302\"", "width=\"301\""),
            source.replace("xMidYMid meet", "xMidYMid slice"),
            source.replace(
                "data-d2-rs-canvas-reference=\"ordinary\"",
                "data-d2-rs-canvas-reference=\"resource\"",
            ),
            source.replace(
                "</svg>",
                "<g id=\"d2-rs-native-canvas-viewport-main\"/></svg>",
            ),
        ];
        for source in authored {
            let (_, sources) = super::tag_embedded_svg_images(&source);
            assert!(
                sources
                    .native_canvas_geometry("d2-rs-native-canvas-viewport-main")
                    .is_none(),
                "{source}"
            );
        }
    }

    #[test]
    fn logical_canvas_precision_is_bound_to_root_and_resolved_viewport() {
        let root = r#"<svg xmlns="http://www.w3.org/2000/svg" width="300.0000001px" height="120.0000001"><svg width="9" height="7"/></svg>"#;
        let (tagged, sources) = super::tag_embedded_svg_images(root);
        assert_eq!(tagged, root);
        let parsed = resvg::usvg::Size::from_wh(300.0, 120.0).unwrap();
        assert_eq!(sources.logical_size(parsed), [300.0000001, 120.0000001]);
        let overridden = resvg::usvg::Size::from_wh(301.0, 121.0).unwrap();
        assert_eq!(sources.logical_size(overridden), [301.0, 121.0]);
        let (_, viewbox) =
            super::tag_embedded_svg_images(r#"<svg viewBox="0,0,300.0000001,120.0000001"/>"#);
        assert_eq!(viewbox.logical_size(parsed), [300.0000001, 120.0000001]);
        for source in [
            r#"<svg width="50%" height="20%"/>"#,
            r#"<svg width="3cm" height="1cm"/>"#,
            r#"<svg width="NaN" height="120"/>"#,
            r#"<svg width="300" height="-1"/>"#,
            r#"<svg width="300"/>"#,
        ] {
            let (_, sources) = super::tag_embedded_svg_images(source);
            assert!(sources.logical_size.is_none());
            assert_eq!(sources.logical_size(parsed), [300.0, 120.0]);
        }
    }

    #[test]
    fn scaled_canvas_rejects_invalid_and_unrepresentable_storage_dimensions() {
        assert_eq!(super::scaled_dimension(0.125, 3), Some(1));
        assert_eq!(
            super::scaled_dimension(f64::from(u32::MAX), 1),
            Some(u32::MAX)
        );
        for (logical, scale) in [
            (1.0, 0),
            (0.0, 1),
            (-1.0, 1),
            (f64::NAN, 1),
            (f64::INFINITY, 1),
            (f64::from(u32::MAX) + 0.125, 1),
            (f64::from(u32::MAX), 2),
        ] {
            assert_eq!(super::scaled_dimension(logical, scale), None);
        }
    }

    #[test]
    fn raster_canvas_reference_collects_only_marked_main_svg_viewport() {
        let source = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
          <svg id="d2-rs-native-canvas-viewport-main" class="d2-main d2-svg" data-d2-rs-canvas-reference="ordinary"><text x="1" y="10">main</text></svg>
          <svg id="d2-rs-native-canvas-viewport-unmarked" class="d2-svg"><text x="1" y="10">unmarked</text></svg>
          <g><svg id="d2-rs-native-canvas-viewport-nonmain" class="d2-svg" data-d2-rs-canvas-reference="ordinary"><text x="1" y="10">nested resource</text></svg></g>
          <g id="d2-rs-native-canvas-viewport-wrongtype" class="d2-svg" data-d2-rs-canvas-reference="ordinary"><text x="1" y="10">group</text></g>
          <svg id="d2-rs-native-canvas-viewport-authored-clip" class="d2-svg" data-d2-rs-canvas-reference="ordinary" clip-path="url(#real)"><text x="1" y="10">authored clip</text></svg>
          <svg id="d2-rs-native-canvas-viewport-authored-style" class="d2-svg" data-d2-rs-canvas-reference="ordinary" style="clip-path:url(#real)"><text x="1" y="10">authored style</text></svg>
        </svg>"#;
        let (_, sources) = super::tag_embedded_svg_images(source);
        assert!(sources.is_native_canvas_viewport("d2-rs-native-canvas-viewport-main"));
        for id in [
            "d2-rs-native-canvas-viewport-unmarked",
            "d2-rs-native-canvas-viewport-nonmain",
            "d2-rs-native-canvas-viewport-wrongtype",
            "d2-rs-native-canvas-viewport-authored-clip",
            "d2-rs-native-canvas-viewport-authored-style",
        ] {
            assert!(
                !sources.is_native_canvas_viewport(id),
                "unqualified source: {id}"
            );
        }
    }

    use super::*;

    #[test]
    fn effect_opacity_source_does_not_override_css_or_duplicate_identities() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><g id="unique" opacity="0.9" data-d2-rs-exact-opacity="0.9"/><g id="duplicate" opacity="0.9" data-d2-rs-exact-opacity="0.9"/><g id="duplicate"/><g id="invalid" data-d2-rs-exact-opacity="NaN"/></svg>"#;
        let (_, sources) = tag_embedded_svg_images(svg);
        assert_eq!(sources.exact_effect_opacity("unique", 0.9), 0.9_f64);
        assert_eq!(
            sources.exact_effect_opacity("unique", 0.2),
            f64::from(0.2_f32)
        );
        assert_eq!(
            sources.exact_effect_opacity("duplicate", 0.9),
            f64::from(0.9_f32)
        );
        assert_eq!(
            sources.exact_effect_opacity("invalid", 0.9),
            f64::from(0.9_f32)
        );
    }

    #[test]
    fn relative_commands_accumulate_in_float64() {
        let path = parse_path("m0.1 0.2l0.2 0.2l0.2 0.2").unwrap();
        assert_eq!(
            path.commands,
            [
                SvgPathCommand::MoveTo(SvgPoint { x: 0.1, y: 0.2 }),
                SvgPathCommand::LineTo(SvgPoint {
                    x: 0.30000000000000004,
                    y: 0.4,
                }),
                SvgPathCommand::LineTo(SvgPoint {
                    x: 0.5,
                    y: 0.6000000000000001,
                }),
            ]
        );
    }

    #[test]
    fn smooth_commands_expand_using_the_previous_control_point() {
        let path = parse_path("M0 0C1 0 1 1 2 1s1 1 2 0Q6 0 8 1t2 0").unwrap();
        assert_eq!(
            path.commands,
            [
                SvgPathCommand::MoveTo(SvgPoint { x: 0.0, y: 0.0 }),
                SvgPathCommand::CubicTo(
                    SvgPoint { x: 1.0, y: 0.0 },
                    SvgPoint { x: 1.0, y: 1.0 },
                    SvgPoint { x: 2.0, y: 1.0 },
                ),
                SvgPathCommand::CubicTo(
                    SvgPoint { x: 3.0, y: 1.0 },
                    SvgPoint { x: 3.0, y: 2.0 },
                    SvgPoint { x: 4.0, y: 1.0 },
                ),
                SvgPathCommand::QuadraticTo(
                    SvgPoint { x: 6.0, y: 0.0 },
                    SvgPoint { x: 8.0, y: 1.0 },
                ),
                SvgPathCommand::QuadraticTo(
                    SvgPoint { x: 10.0, y: 2.0 },
                    SvgPoint { x: 10.0, y: 1.0 },
                ),
            ]
        );
    }

    #[test]
    fn simple_embedded_svg_gets_a_private_source_id() {
        let source = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="m1 1h8v8z"/></svg>"#;
        let encoded = base64::engine::general_purpose::STANDARD.encode(source);
        let input = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><image href="{SVG_DATA_PREFIX}{encoded}"/></svg>"#
        );
        let (output, paths) = tag_embedded_svg_images(&input);
        assert!(output.contains(&format!("id=\"{SOURCE_ID_PREFIX}0\"")));
        assert_eq!(paths.len(), 1);
        assert_eq!(
            paths.image_paths(&format!("{SOURCE_ID_PREFIX}0")).unwrap()[0].commands,
            [
                SvgPathCommand::MoveTo(SvgPoint { x: 1.0, y: 1.0 }),
                SvgPathCommand::LineTo(SvgPoint { x: 9.0, y: 1.0 }),
                SvgPathCommand::LineTo(SvgPoint { x: 9.0, y: 9.0 }),
                SvgPathCommand::Close,
            ]
        );
    }

    #[test]
    fn text_origins_preserve_decimal_precision_and_choose_nearest_duplicate() {
        let input = r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="367.5" y="54.610964">document</text><text x="712.5" y="187.3">document</text></svg>"#;
        let (output, sources) = tag_embedded_svg_images(input);

        assert!(matches!(output, Cow::Borrowed(_)));
        assert_eq!(
            sources.exact_text_origin("document", 367.5, 54.610965728759766),
            Some(SvgPoint {
                x: 367.5,
                y: 54.610964,
            }),
        );
        assert_eq!(
            sources.exact_text_origin("document", 712.5, 187.3000030517578),
            Some(SvgPoint { x: 712.5, y: 187.3 }),
        );
    }

    #[test]
    fn raster_multiline_origins_preserve_exact_private_scene_metadata() {
        let x = 100.0_f64;
        let y = 106.666_666_666_666_67_f64;
        let input = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><text id="native-edge-0-line-1" x="100" y="106.666667" data-d2-rs-text-x-bits="{:016x}" data-d2-rs-text-y-bits="{:016x}">identical</text></svg>"#,
            x.to_bits(),
            y.to_bits(),
        );
        let (_, sources) = tag_embedded_svg_images(&input);
        let origin = sources
            .exact_text_origin("identical", x, 106.666_664_123_535_16)
            .expect("leaf text scene origin");
        assert_eq!(origin.x.to_bits(), x.to_bits());
        assert_eq!(origin.y.to_bits(), y.to_bits());
    }

    #[test]
    fn raster_multiline_origins_use_identity_for_coincident_duplicate_text() {
        let x = 100.0_f64;
        let first_y = 106.666_666_666_666_67_f64;
        let second_y = f64::from_bits(first_y.to_bits() + 1);
        // Both scene origins serialize and parse to the same f32 SVG point.
        // Text-content/nearest-point matching cannot preserve their identity.
        let input = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><text id="line-first" x="100" y="106.666667" data-d2-rs-text-x-bits="{:016x}" data-d2-rs-text-y-bits="{:016x}">identical</text><text id="d2rs-synthetic-italic-marker--line-second" x="100" y="106.666667" data-d2-rs-text-x-bits="{:016x}" data-d2-rs-text-y-bits="{:016x}">identical</text></svg>"#,
            x.to_bits(),
            first_y.to_bits(),
            x.to_bits(),
            second_y.to_bits(),
        );
        let (_, sources) = tag_embedded_svg_images(&input);
        let first = sources.exact_text_origin_by_id("line-first").unwrap();
        let second = sources
            .exact_text_origin_by_id("d2rs-synthetic-italic-marker--line-second")
            .unwrap();
        assert_eq!(first.y.to_bits(), first_y.to_bits());
        assert_eq!(second.y.to_bits(), second_y.to_bits());
        assert!(sources.exact_text_origin_by_id("line-missing").is_none());
    }

    #[test]
    fn source_paths_follow_the_image_id_onto_usvgs_wrapper_group() {
        let asset = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 10 10"><g><path d="m1 1h8v8z" fill="#000"/></g></svg>"##;
        let encoded = base64::engine::general_purpose::STANDARD.encode(asset);
        let outer = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><image x="0" y="0" width="10" height="10" href="{SVG_DATA_PREFIX}{encoded}"/></svg>"#
        );
        let (tagged, sources) = tag_embedded_svg_images(&outer);
        let tree = resvg::usvg::Tree::from_str(&tagged, &resvg::usvg::Options::default()).unwrap();
        let wrapper_id = tree
            .root()
            .children()
            .iter()
            .find_map(|node| match node {
                resvg::usvg::Node::Group(group) if group.id().starts_with(SOURCE_ID_PREFIX) => {
                    Some(group.id())
                }
                _ => None,
            })
            .expect("usvg stores the image element ID on its wrapper group");

        assert_eq!(
            sources.image_paths(wrapper_id).map(<[SvgPath]>::len),
            Some(1)
        );
    }

    #[test]
    fn built_in_paper_pattern_retains_float64_path_sources() {
        let pattern = d2_rs_render::assets::PAPER_SVG.replace("%[1]s", "d2-rs");
        let input = format!(
            "<svg xmlns=\"{SVG_NAMESPACE}\"><defs>{pattern}</defs><rect width=\"75\" height=\"75\" fill=\"url(#paper-d2-rs)\"/></svg>"
        );
        let (tagged, sources) = tag_embedded_svg_images(&input);

        assert_eq!(sources.raw_paths.len(), 1042);
        assert_eq!(
            sources
                .raw_paths
                .values()
                .map(|path| path.commands.len())
                .sum::<usize>(),
            10926
        );
        assert!(tagged.contains(&format!("id=\"{RAW_PATH_SOURCE_ID_PREFIX}0\"")));

        let tree = resvg::usvg::Tree::from_str(&tagged, &resvg::usvg::Options::default()).unwrap();
        fn tagged_paths(group: &resvg::usvg::Group) -> usize {
            group
                .children()
                .iter()
                .map(|child| match child {
                    resvg::usvg::Node::Group(group) => tagged_paths(group),
                    resvg::usvg::Node::Path(path)
                        if path.id().starts_with(RAW_PATH_SOURCE_ID_PREFIX) =>
                    {
                        1
                    }
                    resvg::usvg::Node::Path(_)
                    | resvg::usvg::Node::Image(_)
                    | resvg::usvg::Node::Text(_) => 0,
                })
                .sum()
        }
        assert_eq!(
            tree.patterns()
                .iter()
                .map(|pattern| tagged_paths(pattern.root()))
                .sum::<usize>(),
            1042
        );
    }

    #[test]
    fn paper_pattern_mask_paths_are_not_mapped_as_visible_source_paths() {
        let input = format!(
            "<svg xmlns=\"{SVG_NAMESPACE}\"><pattern id=\"paper-test\"><mask><path d=\"M0 0h1v1z\"/></mask><path d=\"M1.00000001 2.00000002h3v4z\"/></pattern></svg>"
        );
        let (tagged, sources) = tag_embedded_svg_images(&input);
        assert_eq!(sources.raw_paths.len(), 1);
        assert_eq!(
            sources.raw_path(&format!("{RAW_PATH_SOURCE_ID_PREFIX}0")),
            Some(&parse_path("M1.00000001 2.00000002h3v4z").unwrap())
        );
        assert_eq!(tagged.matches(RAW_PATH_SOURCE_ID_PREFIX).count(), 1);
    }

    #[test]
    fn embedded_path_identity_survives_metadata_and_unpainted_primitives() {
        let asset = r##"<svg xmlns="http://www.w3.org/2000/svg"
            xmlns:editor="http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd"
            width="40" height="40" viewBox="0 0 20 20">
            <editor:namedview/><path id="ink" d="M1.00000001 2.00000002L3 4"/>
            <rect width="20" height="20" fill="none"/>
        </svg>"##;
        let data = base64::engine::general_purpose::STANDARD.encode(asset);
        let input = format!(
            r##"<svg xmlns="{SVG_NAMESPACE}" width="40" height="40">
            <image width="20" height="20" href="{SVG_DATA_PREFIX}{data}"/>
        </svg>"##
        );
        let (tagged, sources) = tag_embedded_svg_images(&input);
        let id = format!("{SOURCE_ID_PREFIX}0");
        assert!(
            sources.image_paths(&id).unwrap().is_empty(),
            "no ordinal correspondence is assumed"
        );
        let expected = parse_path("M1.00000001 2.00000002L3 4").unwrap();
        assert_eq!(sources.image_paths_by_id(&id).unwrap()["ink"], expected);
        let tree = resvg::usvg::Tree::from_str(&tagged, &resvg::usvg::Options::default()).unwrap();
        fn retained_path(group: &resvg::usvg::Group) -> Option<&resvg::usvg::Path> {
            group.children().iter().find_map(|node| match node {
                resvg::usvg::Node::Path(path) => Some(path.as_ref()),
                resvg::usvg::Node::Group(group) => retained_path(group),
                resvg::usvg::Node::Image(image) => match image.kind() {
                    resvg::usvg::ImageKind::SVG(tree) => retained_path(tree.root()),
                    _ => None,
                },
                _ => None,
            })
        }
        assert_eq!(
            retained_path(tree.root()).unwrap().id(),
            "ink",
            "actual converted tree retains the source path identity"
        );
    }

    #[test]
    fn duplicate_path_id_does_not_publish_ambiguous_coordinates() {
        let paths = parse_svg_paths_by_id(
            r##"<svg xmlns="http://www.w3.org/2000/svg">
            <path id="duplicate" d="M1 2L3 4"/><path id="duplicate" d="M5 6L7 8"/>
            <path id="unique" d="M9 10L11 12"/>
        </svg>"##,
        );
        assert!(!paths.contains_key("duplicate"));
        assert_eq!(paths["unique"], parse_path("M9 10L11 12").unwrap());
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_cancel_lines_keep_complete_primitive_correspondence() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/cancel-lines-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 81);
        for row in rows {
            let source = row["svg"].as_str().unwrap();
            let paths = parse_simple_svg_paths(source)
                .expect("actual source cancellation retains every primitive");
            let document = roxmltree::Document::parse(source).unwrap();
            let nodes: Vec<_> = document
                .descendants()
                .filter(|n| {
                    n.is_element()
                        && matches!(
                            n.tag_name().name(),
                            "path"
                                | "line"
                                | "rect"
                                | "circle"
                                | "ellipse"
                                | "polygon"
                                | "polyline"
                        )
                })
                .collect();
            assert_eq!(
                paths.len(),
                nodes.len(),
                "{}: complete source inventory",
                row["expression"]
            );
            for (path, node) in paths.iter().zip(nodes) {
                if node.has_tag_name("line") {
                    let value = |name| node.attribute(name).unwrap().parse::<f64>().unwrap();
                    assert_eq!(
                        path.commands,
                        vec![
                            SvgPathCommand::MoveTo(SvgPoint {
                                x: value("x1"),
                                y: value("y1")
                            }),
                            SvgPathCommand::LineTo(SvgPoint {
                                x: value("x2"),
                                y: value("y2")
                            })
                        ]
                    );
                }
            }
        }
    }

    #[test]
    fn unpainted_or_unresolved_lines_keep_the_conversion_fallback() {
        for attributes in [
            "",
            "stroke=\"none\"",
            "stroke=\"invalid-paint\"",
            "stroke=\"red\" stroke-width=\"0\"",
            "stroke=\"red\" display=\"none\"",
            "stroke=\"red\" style=\"stroke:none\"",
        ] {
            let source = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\"><line x1=\"1\" y1=\"2\" x2=\"3\" y2=\"4\" {attributes}/><path d=\"M1 1L3 3\"/></svg>"
            );
            assert!(
                parse_simple_svg_paths(&source).is_none(),
                "unresolved ordinal correspondence: {attributes}"
            );
        }
    }

    #[test]
    fn nested_group_transforms_do_not_hide_path_sources() {
        assert_eq!(
            parse_simple_svg_paths(
                r##"<svg xmlns="http://www.w3.org/2000/svg"><g transform="translate(1)" fill="#000"><path d="M1 1L3 3"/></g></svg>"##
            )
            .unwrap()
            .len(),
            1
        );
        assert_eq!(
            parse_simple_svg_paths(
                r#"<svg xmlns="http://www.w3.org/2000/svg"><g><path d="M1 1L3 3"/></g></svg>"#
            )
            .unwrap()
            .len(),
            1
        );
    }

    #[test]
    fn elliptical_arcs_match_d2_arc_command_semantics() {
        let path = parse_path("M0 0A10 20 370 1 0 30 40").unwrap();
        assert_eq!(
            path.commands,
            [
                SvgPathCommand::MoveTo(SvgPoint { x: 0.0, y: 0.0 }),
                SvgPathCommand::ArcTo {
                    radius_x: 10.0,
                    radius_y: 20.0,
                    rotation: 10.0 / 180.0 * std::f64::consts::PI,
                    large_arc: true,
                    sweep: false,
                    end: SvgPoint { x: 30.0, y: 40.0 },
                },
            ]
        );

        let degenerate = parse_path("M0 0A0 4 0 0 1 9 7a5 5 0 0 0 0 0").unwrap();
        assert_eq!(
            degenerate.commands,
            [
                SvgPathCommand::MoveTo(SvgPoint { x: 0.0, y: 0.0 }),
                SvgPathCommand::LineTo(SvgPoint { x: 9.0, y: 7.0 }),
            ]
        );
    }

    #[test]
    fn source_tag_insertion_stays_inside_the_image_start_tag() {
        let svg = r#"<image href="data:image/svg+xml;base64,abc" x="1 > 0" />"#;
        let end = svg.len();
        let insert_at = start_tag_insert_position(svg, 0, end).unwrap();
        let mut result = svg.to_owned();
        result.insert_str(insert_at, " id=\"source\"");
        assert_eq!(
            result,
            r#"<image href="data:image/svg+xml;base64,abc" x="1 > 0"  id="source"/>"#
        );
    }
}

#[cfg(test)]
mod markdown_viewport_number_contract {
    use super::*;
    #[test]
    fn native_viewport_qualification_compares_exact_svg_coordinates() {
        for viewbox in [
            "0 0 235 329",
            "0 0 235.000 329.000",
            "0,0,235,329",
            "0 0 2.35e2 3.29e2",
        ] {
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="500" height="500"><svg id="d2-rs-native-md-viewport-test" data-d2-rs-markdown-reference="ordinary" width="235" height="329" viewBox="{viewbox}"/></svg>"#
            );
            let (_, sources) = tag_embedded_svg_images(&svg);
            assert_eq!(
                sources.native_markdown_viewport("d2-rs-native-md-viewport-test"),
                Some([0.0, 0.0, 235.0, 329.0])
            );
        }
        for viewbox in [
            "1 0 235 329",
            "0 0 235.0000000001 329",
            "0 0 235 329 1",
            "0 0 NaN 329",
            "0 0 235",
        ] {
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="500" height="500"><svg id="d2-rs-native-md-viewport-test" data-d2-rs-markdown-reference="ordinary" width="235" height="329" viewBox="{viewbox}"/></svg>"#
            );
            let (_, sources) = tag_embedded_svg_images(&svg);
            assert!(
                sources
                    .native_markdown_viewport("d2-rs-native-md-viewport-test")
                    .is_none()
            );
        }
    }
}

#[cfg(test)]
mod original_image_transform_contract {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_identity_bound_image_matrices_keep_viewports_and_ancestors() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-image-transforms-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 648);
        let mut outcomes = Vec::new();
        let mut failures = Vec::new();
        for row in rows.as_array().unwrap() {
            let r = &row["request"];
            let b = r["Box"].as_array().unwrap();
            let c = r["Canvas"].as_array().unwrap();
            let f = |v: &serde_json::Value| v.as_f64().unwrap();
            let scale = f(&r["Scale"]);
            let data = base64::engine::general_purpose::STANDARD.encode(r["SVG"].as_str().unwrap());
            let outer = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}" preserveAspectRatio="xMinYMin meet"><svg class="d2-svg" id="d2-rs-native-canvas-viewport-test" data-d2-rs-canvas-reference="ordinary" width="{}" height="{}" viewBox="{} {} {} {}"><image x="{}" y="{}" width="{}" height="{}" preserveAspectRatio="{}" href="data:image/svg+xml;base64,{}"/></svg></svg>"#,
                f(&c[2]) * scale,
                f(&c[3]) * scale,
                c[2],
                c[3],
                c[2],
                c[3],
                c[0],
                c[1],
                c[2],
                c[3],
                b[0],
                b[1],
                b[2],
                b[3],
                r["Aspect"].as_str().unwrap(),
                data
            );
            let (_, sources) = tag_embedded_svg_images(&outer);
            let path = &sources
                .image_paths_by_id(&format!("{SOURCE_ID_PREFIX}0"))
                .unwrap()["ink"];
            let mut actual = path.scene_transform.unwrap();
            for matrix in path.intrinsic_transforms.as_ref().unwrap() {
                actual = source_matrix_mul(actual, *matrix);
            }
            let wanted = row["matrices"]["ink"].as_array().unwrap();
            let matched = actual
                .iter()
                .zip(wanted)
                .all(|(a, e)| a.to_bits() == f(e).to_bits());
            if !matched {
                failures.push((r["Name"].clone(), actual, row["matrices"]["ink"].clone()));
            }
            assert!(path.image_viewport.is_some());
            outcomes.push(serde_json::json!({"request":r,"matrix":actual,"expected":wanted,"matched":matched}));
        }
        if let Some(path) = std::env::var_os("D2_RS_TEST_IMAGE_TRANSFORMS_OUTPUT") {
            std::fs::write(path, serde_json::to_vec(&outcomes).unwrap()).unwrap();
        }
        assert!(
            failures.is_empty(),
            "original image transform differences: {failures:?}"
        );
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod original_anonymous_image_path_contract {
    use super::*;
    use resvg::usvg;
    fn painted<'a>(group: &'a usvg::Group, paths: &mut Vec<&'a usvg::Path>) {
        for node in group.children() {
            match node {
                usvg::Node::Group(child) => painted(child, paths),
                usvg::Node::Path(path)
                    if path.is_visible() && (path.fill().is_some() || path.stroke().is_some()) =>
                {
                    paths.push(path)
                }
                usvg::Node::Path(_) => {}
                _ => panic!("unexpected node in original path controls"),
            }
        }
    }
    fn rgba(paint: &usvg::Paint, opacity: f32, exact: Option<f64>) -> serde_json::Value {
        let color = crate::scanline::solid_paint_exact(paint, opacity, exact)
            .expect("original solid controls");
        serde_json::json!({"R":color[0],"G":color[1],"B":color[2],"A":color[3]})
    }
    #[test]
    fn anonymous_paths_keep_original_geometry_matrices_and_class_paints() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-image-anonymous-paths-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 4536);
        let mut outcomes = Vec::new();
        let mut failures = Vec::new();
        let mut visible = 0;
        let mut hidden = 0;
        for row in rows.as_array().unwrap() {
            let r = &row["request"];
            let b = r["Box"].as_array().unwrap();
            let c = r["Canvas"].as_array().unwrap();
            let f = |v: &serde_json::Value| v.as_f64().unwrap();
            let scale = f(&r["Scale"]);
            let data = base64::engine::general_purpose::STANDARD.encode(r["SVG"].as_str().unwrap());
            let outer = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}" preserveAspectRatio="xMinYMin meet"><svg class="d2-svg" id="d2-rs-native-canvas-viewport-test" data-d2-rs-canvas-reference="ordinary" width="{}" height="{}" viewBox="{} {} {} {}"><image x="{}" y="{}" width="{}" height="{}" preserveAspectRatio="{}" href="data:image/svg+xml;base64,{}"/></svg></svg>"#,
                f(&c[2]) * scale,
                f(&c[3]) * scale,
                c[2],
                c[3],
                c[2],
                c[3],
                c[0],
                c[1],
                c[2],
                c[3],
                b[0],
                b[1],
                b[2],
                b[3],
                r["Aspect"].as_str().unwrap(),
                data
            );
            let (tagged, sources) = tag_embedded_svg_images(&outer);
            let tree = usvg::Tree::from_str(&tagged, &usvg::Options::default()).unwrap();
            let mut outer_paths = Vec::new();
            fn images<'a>(group: &'a usvg::Group, out: &mut Vec<&'a usvg::Path>) {
                for n in group.children() {
                    match n {
                        usvg::Node::Group(g) => images(g, out),
                        usvg::Node::Image(i) => {
                            let usvg::ImageKind::SVG(t) = i.kind() else {
                                panic!("vector control")
                            };
                            painted(t.root(), out)
                        }
                        _ => panic!("unexpected outer control node"),
                    }
                }
            }
            images(tree.root(), &mut outer_paths);
            let expected = row["paths"].as_array().unwrap();
            let mut matrices = Vec::new();
            let mut matrix_match = outer_paths.len() == expected.len();
            let mut geometry_match = matrix_match;
            let mut paint_match = matrix_match;
            for (path, wanted) in outer_paths.iter().zip(expected) {
                let source = &sources
                    .image_paths_by_id(&format!("{SOURCE_ID_PREFIX}0"))
                    .unwrap()[path.id()];
                assert!(path.id().starts_with("d2-rs-imported-path-"));
                let mut matrix = source.scene_transform.unwrap();
                for m in source.intrinsic_transforms.as_ref().unwrap() {
                    matrix = source_matrix_mul(matrix, *m)
                }
                matrix_match &= matrix
                    .iter()
                    .zip(wanted["matrix"].as_array().unwrap())
                    .all(|(a, e)| a.to_bits() == f(e).to_bits());
                matrices.push(matrix);
                let commands = wanted["path"]["Commands"].as_array().unwrap();
                geometry_match &= source.commands.len() == commands.len();
                for (actual, e) in source.commands.iter().zip(commands) {
                    let (kind, point) = match actual {
                        SvgPathCommand::MoveTo(p) => (0, *p),
                        SvgPathCommand::LineTo(p) => (1, *p),
                        SvgPathCommand::Close => (5, SvgPoint { x: 0.0, y: 0.0 }),
                        _ => panic!("unexpected original square command"),
                    };
                    geometry_match &= e["Kind"] == kind
                        && point.x.to_bits() == f(&e["P1"]["X"]).to_bits()
                        && point.y.to_bits() == f(&e["P1"]["Y"]).to_bits();
                }
                let fill = path.fill().unwrap();
                let stroke = path.stroke().unwrap();
                paint_match &= rgba(
                    fill.paint(),
                    fill.opacity().get(),
                    source.paint_opacities.map(|o| o[0]),
                ) == wanted["path"]["Fill"]["Color"]
                    && rgba(
                        stroke.paint(),
                        stroke.opacity().get(),
                        source.paint_opacities.map(|o| o[1]),
                    ) == wanted["path"]["Stroke"]["Paint"]["Color"]
                    && f64::from(stroke.width().get()) == f(&wanted["path"]["Stroke"]["Width"]);
            }
            if expected.is_empty() {
                hidden += 1
            } else {
                visible += 1
            };
            let matched = matrix_match && geometry_match && paint_match;
            if !matched {
                failures.push(r["Name"].clone())
            }
            outcomes.push(serde_json::json!({"request":r,"matrices":matrices,"expected_paths":expected,"matrix_match":matrix_match,"geometry_match":geometry_match,"paint_match":paint_match,"matched":matched}));
        }
        if let Some(path) = std::env::var_os("D2_RS_TEST_ANONYMOUS_IMAGE_OUTPUT") {
            std::fs::write(path, serde_json::to_vec(&outcomes).unwrap()).unwrap()
        }
        assert_eq!((visible, hidden), (3888, 648));
        assert!(
            failures.is_empty(),
            "anonymous original path differences: {failures:?}"
        );
    }
}

#[cfg(test)]
mod original_mixed_png_contract {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    use sha2::{Digest, Sha256};
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_nested_png_matrices_boxes_and_complete_raster() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/mixed-svg-png-source-v090.json"
        ))
        .unwrap();
        let mut outcomes = Vec::new();
        for row in rows.as_array().unwrap() {
            let r = &row["request"];
            let f = |v: &serde_json::Value| v.as_f64().unwrap();
            let b: Vec<_> = r["Box"].as_array().unwrap().iter().map(f).collect();
            let c: Vec<_> = r["Canvas"].as_array().unwrap().iter().map(f).collect();
            let scale = f(&r["Scale"]) as u32;
            let encoded =
                base64::engine::general_purpose::STANDARD.encode(r["SVG"].as_str().unwrap());
            let outer = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}" preserveAspectRatio="xMinYMin meet"><svg id="d2-rs-native-canvas-viewport-test" data-d2-rs-canvas-reference="ordinary" class="d2-svg" width="{}" height="{}" viewBox="{} {} {} {}"><image x="{}" y="{}" width="{}" height="{}" preserveAspectRatio="{}" href="data:image/svg+xml;base64,{}"/></svg></svg>"#,
                c[2],
                c[3],
                c[2],
                c[3],
                c[2],
                c[3],
                c[0],
                c[1],
                c[2],
                c[3],
                b[0],
                b[1],
                b[2],
                b[3],
                r["Aspect"].as_str().unwrap(),
                encoded
            );
            let (tagged, sources) = tag_embedded_svg_images(&outer);
            let carrier = &sources
                .image_paths_by_id(&format!("{SOURCE_ID_PREFIX}0"))
                .unwrap()["raster"];
            let mut matrix = carrier
                .scene_transform
                .unwrap()
                .map(|v| v * f64::from(scale));
            for m in carrier.intrinsic_transforms.as_ref().unwrap() {
                matrix = source_matrix_mul(matrix, *m);
            }
            let image_box = carrier.raster_image.unwrap().image_box;
            let expected_matrix: Vec<_> = row["images"][0]["matrix"]
                .as_array()
                .unwrap()
                .iter()
                .map(f)
                .collect();
            let expected_box: Vec<_> = row["images"][0]["box"]
                .as_array()
                .unwrap()
                .iter()
                .map(f)
                .collect();
            let matrix_match = matrix
                .iter()
                .zip(&expected_matrix)
                .all(|(a, b)| a.to_bits() == b.to_bits());
            let box_match = image_box
                .iter()
                .zip(&expected_box)
                .all(|(a, b)| a.to_bits() == b.to_bits());
            let tree =
                resvg::usvg::Tree::from_str(&tagged, &resvg::usvg::Options::default()).unwrap();
            let (width, height, pixels) =
                crate::scanline::render_simple_with_sources(&tree, scale, &[], &sources).unwrap();
            let digest = format!("{:x}", Sha256::digest(&pixels));
            let dimensions = serde_json::json!([width, height]);
            let pixel_match =
                digest == row["sha256"].as_str().unwrap() && dimensions == row["dimensions"];
            if !pixel_match {
                if let Ok(folder) = std::env::var("D2_RS_TEST_MIXED_PNG_FAILURE_DIR") {
                    std::fs::create_dir_all(&folder).unwrap();
                    let path = std::path::Path::new(&folder)
                        .join(format!("{}.rgba", r["Name"].as_str().unwrap()));
                    std::fs::write(path, &pixels).unwrap();
                }
            }
            outcomes.push(serde_json::json!({"name":r["Name"],"matrix":matrix,"image_box":image_box,"matrix_match":matrix_match,"box_match":box_match,"dimensions":dimensions,"sha256":digest,"pixel_match":pixel_match}));
        }
        if let Ok(path) = std::env::var("D2_RS_TEST_MIXED_PNG_OUTPUT") {
            std::fs::write(path, serde_json::to_vec(&outcomes).unwrap()).unwrap();
        }
        assert_eq!(outcomes.len(), 160);
        let failures: Vec<_> = outcomes
            .iter()
            .filter(|r| {
                r["matrix_match"] != true || r["box_match"] != true || r["pixel_match"] != true
            })
            .collect();
        assert!(
            failures.is_empty(),
            "original mixed PNG differences: {failures:?}"
        );
    }
}

#[cfg(test)]
mod math_scene_transform_tests {
    use super::*;

    #[test]
    fn centered_canvas_maps_embedded_math_in_scene_coordinates() {
        let source = r#"<svg width="200" height="100" viewBox="0 0 100 100" preserveAspectRatio="xMidYMid meet"><svg id="d2-rs-native-canvas-viewport-main" data-d2-rs-canvas-reference="ordinary" width="100" height="100" viewBox="-10 -20 100 100"><image x="5" y="7"/></svg></svg>"#;
        let document = roxmltree::Document::parse(source).unwrap();
        let image = document
            .descendants()
            .find(|node| node.has_tag_name("image"))
            .unwrap();
        assert_eq!(
            math_scene_transform(image, document.root_element()),
            Some([1.0, 0.0, 0.0, 1.0, 65.0, 27.0])
        );
    }
}

#[cfg(test)]
mod native_image_clip_source_contract {
    use super::*;
    #[test]
    fn arc_transport_requires_unique_plain_clip_in_qualified_native_canvas() {
        let source = r#"<svg xmlns="http://www.w3.org/2000/svg" width="101" height="94" viewBox="0 0 302 282" preserveAspectRatio="xMidYMid meet"><svg id="d2-rs-native-canvas-viewport-main" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="302" height="282" viewBox="-101 -101 302 282"><defs><clipPath id="radius" data-d2-rs-image-radius="scene"><path d="M12 0L88 0A12 12 0 0 1 100 12L100 88A12 12 0 0 1 88 100Z"/></clipPath></defs></svg></svg>"#;
        let (_, kept) = tag_embedded_svg_images(source);
        let path = kept.native_image_clip("radius").unwrap();
        assert!(path.commands.iter().any(|c| matches!(
            c,
            SvgPathCommand::ArcTo {
                radius_x: 12.0,
                radius_y: 12.0,
                ..
            }
        )));
        for altered in [
            source.replace("data-d2-rs-canvas-reference=", "data-unqualified="),
            source.replace("data-d2-rs-image-radius=", "data-unqualified="),
            source.replace("<clipPath id=", "<clipPath transform=\"scale(2)\" id="),
            source.replace("<path d=", "<path transform=\"scale(2)\" d="),
            source.replace("</defs>", "<clipPath id=\"radius\"/></defs>"),
            source.replace("</clipPath>", "<path d=\"M0 0L1 1\"/></clipPath>"),
        ] {
            let (_, omitted) = tag_embedded_svg_images(&altered);
            assert!(omitted.native_image_clip("radius").is_none(), "{altered}");
        }
    }
}

#[cfg(test)]
mod legend_shadow_qualification_contract {
    use super::*;
    #[test]
    fn legend_shadow_requires_native_owner_exact_filter_and_exact_rect() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="342" height="282" viewBox="0 0 342 282" preserveAspectRatio="xMinYMin meet"><defs><filter id="ls" x="-100%" y="-100%" width="300%" height="300%" color-interpolation-filters="sRGB"><feDropShadow dx="0" dy="2" stdDeviation="3" flood-color="#000000" flood-opacity="0.10196078431372549"/></filter></defs><svg id="d2-rs-native-canvas-viewport-control" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="342" height="282" viewBox="-101 -101 342 282"><g id="d2-rs-native-legend-shadow-card" data-d2-rs-legend-shadow="scene" filter="url(#ls)"><rect id="d2-rs-native-legend-shadow-rect" x="0" y="0" width="140" height="80" rx="4" fill="#F7F7FA" stroke="#DEE1EB" stroke-width="1"/></g><rect id="d2-rs-native-legend-panel" x="0" y="0" width="140" height="80" rx="4" fill="#ffffff" stroke="#DEE1EB" stroke-width="1"/></svg></svg>"##;
        let id = "d2-rs-native-legend-shadow-card";
        let (_, source) = tag_embedded_svg_images(svg);
        assert!(source.is_native_legend_shadow(id));
        for changed in [
            svg.replace("stdDeviation=\"3\"", "stdDeviation=\"4\""),
            svg.replace("dy=\"2\"", "dy=\"3\""),
            svg.replace("#000000", "#111111"),
            svg.replace("data-d2-rs-canvas-reference=\"ordinary\"", ""),
            svg.replace("data-d2-rs-legend-shadow=\"scene\"", ""),
            svg.replace("#F7F7FA", "#ffffff"),
            svg.replace("filter=\"url(#ls)\"", "opacity=\"0.5\" filter=\"url(#ls)\""),
            svg.replace("</defs>", "<filter id=\"ls\"/></defs>"),
        ] {
            assert!(
                !tag_embedded_svg_images(&changed)
                    .1
                    .is_native_legend_shadow(id),
                "{changed}"
            );
        }
    }
}

#[cfg(test)]
mod legend_shape_shadow_qualification_contract {
    use super::*;
    #[test]
    fn exact_filter_and_owner_are_required_before_retaining_source_operands() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="128" height="96" viewBox="0 0 128 96" preserveAspectRatio="xMinYMin meet"><defs><filter id="ls" x="-100%" y="-100%" width="300%" height="300%" color-interpolation-filters="sRGB"><feDropShadow dx="3" dy="5" stdDeviation="1.7" flood-color="#3d4574" flood-opacity="0.4"/></filter></defs><svg id="d2-rs-native-canvas-viewport-control" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="128" height="96" viewBox="0 0 128 96"><g id="d2-rs-native-legend-shape-icon-0" transform="translate(17.25 13.125) scale(0.2)"><g id="d2-rs-native-legend-shape-shadow-0" data-d2-rs-legend-shape-shadow="scene" filter="url(#ls)"><rect width="120" height="80" fill="red"/></g></g></svg></svg>"##;
        let id = "d2-rs-native-legend-shape-shadow-0";
        assert_eq!(
            tag_embedded_svg_images(svg).1.native_legend_shadow(id),
            Some(NativeDropShadow {
                offset: [3.0, 5.0],
                sigma: [1.7, 1.7],
                color: [61, 69, 116, 102]
            })
        );
        for changed in [
            svg.replace("stdDeviation=\"1.7\"", "stdDeviation=\"1.8\""),
            svg.replace("dx=\"3\"", "dx=\"2\""),
            svg.replace("dy=\"5\"", "dy=\"4\""),
            svg.replace("#3d4574", "#000000"),
            svg.replace("flood-opacity=\"0.4\"", "flood-opacity=\"0.5\""),
            svg.replace("data-d2-rs-canvas-reference=\"ordinary\"", ""),
            svg.replace("data-d2-rs-legend-shape-shadow=\"scene\"", ""),
            svg.replace("filter=\"url(#ls)\"", "opacity=\"0.5\" filter=\"url(#ls)\""),
            svg.replace("d2-rs-native-legend-shape-icon-0\"", "authored\""),
            svg.replace("</defs>", "<filter id=\"ls\"/></defs>"),
        ] {
            assert!(
                tag_embedded_svg_images(&changed)
                    .1
                    .native_legend_shadow(id)
                    .is_none(),
                "{changed}"
            );
        }
    }
}

#[cfg(test)]
mod unavailable_scene_placement_contract {
    use super::*;

    #[test]
    fn unavailable_urls_retain_exact_scene_placement_with_or_without_authored_id() {
        for id in [" id='missing-image'", ""] {
            let svg = format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='192' height='1280' data-d2-rs-canvas-reference='ordinary'><image{id} x='20.4' y='1000.3' width='76.8' height='64.2' preserveAspectRatio='xMidYMid meet' href='https://missing.invalid/icon.svg'/></svg>"
            );
            let mut cache = d2_rs_math::svg_asset_cache::AssetCache::default();
            cache.resolve_unavailable("https://missing.invalid/icon.svg");
            let (tagged, sources) = tag_embedded_svg_images_with_assets(&svg, Some(cache));
            let document = roxmltree::Document::parse(&tagged).unwrap();
            let image = document
                .descendants()
                .find(|node| node.has_tag_name("image"))
                .unwrap();
            let id = image.attribute("id").expect("ordinary image identity");
            assert!(sources.native_image(id).is_some());
            let placement = sources
                .scene_image(id)
                .expect("unavailable image is a native vector, not an unplaced SVG fallback");
            assert_eq!(placement.image_box, [20.4, 1000.3, 76.8, 64.2]);
            assert_eq!(placement.matrix, [1., 0., 0., 1., 0., 0.]);
        }
    }

    #[test]
    fn unavailable_image_does_not_acquire_partial_unsupported_ancestry_transform() {
        let svg = "<svg xmlns='http://www.w3.org/2000/svg' width='192' height='1280' data-d2-rs-canvas-reference='ordinary'><g style='transform:translate(1px)'><image id='missing-image' x='20.4' y='1000.3' width='64' height='64' href='https://missing.invalid/icon.svg'/></g></svg>";
        let mut cache = d2_rs_math::svg_asset_cache::AssetCache::default();
        cache.resolve_unavailable("https://missing.invalid/icon.svg");
        let (_, sources) = tag_embedded_svg_images_with_assets(svg, Some(cache));
        assert!(sources.native_image("missing-image").is_some());
        assert!(sources.scene_image("missing-image").is_none());
    }
}
