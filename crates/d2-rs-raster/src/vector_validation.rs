// SPDX-License-Identifier: MPL-2.0
//! d2raster/vector_validation.go: placement-independent structural
//! validation of retained vector definitions, exact-rational rank tests and
//! the finite affine inverse.

use std::collections::{HashMap, HashSet};

use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{ToPrimitive, Zero};

use d2_rs_scene::{aspect_ratio_matrix, AnimationProperty, Box as SceneBox, FillRule, Image, LineCap, LineJoin, MaskType, Matrix, Node, Paint, PaintUnits, Path, PathCommandKind, PatternPaint, Primitive, Stroke, TextAnchor, TextRun};

use crate::effects::supported_blend_mode;
use crate::fma::{finite, finite_point, go_fmadd, go_fmsub};
use crate::raster::{validate_box, AnimationOverrides, Preflight};

#[derive(Clone, Copy, Debug, Default)]
struct Summary {
    import_height: i64,
    node_height: i64,
}

struct Validator {
    validated: HashMap<String, Summary>,
    active: HashSet<String>,
    nodes: HashSet<usize>,
    ancestors_invertible: bool,
}

impl<'d> Preflight<'d> {
    pub(crate) fn validate_retained_vector_assets(&mut self, ids: &[String]) -> Result<(), String> {
        if self.vectors.is_empty() {
            return Ok(());
        }
        let mut validator = Validator { validated: HashMap::new(), active: HashSet::new(), nodes: HashSet::new(), ancestors_invertible: true };
        for id in ids {
            if !self.vectors.contains_key(id) {
                continue;
            }
            self.retained_asset(&mut validator, id, 1, 2)?;
        }
        Ok(())
    }

    fn retained_asset(&mut self, v: &mut Validator, id: &str, import_depth: i64, node_depth: i64) -> Result<Summary, String> {
        if import_depth > self.options.max_import_depth {
            return Err(format!("d2raster: vector asset {id:?} import depth {import_depth} exceeds limit {}", self.options.max_import_depth));
        }
        if v.active.contains(id) {
            return Err(format!("d2raster: cyclic vector asset reference at {id:?}"));
        }
        if let Some(summary) = v.validated.get(id).copied() {
            self.check_summary_depths(id, summary, import_depth, node_depth)?;
            return Ok(summary);
        }
        let Some((_, root)) = self.vectors.get(id).copied() else {
            return Err(format!("d2raster: missing vector asset {id:?}"));
        };
        v.active.insert(id.to_owned());
        let previous = v.ancestors_invertible;
        v.ancestors_invertible = true;
        let nested = self.retained_node(v, root, 1, import_depth);
        v.ancestors_invertible = previous;
        v.active.remove(id);
        let nested = nested.map_err(|e| format!("d2raster: vector asset {id:?} retained validation: {e}"))?;
        let mut summary = Summary { import_height: 1, node_height: nested.node_height };
        if nested.import_height != 0 {
            summary.import_height += nested.import_height;
        }
        v.validated.insert(id.to_owned(), summary);
        self.check_summary_depths(id, summary, import_depth, node_depth)?;
        Ok(summary)
    }

    fn check_summary_depths(&self, id: &str, summary: Summary, import_depth: i64, node_depth: i64) -> Result<(), String> {
        let deepest_import = import_depth + summary.import_height - 1;
        if deepest_import > self.options.max_import_depth {
            return Err(format!("d2raster: vector asset {id:?} import depth {deepest_import} exceeds limit {}", self.options.max_import_depth));
        }
        let deepest_node = node_depth + summary.node_height - 1;
        if deepest_node > self.options.max_depth {
            return Err(format!("d2raster: vector asset {id:?} composed node depth {deepest_node} exceeds limit {}", self.options.max_depth));
        }
        Ok(())
    }

