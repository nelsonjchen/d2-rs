// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg svg.go, svg_lighting.go, svg_seams.go,
//! svg_surface_resources.go and ground_receivers.go's rasterGroundTriangles:
//! the SVG writer for a native mesh scene. Contraction sites are marked
//! `ARM64:`.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use d2_rs_render::go_float::{go_fmadd, go_fmsub, gomath};
use d2_rs_scene::{go_max, go_min};

use super::mesh::{
    Camera, Material, MaterialRef, RasterPaint, Triangle, Vec, light_directions, lighting_normal,
    material, nadd, ncross, ndot, nlen, nlerp, nmul, nsub, nunit, nv, raster_clamp, raster_fit,
    raster_material, raster_unit, shade,
};
use super::svg_order::{OrderUnit, order_paint_units_with_context};
use super::svg_visibility::{
    SvgPoint, SvgVisibilityFace, edge_distance, go_round, polygon_area, polygon_box, split_polygon,
    svg_projected_point,
};
use super::vector_surface::VectorSurface;

pub const MAX_OUTPUT_BYTES: usize = 64 << 20;
pub const RASTER_SHADOW_NORMAL_OFFSET: f64 = 0.025;

/// A link hit region in world space (nativeLink).
#[derive(Clone, Debug, Default)]
pub struct NativeLink {
    pub points: Vec2<Vec>,
    pub url: String,
    pub tooltip: String,
    pub target: String,
}

type Vec2<T> = std::vec::Vec<T>;

/// Shared nativeScene: still writers and exact-time capture retain one mesh.
#[derive(Clone, Default)]
pub struct NativeScene {
    pub triangles: Vec2<Triangle>,
    pub camera: Camera,
    pub width: i64,
    pub height: i64,
    pub background: d2_rs_scene::Nrgba,
    pub background_paint: Option<Rc<VectorSurface>>,
    pub links: Vec2<NativeLink>,
    pub packets: Vec2<super::geometry_routes::PacketRoute>,
    pub panels: Vec2<super::source_panels::NativePanel>,
    pub animated_nodes: Vec2<super::native_shapes::NativeAnimatedNode>,
    pub pixel_scale: f64,
}

/// Serializes a retained vector surface (vector_surface_svg.go); provided by
/// the vector surface port.
pub type SurfaceWriter = dyn Fn(&VectorSurface, &str) -> Result<String, String>;

struct Writer<'a> {
    context: &'a super::context::RenderContext,
    buf: String,
    err: Option<String>,
    surfaces: HashMap<usize, String>,
    surface_svg: &'a SurfaceWriter,
}

