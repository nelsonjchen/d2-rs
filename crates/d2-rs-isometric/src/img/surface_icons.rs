// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg surface_icons.go: bounded icon artwork printed on a face.
//! Contraction sites are marked `ARM64:`.

use sha2::{Digest, Sha256};
use std::rc::Rc;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use d2_rs_render::go_float::{go_fmadd, go_fmsub, gomath};
use d2_rs_scene::{Clip, Matrix, Path, PathCommand, go_max, go_min};

use super::geometry::LabelSurface;
use super::mesh::{Texture, Vec, nadd, nv};
use super::route_labels::caption_finite;
use crate::target::{SHAPE_CLASS, SHAPE_IMAGE, SHAPE_SQL_TABLE, Shape, Url};

const LABEL_PADDING: f64 = 5.0;
const DEFAULT_ICON_SIZE: f64 = 32.0;

const MAX_PIXELS: usize = 8 << 20;
const MAX_BYTES: usize = 16 << 20;
const MAX_DECODED: usize = 64 << 20;

#[derive(Clone)]
struct IconAsset {
    asset: d2_rs_scene::Asset,
    assets: BTreeMap<String, d2_rs_scene::Asset>,
}

pub struct SurfaceIconPainter {
    pub context: super::context::RenderContext,
    resolver: Arc<dyn d2_rs_scenebuild::ImageResolver>,
    data_only: bool,
    count: usize,
    dimension: usize,
    pub output_density: f64,
    pixels: usize,
    bytes: usize,
    decoded: usize,
    limits: d2_rs_scenebuild::SvgImportLimits,
    remaining: d2_rs_scenebuild::SvgImportBudget,
    sources: HashMap<[u8; 32], Rc<IconAsset>>,
    content: HashMap<[u8; 32], Rc<IconAsset>>,
    textures: HashMap<([u8; 32], usize, usize, u64), Rc<Texture>>,
    vectors: super::vector_surface::VectorCtx,
}
impl Default for SurfaceIconPainter {
    fn default() -> Self {
        Self::new(0, None, None).expect("valid default icon limits")
    }
}

impl SurfaceIconPainter {
    /// surface_icons.go:59–66: check caller cancellation before admission or
    /// resolver allocation. `new` remains the background-context convenience.
    pub fn new_with_context(
        context: super::context::RenderContext,
        count: i64,
        supplied: Option<&d2_rs_scenebuild::AssetOptions>,
        vectors: super::vector_surface::VectorCtx,
    ) -> Result<Self, String> {
        context.check_string()?;
        let mut painter = Self::new(count, supplied, vectors)?;
        painter.context = context;
        Ok(painter)
    }