    fn retained_node(&mut self, v: &mut Validator, node: &'d Node, depth: i64, import_depth: i64) -> Result<Summary, String> {
        if depth > self.options.max_depth {
            return Err(format!("d2raster: node depth {depth} exceeds limit {}", self.options.max_depth));
        }
        let key = node as *const Node as usize;
        if v.nodes.contains(&key) {
            return Err(format!("d2raster: node cycle at {:?}", node.id));
        }
        self.nodes += 1;
        if self.nodes > self.options.max_nodes {
            return Err(format!("d2raster: node count exceeds limit {}", self.options.max_nodes));
        }
        if !node.transform.is_finite() || !finite(node.opacity) || node.opacity < 0.0 || node.opacity > 1.0 {
            return Err(format!("d2raster: node {:?} has invalid transform or opacity", node.id));
        }
        if !supported_blend_mode(node.blend) {
            return Err(format!("d2raster: node {:?} uses invalid or unsupported blend mode {}", node.id, node.blend.0));
        }
        let filters = node.filters.as_deref().unwrap_or_default();
        self.charge_filter_work(&node.id, filters.len())?;
        let (overrides, node_transform) = self.retained_animations(node)?;
        crate::filter::normalize_node_filters(&node.id, filters, &overrides.drop_shadows)?;
        let previous = v.ancestors_invertible;
        v.ancestors_invertible = previous && finite_linear_transform_invertible(&node_transform);
        v.nodes.insert(key);
        let result = self.retained_node_body(v, node, &overrides, depth, import_depth);
        v.nodes.remove(&key);
        v.ancestors_invertible = previous;
        result
    }

    fn retained_node_body(&mut self, v: &mut Validator, node: &'d Node, overrides: &AnimationOverrides, depth: i64, import_depth: i64) -> Result<Summary, String> {
        let (primitive_summary, primitive) = self.retained_primitive(v, &node.id, node.primitive.as_ref(), overrides, depth, import_depth)?;
        let mut summary = Summary { import_height: primitive_summary.import_height, node_height: 1 };
        if primitive_summary.node_height != 0 {
            summary.node_height = 1 + primitive_summary.node_height;
        }
        if !primitive && (overrides.fill_color.is_some() || overrides.stroke_color.is_some() || overrides.dash_offset.is_some()) {
            return Err(format!("d2raster: node {:?} has a paint or stroke animation but no primitive", node.id));
        }
        if let Some(clip) = &node.clip {
            if !clip.transform.is_finite() {
                return Err(format!("d2raster: node {:?} has invalid clip transform", node.id));
            }
            if clip.path.fill_rule.0 > FillRule::EVEN_ODD.0 {
                return Err(format!("d2raster: node {:?} clip has invalid fill rule {}", node.id, clip.path.fill_rule.0));
            }
            self.retained_path(&clip.path).map_err(|e| format!("d2raster: node {:?} clip: {e}", node.id))?;
        }
        if let Some(mask) = &node.mask {
            let Some(root) = mask.root.as_deref().filter(|_| mask.transform.is_finite() && mask.r#type.0 <= MaskType::LUMINANCE.0) else {
                return Err(format!("d2raster: node {:?} has invalid mask", node.id));
            };
            let before = v.ancestors_invertible;
            v.ancestors_invertible = before && finite_linear_transform_invertible(&mask.transform);
            let child = self.retained_node(v, root, depth + 1, import_depth);
            v.ancestors_invertible = before;
            let child = child.map_err(|e| format!("d2raster: node {:?} mask: {e}", node.id))?;
            summary = merge_node_summary(summary, child);
        }
        for child in node.children() {
            let child_summary = self.retained_node(v, child, depth + 1, import_depth)?;
            summary = merge_node_summary(summary, child_summary);
        }
        Ok(summary)
    }