impl Writer<'_> {
    fn write(&mut self, s: &str) {
        if self.err.is_some() {
            return;
        }
        if let Err(error) = self.context.check_string() {
            self.err = Some(error);
            return;
        }
        self.buf.push_str(s);
        if self.buf.len() > MAX_OUTPUT_BYTES {
            self.err = Some(format!(
                "native SVG exceeds {MAX_OUTPUT_BYTES} output bytes"
            ));
        }
    }

    /// surfaceDefinition: one `<defs><g id>` per retained surface.
    fn surface_definition(&mut self, surface: &Rc<VectorSurface>) -> String {
        if self.err.is_some() {
            return String::new();
        }
        let key = Rc::as_ptr(surface) as usize;
        if let Some(id) = self.surfaces.get(&key) {
            return id.clone();
        }
        let id = format!("surface-{}", self.surfaces.len());
        match (self.surface_svg)(surface, &format!("{id}-")) {
            Ok(fragment) => {
                self.write(&format!(r#"<defs><g id="{id}">{fragment}</g></defs>"#));
                if self.err.is_some() {
                    return String::new();
                }
                self.surfaces.insert(key, id.clone());
                id
            }
            Err(error) => {
                self.err = Some(error);
                String::new()
            }
        }
    }
}

/// svgNumber: six decimals with trailing zeros removed; tiny values are 0.
pub fn svg_number(v: f64) -> String {
    if v.abs() < 0.0000005 {
        return "0".into();
    }
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0');
    s.trim_end_matches('.').to_owned()
}

pub fn svg_color(r: f64, g: f64, b: f64) -> String {
    let c = |v: f64| go_round(raster_clamp(v) * 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", c(r), c(g), c(b))
}

/// svgPolygonPath: canonical compound loops at SVG precision.
pub fn svg_polygon_path(polygons: &[Vec2<SvgPoint>]) -> String {
    let mut s = String::new();
    for polygon in polygons {
        let mut p: Vec2<SvgPoint> = Vec2::with_capacity(polygon.len());
        for v in polygon {
            let v = SvgPoint {
                x: go_round(v.x * 1e6) / 1e6,
                y: go_round(v.y * 1e6) / 1e6,
                z: 0.0,
            };
            if p.last() != Some(&v) {
                p.push(v);
            }
        }
        if p.len() > 1 && p[0] == p[p.len() - 1] {
            p.pop();
        }
        let p = omit_collinear_splits(p);
        if p.len() < 3 {
            continue;
        }
        let area = 2.0 * polygon_area(&p);
        if area.abs() < 1e-10 {
            continue;
        }
        let mut first = 0;
        for i in 0..p.len() {
            if p[i].x < p[first].x || p[i].x == p[first].x && p[i].y < p[first].y {
                first = i;
            }
        }
        for i in 0..p.len() {
            let j = if area < 0.0 {
                (first + p.len() - i) % p.len()
            } else {
                (first + i) % p.len()
            };
            s.push(if i == 0 { 'M' } else { 'L' });
            s.push_str(&svg_number(p[j].x));
            s.push(' ');
            s.push_str(&svg_number(p[j].y));
        }
        s.push('Z');
    }
    s
}

/// svgOmitCollinearSplits on the serializer's rounded copy.
fn omit_collinear_splits(points: Vec2<SvgPoint>) -> Vec2<SvgPoint> {
    if points.len() <= 3 {
        return points;
    }
    let redundant = |a: SvgPoint, b: SvgPoint, c: SvgPoint| {
        let (dx, dy) = (c.x - a.x, c.y - a.y);
        let (px, py) = (b.x - a.x, b.y - a.y);
        let length2 = dx * dx + dy * dy;
        let along = px * dx + py * dy;
        if length2 == 0.0 || along < 0.0 || along > length2 {
            return false;
        }
        let cross = px * dy - py * dx;
        cross.abs() <= 1e-6 * length2.sqrt()
    };
    let mut clean: Vec2<SvgPoint> = Vec2::with_capacity(points.len());
    for (i, &p) in points.iter().enumerate() {
        while clean.len() > 1
            && clean.len() + points.len() - i > 3
            && redundant(clean[clean.len() - 2], clean[clean.len() - 1], p)
        {
            clean.pop();
        }
        clean.push(p);
    }
    while clean.len() > 3 {
        if redundant(clean[clean.len() - 1], clean[0], clean[1]) {
            clean.remove(0);
        } else if redundant(clean[clean.len() - 2], clean[clean.len() - 1], clean[0]) {
            clean.pop();
        } else {
            break;
        }
    }
    clean
}

struct Batch {
    cover: Option<usize>,
    triangle: Triangle,
    material: MaterialRef,
    polygons: Vec2<Vec2<SvgPoint>>,
    depth: f64,
    first: usize,
    affine: [f64; 6],
    texture: bool,
    color: String,
    points: [SvgPoint; 3],
}

struct Unit {
    batches: Vec2<usize>,
    depth: f64,
    count: usize,
    first: usize,
    opacity: f64,
    opaque: bool,
}

fn rc_addr<T>(rc: &Option<Rc<T>>) -> usize {
    rc.as_ref()
        .map_or(0, |rc| Rc::as_ptr(rc) as *const u8 as usize)
}

fn is_decal(t: &Triangle, m: &Material) -> bool {
    t.no_depth_write || (m.svg_contour || m.texture.is_some() || m.multiply || m.color.a < 255)
}

/// svgTextureAffine: UV → projected device transform.
pub fn svg_texture_affine(t: &Triangle, p: [SvgPoint; 3]) -> Result<[f64; 6], String> {
    let (u, v) = (t.v[1].u - t.v[0].u, t.v[1].v - t.v[0].v);
    let (x, y) = (t.v[2].u - t.v[0].u, t.v[2].v - t.v[0].v);
    let det = u * y - x * v;
    if det.abs() < 1e-15 {
        return Err("native SVG texture has a degenerate UV mapping".into());
    }
    let (dx, dy, ex, ey) = (
        p[1].x - p[0].x,
        p[1].y - p[0].y,
        p[2].x - p[0].x,
        p[2].y - p[0].y,
    );
    let (a, b, c, d) = (
        (dx * y - ex * v) / det,
        (dy * y - ey * v) / det,
        (u * ex - x * dx) / det,
        (u * ey - x * dy) / det,
    );
    Ok([
        a,
        b,
        c,
        d,
        p[0].x - a * t.v[0].u - c * t.v[0].v,
        p[0].y - b * t.v[0].u - d * t.v[0].v,
    ])
}

fn triangle_normal(t: &Triangle, i: usize) -> Vec {
    let normal = t.v[i].normal;
    if nlen(normal) < 1e-12 {
        return nunit(ncross(
            nsub(t.v[1].position, t.v[0].position),
            nsub(t.v[2].position, t.v[0].position),
        ));
    }
    normal
}

// ---------------------------------------------------------- svg_lighting.go

/// svgTextureLighting: diffuse slope and additive intercept per channel.
fn texture_lighting(camera: &Camera, t: &Triangle, m: &Material) -> ([f64; 3], [f64; 3]) {
    let p = raster_material(Some(m));
    let mut slope = [p.base.x, p.base.y, p.base.z];
    if p.unlit {
        return (slope, [0.0; 3]);
    }
    let normal = lighting_normal(triangle_normal(t, 0), camera.direction);
    // ARM64 svg_lighting.go:19.
    let ambient = go_fmadd(0.12, raster_clamp(normal.y * 0.5 + 0.5), 0.32);
    let mut illumination = nv(ambient, ambient, ambient);
    let mut spec = Vec::default();
    let strengths = [0.55, 0.16, 0.07];
    let colors = [nv(1.0, 1.0, 1.0), nv(0.93, 0.96, 1.0), nv(1.0, 1.0, 1.0)];
    for (i, light) in light_directions().iter().enumerate() {
        let diffuse = go_max(0.0, ndot(normal, *light)) * strengths[i];
        illumination = nadd(illumination, nmul(colors[i], diffuse));
        let half = raster_unit(nadd(*light, camera.direction));
        let power = go_max(2.0, 2.0 / (p.roughness * p.roughness) - 2.0);
        // ARM64 svg_lighting.go:27.
        let s = gomath::pow(go_max(0.0, ndot(normal, half)), power)
            * diffuse
            * go_fmadd(p.metalness, 0.10, 0.025);
        spec = nadd(spec, nmul(colors[i], s));
    }
    // ARM64 svg_lighting.go:30.
    let factor = go_fmsub(p.metalness, 0.08, 1.0);
    slope = [
        slope[0] * illumination.x * factor,
        slope[1] * illumination.y * factor,
        slope[2] * illumination.z * factor,
    ];
    (
        slope,
        [
            spec.x + p.emission.x,
            spec.y + p.emission.y,
            spec.z + p.emission.z,
        ],
    )
}

#[derive(Clone, Copy)]
struct LightingVertex {
    point: SvgPoint,
    position: Vec,
    normal: Vec,
}

#[derive(Clone, Default)]
struct Gradient {
    start: SvgPoint,
    end: SvgPoint,
    stops: Vec2<(f64, Vec)>,
    values: [f64; 3],
    valid: bool,
}

fn lighting_color(camera: &Camera, material: &RasterPaint, vertex: &LightingVertex) -> Vec {
    let (r, g, b) = shade(
        material,
        camera.direction,
        vertex.position,
        vertex.normal,
        0.0,
    );
    nv(r, g, b)
}

fn lighting_mix(a: &LightingVertex, b: &LightingVertex, t: f64) -> LightingVertex {
    // ARM64 svg_lighting.go:74: three FMADDD.
    LightingVertex {
        point: SvgPoint {
            x: go_fmadd(t, b.point.x - a.point.x, a.point.x),
            y: go_fmadd(t, b.point.y - a.point.y, a.point.y),
            z: go_fmadd(t, b.point.z - a.point.z, a.point.z),
        },
        position: nlerp(a.position, b.position, t),
        normal: nlerp(a.normal, b.normal, t),
    }
}

fn color_error(a: Vec, b: Vec) -> f64 {
    go_max(
        (a.x - b.x).abs(),
        go_max((a.y - b.y).abs(), (a.z - b.z).abs()),
    )
}

fn gradient_color(stops: &[(f64, Vec)], t: f64) -> Vec {
    if t <= stops[0].0 {
        return stops[0].1;
    }
    for i in 1..stops.len() {
        if t <= stops[i].0 {
            let (a, b) = (stops[i - 1], stops[i]);
            return nlerp(a.1, b.1, (t - a.0) / (b.0 - a.0));
        }
    }
    stops[stops.len() - 1].1
}

fn fit_lighting_gradient(
    camera: &Camera,
    material: &RasterPaint,
    v: &[LightingVertex; 3],
) -> (Gradient, f64) {
    let (mut lo, mut hi, mut distance) = (0, 0, 0.0f64);
    for i in 0..3 {
        for j in i + 1..3 {
            let delta = nsub(v[j].normal, v[i].normal);
            let d = ndot(delta, delta);
            if d > distance {
                (lo, hi, distance) = (i, j, d);
            }
        }
    }
    let mut gradient = Gradient::default();
    let flat = |gradient: &mut Gradient| {
        gradient.stops = vec![(0.0, lighting_color(camera, material, &v[0]))];
    };
    if distance < 1e-20 {
        flat(&mut gradient);
        return (gradient, 0.0);
    }
    let direction = nsub(v[hi].normal, v[lo].normal);
    for i in 0..3 {
        gradient.values[i] = ndot(nsub(v[i].normal, v[lo].normal), direction) / distance;
    }
    let (a, b, c) = (v[0].point, v[1].point, v[2].point);
    // ARM64 svg_lighting.go:121.
    let det = go_fmsub(c.x - a.x, b.y - a.y, (b.x - a.x) * (c.y - a.y));
    if det.abs() < 1e-12 {
        flat(&mut gradient);
        return (gradient, 0.0);
    }
    let values = gradient.values;
    // ARM64 svg_lighting.go:127–129.
    let gx = go_fmsub(
        values[2] - values[0],
        b.y - a.y,
        (values[1] - values[0]) * (c.y - a.y),
    ) / det;
    let gy = go_fmsub(
        c.x - a.x,
        values[1] - values[0],
        (b.x - a.x) * (values[2] - values[0]),
    ) / det;
    let norm = go_fmadd(gy, gy, gx * gx);
    if norm < 1e-24 {
        flat(&mut gradient);
        return (gradient, 0.0);
    }
    gradient.start = v[lo].point;
    gradient.end = SvgPoint {
        x: gradient.start.x + gx / norm,
        y: gradient.start.y + gy / norm,
        z: 0.0,
    };
    gradient.valid = true;
    let first = lighting_color(camera, material, &v[lo]);
    let last = lighting_color(camera, material, &v[hi]);
    gradient.stops.push((0.0, first));
    fn sample(
        camera: &Camera,
        material: &RasterPaint,
        v: &[LightingVertex; 3],
        lo: usize,
        hi: usize,
        left: f64,
        right: f64,
        lc: Vec,
        rc: Vec,
        depth: usize,
        stops: &mut Vec2<(f64, Vec)>,
    ) {
        let middle = (left + right) / 2.0;
        let actual = lighting_color(camera, material, &lighting_mix(&v[lo], &v[hi], middle));
        if depth < 6 && color_error(actual, nlerp(lc, rc, 0.5)) > 0.6 / 255.0 {
            sample(
                camera,
                material,
                v,
                lo,
                hi,
                left,
                middle,
                lc,
                actual,
                depth + 1,
                stops,
            );
            sample(
                camera,
                material,
                v,
                lo,
                hi,
                middle,
                right,
                actual,
                rc,
                depth + 1,
                stops,
            );
        } else {
            stops.push((right, rc));
        }
    }
    sample(
        camera,
        material,
        v,
        lo,
        hi,
        0.0,
        1.0,
        first,
        last,
        0,
        &mut gradient.stops,
    );
    let mut err = 0.0f64;
    for i in 0..3 {
        err = go_max(
            err,
            color_error(
                lighting_color(camera, material, &v[i]),
                gradient_color(&gradient.stops, values[i]),
            ),
        );
        let j = (i + 1) % 3;
        let midpoint = lighting_mix(&v[i], &v[j], 0.5);
        err = go_max(
            err,
            color_error(
                lighting_color(camera, material, &midpoint),
                gradient_color(&gradient.stops, (values[i] + values[j]) / 2.0),
            ),
        );
    }
    let center = lighting_mix(&lighting_mix(&v[0], &v[1], 0.5), &v[2], 1.0 / 3.0);
    err = go_max(
        err,
        color_error(
            lighting_color(camera, material, &center),
            gradient_color(&gradient.stops, (values[0] + values[1] + values[2]) / 3.0),
        ),
    );
    (gradient, err)
}

fn lighting_patches(
    camera: &Camera,
    material: &RasterPaint,
    vertices: [LightingVertex; 3],
) -> Vec2<([LightingVertex; 3], Gradient)> {
    let mut patches = Vec2::new();
    fn subdivide(
        camera: &Camera,
        material: &RasterPaint,
        v: [LightingVertex; 3],
        depth: usize,
        patches: &mut Vec2<([LightingVertex; 3], Gradient)>,
    ) {
        let (gradient, err) = fit_lighting_gradient(camera, material, &v);
        if err <= 1.5 / 255.0 || depth == 3 {
            patches.push((v, gradient));
            return;
        }
        let a = lighting_mix(&v[0], &v[1], 0.5);
        let b = lighting_mix(&v[1], &v[2], 0.5);
        let c = lighting_mix(&v[2], &v[0], 0.5);
        subdivide(camera, material, [v[0], a, c], depth + 1, patches);
        subdivide(camera, material, [a, v[1], b], depth + 1, patches);
        subdivide(camera, material, [c, b, v[2]], depth + 1, patches);
        subdivide(camera, material, [a, b, c], depth + 1, patches);
    }
    subdivide(camera, material, vertices, 0, &mut patches);
    patches
}

fn write_lighting_gradient(w: &mut Writer, gradient: &Gradient, id: &str) -> String {
    if !gradient.valid {
        let c = gradient.stops[0].1;
        return svg_color(c.x, c.y, c.z);
    }
    w.write(&format!(
        r#"<linearGradient id="{id}" gradientUnits="userSpaceOnUse" color-interpolation="sRGB" x1="{}" y1="{}" x2="{}" y2="{}">"#,
        svg_number(gradient.start.x),
        svg_number(gradient.start.y),
        svg_number(gradient.end.x),
        svg_number(gradient.end.y)
    ));
    for (offset, color) in &gradient.stops {
        w.write(&format!(
            r#"<stop offset="{}" stop-color="{}"/>"#,
            svg_number(*offset),
            svg_color(color.x, color.y, color.z)
        ));
    }
    w.write("</linearGradient>");
    format!("url(#{id})")
}

fn write_face_gradient(w: &mut Writer, b: &Batch, id: i64, camera: &Camera) -> String {
    let mut vertices = [LightingVertex {
        point: SvgPoint::default(),
        position: Vec::default(),
        normal: Vec::default(),
    }; 3];
    for (i, vertex) in b.triangle.v.iter().enumerate() {
        vertices[i] = LightingVertex {
            point: b.points[i],
            position: vertex.position,
            normal: triangle_normal(&b.triangle, i),
        };
    }
    let paint = raster_material(Some(&b.material.borrow()));
    let patches = lighting_patches(camera, &paint, vertices);
    w.write("<defs>");
    let fills: Vec2<String> = patches
        .iter()
        .enumerate()
        .map(|(i, (_, gradient))| write_lighting_gradient(w, gradient, &format!("shade-{id}-{i}")))
        .collect();
    if patches.len() == 1 {
        w.write("</defs>");
        return fills[0].clone();
    }
    let bx = polygon_box(&b.points);
    let (base, _) = fit_lighting_gradient(camera, &paint, &vertices);
    let base_fill = write_lighting_gradient(w, &base, &format!("shade-{id}-base"));
    let (x, y, width, height) = (bx.min_x, bx.min_y, bx.max_x - bx.min_x, bx.max_y - bx.min_y);
    w.write(&format!(
        r#"<pattern id="shade-{id}" patternUnits="userSpaceOnUse" x="{0}" y="{1}" width="{2}" height="{3}" viewBox="{0} {1} {2} {3}" preserveAspectRatio="none">"#,
        svg_number(x),
        svg_number(y),
        svg_number(width),
        svg_number(height)
    ));
    w.write(&format!(
        r#"<path d="{}" fill="{base_fill}"/>"#,
        svg_polygon_path(&[b.points.to_vec()])
    ));
    for (i, (patch, _)) in patches.iter().enumerate() {
        let p = vec![patch[0].point, patch[1].point, patch[2].point];
        w.write(&format!(
            r#"<path d="{}" fill="{}"/>"#,
            svg_polygon_path(&[p]),
            fills[i]
        ));
    }
    w.write("</pattern></defs>");
    format!("url(#shade-{id})")
}

// ------------------------------------------------------------- svg_seams.go

struct Cover {
    batches: Vec2<usize>,
    polygons: Vec2<Vec2<SvgPoint>>,
    first: usize,
    written: bool,
}

/// svgPreparePaintCovers: batches sharing a material silhouette.
fn prepare_covers(
    batches: &[Batch],
    unit_batches: &[usize],
    covers: &mut Vec2<Cover>,
) -> Vec2<(usize, usize)> {
    let mut by_material: BTreeMap<(usize, bool, bool), usize> = BTreeMap::new();
    let mut local: Vec2<Cover> = Vec2::new();
    for &index in unit_batches {
        let batch = &batches[index];
        if batch.polygons.is_empty() {
            continue;
        }
        let m = batch.material.borrow();
        if batch.texture && (!m.svg_solid_texture || batch.triangle.no_depth_write || m.multiply) {
            continue;
        }
        let key = (
            Rc::as_ptr(&batch.material) as *const u8 as usize,
            batch.triangle.no_depth_write,
            batch.texture,
        );
        let cover = *by_material.entry(key).or_insert_with(|| {
            local.push(Cover {
                batches: Vec2::new(),
                polygons: Vec2::new(),
                first: batch.first,
                written: false,
            });
            local.len() - 1
        });
        local[cover].batches.push(index);
        local[cover].polygons.extend(batch.polygons.iter().cloned());
    }
    let mut result = Vec2::new();
    for cover in local {
        if cover.batches.len() < 2 {
            continue;
        }
        let id = covers.len();
        for &batch in &cover.batches {
            result.push((batch, id));
        }
        covers.push(cover);
    }
    result
}

fn expand_cover_polygon(polygon: &[SvgPoint], width: f64) -> Vec2<SvgPoint> {
    if polygon.len() < 3 || width <= 0.0 {
        return polygon.to_vec();
    }
    let b = polygon_box(polygon);
    let mut points = vec![
        SvgPoint {
            x: b.min_x - width,
            y: b.min_y - width,
            z: 0.0,
        },
        SvgPoint {
            x: b.max_x + width,
            y: b.min_y - width,
            z: 0.0,
        },
        SvgPoint {
            x: b.max_x + width,
            y: b.max_y + width,
            z: 0.0,
        },
        SvgPoint {
            x: b.min_x - width,
            y: b.max_y + width,
            z: 0.0,
        },
    ];
    let sign = if polygon_area(polygon) < 0.0 {
        -1.0
    } else {
        1.0
    };
    for i in 0..polygon.len() {
        let (a, bb) = (polygon[i], polygon[(i + 1) % polygon.len()]);
        let length = gomath::hypot(bb.x - a.x, bb.y - a.y);
        if length < 1e-12 {
            continue;
        }
        let distance = edge_distance(a, bb);
        let (inside, _) = split_polygon(&points, &|p| sign * distance(p) / length + width);
        points = inside.resolve(&points).unwrap_or_default();
    }
    points
}

fn write_cover(
    w: &mut Writer,
    covers: &mut [Cover],
    cover_id: usize,
    batches: &[Batch],
    camera: &Camera,
) {
    if covers[cover_id].written || w.err.is_some() {
        return;
    }
    covers[cover_id].written = true;
    let cover = &covers[cover_id];
    let points: Vec2<SvgPoint> = cover.polygons.iter().flatten().copied().collect();
    let mut b = polygon_box(&points);
    b.min_x -= 1.0;
    b.min_y -= 1.0;
    b.max_x += 1.0;
    b.max_y += 1.0;
    let name = format!("paint-cover-{}", cover.first);
    let mut fills = vec![String::new(); cover.batches.len()];
    for (i, &index) in cover.batches.iter().enumerate() {
        let batch = &batches[index];
        if batch.texture {
            continue;
        }
        fills[i] = batch.color.clone();
        if fills[i].is_empty() {
            fills[i] = write_face_gradient(w, batch, -(batch.first as i64) - 1, camera);
        }
        if w.err.is_some() {
            return;
        }
    }
    let (x, y, width, height) = (b.min_x, b.min_y, b.max_x - b.min_x, b.max_y - b.min_y);
    w.write(&format!(
        r#"<defs><pattern id="{name}" patternUnits="userSpaceOnUse" x="{0}" y="{1}" width="{2}" height="{3}" viewBox="{0} {1} {2} {3}" preserveAspectRatio="none">"#,
        svg_number(x),
        svg_number(y),
        svg_number(width),
        svg_number(height)
    ));
    for (i, &index) in cover.batches.iter().enumerate() {
        let batch = &batches[index];
        if batch.texture {
            let mut m = batch.material.borrow().clone();
            m.color.a = 255;
            m.multiply = false;
            let part = Batch {
                cover: None,
                triangle: batch.triangle.clone(),
                material: material(m),
                polygons: batch
                    .polygons
                    .iter()
                    .map(|p| expand_cover_polygon(p, 0.4))
                    .collect(),
                depth: batch.depth,
                first: batch.first,
                affine: batch.affine,
                texture: batch.texture,
                color: batch.color.clone(),
                points: batch.points,
            };
            write_batch_plain(w, &part, -(batch.first as i64) - 1, camera);
            continue;
        }
        w.write(&format!(
            r#"<path d="{}" fill="{1}" stroke="{1}" stroke-width="0.8" stroke-linejoin="round"/>"#,
            svg_polygon_path(&batch.polygons),
            fills[i]
        ));
    }
    w.write("</pattern></defs>");
    let m = batches[cover.batches[0]].material.borrow();
    let blend = if m.multiply {
        r#" style="mix-blend-mode:multiply""#
    } else {
        ""
    };
    w.write(&format!(
        r#"<path d="{}" fill="url(#{name})" fill-opacity="{}"{blend}/>"#,
        svg_polygon_path(&cover.polygons),
        svg_number(m.color.a as f64 / 255.0)
    ));
}

/// writeSVGBatch without its cover indirection.
fn write_batch_plain(w: &mut Writer, b: &Batch, id: i64, camera: &Camera) {
    let path = svg_polygon_path(&b.polygons);
    if path.is_empty() {
        return;
    }
    let m = b.material.borrow().clone();
    let alpha = m.color.a as f64 / 255.0;
    let blend = if m.multiply {
        r#" style="mix-blend-mode:multiply""#
    } else {
        ""
    };
    if b.texture {
        let Some(vector) = m.vector.as_ref() else {
            w.err = Some(format!(
                "native SVG surface {} has no retained vector source",
                b.first
            ));
            return;
        };
        let surface_id = w.surface_definition(vector);
        if w.err.is_some() {
            return;
        }
        w.write(&format!(r#"<defs><clipPath id="face-{id}" clipPathUnits="userSpaceOnUse"><path d="{path}"/></clipPath></defs>"#));
        let mut filter = String::new();
        if !m.unlit {
            let (slope, intercept) = texture_lighting(camera, &b.triangle, &m);
            w.write(&format!(
                r#"<defs><filter id="light-{id}" color-interpolation-filters="linearRGB"><feComponentTransfer><feFuncR type="linear" slope="{}" intercept="{}"/><feFuncG type="linear" slope="{}" intercept="{}"/><feFuncB type="linear" slope="{}" intercept="{}"/></feComponentTransfer></filter></defs>"#,
                svg_number(slope[0]),
                svg_number(intercept[0]),
                svg_number(slope[1]),
                svg_number(intercept[1]),
                svg_number(slope[2]),
                svg_number(intercept[2])
            ));
            filter = format!(r#" filter="url(#light-{id})""#);
        }
        let a = b.affine;
        w.write(&format!(
            r##"<g clip-path="url(#face-{id})" opacity="{}"{blend}><g transform="matrix({} {} {} {} {} {})"{filter}><use xlink:href="#{surface_id}"/></g></g>"##,
            svg_number(alpha),
            svg_number(a[0]),
            svg_number(a[1]),
            svg_number(a[2]),
            svg_number(a[3]),
            svg_number(a[4]),
            svg_number(a[5])
        ));
        return;
    }
    let mut fill = b.color.clone();
    if fill.is_empty() {
        fill = write_face_gradient(w, b, id, camera);
    }
    w.write(&format!(
        r#"<path d="{path}" fill="{fill}" fill-opacity="{}"{blend}/>"#,
        svg_number(alpha)
    ));
}

// ------------------------------------------------------ ground shadow (svg_lighting.go)

pub fn raster_shadow_ground(ts: &[Triangle]) -> f64 {
    let mut ground = 0.0f64;
    for t in ts {
        for v in &t.v {
            ground = go_min(ground, v.position.y);
        }
    }
    ground + 0.08
}

/// ground_receivers.go rasterGroundTriangles.
pub fn raster_ground_triangles(ts: &[Triangle], ground: f64, view: Vec) -> Cow<'_, [Triangle]> {
    let changed = ts
        .iter()
        .any(|t| t.shadow_ground.is_some_and(|g| g != ground));
    if !changed {
        return Cow::Borrowed(ts);
    }
    let mut out = ts.to_vec();
    for t in &mut out {
        let Some(shadow_ground) = t.shadow_ground.filter(|g| *g != ground) else {
            continue;
        };
        let delta = nmul(view, (ground - shadow_ground) / view.y);
        for v in &mut t.v {
            v.position = nadd(v.position, delta);
        }
    }
    Cow::Owned(out)
}

fn caster_opacity(t: &Triangle) -> u8 {
    let m = t.material.as_ref().map(|m| m.borrow());
    if !t.cast_shadow || m.as_ref().is_some_and(|m| m.color.a == 0) {
        return 0;
    }
    let mut opacity = m.as_ref().map_or(255, |m| m.color.a);
    if let Some(alpha) = t.shadow_fill_alpha {
        opacity = alpha;
    }
    if let Some(shadow) = t.shadow_opacity {
        opacity = go_round(opacity as f64 * shadow) as u8;
    }
    if let Some(group) = &t.opacity_group {
        opacity = go_round(opacity as f64 * group.borrow().opacity) as u8;
    }
    opacity
}

fn ground_caster(
    t: &Triangle,
    ground: f64,
    receiver: f64,
    camera: &Camera,
    light: Vec,
) -> Vec2<SvgPoint> {
    let mut vertices = Vec2::new();
    for i in 0..3 {
        let (a, b) = (t.v[i].position, t.v[(i + 1) % 3].position);
        if a.y >= receiver {
            vertices.push(a);
        }
        if (a.y < receiver) != (b.y < receiver) {
            vertices.push(nlerp(a, b, (receiver - a.y) / (b.y - a.y)));
        }
    }
    vertices
        .iter()
        .map(|&vertex| {
            let mut p = nsub(
                vertex,
                nmul(
                    light,
                    (vertex.y - (ground + RASTER_SHADOW_NORMAL_OFFSET)) / light.y,
                ),
            );
            p.y = ground;
            let q = camera.project(p);
            svg_projected_point(q.x, q.y, ndot(vertex, light))
        })
        .collect()
}

fn write_ground_shadow(w: &mut Writer, scene: &NativeScene) {
    if scene.triangles.is_empty() {
        return;
    }
    let (ground, light) = (raster_shadow_ground(&scene.triangles), nv(-1.0, 1.8, 1.0));
    let triangles = raster_ground_triangles(&scene.triangles, ground, scene.camera.direction);
    let light_camera = raster_fit(&triangles, light, 2048, 2048, 1.08);
    let bias = go_max(0.012, 2.0 / light_camera.scale);
    // ARM64 svg_lighting.go:282.
    let receiver = go_fmadd(bias, light.y, ground + RASTER_SHADOW_NORMAL_OFFSET);
    let mut faces = Vec2::new();
    let mut sources = Vec2::new();
    let mut opacities = Vec2::new();
    for (i, triangle) in triangles.iter().enumerate() {
        let opacity = caster_opacity(triangle);
        if opacity == 0 {
            continue;
        }
        let points = ground_caster(triangle, ground, receiver, &scene.camera, light);
        if super::svg_visibility::valid_fragment_public(points.clone()).is_none() {
            continue;
        }
        let texture = triangle.shadow_fill_alpha.is_none()
            && triangle
                .material
                .as_ref()
                .is_some_and(|m| m.borrow().texture.is_some());
        faces.push(SvgVisibilityFace {
            points,
            opaque: !texture,
            order: -(i as i64),
            ..Default::default()
        });
        sources.push(i);
        opacities.push(opacity);
    }
    if faces.is_empty() {
        return;
    }
    let mut limits = super::svg_visibility::SVG_DEFAULT_VISIBILITY_LIMITS;
    limits.work = super::svg_visibility::svg_visibility_work_limit(faces.len());
    let visible =
        match super::svg_visibility::svg_visible_faces_with_context(&faces, limits, w.context) {
            Ok(visible) => visible,
            Err(error) => {
                w.err = Some(format!("native SVG ground shadow: {error}"));
                return;
            }
        };
    let right = nsub(
        light_camera.right,
        nmul(light, light_camera.right.y / light.y),
    );
    let up = nsub(light_camera.up, nmul(light, light_camera.up.y / light.y));
    let scale = scene.camera.scale / light_camera.scale * (2.0f64 / 3.0).sqrt();
    let sx = gomath::hypot(
        ndot(right, scene.camera.right),
        ndot(up, scene.camera.right),
    ) * scale;
    let sy = gomath::hypot(ndot(right, scene.camera.up), ndot(up, scene.camera.up)) * scale;
    w.write(&format!(
        r#"<defs><filter id="ground-softness" filterUnits="userSpaceOnUse" x="0" y="0" width="{}" height="{}" color-interpolation-filters="sRGB"><feGaussianBlur stdDeviation="{} {}"/></filter></defs><g filter="url(#ground-softness)">"#,
        scene.width,
        scene.height,
        svg_number(sx),
        svg_number(sy)
    ));
    let mut merged: BTreeMap<u8, Vec2<Vec2<SvgPoint>>> = BTreeMap::new();
    let mut textured: BTreeMap<u8, Vec2<usize>> = BTreeMap::new();
    for (i, polygons) in visible.iter().enumerate() {
        if polygons.is_empty() {
            continue;
        }
        let t = &triangles[sources[i]];
        if t.shadow_fill_alpha.is_none()
            && t.material
                .as_ref()
                .is_some_and(|m| m.borrow().texture.is_some())
        {
            textured.entry(opacities[i]).or_default().push(i);
        } else {
            merged
                .entry(opacities[i])
                .or_default()
                .extend(polygons.iter().cloned());
        }
    }
    for alpha in 1..=255u8 {
        let polygons = merged.remove(&alpha).unwrap_or_default();
        let mut textures = textured.remove(&alpha).unwrap_or_default();
        if polygons.is_empty() && textures.is_empty() {
            continue;
        }
        w.write(&format!(
            r#"<g opacity="{}">"#,
            svg_number(0.11 * alpha as f64 / 255.0)
        ));
        if !polygons.is_empty() {
            w.write(&format!(
                r##"<path d="{}" fill="#243754"/>"##,
                svg_polygon_path(&polygons)
            ));
        }
        let depth = |i: usize| {
            let sum: f64 = faces[i].points.iter().map(|p| p.z).sum();
            sum / faces[i].points.len() as f64
        };
        textures.sort_by(|&a, &b| {
            depth(a)
                .partial_cmp(&depth(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for i in textures {
            let t = &triangles[sources[i]];
            let vector = t.material.as_ref().and_then(|m| m.borrow().vector.clone());
            let Some(vector) = vector else {
                w.err = Some(format!(
                    "native SVG shadow surface {} has no retained vector source",
                    sources[i]
                ));
                return;
            };
            let surface_id = w.surface_definition(&vector);
            if w.err.is_some() {
                return;
            }
            let mut p = [SvgPoint::default(); 3];
            for (j, v) in t.v.iter().enumerate() {
                let mut world = nsub(
                    v.position,
                    nmul(
                        light,
                        (v.position.y - (ground + RASTER_SHADOW_NORMAL_OFFSET)) / light.y,
                    ),
                );
                world.y = ground;
                let q = scene.camera.project(world);
                p[j] = SvgPoint {
                    x: q.x,
                    y: q.y,
                    z: q.z,
                };
            }
            let affine = match svg_texture_affine(t, p) {
                Ok(affine) => affine,
                Err(error) => {
                    w.err = Some(error);
                    return;
                }
            };
            w.write(&format!(
                r#"<defs><clipPath id="shadow-clip-{i}"><path d="{}"/></clipPath><filter id="shadow-ink-{i}" color-interpolation-filters="sRGB"><feColorMatrix values="0 0 0 0 0.14117647 0 0 0 0 0.21568627 0 0 0 0 0.32941176 0 0 0 1 0"/></filter></defs>"#,
                svg_polygon_path(&visible[i])
            ));
            w.write(&format!(
                r##"<g clip-path="url(#shadow-clip-{i})"><g transform="matrix({} {} {} {} {} {})" filter="url(#shadow-ink-{i})"><use xlink:href="#{surface_id}"/></g></g>"##,
                svg_number(affine[0]),
                svg_number(affine[1]),
                svg_number(affine[2]),
                svg_number(affine[3]),
                svg_number(affine[4]),
                svg_number(affine[5])
            ));
        }
        w.write("</g>");
    }
    w.write("</g>");
}

/// html.EscapeString.
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '\'' => out.push_str("&#39;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&#34;"),
            _ => out.push(c),
        }
    }
    out
}

fn write_links(w: &mut Writer, scene: &NativeScene) {
    let mut links: Vec2<(&NativeLink, Vec2<SvgPoint>, f64)> = scene
        .links
        .iter()
        .map(|link| {
            let points: Vec2<SvgPoint> = link
                .points
                .iter()
                .map(|v| {
                    let p = scene.camera.project(*v);
                    svg_projected_point(p.x, p.y, p.z)
                })
                .collect();
            let area = polygon_area(&points).abs();
            (link, points, area)
        })
        .collect();
    links.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    for (link, points, _) in links {
        let mut destination = link.url.clone();
        if destination.is_empty() && !link.target.is_empty() {
            destination = format!("#{}", link.target);
        }
        w.write(&format!(
            r#"<a xlink:href="{}">"#,
            html_escape(&destination)
        ));
        if !link.tooltip.is_empty() {
            w.write(&format!("<title>{}</title>", html_escape(&link.tooltip)));
        }
        w.write(&format!(
            r#"<path d="{}" fill="transparent" pointer-events="all"/></a>"#,
            svg_polygon_path(&[points])
        ));
    }
}

/// nativeSceneSVG.
pub fn native_scene_svg(
    scene: &mut NativeScene,
    surface_svg: &SurfaceWriter,
) -> Result<String, String> {
    native_scene_svg_with_context(scene, surface_svg, &Default::default())
}
pub fn native_scene_svg_with_context(
    scene: &mut NativeScene,
    surface_svg: &SurfaceWriter,
    context: &super::context::RenderContext,
) -> Result<String, String> {
    context.check_string()?;
    let camera = scene.camera;
    let mut faces = Vec2::with_capacity(scene.triangles.len());
    for (i, t) in scene.triangles.iter().enumerate() {
        context.check_string()?;
        let mut f = SvgVisibilityFace {
            order: i as i64,
            group: t
                .opacity_group
                .as_ref()
                .map(|g| Rc::as_ptr(g) as *const u8 as usize),
            owner: t
                .paint_owner
                .as_ref()
                .map(|o| Rc::as_ptr(o) as *const u8 as usize),
            ..Default::default()
        };
        let m = t.material.as_ref().map(|m| m.borrow().clone());
        f.contour = m.as_ref().is_some_and(|m| m.svg_contour);
        let paint = raster_material(m.as_ref());
        if paint.alpha == 0.0 {
            faces.push(f);
            continue;
        }
        if paint.alpha < 0.999 || paint.multiply {
            let n = nadd(nadd(t.v[0].normal, t.v[1].normal), t.v[2].normal);
            if ndot(n, n) > 1e-20 && ndot(n, camera.direction) <= 0.0 {
                faces.push(f);
                continue;
            }
        }
        f.opaque = !t.no_depth_write
            && paint.alpha == 1.0
            && (!paint.texture || m.as_ref().is_some_and(|m| m.svg_solid_texture))
            && !paint.multiply;
        for v in &t.v {
            let p = camera.project(v.position);
            f.points
                .push(svg_projected_point(p.x, p.y, p.z + t.depth_bias));
        }
        faces.push(f);
    }
    let mut limits = super::svg_visibility::SVG_DEFAULT_VISIBILITY_LIMITS;
    limits.work = super::svg_visibility::svg_visibility_work_limit(faces.len());
    let visible = super::svg_visibility::svg_visible_faces_with_context(&faces, limits, context)?;
    let mut units: Vec2<Unit> = Vec2::new();
    let mut unit_index: HashMap<(usize, usize, usize), usize> = HashMap::new();
    let mut batch_indexes: Vec2<HashMap<(usize, [i64; 6], String, usize), usize>> = Vec2::new();
    let mut batches: Vec2<Batch> = Vec2::new();
    for i in 0..scene.triangles.len() {
        context.check_string()?;
        let t = &mut scene.triangles[i];
        let alpha_zero = t.material.as_ref().is_some_and(|m| m.borrow().color.a == 0);
        if t.svg_coverage_only || visible[i].is_empty() || alpha_zero {
            continue;
        }
        let m = t
            .material
            .get_or_insert_with(|| material(super::mesh::default_material()))
            .clone();
        let t = t.clone();
        let group = rc_addr(&t.opacity_group);
        let owner = if group == 0 {
            rc_addr(&t.paint_owner)
        } else {
            0
        };
        let material_key = if group == 0 && owner == 0 {
            Rc::as_ptr(&m) as *const u8 as usize
        } else {
            0
        };
        let key = (group, owner, material_key);
        let unit = match unit_index.get(&key) {
            Some(&unit) => unit,
            None => {
                let mut u = Unit {
                    batches: Vec2::new(),
                    depth: 0.0,
                    count: 0,
                    first: i,
                    opacity: 1.0,
                    opaque: faces[i].opaque,
                };
                if owner != 0 {
                    u.opaque = t.paint_owner.as_ref().unwrap().borrow().opaque;
                }
                if let Some(g) = &t.opacity_group {
                    u.opacity = g.borrow().opacity;
                    u.opaque = false;
                }
                units.push(u);
                batch_indexes.push(HashMap::new());
                unit_index.insert(key, units.len() - 1);
                units.len() - 1
            }
        };
        let mut depth = 0.0;
        for p in &faces[i].points {
            depth += p.z / 3.0;
        }
        units[unit].depth += depth;
        units[unit].count += 1;
        let mut points = [SvgPoint::default(); 3];
        points.copy_from_slice(&faces[i].points);
        let mut batch = Batch {
            cover: None,
            triangle: t.clone(),
            material: m.clone(),
            polygons: visible[i].clone(),
            depth,
            first: i,
            affine: [0.0; 6],
            texture: false,
            color: String::new(),
            points,
        };
        let mm = m.borrow().clone();
        let mut bk = (
            Rc::as_ptr(&m) as *const u8 as usize,
            [0i64; 6],
            String::new(),
            0usize,
        );
        if mm.texture.is_some() {
            if mm.vector.is_none() {
                return Err(format!(
                    "native SVG surface {i} has no retained vector source"
                ));
            }
            batch.texture = true;
            batch.affine = svg_texture_affine(&t, batch.points)?;
            for (j, v) in batch.affine.iter().enumerate() {
                bk.1[j] = go_round(v * 1e8) as i64;
            }
        } else {
            let mut flat = true;
            let normal = nunit(ncross(
                nsub(t.v[1].position, t.v[0].position),
                nsub(t.v[2].position, t.v[0].position),
            ));
            let paint = raster_material(Some(&mm));
            let mut first_normal = Vec::default();
            let mut gradient = [Vec::default(); 3];
            for (j, v) in t.v.iter().enumerate() {
                let mut n = v.normal;
                if nlen(n) < 1e-12 {
                    n = normal;
                }
                let (r, g, b) = shade(&paint, camera.direction, v.position, n, 0.0);
                gradient[j] = nv(r, g, b);
                if j == 0 {
                    first_normal = nunit(n);
                } else if !mm.unlit && ndot(first_normal, nunit(n)) < 1.0 - 1e-10 {
                    flat = false;
                }
                if j > 0 && nlen(nsub(gradient[0], gradient[j])) > 1.0 / 510.0 {
                    flat = false;
                }
            }
            if flat {
                let c = gradient[0];
                batch.color = svg_color(c.x, c.y, c.z);
                bk.2 = batch.color.clone();
            } else {
                bk.3 = i + 1;
            }
        }
        if let Some(&prior) = batch_indexes[unit].get(&bk) {
            let polygons = batch.polygons;
            batches[prior].polygons.extend(polygons);
        } else {
            batches.push(batch);
            let index = batches.len() - 1;
            units[unit].batches.push(index);
            batch_indexes[unit].insert(bk, index);
        }
    }
    let mut covers: Vec2<Cover> = Vec2::new();
    for unit in &mut units {
        unit.depth /= unit.count as f64;
        let decal = |b: &Batch| is_decal(&b.triangle, &b.material.borrow());
        let mut order = unit.batches.clone();
        order.sort_by(|&a, &b| {
            let (ba, bb) = (&batches[a], &batches[b]);
            let (pa, pb) = (decal(ba), decal(bb));
            if pa != pb {
                return if !pa {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
            }
            if pa {
                return ba.first.cmp(&bb.first);
            }
            ba.depth
                .partial_cmp(&bb.depth)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        unit.batches = order;
        for (batch, cover) in prepare_covers(&batches, &unit.batches, &mut covers) {
            batches[batch].cover = Some(cover);
        }
    }
    let order_units: Vec2<OrderUnit> = units
        .iter()
        .map(|u| OrderUnit {
            polygons: u
                .batches
                .iter()
                .flat_map(|&b| batches[b].polygons.iter().cloned())
                .collect(),
            first: u.first as i64,
            opaque: u.opaque,
            depth: u.depth,
        })
        .collect();
    let (order, _) = order_paint_units_with_context(
        &order_units,
        super::svg_visibility::SVG_DEFAULT_VISIBILITY_LIMITS,
        context,
    )?;
    let mut w = Writer {
        context,
        buf: String::new(),
        err: None,
        surfaces: HashMap::new(),
        surface_svg,
    };
    w.write(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{0}" height="{1}" viewBox="0 0 {0} {1}" role="img"><title>D2 isometric diagram</title>"#,
        scene.width, scene.height
    ));
    let bg = scene.background;
    w.write(&format!(
        r#"<rect width="100%" height="100%" fill="{}"/>"#,
        svg_color(
            bg.r as f64 / 255.0,
            bg.g as f64 / 255.0,
            bg.b as f64 / 255.0
        )
    ));
    if let Some(background) = &scene.background_paint {
        let fragment = surface_svg(background, "background")?;
        w.write(&format!(
            r#"<g transform="scale({} {})">{fragment}</g>"#,
            scene.width, scene.height
        ));
    }
    write_ground_shadow(&mut w, scene);
    let mut id = 0i64;
    for &u in &order {
        let unit = &units[u];
        w.write(&format!(r#"<g opacity="{}">"#, svg_number(unit.opacity)));
        for &b in &unit.batches {
            id += 1;
            if let Some(cover) = batches[b].cover {
                write_cover(&mut w, &mut covers, cover, &batches, &camera);
            } else {
                write_batch_plain(&mut w, &batches[b], id, &camera);
            }
            if let Some(error) = w.err.take() {
                return Err(error);
            }
        }
        w.write("</g>");
    }
    write_links(&mut w, scene);
    w.write("</svg>");
    if let Some(error) = w.err {
        return Err(error);
    }
    Ok(w.buf)
}

#[cfg(test)]
#[path = "hierarchy_api_tests.rs"]
pub(super) mod hierarchy_api_tests;

#[cfg(test)]
#[path = "coverage_api_tests.rs"]
mod coverage_api_tests;