    /// surface_document.go: nativeSurfaceAssets shares the icon painter's
    /// default resolver and clamps caller limits to the native render ceilings.
    pub(crate) fn surface_assets(
        supplied: Option<&d2_rs_scenebuild::AssetOptions>,
    ) -> Result<d2_rs_scenebuild::AssetOptions, String> {
        let painter = Self::new(1, supplied, None)?;
        Ok(d2_rs_scenebuild::AssetOptions {
            resolver_impl: Some(painter.resolver),
            svg_import_limits: painter.limits,
            svg_import_budget: painter.remaining,
            ..Default::default()
        })
    }
    pub fn new(
        count: i64,
        supplied: Option<&d2_rs_scenebuild::AssetOptions>,
        vectors: super::vector_surface::VectorCtx,
    ) -> Result<Self, String> {
        if !(0..=7000).contains(&count) {
            return Err("isometric surface icon count exceeds 7000".into());
        }
        let options = super::surface_document::surface_assets();
        let mut limits = options.svg_import_limits;
        let mut remaining = options.svg_import_budget;
        let limit = |v: &mut i64, s: i64| {
            if s > 0 {
                *v = (*v).min(s);
            }
        };
        if let Some(s) = supplied {
            let l = s.svg_import_limits;
            let b = s.svg_import_budget;
            limit(&mut limits.max_bytes, l.max_bytes);
            limit(&mut limits.max_depth, l.max_depth);
            limit(&mut limits.max_elements, l.max_elements);
            limit(&mut limits.max_attributes, l.max_attributes);
            limit(&mut limits.max_attribute_bytes, l.max_attribute_bytes);
            limit(&mut limits.max_path_commands, l.max_path_commands);
            limit(
                &mut limits.max_transform_functions,
                l.max_transform_functions,
            );
            limit(&mut limits.max_use_depth, l.max_use_depth);
            limit(&mut limits.max_resources, l.max_resources);
            limit(&mut remaining.max_source_bytes, b.max_source_bytes);
            limit(&mut remaining.max_elements, b.max_elements);
            limit(&mut remaining.max_attributes, b.max_attributes);
            limit(&mut remaining.max_attribute_bytes, b.max_attribute_bytes);
            limit(&mut remaining.max_path_commands, b.max_path_commands);
            limit(
                &mut remaining.max_transform_functions,
                b.max_transform_functions,
            );
            limit(
                &mut remaining.max_declared_resources,
                b.max_declared_resources,
            );
            limit(
                &mut remaining.max_expanded_use_instances,
                b.max_expanded_use_instances,
            );
        }
        let resolver = supplied.and_then(|a| a.resolver_impl.clone());
        let data_only = resolver.is_none();
        let resolver = match resolver {
            Some(r) => r,
            None => Arc::new(
                d2_rs_scenebuild::RasterImageResolver::new(
                    std::path::Path::new("."),
                    d2_rs_scenebuild::ImageAssetLimits {
                        max_fetched_bytes: 12 << 20,
                        max_encoded_bytes: 8 << 20,
                        max_decompressed_bytes: 8 << 20,
                        max_svg_bytes: limits.max_bytes,
                        max_decoded_width: 8192,
                        max_decoded_height: 8192,
                        max_decoded_pixels: 8 << 20,
                        max_assets: 7000,
                        max_cumulative_encoded_bytes: MAX_BYTES as i64,
                        max_cumulative_decoded_bytes: MAX_DECODED as i64,
                    },
                    // surface_icons.go:82: the default imageasset session
                    // retains its zero-value (denied) local-file policy even
                    // when a caller explicitly reuses that resolver.
                    Arc::new(d2_rs_scenebuild::ImageLocalFiles::Denied),
                )
                .map_err(|e| e.to_string())?,
            ),
        };
        let count = count.max(1) as usize;
        Ok(Self {
            context: Default::default(),
            resolver,
            data_only,
            count,
            dimension: 512.min(((MAX_PIXELS / count) as f64).sqrt() as usize),
            output_density: 0.0,
            pixels: 0,
            bytes: 0,
            decoded: 0,
            limits,
            remaining,
            sources: HashMap::new(),
            content: HashMap::new(),
            textures: HashMap::new(),
            vectors,
        })
    }
    pub fn texture(
        &mut self,
        source: &Url,
        width: f64,
        depth: f64,
        border_radius_world: f64,
    ) -> Result<Option<Rc<Texture>>, String> {
        self.context.check_string()?;
        use d2_rs_scene::*;
        if !caption_finite(&[width, depth]) || width <= 0.0 || depth <= 0.0 {
            return Err(
                "isometric surface icon requires a source and positive finite dimensions".into(),
            );
        }
        let raw = source.string();
        if raw.len() > 12 << 20 {
            return Err("isometric surface icon source exceeds byte limit".into());
        }
        if self.data_only && !source.scheme.eq_ignore_ascii_case("data") {
            return Err(
                "isometric surface icon requires an explicit asset resolver for external sources"
                    .into(),
            );
        }
        let id: [u8; 32] = Sha256::digest(raw.as_bytes()).into();
        let (mut w, mut h) = surface_icon_dimensions(width, depth, self.dimension);
        if self.output_density > 0.0 && self.output_density.is_finite() {
            (w, h) = super::text::surface_texture_dimensions_at_density(
                width,
                depth,
                4096,
                MAX_PIXELS / self.count,
                self.output_density,
            );
        }
        if !border_radius_world.is_finite() || border_radius_world < 0.0 {
            return Err("isometric surface icon has invalid border radius".into());
        }
        let radius = go_min(border_radius_world, go_min(width, depth) / 2.0)
            * go_min(w as f64 / width, h as f64 / depth);
        let key = (id, w, h, radius.to_bits());
        if let Some(t) = self.textures.get(&key) {
            return Ok(Some(t.clone()));
        }
        let pixels = w * h;
        if self.textures.len() >= self.count || self.pixels + pixels > MAX_PIXELS {
            return Err("isometric surface icon textures exceed the render budget".into());
        }
        let asset = self.resolve(id, &raw)?;
        let view = Box {
            width: w as f64,
            height: h as f64,
            ..Default::default()
        };
        let mut node = Node::new(Some(Primitive::Image(Image {
            asset: "surface-icon".into(),
            r#box: view,
            aspect: AspectRatio {
                align: AspectAlign::X_MID_Y_MID,
                fit: AspectFit::MEET,
            },
        })));
        if radius > 0.0 {
            node.clip = Some(rounded_surface_icon_clip(w as f64, h as f64, radius));
        }
        let mut doc = Document::new(view, node);
        doc.assets = asset.assets.clone();
        doc.assets
            .insert("surface-icon".into(), asset.asset.clone());
        let tex = super::backends::render(
            &doc,
            &d2_rs_raster::FrameOptions {
                scale: 1.0,
                max_width: w as i64,
                max_height: h as i64,
                max_pixels: pixels as i64,
                max_nodes: 100000,
                max_depth: 128,
                max_path_commands: 500000,
                max_animation_tracks: 1,
                max_animation_keyframes: 1,
                max_assets: 4097,
                max_asset_bytes: MAX_BYTES as i64,
                max_decoded_asset_bytes: MAX_DECODED as i64,
                max_import_depth: 4,
                max_offscreen_bytes: 32 << 20,
                max_even_odd_clip_work: 32 << 20,
                max_scanline_work: 100000000,
                ..Default::default()
            },
        )
        .map_err(|e| format!("isometric surface icon rasterization: {e}"))?;
        self.context.check_string()?;
        let tex = Rc::new(tex);
        self.pixels += pixels;
        self.textures.insert(key, tex.clone());
        super::vector_surface::retain_native_vector_surface(&self.vectors, &tex, &doc)?;
        Ok(Some(tex))
    }
    fn resolve(&mut self, id: [u8; 32], source: &str) -> Result<Rc<IconAsset>, String> {
        if let Some(a) = self.sources.get(&id) {
            return Ok(a.clone());
        }
        if self.sources.len() >= self.count {
            return Err("isometric surface icon sources exceed the render budget".into());
        }
        let resource = self
            .resolver
            .resolve(source)
            .map_err(|e| format!("isometric surface icon: {e}"))?
            .ok_or("isometric surface icon resolver returned no resource")?;
        let encoded = resource.data.len();
        let decoded = resource.decoded_bytes;
        if encoded > 8 << 20
            || decoded > 32 << 20
            || self.bytes + encoded > MAX_BYTES
            || self.decoded as i64 + decoded > MAX_DECODED as i64
        {
            return Err("isometric surface icon assets exceed byte budget".into());
        }
        self.context.check_string()?;
        let digest: [u8; 32] = Sha256::digest(&resource.data).into();
        if let Some(a) = self.content.get(&digest) {
            self.sources.insert(id, a.clone());
            return Ok(a.clone());
        }
        let (asset, extra_bytes, extra_decoded) = match resource.kind {
            d2_rs_scenebuild::ImageKind::Raster => (
                IconAsset {
                    asset: d2_rs_scene::Asset::RasterAsset {
                        mime_type: resource.mime_type,
                        data: resource.data.to_vec(),
                        pixel_width: resource.pixel_width,
                        pixel_height: resource.pixel_height,
                        decoded_bytes: decoded,
                    },
                    assets: BTreeMap::new(),
                },
                0,
                0,
            ),
            d2_rs_scenebuild::ImageKind::Svg => {
                let imported =
                    super::backends::import_icon_svg(&resource.data, self.import_limits()?)
                        .map_err(|e| format!("isometric surface icon SVG: {e}"))?;
                let m = &imported.metrics;
                let (extra, decode) = (m.embedded_raster_bytes, m.decoded_raster_bytes);
                if self.bytes + encoded + extra > MAX_BYTES
                    || self.decoded as i64 + decoded + decode as i64 > MAX_DECODED as i64
                {
                    return Err("isometric surface icon embedded assets exceed byte budget".into());
                }
                let b = &mut self.remaining;
                b.max_source_bytes -= m.source_bytes as i64;
                b.max_elements -= m.parsed_elements.max(m.emitted_elements) as i64;
                b.max_attributes -= m.parsed_attributes as i64;
                b.max_attribute_bytes -= m.parsed_attribute_bytes as i64;
                b.max_path_commands -= m.parsed_path_commands.max(m.emitted_path_commands) as i64;
                b.max_transform_functions -= m.parsed_transform_funcs as i64;
                b.max_declared_resources -= m.declared_resources as i64;
                b.max_expanded_use_instances -= m.expanded_use_instances as i64;
                let (asset, assets) = super::backends::imported_icon_asset(imported);
                (IconAsset { asset, assets }, extra, decode)
            }
            _ => return Err("isometric surface icon has unsupported resource kind".into()),
        };
        self.bytes += encoded + extra_bytes;
        self.decoded += decoded as usize + extra_decoded;
        let asset = Rc::new(asset);
        self.sources.insert(id, asset.clone());
        self.content.insert(digest, asset.clone());
        Ok(asset)
    }
    fn import_limits(&self) -> Result<[usize; 9], String> {
        let l = self.limits;
        let b = self.remaining;
        let values = [
            l.max_bytes.min(b.max_source_bytes),
            l.max_depth,
            l.max_elements.min(b.max_elements),
            l.max_attributes.min(b.max_attributes),
            l.max_attribute_bytes.min(b.max_attribute_bytes),
            l.max_path_commands.min(b.max_path_commands),
            l.max_transform_functions.min(b.max_transform_functions),
            l.max_use_depth,
            l.max_resources
                .min(b.max_declared_resources.min(b.max_expanded_use_instances)),
        ];
        if values.iter().any(|&n| n <= 0) {
            return Err("isometric surface icons exceed the SVG import budget".into());
        }
        Ok(values.map(|v| v as usize))
    }
}