    fn retained_animations(&mut self, node: &Node) -> Result<(AnimationOverrides, Matrix), String> {
        let tracks = node.animations.as_deref().unwrap_or_default();
        self.charge_animation_work(&node.id, tracks)?;
        let mut overrides = AnimationOverrides::default();
        let mut node_transform = node.transform;
        let mut seen = HashSet::new();
        for (track_index, track) in tracks.iter().enumerate() {
            if !seen.insert((track.property.0, track.target_index)) {
                return Err(format!(
                    "d2raster: node {:?} has duplicate animation target property {} index {}",
                    node.id, track.property.0, track.target_index
                ));
            }
            let value = crate::animation::animation_value_at(track, self.options.time)
                .map_err(|e| format!("d2raster: node {:?} animation {track_index}: {e}", node.id))?;
            if track.property != AnimationProperty::DROP_SHADOW && track.target_index != 0 {
                return Err(format!(
                    "d2raster: node {:?} animation {track_index} uses non-zero target index for scalar property {}",
                    node.id, track.property.0
                ));
            }
            match track.property {
                AnimationProperty::OPACITY => {
                    if value.number < 0.0 || value.number > 1.0 {
                        return Err(format!("d2raster: node {:?} animation {track_index} resolves opacity outside [0,1]", node.id));
                    }
                }
                AnimationProperty::TRANSFORM => node_transform = value.transform,
                AnimationProperty::STROKE_DASH_OFFSET => overrides.dash_offset = Some(value.number),
                AnimationProperty::FILL_COLOR => overrides.fill_color = Some(value.color),
                AnimationProperty::STROKE_COLOR => overrides.stroke_color = Some(value.color),
                AnimationProperty::DROP_SHADOW => {
                    overrides.drop_shadows.insert(track.target_index, value.shadow);
                }
                property => return Err(format!("d2raster: node {:?} animation {track_index} uses unknown property {}", node.id, property.0)),
            }
        }
        Ok((overrides, node_transform))
    }

