// SPDX-License-Identifier: MPL-2.0
//! Bounded still-image mesh rasterizer, translated from d2isometricimg/raster.go.
//! Raster arithmetic follows the pinned ARM64 instruction order.
use super::{
    framing::{ProjectedExtent, camera_at_resolution, raster_shadow_direction},
    mesh::*,
    svg::{raster_ground_triangles, raster_shadow_ground},
};
use d2_rs_render::go_float::{go_fmadd, go_fmsub, go_fnmsub};
use d2_rs_scene::{Nrgba, go_max, go_min};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    rc::Rc,
    time::Duration,
};
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

const MAX_WORK: i64 = 500_000_000;
const AA: i64 = 2;
const STRIP_ROWS: i64 = 64;
const SHADOW_SIZE: i64 = 2048;
type Pixels = std::vec::Vec<u8>;

pub(crate) struct Work {
    remaining: i64,
    deadline: Instant,
    context: super::context::RenderContext,
}
impl Work {
    fn new() -> Self {
        Self {
            remaining: MAX_WORK,
            deadline: Instant::now() + Duration::from_secs(120),
            context: Default::default(),
        }
    }
    fn charge(&mut self, n: i64) -> Result<(), String> {
        self.check()?;
        if n < 0 || n > self.remaining {
            return Err(format!(
                "native isometric raster work exceeds {MAX_WORK} sample operations"
            ));
        }
        self.remaining -= n;
        Ok(())
    }
    fn check(&self) -> Result<(), String> {
        self.context.check_string()?;
        if Instant::now() >= self.deadline {
            Err("context deadline exceeded".into())
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Bounds {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
}
impl Bounds {
    fn width(self) -> i64 {
        (self.x1 - self.x0).max(0)
    }
    fn height(self) -> i64 {
        (self.y1 - self.y0).max(0)
    }
    fn empty(self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }
    fn intersect(self, b: Self) -> Self {
        Self {
            x0: self.x0.max(b.x0),
            y0: self.y0.max(b.y0),
            x1: self.x1.min(b.x1),
            y1: self.y1.min(b.y1),
        }
    }
    fn union(self, b: Self) -> Self {
        if self.empty() {
            return b;
        }
        if b.empty() {
            return self;
        }
        Self {
            x0: self.x0.min(b.x0),
            y0: self.y0.min(b.y0),
            x1: self.x1.max(b.x1),
            y1: self.y1.max(b.y1),
        }
    }
    fn at(self, x: i64, y: i64) -> usize {
        ((y - self.y0) * self.width() + x - self.x0) as usize
    }
}
#[derive(Default)]
struct Shadow {
    camera: Camera,
    depth: std::vec::Vec<f32>,
    opacity: Pixels,
    bias: f64,
}
impl Shadow {
    fn occlusion(&self, world: Vec, normal: Vec) -> f64 {
        if self.depth.is_empty() {
            return 0.0;
        }
        let p = self.camera.project(nadd(world, nmul(normal, 0.025)));
        if p.x < 1.0
            || p.y < 1.0
            || p.x >= (self.camera.width - 1) as f64
            || p.y >= (self.camera.height - 1) as f64
        {
            return 0.0;
        }
        let (x, y) = (p.x as i64, p.y as i64);
        let mut sum = 0.0;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let at = ((y + dy) * self.camera.width + x + dx) as usize;
                if self.depth[at] as f64 > p.z + self.bias {
                    sum += self.opacity[at] as f64 / 255.0;
                }
            }
        }
        sum / 9.0
    }
}

/// The caller owns the returned opaque RGBA image. No mesh or texture aliases
/// escape this synchronous still render, including on resource-limit failure.
pub fn render(scene: &super::svg::NativeScene) -> Result<d2_rs_raster::NrgbaImage, String> {
    let texture = scene
        .background_paint
        .as_ref()
        .map(|surface| {
            surface
                .with_document(|doc| super::backends::raster_surface(doc, 256, 256))
                .map(|pixels| Texture::new(256, 256, pixels))
        })
        .transpose()?;
    render_mesh(
        scene.width,
        scene.height,
        &scene.triangles,
        scene.background,
        Some(&scene.camera),
        texture.as_ref(),
    )
}
pub fn render_mesh(
    width: i64,
    height: i64,
    ts: &[Triangle],
    background: Nrgba,
    camera: Option<&Camera>,
    background_texture: Option<&Texture>,
) -> Result<d2_rs_raster::NrgbaImage, String> {
    let context = super::context::RenderContext::default();
    let owned = deferred_snapshot(width, height, ts, &context)?;
    render_mesh_frame(
        width,
        height,
        owned.as_deref().unwrap_or(ts),
        &[],
        background,
        camera,
        background_texture,
        None,
        None,
        &context,
    )
}

/// Render a caller-owned mesh and one dynamic overlay with the camera fitted
/// only to the static triangles, as NewRaster followed by Frame does upstream.
/// Both meshes are validated; dynamic triangles test the static depth. Each
/// call returns an independent frame and observes the caller's cancellation.
/// This complete-render entry point does not retain prepared buffers between
/// calls or change the existing static-render path.
pub fn render_mesh_with_dynamic(
    width: i64,
    height: i64,
    triangles: &[Triangle],
    dynamic: &[Triangle],
    background: Nrgba,
    context: &super::context::RenderContext,
) -> Result<d2_rs_raster::NrgbaImage, String> {
    let owned = deferred_snapshot(width, height, triangles, context)?;
    render_mesh_frame(
        width,
        height,
        owned.as_deref().unwrap_or(triangles),
        dynamic,
        background,
        None,
        None,
        None,
        None,
        context,
    )
}

// NewRaster snapshots caller inputs only for valid deferred canvases. Internal
// frame builders already own their scene and retain the direct rendering path.
fn deferred_snapshot(
    width: i64,
    height: i64,
    ts: &[Triangle],
    context: &super::context::RenderContext,
) -> Result<Option<std::vec::Vec<Triangle>>, String> {
    if width > 0
        && height > 0
        && width <= 4096
        && height <= 4096
        && width * height <= 12_000_000
        && (width * height * AA * AA > 16_000_000 || ts.iter().any(|t| t.opacity_group.is_some()))
    {
        return snapshot_triangles(ts, context).map(Some);
    }
    Ok(None)
}

/// Exact-time capture draws packets after the prepared static scene and keeps
/// both shadow cameras anchored to the original geometry (source_panels.go).
pub(crate) fn render_frame(
    scene: &super::svg::NativeScene,
    animated: Option<&[Triangle]>,
    dynamic: &[Triangle],
    context: &super::context::RenderContext,
) -> Result<d2_rs_raster::NrgbaImage, String> {
    let texture = scene
        .background_paint
        .as_ref()
        .map(|s| {
            s.with_document(|doc| super::backends::raster_surface(doc, 256, 256))
                .map(|p| Texture::new(256, 256, p))
        })
        .transpose()?;
    let shadow = animated.map(|_| {
        raster_fit(
            &scene.triangles,
            raster_shadow_direction(),
            SHADOW_SIZE,
            SHADOW_SIZE,
            1.08,
        )
    });
    let ground_shadow = animated.map(|_| {
        let ground = raster_shadow_ground(&scene.triangles);
        raster_fit(
            &raster_ground_triangles(&scene.triangles, ground, native_view_direction()),
            raster_shadow_direction(),
            SHADOW_SIZE,
            SHADOW_SIZE,
            1.08,
        )
    });
    render_mesh_frame(
        scene.width,
        scene.height,
        animated.unwrap_or(&scene.triangles),
        dynamic,
        scene.background,
        Some(&scene.camera),
        texture.as_ref(),
        shadow.as_ref(),
        ground_shadow.as_ref(),
        context,
    )
}

#[allow(clippy::too_many_arguments)]
fn render_mesh_frame(
    width: i64,
    height: i64,
    ts: &[Triangle],
    dynamic: &[Triangle],
    background: Nrgba,
    camera: Option<&Camera>,
    background_texture: Option<&Texture>,
    shadow_camera: Option<&Camera>,
    ground_camera: Option<&Camera>,
    context: &super::context::RenderContext,
) -> Result<d2_rs_raster::NrgbaImage, String> {
    let mut work = Work {
        context: context.clone(),
        ..Work::new()
    };
    work.charge(0)?;
    if width < 1 || height < 1 || width > 4096 || height > 4096 || width * height > 12_000_000 {
        return Err("native isometric raster dimensions exceed limits".into());
    }
    validate(ts, &work)?;
    validate(dynamic, &work)?;
    if camera.is_some_and(|c| c.width < 1 || c.height < 1) {
        return Err("native raster camera requires positive dimensions".into());
    }
    let camera = camera
        .map(|c| camera_at_resolution(*c, width * AA, height * AA))
        .unwrap_or_else(|| raster_fit(ts, native_view_direction(), width * AA, height * AA, 1.08));
    if camera.width < 1 || camera.height < 1 {
        return Err("native raster camera requires positive dimensions".into());
    }
    let shadow = build_shadow_at(ts, &mut work, shadow_camera)?;
    let ground = raster_shadow_ground(ts);
    let ground_triangles = raster_ground_triangles(ts, ground, camera.direction);
    // Ground remapping returns a copy only when a receiver differs. Reusing a
    // map with identical caster positions is exact and avoids a second buffer.
    let ground_shadow = if matches!(ground_triangles, Cow::Owned(_)) {
        work.charge(ts.len() as i64)?;
        Some(build_shadow_at(
            &ground_triangles,
            &mut work,
            ground_camera,
        )?)
    } else {
        None
    };
    let ground_shadow = ground_shadow.as_ref().unwrap_or(&shadow);
    let mut ground_bounds = Bounds::default();
    if !ground_shadow.depth.is_empty() {
        let mut e = ProjectedExtent::default();
        e.prepared_ground_shadows(
            &ground_triangles,
            &[],
            0.0,
            ground,
            Some(&ground_shadow.camera),
        );
        if e.valid {
            let x0 = go_fmadd(
                e.min_x - camera.center_x,
                camera.scale,
                camera.width as f64 / 2.0,
            );
            let x1 = go_fmadd(
                e.max_x - camera.center_x,
                camera.scale,
                camera.width as f64 / 2.0,
            );
            let y0 = go_fmsub(
                e.max_y - camera.center_y,
                camera.scale,
                camera.height as f64 / 2.0,
            );
            let y1 = go_fmsub(
                e.min_y - camera.center_y,
                camera.scale,
                camera.height as f64 / 2.0,
            );
            ground_bounds = Bounds {
                x0: clamp_int(x0.floor(), camera.width),
                x1: clamp_int(x1.ceil(), camera.width),
                y0: clamp_int(y0.floor(), camera.height),
                y1: clamp_int(y1.ceil(), camera.height),
            };
        }
    }
    drop(ground_triangles);
    let raster = Raster {
        camera,
        shadow: &shadow,
        ground_shadow,
        ground,
        ground_bounds,
        background,
        background_texture,
    };
    let static_plan = raster.prepare(ts);
    let dynamic_plan = raster.prepare(dynamic);
    let combined =
        !dynamic.is_empty() && (!static_plan.groups.is_empty() || !dynamic_plan.groups.is_empty());
    let mut combined_triangles = vec![];
    if combined {
        if ts.len() + dynamic.len() > RASTER_MAX_TRIANGLES {
            return Err(format!(
                "native isometric triangles exceed {RASTER_MAX_TRIANGLES}"
            ));
        }
        work.charge((ts.len() + dynamic.len()) as i64)?;
        combined_triangles.extend_from_slice(ts);
        combined_triangles.extend_from_slice(dynamic);
    }
    let plan = if combined {
        raster.prepare(&combined_triangles)
    } else {
        static_plan
    };
    // newRaster retains samples up to rasterMaxSamples; grouped/large images
    // use renderStrips. Both paths use the same global sample coordinates.
    let rows = if camera.width * camera.height <= 16_000_000 && plan.groups.is_empty() {
        height
    } else {
        STRIP_ROWS.min(height)
    };
    let mut output = vec![0u8; (width * height * 4) as usize];
    for y0 in (0..height).step_by(rows as usize) {
        work.charge(0)?;
        let y1 = height.min(y0 + rows);
        let bounds = Bounds {
            x0: 0,
            y0: y0 * AA,
            x1: width * AA,
            y1: y1 * AA,
        };
        let mut pixels = vec![0; (bounds.width() * bounds.height() * 4) as usize];
        let mut depth = vec![f32::NEG_INFINITY; (bounds.width() * bounds.height()) as usize];
        raster.paint_ground(&mut work, &mut pixels, bounds)?;
        raster.draw(&mut work, &mut pixels, &mut depth, bounds, &plan)?;
        if !dynamic.is_empty() && !combined {
            raster.draw(&mut work, &mut pixels, &mut depth, bounds, &dynamic_plan)?;
        }
        work.charge(bounds.width() * bounds.height())?;
        for y in y0..y1 {
            for x in 0..width {
                let mut sums = [0u32; 3];
                for dy in 0..AA {
                    for dx in 0..AA {
                        let at = bounds.at(x * AA + dx, y * AA + dy) * 4;
                        for c in 0..3 {
                            sums[c] += pixels[at + c] as u32;
                        }
                    }
                }
                let at = ((y * width + x) * 4) as usize;
                for c in 0..3 {
                    output[at + c] = ((sums[c] + 2) / 4) as u8;
                }
                output[at + 3] = 255;
            }
        }
    }
    work.check()?;
    Ok(d2_rs_raster::NrgbaImage {
        pix: output,
        stride: width as usize * 4,
        rect: d2_rs_raster::Rect::new(0, 0, width, height),
    })
}

/// rasterSnapshot owns deferred geometry and raster bytes while preserving
/// sharing within the snapshot. Source vector documents remain shared, as Go
/// copies only their material pointer. Color models survive row compaction.
pub fn snapshot_triangles(
    ts: &[Triangle],
    context: &super::context::RenderContext,
) -> Result<std::vec::Vec<Triangle>, String> {
    let work = Work {
        context: context.clone(),
        ..Work::new()
    };
    validate(ts, &work)?;
    let mut materials = HashMap::new();
    let mut textures = HashMap::new();
    let mut groups = HashMap::new();
    let mut owners = HashMap::new();
    let mut out = ts.to_vec();
    for t in &mut out {
        work.check()?;
        if let Some(group) = &t.opacity_group {
            t.opacity_group = Some(
                groups
                    .entry(Rc::as_ptr(group) as usize)
                    .or_insert_with(|| {
                        Rc::new(std::cell::RefCell::new(OpacityGroup {
                            opacity: group.borrow().opacity,
                        }))
                    })
                    .clone(),
            );
        }
        if let Some(owner) = &t.paint_owner {
            t.paint_owner = Some(
                owners
                    .entry(Rc::as_ptr(owner) as usize)
                    .or_insert_with(|| {
                        Rc::new(std::cell::RefCell::new(PaintOwner {
                            opaque: owner.borrow().opaque,
                        }))
                    })
                    .clone(),
            );
        }
        if let Some(source) = &t.material {
            let id = Rc::as_ptr(source) as usize;
            if let std::collections::hash_map::Entry::Vacant(entry) = materials.entry(id) {
                let mut copy = source.borrow().clone();
                if let Some(texture) = &copy.texture {
                    let id = Rc::as_ptr(texture) as usize;
                    if let std::collections::hash_map::Entry::Vacant(entry) = textures.entry(id) {
                        let stride = texture.validate_storage()?;
                        let row_bytes = texture.width * texture.channels();
                        let source_pixels = texture.pixels.borrow();
                        let mut pixels = std::vec::Vec::with_capacity(row_bytes * texture.height);
                        for y in 0..texture.height {
                            if y % 64 == 0 {
                                work.check()?;
                            }
                            pixels.extend_from_slice(
                                &source_pixels[y * stride..y * stride + row_bytes],
                            );
                        }
                        entry.insert(Rc::new(Texture::with_storage(
                            texture.width,
                            texture.height,
                            row_bytes as i64,
                            texture.format,
                            pixels,
                        )));
                    }
                    copy.texture = Some(textures[&id].clone());
                }
                entry.insert(material(copy));
            }
            t.material = Some(materials[&id].clone());
        }
    }
    work.check()?;
    Ok(out)
}

fn validate(ts: &[Triangle], work: &Work) -> Result<(), String> {
    if ts.len() > RASTER_MAX_TRIANGLES {
        return Err(format!(
            "native isometric triangles exceed {RASTER_MAX_TRIANGLES}"
        ));
    }
    let mut textures = HashSet::new();
    let mut texture_pixels = 0usize;
    for (i, t) in ts.iter().enumerate() {
        if i % 1024 == 0 {
            work.check()?;
        }
        if !t.depth_bias.is_finite() || t.depth_bias.abs() > RASTER_COORDINATE_LIMIT {
            return Err(format!("triangle {i} has invalid depth bias"));
        }
        if t.shadow_opacity
            .is_some_and(|x| !x.is_finite() || !(0.0..=1.0).contains(&x))
        {
            return Err(format!("triangle {i} has invalid shadow opacity"));
        }
        if t.shadow_ground
            .is_some_and(|x| !x.is_finite() || x.abs() > RASTER_COORDINATE_LIMIT)
        {
            return Err(format!("triangle {i} has invalid shadow ground"));
        }
        if t.opacity_group.as_ref().is_some_and(|g| {
            let x = g.borrow().opacity;
            !x.is_finite() || !(0.0..=1.0).contains(&x)
        }) {
            return Err(format!("triangle {i} has invalid object opacity"));
        }
        for v in &t.v {
            for x in [
                v.position.x,
                v.position.y,
                v.position.z,
                v.normal.x,
                v.normal.y,
                v.normal.z,
                v.u,
                v.v,
            ] {
                if !x.is_finite() || x.abs() > RASTER_COORDINATE_LIMIT {
                    return Err(format!("triangle {i} has invalid vertex"));
                }
            }
        }
        if let Some(m) = &t.material {
            let m = m.borrow();
            if !m.roughness.is_finite() || !m.metalness.is_finite() {
                return Err(format!("triangle {i} has nonfinite material"));
            }
            if let Some(texture) = &m.texture {
                if textures.insert(Rc::as_ptr(texture) as usize) {
                    texture.validate_storage()?;
                    texture_pixels += texture.width * texture.height;
                    if texture_pixels > 64_000_000 {
                        return Err("native isometric texture pixels exceed limit".into());
                    }
                }
            }
        }
    }
    Ok(())
}
fn clamp_int(x: f64, max: i64) -> i64 {
    go_max(0.0, go_min(max as f64, x)) as i64
}
fn orient(mut a: RasterPoint, mut b: RasterPoint, c: RasterPoint) -> f64 {
    let mut sign = 1.0;
    if a.x > b.x || a.x == b.x && a.y > b.y {
        std::mem::swap(&mut a, &mut b);
        sign = -1.0;
    }
    sign * go_fmsub(b.y - a.y, c.x - a.x, (b.x - a.x) * (c.y - a.y))
}
fn top_left(a: RasterPoint, b: RasterPoint) -> bool {
    b.y < a.y || b.y == a.y && b.x > a.x
}
fn bounds(p: [RasterPoint; 3], width: i64, height: i64) -> Bounds {
    let min_x = go_min(p[0].x, go_min(p[1].x, p[2].x));
    let max_x = go_max(p[0].x, go_max(p[1].x, p[2].x));
    let min_y = go_min(p[0].y, go_min(p[1].y, p[2].y));
    let max_y = go_max(p[0].y, go_max(p[1].y, p[2].y));
    Bounds {
        x0: clamp_int(min_x.floor(), width),
        x1: clamp_int(max_x.ceil(), width),
        y0: clamp_int(min_y.floor(), height),
        y1: clamp_int(max_y.ceil(), height),
    }
}
fn row_span(p: [RasterPoint; 3], y: i64, x0: i64, x1: i64) -> (i64, i64) {
    let (mut left, mut right) = (x0 as f64 + 0.5, x1 as f64 - 0.5);
    let row = y as f64 + 0.5;
    for i in 0..3 {
        let (a, b) = (p[i], p[(i + 1) % 3]);
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        if dy == 0.0 {
            if dx * (row - a.y) < 0.0 {
                return (x0, x0);
            }
            continue;
        }
        let crossing = go_fmadd(dx, (row - a.y) / dy, a.x);
        if dy < 0.0 {
            left = go_max(left, crossing);
        } else {
            right = go_min(right, crossing);
        }
    }
    if left > right + 1e-9 {
        return (x0, x0);
    }
    (
        go_max(x0 as f64, go_min(x1 as f64, left.floor() - 1.0)) as i64,
        go_max(x0 as f64, go_min(x1 as f64, right.ceil() + 1.0)) as i64,
    )
}
// Go rounds the middle product, then fuses the first and third.
fn weighted3(a: f64, b: f64, c: f64, x: f64, y: f64, z: f64) -> f64 {
    go_fmadd(c, z, go_fmadd(a, x, b * y))
}
fn weighted(a: Vec, b: Vec, c: Vec, x: f64, y: f64, z: f64) -> Vec {
    nv(
        weighted3(a.x, b.x, c.x, x, y, z),
        weighted3(a.y, b.y, c.y, x, y, z),
        weighted3(a.z, b.z, c.z, x, y, z),
    )
}
fn texture_sample(t: &Texture, u: f64, v: f64) -> [f64; 4] {
    let x = go_fnmsub(raster_clamp(u), t.width as f64, 0.5);
    let y = go_fnmsub(raster_clamp(v), t.height as f64, 0.5);
    let (x0, y0) = (x.floor() as i64, y.floor() as i64);
    let (fx, fy) = (x - x0 as f64, y - y0 as f64);
    let mut c = [0.0; 4];
    let pixels = t.pixels.borrow();
    for dy in 0..=1 {
        for dx in 0..=1 {
            let weight =
                (if dx == 0 { 1.0 - fx } else { fx }) * (if dy == 0 { 1.0 - fy } else { fy });
            let sx = (x0 + dx).clamp(0, t.width as i64 - 1) as usize;
            let sy = (y0 + dy).clamp(0, t.height as i64 - 1) as usize;
            let channels = t.channels();
            let stride = t.stride.unwrap_or((t.width * channels) as i64) as usize;
            // A single row may legally have an arbitrarily large stride.
            let at = sy * stride + sx * channels;
            let alpha = if t.format == TextureFormat::Alpha {
                pixels[at]
            } else {
                pixels[at + 3]
            } as u32;
            for ch in 0..4 {
                let value = match t.format {
                    TextureFormat::Alpha => alpha * 257,
                    TextureFormat::Nrgba if ch < 3 => pixels[at + ch] as u32 * 257 * alpha / 255,
                    _ => pixels[at + ch] as u32 * 257,
                };
                c[ch] = go_fmadd(value as f64 / 65535.0, weight, c[ch]);
            }
        }
    }
    if c[3] <= 1e-10 {
        return [0.0; 4];
    }
    [c[0] / c[3], c[1] / c[3], c[2] / c[3], c[3]]
}
fn build_shadow_at(
    ts: &[Triangle],
    work: &mut Work,
    fixed_camera: Option<&Camera>,
) -> Result<Shadow, String> {
    let casts = |t: &Triangle| {
        t.cast_shadow
            && t.shadow_opacity.is_none_or(|o| o > 0.0)
            && t.material.as_ref().is_none_or(|m| m.borrow().color.a > 0)
    };
    if !ts.iter().any(casts) {
        return Ok(Shadow::default());
    }
    let camera = fixed_camera.copied().unwrap_or_else(|| {
        raster_fit(
            ts,
            raster_shadow_direction(),
            SHADOW_SIZE,
            SHADOW_SIZE,
            1.08,
        )
    });
    let mut s = Shadow {
        camera,
        depth: vec![f32::NEG_INFINITY; (SHADOW_SIZE * SHADOW_SIZE) as usize],
        opacity: vec![0; (SHADOW_SIZE * SHADOW_SIZE) as usize],
        bias: go_max(0.012, 2.0 / camera.scale),
    };
    for (i, t) in ts.iter().enumerate() {
        if i % 512 == 0 {
            work.charge(0)?;
        }
        if !casts(t) {
            continue;
        }
        let mut p = t.v.map(|v| camera.project(v.position));
        let mut vertices = t.v;
        let mut area = orient(p[0], p[1], p[2]);
        if area.abs() < 1e-10 {
            continue;
        }
        if area < 0.0 {
            p.swap(1, 2);
            vertices.swap(1, 2);
            area = -area;
        }
        let b = bounds(p, SHADOW_SIZE, SHADOW_SIZE);
        work.charge(b.height() * 3)?;
        let m = t.material.as_ref().map(|m| m.borrow());
        let mut opacity = m.as_ref().map_or(255, |m| m.color.a);
        if let Some(a) = t.shadow_fill_alpha {
            opacity = a;
        }
        if let Some(a) = t.shadow_opacity {
            opacity = (opacity as f64 * a).round() as u8;
        }
        if let Some(g) = &t.opacity_group {
            opacity = (opacity as f64 * g.borrow().opacity).round() as u8;
        }
        for y in b.y0..b.y1 {
            if y % 32 == 0 {
                work.check()?;
            }
            let (start, end) = row_span(p, y, b.x0, b.x1);
            work.charge(end - start)?;
            for x in start..end {
                let q = RasterPoint {
                    x: x as f64 + 0.5,
                    y: y as f64 + 0.5,
                    z: 0.0,
                };
                let (u, v, w) = (
                    orient(p[1], p[2], q),
                    orient(p[2], p[0], q),
                    orient(p[0], p[1], q),
                );
                if u < 0.0 || v < 0.0 || w < 0.0 {
                    continue;
                }
                let z = (weighted3(p[0].z, p[1].z, p[2].z, u, v, w) / area) as f32;
                let mut sample = opacity;
                if t.shadow_fill_alpha.is_none() {
                    if let Some(tex) = m.as_ref().and_then(|m| m.texture.as_ref()) {
                        let tx =
                            weighted3(vertices[0].u, vertices[1].u, vertices[2].u, u, v, w) / area;
                        let ty =
                            weighted3(vertices[0].v, vertices[1].v, vertices[2].v, u, v, w) / area;
                        sample = (opacity as f64 * texture_sample(tex, tx, ty)[3]).round() as u8;
                        if sample == 0 {
                            continue;
                        }
                    }
                }
                let at = (y * SHADOW_SIZE + x) as usize;
                if z > s.depth[at] {
                    s.depth[at] = z;
                    s.opacity[at] = sample;
                }
            }
        }
    }
    Ok(s)
}
struct Raster<'a> {
    camera: Camera,
    shadow: &'a Shadow,
    ground_shadow: &'a Shadow,
    ground: f64,
    ground_bounds: Bounds,
    background: Nrgba,
    background_texture: Option<&'a Texture>,
}
struct Plan<'a> {
    triangles: Cow<'a, [Triangle]>,
    order: std::vec::Vec<usize>,
    paints: HashMap<usize, RasterPaint>,
    groups: HashMap<usize, Group<'a>>,
}
struct Group<'a> {
    plan: Box<Plan<'a>>,
    bounds: Bounds,
    opacity: f64,
    depth: f64,
    opaque: bool,
    write_depth: bool,
}
fn material_key(t: &Triangle) -> usize {
    t.material.as_ref().map_or(0, |m| Rc::as_ptr(m) as usize)
}
impl Raster<'_> {
    fn paint_ground(&self, w: &mut Work, pixels: &mut [u8], bounds: Bounds) -> Result<(), String> {
        let sb = bounds.intersect(self.ground_bounds);
        w.charge(bounds.width() * bounds.height() + sb.width() * sb.height() * 9)?;
        for y in bounds.y0..bounds.y1 {
            if y % 32 == 0 {
                w.check()?;
            }
            for x in bounds.x0..bounds.x1 {
                let mut shadow = 0.0;
                if x >= sb.x0 && x < sb.x1 && y >= sb.y0 && y < sb.y1 {
                    let p = self.camera.world(x as f64 + 0.5, y as f64 + 0.5, 0.0);
                    let p = nadd(
                        p,
                        nmul(
                            self.camera.direction,
                            (self.ground - p.y) / self.camera.direction.y,
                        ),
                    );
                    shadow = self.ground_shadow.occlusion(p, nv(0.0, 1.0, 0.0)) * 0.11;
                }
                let mut paint = self.background;
                if let Some(tex) = self.background_texture {
                    let c = texture_sample(
                        tex,
                        (x as f64 + 0.5) / self.camera.width as f64,
                        (y as f64 + 0.5) / self.camera.height as f64,
                    );
                    paint = Nrgba::new(
                        (go_fmadd(paint.r as f64 / 255.0, 1.0 - c[3], c[0] * c[3]) * 255.0).round()
                            as u8,
                        (go_fmadd(paint.g as f64 / 255.0, 1.0 - c[3], c[1] * c[3]) * 255.0).round()
                            as u8,
                        (go_fmadd(paint.b as f64 / 255.0, 1.0 - c[3], c[2] * c[3]) * 255.0).round()
                            as u8,
                        255,
                    );
                }
                let at = bounds.at(x, y) * 4;
                for (ch, (c, shade)) in [(paint.r, 36.0), (paint.g, 55.0), (paint.b, 84.0)]
                    .into_iter()
                    .enumerate()
                {
                    pixels[at + ch] =
                        go_fmadd(c as f64, 1.0 - shadow, shade * shadow).round() as u8;
                }
                pixels[at + 3] = 255;
            }
        }
        Ok(())
    }
    fn prepare<'a>(&self, ts: &'a [Triangle]) -> Plan<'a> {
        self.prepare_triangles(Cow::Borrowed(ts))
    }

    // rasterPrepare retains its input slice and caches one paint per material.
    // Opacity-group members already own a private mesh; move it into the inner
    // plan instead of copying it again (opacity_groups.go).
    fn prepare_triangles<'a>(&self, triangles: Cow<'a, [Triangle]>) -> Plan<'a> {
        let mut plan = Plan {
            triangles,
            order: vec![],
            paints: HashMap::new(),
            groups: HashMap::new(),
        };
        let ts = plan.triangles.as_ref();
        for t in ts {
            plan.paints.entry(material_key(t)).or_insert_with(|| {
                raster_material(t.material.as_ref().map(|m| m.borrow()).as_deref())
            });
        }
        if !ts.iter().any(|t| t.opacity_group.is_some()) {
            plan.order = (0..ts.len()).collect();
            plan.order.sort_by(|&a, &b| {
                let (ta, tb) = (&ts[a], &ts[b]);
                if ta.no_depth_write != tb.no_depth_write {
                    return ta.no_depth_write.cmp(&tb.no_depth_write);
                }
                let (aa, bb) = (
                    transparent(ta, &plan.paints[&material_key(ta)]),
                    transparent(tb, &plan.paints[&material_key(tb)]),
                );
                if aa != bb {
                    return aa.cmp(&bb);
                }
                if !aa {
                    return std::cmp::Ordering::Equal;
                }
                depth_sum(ta, self.camera.direction)
                    .partial_cmp(&depth_sum(tb, self.camera.direction))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            return plan;
        }
        let mut first = HashMap::new();
        let mut members: HashMap<usize, std::vec::Vec<Triangle>> = HashMap::new();
        for (i, t) in ts.iter().enumerate() {
            if t.opacity_group.is_none() && t.paint_owner.is_none() {
                plan.order.push(i);
                continue;
            }
            let key = if let Some(g) = &t.opacity_group {
                (true, Rc::as_ptr(g) as usize)
            } else {
                (false, Rc::as_ptr(t.paint_owner.as_ref().unwrap()) as usize)
            };
            let index = *first.entry(key).or_insert_with(|| {
                plan.order.push(i);
                i
            });
            let g = plan.groups.entry(index).or_insert_with(|| Group {
                plan: Box::new(Plan {
                    triangles: Cow::Borrowed(&[]),
                    order: vec![],
                    paints: HashMap::new(),
                    groups: HashMap::new(),
                }),
                bounds: Bounds::default(),
                opacity: 0.0,
                depth: 0.0,
                opaque: false,
                write_depth: false,
            });
            if let Some(fade) = &t.opacity_group {
                g.opacity = fade.borrow().opacity;
            } else {
                g.opacity = 1.0;
                g.opaque = t.paint_owner.as_ref().unwrap().borrow().opaque;
                g.write_depth = true;
            }
            let p = t.v.map(|v| self.camera.project(v.position));
            for v in &t.v {
                g.depth += ndot(v.position, self.camera.direction);
            }
            g.bounds = g
                .bounds
                .union(bounds(p, self.camera.width, self.camera.height));
            let mut t = t.clone();
            t.opacity_group = None;
            t.paint_owner = None;
            members.entry(index).or_default().push(t);
        }
        for (index, triangles) in members {
            let g = plan.groups.get_mut(&index).unwrap();
            g.depth /= triangles.len() as f64;
            *g.plan = self.prepare_triangles(Cow::Owned(triangles));
        }
        plan.order.sort_by(|&a, &b| {
            let trans = |i| {
                plan.groups.get(&i).map_or_else(
                    || transparent(&ts[i], &plan.paints[&material_key(&ts[i])]),
                    |g| !g.opaque,
                )
            };
            let depth = |i| {
                plan.groups
                    .get(&i)
                    .map_or_else(|| depth_sum(&ts[i], self.camera.direction), |g| g.depth)
            };
            let (aa, bb) = (trans(a), trans(b));
            if aa != bb {
                return aa.cmp(&bb);
            }
            if aa {
                depth(a)
                    .partial_cmp(&depth(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            } else {
                std::cmp::Ordering::Equal
            }
        });
        plan
    }
    fn draw(
        &self,
        w: &mut Work,
        pixels: &mut [u8],
        depth: &mut [f32],
        rect: Bounds,
        plan: &Plan<'_>,
    ) -> Result<(), String> {
        for &index in &plan.order {
            w.charge(1)?;
            if let Some(g) = plan.groups.get(&index) {
                self.draw_group(w, pixels, depth, rect, g)?;
                continue;
            }
            let mut t = plan.triangles[index].clone();
            let paint = &plan.paints[&material_key(&t)];
            if paint.alpha == 0.0 {
                continue;
            }
            if paint.alpha < 0.999 || paint.multiply {
                let n = nadd(nadd(t.v[0].normal, t.v[1].normal), t.v[2].normal);
                if ndot(n, n) > 1e-20 && ndot(n, self.camera.direction) <= 0.0 {
                    continue;
                }
            }
            let mut p = t.v.map(|v| self.camera.project(v.position));
            let mut area = orient(p[0], p[1], p[2]);
            if area.abs() < 1e-10 {
                continue;
            }
            if area < 0.0 {
                p.swap(1, 2);
                t.v.swap(1, 2);
                area = -area;
            }
            let normal = raster_unit(ncross(
                nsub(t.v[1].position, t.v[0].position),
                nsub(t.v[2].position, t.v[0].position),
            ));
            for v in &mut t.v {
                if ndot(v.normal, v.normal) < 1e-20 {
                    v.normal = normal;
                }
            }
            let b = bounds(p, self.camera.width, self.camera.height).intersect(rect);
            if b.empty() {
                continue;
            }
            w.charge(b.height() * 3)?;
            let tl = [
                top_left(p[1], p[2]),
                top_left(p[2], p[0]),
                top_left(p[0], p[1]),
            ];
            let m = t.material.as_ref().map(|m| m.borrow());
            for y in b.y0..b.y1 {
                if y % 16 == 0 {
                    w.check()?;
                }
                let (start, end) = row_span(p, y, b.x0, b.x1);
                w.charge(end - start)?;
                for x in start..end {
                    let q = RasterPoint {
                        x: x as f64 + 0.5,
                        y: y as f64 + 0.5,
                        z: 0.0,
                    };
                    let (mut a, mut b, mut c) = (
                        orient(p[1], p[2], q),
                        orient(p[2], p[0], q),
                        orient(p[0], p[1], q),
                    );
                    if a < 0.0
                        || b < 0.0
                        || c < 0.0
                        || a == 0.0 && !tl[0]
                        || b == 0.0 && !tl[1]
                        || c == 0.0 && !tl[2]
                    {
                        continue;
                    }
                    a /= area;
                    b /= area;
                    c /= area;
                    let z = (weighted3(p[0].z, p[1].z, p[2].z, a, b, c) + t.depth_bias) as f32;
                    let at = rect.at(x, y);
                    if z < depth[at] {
                        continue;
                    }
                    let mut alpha = paint.alpha;
                    let (r, g, bl) = if let Some(tex) = m.as_ref().and_then(|m| m.texture.as_ref())
                    {
                        let u = weighted3(t.v[0].u, t.v[1].u, t.v[2].u, a, b, c);
                        let v = weighted3(t.v[0].v, t.v[1].v, t.v[2].v, a, b, c);
                        let [r, g, bl, ta] = texture_sample(tex, u, v);
                        alpha *= ta;
                        if alpha < 0.015 {
                            continue;
                        }
                        if paint.unlit {
                            (r, g, bl)
                        } else {
                            let position = weighted(
                                t.v[0].position,
                                t.v[1].position,
                                t.v[2].position,
                                a,
                                b,
                                c,
                            );
                            let normal =
                                weighted(t.v[0].normal, t.v[1].normal, t.v[2].normal, a, b, c);
                            let mut tp = paint.clone();
                            tp.base = nv(
                                raster_linear(r) * paint.base.x,
                                raster_linear(g) * paint.base.y,
                                raster_linear(bl) * paint.base.z,
                            );
                            self.shade(&tp, position, normal)
                        }
                    } else {
                        let position =
                            weighted(t.v[0].position, t.v[1].position, t.v[2].position, a, b, c);
                        let normal = weighted(t.v[0].normal, t.v[1].normal, t.v[2].normal, a, b, c);
                        self.shade(paint, position, normal)
                    };
                    let pi = at * 4;
                    if paint.multiply {
                        multiply_over(&mut pixels[pi..pi + 4], r, g, bl, alpha);
                    } else {
                        for (ch, color) in [r, g, bl].into_iter().enumerate() {
                            pixels[pi + ch] = go_fmadd(
                                raster_clamp(color) * alpha,
                                255.0,
                                pixels[pi + ch] as f64 * (1.0 - alpha),
                            )
                            .round() as u8;
                        }
                    }
                    if !t.no_depth_write && alpha >= 0.999 {
                        depth[at] = z;
                    }
                }
            }
        }
        Ok(())
    }
    fn shade(&self, p: &RasterPaint, position: Vec, normal: Vec) -> (f64, f64, f64) {
        let n = lighting_normal(normal, self.camera.direction);
        shade(
            p,
            self.camera.direction,
            position,
            normal,
            self.shadow.occlusion(position, n),
        )
    }
    fn draw_group(
        &self,
        w: &mut Work,
        dst: &mut [u8],
        depth: &mut [f32],
        rect: Bounds,
        g: &Group<'_>,
    ) -> Result<(), String> {
        if g.opacity <= 0.0 {
            return Ok(());
        }
        let b = g.bounds.intersect(rect);
        if b.empty() {
            return Ok(());
        }
        for y0 in (b.y0..b.y1).step_by((STRIP_ROWS * AA) as usize) {
            let r = Bounds {
                x0: b.x0,
                x1: b.x1,
                y0,
                y1: b.y1.min(y0 + STRIP_ROWS * AA),
            };
            let samples = (r.width() * r.height()) as usize;
            w.charge(samples as i64 * (if g.write_depth { 4 } else { 3 }))?;
            let mut pixels = vec![0; samples * 4];
            let mut local_depth = vec![0.0; samples];
            for y in r.y0..r.y1 {
                let local = r.at(r.x0, y);
                let global = rect.at(r.x0, y);
                let len = r.width() as usize;
                pixels[local * 4..(local + len) * 4]
                    .copy_from_slice(&dst[global * 4..(global + len) * 4]);
                local_depth[local..local + len].copy_from_slice(&depth[global..global + len]);
            }
            self.draw(w, &mut pixels, &mut local_depth, r, &g.plan)?;
            for y in r.y0..r.y1 {
                for x in r.x0..r.x1 {
                    let from = r.at(x, y) * 4;
                    let to = rect.at(x, y) * 4;
                    for ch in 0..4 {
                        dst[to + ch] = go_fmadd(
                            dst[to + ch] as f64,
                            1.0 - g.opacity,
                            pixels[from + ch] as f64 * g.opacity,
                        )
                        .round() as u8;
                    }
                }
                if g.write_depth {
                    let local = r.at(r.x0, y);
                    let global = rect.at(r.x0, y);
                    let len = r.width() as usize;
                    depth[global..global + len].copy_from_slice(&local_depth[local..local + len]);
                }
            }
        }
        Ok(())
    }
}
fn transparent(t: &Triangle, p: &RasterPaint) -> bool {
    t.no_depth_write || p.multiply || p.alpha < 0.999 || p.texture
}
fn depth_sum(t: &Triangle, direction: Vec) -> f64 {
    let mut z = 0.0;
    for v in &t.v {
        z += ndot(v.position, direction);
    }
    z
}
fn multiply_over(dst: &mut [u8], r: f64, g: f64, b: f64, alpha: f64) {
    let alpha = raster_clamp(alpha);
    let ba = dst[3] as f64 / 255.0;
    for (ch, c) in [r, g, b].into_iter().enumerate() {
        let s = raster_clamp(c);
        let back = dst[ch] as f64 / 255.0;
        let out = go_fmadd(
            back,
            go_fmadd(s, alpha, 1.0 - alpha),
            s * alpha * (1.0 - ba),
        );
        dst[ch] = (raster_clamp(out) * 255.0).round() as u8;
    }
    dst[3] = (go_fmadd(ba, 1.0 - alpha, alpha) * 255.0).round() as u8;
}

/// render.go/native.go PNG still branch. Mesh construction remains native.
pub fn render_png(
    diagram: &crate::target::Diagram,
    options: &super::render::Options,
) -> Result<std::vec::Vec<u8>, String> {
    super::render::render_png_with_context(None, Some(diagram), Some(options))
        .map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "raster_api_tests.rs"]
pub(super) mod api_tests;

// Input-only original coverage-test transport; no production diagnostics.
#[cfg(test)]
pub(super) fn coverage_validate(
    ts: &[Triangle],
    context: &super::context::RenderContext,
) -> Result<(), String> {
    validate(
        ts,
        &Work {
            context: context.clone(),
            ..Work::new()
        },
    )
}