pub fn surface_icon_dimensions(width: f64, depth: f64, maximum: usize) -> (usize, usize) {
    let long = go_min(maximum as f64, go_max(1.0, go_max(width, depth) * 160.0));
    (
        (long * (width / go_max(width, depth))).round().max(1.0) as usize,
        (long * (depth / go_max(width, depth))).round().max(1.0) as usize,
    )
}

/// surfaceIconLayout partitions an existing printable face without changing
/// the physical shape footprint.
pub fn surface_icon_layout(
    s: &LabelSurface,
    original: &Shape,
    scale: f64,
    kind: &str,
) -> (LabelSurface, LabelSurface) {
    let mut text = s.clone();
    if original.icon.is_none()
        || !caption_finite(&[
            s.width, s.depth, s.angle, s.center.x, s.center.y, s.center.z,
        ])
        || s.width <= 0.0
        || s.depth <= 0.0
    {
        return (LabelSurface::default(), text);
    }
    let scale = if scale <= 0.0 || !caption_finite(&[scale]) {
        0.01
    } else {
        scale
    };
    let (columns, fields, methods) = (
        original.columns.as_ref().map_or(0, |v| v.len()),
        original.fields.as_ref().map_or(0, |v| v.len()),
        original.methods.as_ref().map_or(0, |v| v.len()),
    );
    let structured = original.type_ == SHAPE_SQL_TABLE
        || original.type_ == SHAPE_CLASS
        || columns + fields + methods > 0;
    if original.text.label.is_empty() && !structured {
        return (s.clone(), LabelSurface::default());
    }
    // ARM64 surface_icons.go:335/368.
    let offset = |x: f64, z: f64| -> Vec {
        let (cos, sin) = (gomath::cos(s.angle), gomath::sin(s.angle));
        nadd(
            s.center,
            nv(go_fmsub(sin, z, cos * x), 0.0, go_fmadd(cos, z, sin * x)),
        )
    };
    if structured {
        // A table's icon uses the existing header.
        let mut header = s.depth / (1 + columns) as f64;
        if original.type_ == SHAPE_CLASS {
            // ARM64 surface_icons.go:323.
            header = go_max(
                2.0 * s.depth / (2 + fields + methods) as f64,
                go_fmadd(
                    2.0 * LABEL_PADDING,
                    scale,
                    original.text.label_height as f64 * scale,
                ),
            );
        }
        let pad = go_min(LABEL_PADDING * scale, go_min(s.width, header) * 0.1);
        let size = go_max(
            0.0,
            go_min(
                DEFAULT_ICON_SIZE * scale,
                go_min(s.width - 2.0 * pad, header - 2.0 * pad),
            ),
        );
        let mut icon = s.clone();
        icon.width = size;
        icon.depth = size;
        let mut x = (s.width - size) / 2.0 - pad;
        if original.icon_position.contains("LEFT") {
            x = -x;
        }
        let z = -s.depth / 2.0 + header / 2.0;
        icon.center = offset(x, z);
        return (icon, s.clone());
    }
    let mut position = original.icon_position.as_str();
    let mut horizontal = position.contains("LEFT")
        || position.contains("RIGHT")
        || kind == "board"
        || kind == "edge";
    if position.is_empty() || position.contains("MIDDLE_CENTER") {
        // ARM64 surface_icons.go:343–344.
        let free_x = go_fmsub(original.text.label_width as f64, scale, s.width);
        let free_z = go_fmsub(original.text.label_height as f64, scale, s.depth);
        horizontal = free_x >= free_z || kind == "board" || kind == "edge";
    }
    if original.type_ == SHAPE_IMAGE {
        horizontal = false;
        position = "INSIDE_TOP_CENTER";
    }
    let mut icon = s.clone();
    let gap = go_min(0.08, go_min(s.width, s.depth) * 0.08);
    // ARM64 surface_icons.go:361–362.
    let required_width = go_fmadd(
        original.text.label_width as f64 * scale,
        1.06,
        go_min(0.04, s.width * 0.04),
    );
    let required_depth = go_fmadd(
        original.text.label_height as f64 * scale,
        1.06,
        go_min(0.04, s.depth * 0.04),
    );
    let available_width = s.width - required_width - gap;
    if horizontal && available_width < go_min(s.depth * 0.22, s.width * 0.12) {
        horizontal = false;
    }
    if horizontal {
        let mut size = go_min(s.depth, go_min(s.width * 0.48, available_width));
        if kind == "board" || kind == "edge" {
            size = go_min(size, s.width * 0.28);
        }
        icon.width = size;
        icon.depth = go_min(size, s.depth);
        text.width = s.width - size - gap;
        let direction = if position.contains("RIGHT") {
            -1.0
        } else {
            1.0
        };
        icon.center = offset(-direction * (s.width - size) / 2.0, 0.0);
        if position.contains("TOP") {
            icon.center = offset(
                -direction * (s.width - size) / 2.0,
                -(s.depth - icon.depth) / 2.0,
            );
        } else if position.contains("BOTTOM") {
            icon.center = offset(
                -direction * (s.width - size) / 2.0,
                (s.depth - icon.depth) / 2.0,
            );
        }
        text.center = offset(direction * (size + gap) / 2.0, 0.0);
    } else {
        let size = go_min(
            go_min(
                s.width,
                go_max(s.depth * 0.14, s.depth - required_depth - gap),
            ),
            s.depth * 0.72,
        );
        icon.width = go_min(size, s.width);
        icon.depth = size;
        if original.type_ == SHAPE_IMAGE {
            // A wide image gets the whole image panel, not a square badge.
            icon.width = s.width;
        }
        text.depth = s.depth - size - gap;
        let direction = if position.contains("BOTTOM") {
            -1.0
        } else {
            1.0
        };
        icon.center = offset(0.0, -direction * (s.depth - size) / 2.0);
        text.center = offset(0.0, direction * (size + gap) / 2.0);
    }
    (icon, text)
}