    fn retained_primitive(&mut self, v: &mut Validator, node_id: &str, primitive: Option<&'d Primitive>, animation: &AnimationOverrides, depth: i64, import_depth: i64) -> Result<(Summary, bool), String> {
        let animated_fill = animation.fill_color.is_some();
        let animated_stroke = animation.stroke_color.is_some();
        let animated_dash = animation.dash_offset.is_some();
        match primitive {
            None => Ok((Summary::default(), false)),
            Some(Primitive::Rect(rect)) => {
                validate_box(rect.r#box).map_err(|e| format!("d2raster: node {node_id:?} rectangle: {e}"))?;
                if !finite(rect.radius_x) || !finite(rect.radius_y) || rect.radius_x < 0.0 || rect.radius_y < 0.0 {
                    return Err(format!("d2raster: node {node_id:?} rectangle has invalid corner radius"));
                }
                let object_bounds = Some(rect.r#box);
                let fill = self.retained_paint(v, rect.fill.as_ref(), animated_fill, object_bounds, import_depth).map_err(|e| format!("d2raster: node {node_id:?} rectangle fill: {e}"))?.0;
                let stroke = self.retained_stroke(v, rect.stroke.as_ref(), animated_stroke, animated_dash, object_bounds, import_depth).map_err(|e| format!("d2raster: node {node_id:?} rectangle stroke: {e}"))?;
                Ok((merge_primitive_summary(fill, stroke), true))
            }
            Some(Primitive::Ellipse(ellipse)) => {
                if !finite_point(ellipse.center) || !finite(ellipse.radius_x) || !finite(ellipse.radius_y) || ellipse.radius_x < 0.0 || ellipse.radius_y < 0.0 {
                    return Err(format!("d2raster: node {node_id:?} ellipse has invalid geometry"));
                }
                let object_bounds = Some(SceneBox {
                    x: ellipse.center.x - ellipse.radius_x,
                    y: ellipse.center.y - ellipse.radius_y,
                    width: 2.0 * ellipse.radius_x,
                    height: 2.0 * ellipse.radius_y,
                });
                let fill = self.retained_paint(v, ellipse.fill.as_ref(), animated_fill, object_bounds, import_depth).map_err(|e| format!("d2raster: node {node_id:?} ellipse fill: {e}"))?.0;
                let stroke = self.retained_stroke(v, ellipse.stroke.as_ref(), animated_stroke, animated_dash, object_bounds, import_depth).map_err(|e| format!("d2raster: node {node_id:?} ellipse stroke: {e}"))?;
                Ok((merge_primitive_summary(fill, stroke), true))
            }
            Some(Primitive::Path(path)) => {
                if path.fill_rule.0 > FillRule::EVEN_ODD.0 {
                    return Err(format!("d2raster: node {node_id:?} path has invalid fill rule {}", path.fill_rule.0));
                }
                self.retained_path(path).map_err(|e| format!("d2raster: node {node_id:?} path: {e}"))?;
                let mut object_bounds = None;
                if paint_uses_object_bounding_box(path.fill.as_ref()) || stroke_uses_object_bounding_box(path.stroke.as_ref()) {
                    let bounds = path.geometry_bounds().map_err(|e| format!("d2raster: node {node_id:?} path bounds: {e}"))?;
                    object_bounds = Some(bounds.to_box());
                }
                let fill = self.retained_paint(v, path.fill.as_ref(), animated_fill, object_bounds, import_depth).map_err(|e| format!("d2raster: node {node_id:?} path fill: {e}"))?.0;
                let stroke = self.retained_stroke(v, path.stroke.as_ref(), animated_stroke, animated_dash, object_bounds, import_depth).map_err(|e| format!("d2raster: node {node_id:?} path stroke: {e}"))?;
                Ok((merge_primitive_summary(fill, stroke), true))
            }
            Some(Primitive::TextRun(text)) => self.retained_text(v, node_id, text, animation, import_depth),
            Some(Primitive::Image(image)) => self.retained_image(v, node_id, image, animation, depth, import_depth),
        }
    }

    fn retained_text(&mut self, v: &mut Validator, node_id: &str, text: &'d TextRun, animation: &AnimationOverrides, import_depth: i64) -> Result<(Summary, bool), String> {
        let size = text.font.size();
        if !finite_point(text.origin) || !finite(size) || size <= 0.0 || size > i32::MAX as f64 / 64.0 {
            return Err(format!("d2raster: node {node_id:?} text has invalid origin or font size"));
        }
        if text.anchor.0 > TextAnchor::END.0 {
            return Err(format!("d2raster: node {node_id:?} text has invalid anchor {}", text.anchor.0));
        }
        if !text.ink.is_finite() {
            return Err(format!("d2raster: node {node_id:?} text has invalid ink bounds"));
        }
        if text.font.asset.is_empty() {
            return Err(format!("d2raster: node {node_id:?} text has empty font asset ID"));
        }
        if !self.fonts.contains_key(&text.font.asset) {
            if self.document.assets.contains_key(&text.font.asset) {
                return Err(format!("d2raster: node {node_id:?} text asset {:?} is not a usable font", text.font.asset));
            }
            return Err(format!("d2raster: node {node_id:?} text references missing font asset {:?}", text.font.asset));
        }
        let ppem = (size * 64.0).round() as i32;
        let (glyphs, advance) = self.position_glyphs(text, ppem).map_err(|e| format!("d2raster: node {node_id:?} text: {e}"))?;
        let mut object_bounds = None;
        if text.ink.valid {
            object_bounds = Some(text.ink.to_box());
        } else if paint_uses_object_bounding_box(text.fill.as_ref()) || stroke_uses_object_bounding_box(text.stroke.as_ref()) {
            let anchor_offset = match text.anchor {
                TextAnchor::MIDDLE => -advance / 2.0,
                TextAnchor::END => -advance,
                _ => 0.0,
            };
            let mut bounds = d2_rs_scene::Bounds::default();
            for (index, glyph) in glyphs.iter().enumerate() {
                if glyph.empty {
                    continue;
                }
                let glyph_bounds = glyph
                    .font
                    .glyph_render_bounds(glyph.id, ppem)
                    .map_err(|e| format!("d2raster: node {node_id:?} text bounds: load glyph {index} (ID {}) render bounds from font asset {:?}: {e}", glyph.id, glyph.asset))?;
                let origin_x = text.origin.x + anchor_offset + glyph.position.x;
                let origin_y = text.origin.y + glyph.position.y;
                // ARM64 vector_validation.go:464–467: the 26.6 conversions are
                // exact products, so their fusion is unobservable.
                bounds = bounds.union(d2_rs_scene::Bounds::new(
                    origin_x + glyph_bounds.0 as f64 / 64.0,
                    origin_y + glyph_bounds.1 as f64 / 64.0,
                    origin_x + glyph_bounds.2 as f64 / 64.0,
                    origin_y + glyph_bounds.3 as f64 / 64.0,
                ));
            }
            if (text.underline || text.strike) && advance > 0.0 {
                let left = text.origin.x + anchor_offset;
                bounds = bounds.union(d2_rs_scene::Bounds::new(left, text.origin.y, left + advance, text.origin.y + size / 16.0));
            }
            object_bounds = Some(bounds.to_box());
        }
        let fill = self
            .retained_paint(v, text.fill.as_ref(), animation.fill_color.is_some(), object_bounds, import_depth)
            .map_err(|e| format!("d2raster: node {node_id:?} text fill: {e}"))?
            .0;
        let stroke = self
            .retained_stroke(v, text.stroke.as_ref(), animation.stroke_color.is_some(), animation.dash_offset.is_some(), object_bounds, import_depth)
            .map_err(|e| format!("d2raster: node {node_id:?} text stroke: {e}"))?;
        Ok((merge_primitive_summary(fill, stroke), true))
    }

    fn retained_image(&mut self, v: &mut Validator, node_id: &str, image: &'d Image, animation: &AnimationOverrides, depth: i64, import_depth: i64) -> Result<(Summary, bool), String> {
        if animation.fill_color.is_some() || animation.stroke_color.is_some() || animation.dash_offset.is_some() {
            return Err(format!("d2raster: node {node_id:?} image cannot be targeted by paint or stroke animation"));
        }
        if image.asset.is_empty() {
            return Err(format!("d2raster: node {node_id:?} image has an empty asset ID"));
        }
        validate_box(image.r#box).map_err(|e| format!("d2raster: node {node_id:?} image: {e}"))?;
        if image.aspect.align.0 > d2_rs_scene::AspectAlign::X_MAX_Y_MAX.0 || image.aspect.fit.0 > d2_rs_scene::AspectFit::SLICE.0 {
            return Err(format!(
                "d2raster: node {node_id:?} image has invalid aspect-ratio policy align={} fit={}",
                image.aspect.align.0, image.aspect.fit.0
            ));
        }
        if self.rasters.contains_key(&image.asset) {
            return Ok((Summary::default(), true));
        }
        if let Some((view_box, _)) = self.vectors.get(&image.asset).copied() {
            if image.r#box.width != 0.0 && image.r#box.height != 0.0 && v.ancestors_invertible {
                let mapping = aspect_ratio_matrix(view_box, image.r#box, image.aspect)
                    .map_err(|e| format!("d2raster: node {node_id:?} vector asset {:?} aspect ratio: {e}", image.asset))?;
                if !finite_linear_transform_invertible(&mapping) {
                    return Err(format!(
                        "d2raster: node {node_id:?} vector asset {:?} aspect ratio: mapping is singular in the finite numeric domain",
                        image.asset
                    ));
                }
            }
            let summary = self.retained_asset(v, &image.asset, import_depth + 1, depth + 1)?;
            return Ok((summary, true));
        }
        if let Some(raw) = self.document.assets.get(&image.asset) {
            return Err(format!(
                "d2raster: node {node_id:?} image asset {:?} is not a raster or vector asset (got {})",
                image.asset,
                crate::raster::asset_go_type(raw)
            ));
        }
        Err(format!("d2raster: node {node_id:?} image references missing asset {:?}", image.asset))
    }

    fn retained_path(&mut self, path: &Path) -> Result<(), String> {
        if path.commands.len() as i64 > self.options.max_path_commands - self.path_segments {
            return Err(format!("path command count exceeds limit {}", self.options.max_path_commands));
        }
        self.path_segments += path.commands.len() as i64;
        let mut have_current = false;
        for (index, command) in path.commands.iter().enumerate() {
            let before = |name: &str| Err(format!("command {index}: {name} before move"));
            let non_finite = || Err(format!("command {index}: non-finite point"));
            match command.kind {
                PathCommandKind::MOVE => {
                    if !finite_point(command.p1) {
                        return non_finite();
                    }
                    have_current = true;
                }
                PathCommandKind::LINE => {
                    if !have_current {
                        return before("line");
                    }
                    if !finite_point(command.p1) {
                        return non_finite();
                    }
                }
                PathCommandKind::QUADRATIC => {
                    if !have_current {
                        return before("quadratic");
                    }
                    if !finite_point(command.p1) || !finite_point(command.p2) {
                        return non_finite();
                    }
                }
                PathCommandKind::CUBIC => {
                    if !have_current {
                        return before("cubic");
                    }
                    if !finite_point(command.p1) || !finite_point(command.p2) || !finite_point(command.p3) {
                        return non_finite();
                    }
                }
                PathCommandKind::ARC => {
                    if !have_current {
                        return before("arc");
                    }
                    if !finite_point(command.p1) || !finite(command.radius_x) || !finite(command.radius_y) || !finite(command.rotation) {
                        return Err(format!("command {index}: non-finite arc"));
                    }
                }
                PathCommandKind::CLOSE => {
                    if !have_current {
                        return before("close");
                    }
                }
                kind => return Err(format!("command {index}: unknown kind {}", kind.0)),
            }
        }
        Ok(())
    }

    fn retained_paint(&mut self, v: &mut Validator, paint: Option<&'d Paint>, animated_color: bool, object_bounds: Option<SceneBox>, import_depth: i64) -> Result<(Summary, bool), String> {
        match paint {
            None => {
                if animated_color {
                    return Err("color animation targets missing paint".into());
                }
                Ok((Summary::default(), false))
            }
            Some(Paint::SolidPaint { .. }) => Ok((Summary::default(), true)),
            Some(Paint::LinearGradient(gradient)) => {
                if animated_color {
                    return Err("color animation targets non-solid paint".into());
                }
                if !finite_point(gradient.start) || !finite_point(gradient.end) {
                    return Err("linear gradient has non-finite geometry".into());
                }
                gradient_common(v, gradient.units, &gradient.transform, &gradient.stops, gradient.spread, object_bounds).map_err(|e| format!("linear gradient: {e}"))?;
                let (dx, dy) = (gradient.end.x - gradient.start.x, gradient.end.y - gradient.start.y);
                // ARM64 vector_validation.go:689.
                if !finite(go_fmadd(dy, dy, dx * dx)) {
                    return Err("linear gradient vector is outside the finite numeric domain".into());
                }
                Ok((Summary::default(), true))
            }
            Some(Paint::RadialGradient(gradient)) => {
                if animated_color {
                    return Err("color animation targets non-solid paint".into());
                }
                if !finite_point(gradient.center) || !finite_point(gradient.focal) || !finite(gradient.radius) || !finite(gradient.focal_radius) {
                    return Err("radial gradient has non-finite geometry".into());
                }
                if gradient.radius < 0.0 || gradient.focal_radius < 0.0 {
                    return Err("radial gradient has negative radius".into());
                }
                gradient_common(v, gradient.units, &gradient.transform, &gradient.stops, gradient.spread, object_bounds).map_err(|e| format!("radial gradient: {e}"))?;
                if gradient.stops.len() == 1 {
                    return Ok((Summary::default(), true));
                }
                let (dx, dy) = (gradient.center.x - gradient.focal.x, gradient.center.y - gradient.focal.y);
                let dr = gradient.radius - gradient.focal_radius;
                let a = go_fmsub(dr, dr, go_fmadd(dy, dy, dx * dx));
                if !finite(a) || !finite(dr) {
                    return Err("radial gradient cone is outside the finite numeric domain".into());
                }
                Ok((Summary::default(), true))
            }
            Some(Paint::PatternPaint(pattern)) => {
                if animated_color {
                    return Err("color animation targets pattern paint".into());
                }
                let summary = self.retained_pattern(v, pattern, object_bounds, import_depth)?;
                Ok((summary, true))
            }
        }
    }

    fn retained_pattern(&mut self, v: &mut Validator, pattern: &'d PatternPaint, object_bounds: Option<SceneBox>, import_depth: i64) -> Result<Summary, String> {
        validate_box(pattern.tile).map_err(|e| format!("pattern tile: {e}"))?;
        if pattern.tile.width == 0.0 || pattern.tile.height == 0.0 {
            return Err("pattern tile has zero width or height".into());
        }
        if !finite(pattern.tile.x + pattern.tile.width) || !finite(pattern.tile.y + pattern.tile.height) {
            return Err("pattern tile endpoints are non-finite".into());
        }
        if pattern.units.0 > PaintUnits::USER_SPACE_ON_USE.0 {
            return Err(format!("invalid paint units {}", pattern.units.0));
        }
        if !pattern.transform.is_finite() {
            return Err("non-finite pattern transform".into());
        }
        if !finite_linear_transform_invertible(&pattern.transform) || !v.ancestors_invertible {
            return Err("singular pattern transform".into());
        }
        if pattern.units == PaintUnits::OBJECT_BOUNDING_BOX {
            if let Some(bounds) = object_bounds {
                validate_box(bounds).map_err(|e| format!("invalid object bounding box: {e}"))?;
                if bounds.width == 0.0 || bounds.height == 0.0 {
                    return Err("object bounding box has zero width or height".into());
                }
            }
        }
        let Some(root) = pattern.root.as_deref() else {
            return Err("pattern has no root node".into());
        };
        let previous = v.ancestors_invertible;
        v.ancestors_invertible = true;
        let summary = self.retained_node(v, root, 1, import_depth + 1);
        v.ancestors_invertible = previous;
        let mut summary = summary.map_err(|e| format!("pattern root: {e}"))?;
        summary.node_height = 0;
        if summary.import_height != 0 {
            summary.import_height += 1;
        }
        Ok(summary)
    }

    fn retained_stroke(&mut self, v: &mut Validator, stroke: Option<&'d Stroke>, animated_color: bool, animated_dash_offset: bool, object_bounds: Option<SceneBox>, import_depth: i64) -> Result<Summary, String> {
        let Some(stroke) = stroke else {
            if animated_color || animated_dash_offset {
                return Err("stroke animation targets missing stroke".into());
            }
            return Ok(Summary::default());
        };
        if !finite(stroke.width) || stroke.width < 0.0 || !finite(stroke.miter_limit) || !finite(stroke.dash_offset) {
            return Err("invalid stroke".into());
        }
        let (paint_summary, paint) = self.retained_paint(v, stroke.paint.as_ref(), animated_color, object_bounds, import_depth)?;
        let mut dash_total = 0.0;
        for dash in stroke.dashes.as_deref().unwrap_or_default() {
            if !finite(*dash) || *dash <= 0.0 {
                return Err("invalid stroke dash".into());
            }
            dash_total += *dash;
            if !finite(dash_total) {
                return Err("invalid stroke dash total".into());
            }
        }
        if stroke.width == 0.0 || !paint {
            return Ok(paint_summary);
        }
        if !matches!(stroke.cap, LineCap::BUTT | LineCap::ROUND | LineCap::SQUARE) {
            return Err(format!("unsupported line cap {}", stroke.cap.0));
        }
        if !matches!(stroke.join, LineJoin::MITER | LineJoin::ROUND | LineJoin::BEVEL) {
            return Err(format!("unsupported line join {}", stroke.join.0));
        }
        if stroke.join == LineJoin::MITER && stroke.miter_limit != 0.0 && stroke.miter_limit < 1.0 {
            return Err(format!("invalid miter limit {}", crate::raster::go_g(stroke.miter_limit)));
        }
        Ok(paint_summary)
    }
}

fn gradient_common(v: &Validator, units: PaintUnits, transform: &Matrix, stops: &[d2_rs_scene::GradientStop], spread: d2_rs_scene::SpreadMethod, object_bounds: Option<SceneBox>) -> Result<(), String> {
    if units.0 > PaintUnits::USER_SPACE_ON_USE.0 {
        return Err(format!("invalid paint units {}", units.0));
    }
    if !transform.is_finite() {
        return Err("non-finite gradient transform".into());
    }
    if !finite_linear_transform_invertible(transform) {
        return Err("singular gradient transform".into());
    }
    if !v.ancestors_invertible {
        return Err("singular gradient transform".into());
    }
    if units == PaintUnits::OBJECT_BOUNDING_BOX {
        if let Some(bounds) = object_bounds {
            validate_box(bounds).map_err(|e| format!("invalid object bounding box: {e}"))?;
            if bounds.width == 0.0 || bounds.height == 0.0 {
                return Err("object bounding box has zero width or height".into());
            }
        }
    }
    if stops.is_empty() {
        return Err("gradient has no stops".into());
    }
    for (index, stop) in stops.iter().enumerate() {
        if !finite(stop.offset) {
            return Err(format!("gradient stop {index} has non-finite offset"));
        }
    }
    if spread.0 > d2_rs_scene::SpreadMethod::REPEAT.0 {
        return Err(format!("invalid spread method {}", spread.0));
    }
    Ok(())
}

fn merge_node_summary(mut parent: Summary, child: Summary) -> Summary {
    parent.import_height = parent.import_height.max(child.import_height);
    parent.node_height = parent.node_height.max(1 + child.node_height);
    parent
}

fn merge_primitive_summary(left: Summary, right: Summary) -> Summary {
    Summary { import_height: left.import_height.max(right.import_height), node_height: left.node_height.max(right.node_height) }
}

fn paint_uses_object_bounding_box(paint: Option<&Paint>) -> bool {
    match paint {
        Some(Paint::LinearGradient(g)) => g.units == PaintUnits::OBJECT_BOUNDING_BOX,
        Some(Paint::RadialGradient(g)) => g.units == PaintUnits::OBJECT_BOUNDING_BOX,
        Some(Paint::PatternPaint(p)) => p.units == PaintUnits::OBJECT_BOUNDING_BOX,
        _ => false,
    }
}

fn stroke_uses_object_bounding_box(stroke: Option<&Stroke>) -> bool {
    stroke.is_some_and(|s| paint_uses_object_bounding_box(s.paint.as_ref()))
}

fn rational(value: f64) -> BigRational {
    BigRational::from_float(value).unwrap_or_else(BigRational::zero)
}

/// finiteLinearTransformInvertible: rank via an exact rational determinant.
pub fn finite_linear_transform_invertible(transform: &Matrix) -> bool {
    if !transform.is_finite() {
        return false;
    }
    if (transform.a == 0.0 && transform.c == 0.0) || (transform.b == 0.0 && transform.d == 0.0) {
        return false;
    }
    let left_float = transform.a * transform.d;
    let right_float = transform.b * transform.c;
    if finite(left_float) && finite(right_float) && left_float != right_float {
        return true;
    }
    rational(transform.a) * rational(transform.d) != rational(transform.b) * rational(transform.c)
}

/// finiteAffineInverse: Ok(None) for a singular transform.
pub fn finite_affine_inverse(transform: &Matrix) -> Result<Option<Matrix>, String> {
    if !transform.is_finite() {
        return Err("non-finite transform".into());
    }
    if let Ok(inverse) = transform.inverse() {
        if inverse.is_finite() {
            return Ok(Some(inverse));
        }
    }
    let (a, b, c, d, e, f) = (rational(transform.a), rational(transform.b), rational(transform.c), rational(transform.d), rational(transform.e), rational(transform.f));
    let determinant = &a * &d - &b * &c;
    if determinant.is_zero() {
        return Ok(None);
    }
    let quotient = |numerator: BigRational| -> Result<f64, String> {
        let value = numerator / &determinant;
        let result = rational_to_f64(&value);
        if !finite(result) {
            return Err("inverse is outside the finite numeric domain".into());
        }
        Ok(result)
    };
    Ok(Some(Matrix {
        a: quotient(d.clone())?,
        b: quotient(-b.clone())?,
        c: quotient(-c.clone())?,
        d: quotient(a.clone())?,
        e: quotient(&c * &f - &d * &e)?,
        f: quotient(&b * &e - &a * &f)?,
    }))
}

/// big.Rat.Float64: the nearest float64 (ties to even).
fn rational_to_f64(value: &BigRational) -> f64 {
    let negative = value.numer() < &BigInt::zero();
    let numerator = value.numer().magnitude().clone();
    let denominator = value.denom().magnitude().clone();
    if numerator.bits() == 0 {
        return if negative { -0.0 } else { 0.0 };
    }
    // Scale to a 54+-bit quotient, then round to 53 bits with a sticky bit.
    let shift = numerator.bits() as i64 - denominator.bits() as i64 - 55;
    let (n, d) = if shift >= 0 { (numerator, denominator << shift as usize) } else { (numerator << (-shift) as usize, denominator) };
    let quotient = &n / &d;
    let remainder_nonzero = (&n % &d).bits() != 0;
    let mut mantissa = quotient.to_u64().unwrap_or(u64::MAX);
    let mut exponent = shift;
    // Normalize mantissa to [2^54, 2^56).
    while mantissa >= 1 << 56 {
        let sticky = mantissa & 1;
        mantissa = (mantissa >> 1) | sticky;
        exponent += 1;
    }
    let sticky = remainder_nonzero as u64;
    mantissa |= sticky;
    let result = mantissa as f64 * 2f64.powi(exponent.clamp(-1100, 1100) as i32);
    // Two-step scaling can double-round for subnormal results; those are
    // far outside any renderable transform and only feed finiteness checks.
    if negative {
        -result
    } else {
        result
    }
}