/// Vector clipping keeps transparent image corners premultiplied correctly.
pub fn rounded_surface_icon_clip(w: f64, h: f64, r: f64) -> Clip {
    let r = go_min(r, go_min(w, h) / 2.0);
    const KAPPA: f64 = 0.5522847498307936;
    // ARM64 surface_icons.go:416–418: every `±k` (k = r*KAPPA) sum fuses.
    let plus = |a: f64| go_fmadd(r, KAPPA, a);
    let minus = go_fmsub(r, KAPPA, r);
    Clip {
        transform: Matrix::identity(),
        path: Path {
            commands: vec![
                PathCommand::move_to(r, 0.0),
                PathCommand::line_to(w - r, 0.0),
                PathCommand::cubic_to(plus(w - r), 0.0, w, minus, w, r),
                PathCommand::line_to(w, h - r),
                PathCommand::cubic_to(w, plus(h - r), plus(w - r), h, w - r, h),
                PathCommand::line_to(r, h),
                PathCommand::cubic_to(minus, h, 0.0, plus(h - r), 0.0, h - r),
                PathCommand::line_to(0.0, r),
                PathCommand::cubic_to(0.0, minus, minus, 0.0, r, 0.0),
                PathCommand::close(),
            ],
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_rs_scenebuild::{
        AssetOptions, ImageKind, ImageResolveError, ImageResolver, ImageResource,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[derive(Debug, Default)]
    struct Icons(AtomicUsize);
    impl ImageResolver for Icons {
        fn resolve(&self, _: &str) -> Result<Option<ImageResource>, ImageResolveError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(Some(ImageResource { kind: ImageKind::Svg, mime_type: "image/svg+xml".into(), data: Arc::from(&b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"8\" height=\"8\"><rect width=\"8\" height=\"8\" fill=\"red\"/></svg>"[..]), pixel_width: 0, pixel_height: 0, decoded_bytes: 0 }))
        }
    }
    fn icon(path: &str) -> Url {
        Url {
            scheme: "https".into(),
            host: "example.test".into(),
            path: path.into(),
            ..Default::default()
        }
    }
    #[test]
    fn icon_assets_and_textures_reuse_identity_with_bounded_storage() {
        let resolver = Arc::new(Icons::default());
        let vectors = super::super::vector_surface::native_vector_context();
        let mut painter = SurfaceIconPainter::new(
            2,
            Some(&AssetOptions {
                resolver_impl: Some(resolver.clone()),
                ..Default::default()
            }),
            vectors.clone(),
        )
        .unwrap();
        let first = painter
            .texture(&icon("/a"), 0.1, 0.1, 0.02)
            .unwrap()
            .unwrap();
        assert!(Rc::ptr_eq(
            &first,
            &painter
                .texture(&icon("/a"), 0.1, 0.1, 0.02)
                .unwrap()
                .unwrap()
        ));
        let second = painter
            .texture(&icon("/b"), 0.2, 0.1, 0.0)
            .unwrap()
            .unwrap();
        assert!(!Rc::ptr_eq(&first, &second));
        assert_eq!(resolver.0.load(Ordering::SeqCst), 2);
        assert_eq!(painter.content.len(), 1);
        assert_eq!(painter.sources.len(), 2);
        let surface =
            super::super::vector_surface::native_vector_for_texture(&vectors, &first).unwrap();
        assert!(
            surface
                .document
                .borrow()
                .root
                .as_ref()
                .unwrap()
                .clip
                .is_some()
        );
        assert!(
            first
                .pixels
                .borrow()
                .chunks_exact(4)
                .any(|p| p[0] > 0 && p[3] > 0)
        );
        assert_eq!(
            painter.texture(&icon("/a"), 0.3, 0.1, 0.0).unwrap_err(),
            "isometric surface icon textures exceed the render budget"
        );
        assert_eq!(resolver.0.load(Ordering::SeqCst), 2);
    }
    #[test]
    fn default_external_sources_and_invalid_render_limits_fail_explicitly() {
        assert!(SurfaceIconPainter::new(7001, None, None).is_err());
        assert!(SurfaceIconPainter::new(-1, None, None).is_err());
        let mut painter = SurfaceIconPainter::default();
        assert_eq!(
            painter.texture(&icon("/a"), 1.0, 1.0, 0.0).unwrap_err(),
            "isometric surface icon requires an explicit asset resolver for external sources"
        );
        assert_eq!(
            painter
                .texture(&icon("/a"), f64::NAN, 1.0, 0.0)
                .unwrap_err(),
            "isometric surface icon requires a source and positive finite dimensions"
        );
        assert_eq!(painter.pixels, 0);
        assert!(painter.sources.is_empty());
    }

    #[test]
    fn cancelled_constructor_precedes_count_admission() {
        let token = d2_rs_core::CancellationToken::new();
        let context = super::super::context::RenderContext::from_token(&token);
        token.cancel();
        for count in [1, -1, 7001] {
            let error = SurfaceIconPainter::new_with_context(context.clone(), count, None, None)
                .err()
                .expect("cancelled constructor must fail");
            assert_eq!(error, "context canceled");
        }
    }
}

#[cfg(test)]
#[path = "surface_icons_api_tests.rs"]
pub(crate) mod api_tests;

#[cfg(test)]
#[path = "artwork_api_tests.rs"]
mod artwork_api_tests;
