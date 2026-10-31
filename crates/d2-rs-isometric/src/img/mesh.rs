// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg core mesh types and arithmetic: raster.go (Vertex,
//! Material, Triangle, rasterCamera, rasterFit, rasterMaterial, sRGB),
//! lighting.go (shade) and geometry.go's vector helpers. Go rounds most
//! products explicitly (float64(...)), so they are plain here; the remaining
//! contraction sites are marked `ARM64:` (`.reference/isometric-port/fma-sites.txt`).
//! Pointer identities (shared materials, opacity groups, paint owners) are
//! Rc identities.

use std::cell::RefCell;
use std::rc::Rc;

use d2_rs_render::go_float::{go_fmadd, go_fmsub, go_fnmsub, gomath};
use d2_rs_scene::{go_max, go_min, Nrgba};

pub use crate::scene::Vec3 as Vec;

pub fn nv(x: f64, y: f64, z: f64) -> Vec {
    Vec { x, y, z }
}
pub fn nadd(a: Vec, b: Vec) -> Vec {
    nv(a.x + b.x, a.y + b.y, a.z + b.z)
}
pub fn nsub(a: Vec, b: Vec) -> Vec {
    nv(a.x - b.x, a.y - b.y, a.z - b.z)
}
pub fn nmul(a: Vec, s: f64) -> Vec {
    nv(a.x * s, a.y * s, a.z * s)
}
pub fn ndot(a: Vec, b: Vec) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}
pub fn ncross(a: Vec, b: Vec) -> Vec {
    nv(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x)
}
pub fn nlen(a: Vec) -> f64 {
    ndot(a, a).sqrt()
}
pub fn nunit(a: Vec) -> Vec {
    let l = nlen(a);
    if l > 1e-12 {
        return nmul(a, 1.0 / l);
    }
    nv(0.0, 1.0, 0.0)
}
pub fn nlerp(a: Vec, b: Vec, t: f64) -> Vec {
    nadd(a, nmul(nsub(b, a), t))
}

// raster.go's twins (identical rounding).
pub fn raster_unit(a: Vec) -> Vec {
    let l = ndot(a, a).sqrt();
    if l < 1e-15 {
        return nv(0.0, 1.0, 0.0);
    }
    nmul(a, 1.0 / l)
}
pub fn raster_clamp(v: f64) -> f64 {
    go_max(0.0, go_min(1.0, v))
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vertex {
    pub position: Vec,
    pub normal: Vec,
    pub u: f64,
    pub v: f64,
}

/// Standard raster formats admitted by rasterValidate. NRGBA stays straight
/// alpha until sampling, preserving Go color.NRGBA's 16-bit premultiplication.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextureFormat {
    #[default]
    Rgba,
    Nrgba,
    Alpha,
}

/// Caller-owned raster rows. Existing painters use packed premultiplied RGBA;
/// raw image callers can retain an explicit row stride and color model.
/// Pixels are a cell because native painters can edit shared atlases.
#[derive(Clone, Debug, Default)]
pub struct Texture {
    pub width: usize,
    pub height: usize,
    pub pixels: RefCell<std::vec::Vec<u8>>,
    pub format: TextureFormat,
    pub stride: Option<i64>,
}

impl Texture {
    pub fn new(width: usize, height: usize, pixels: std::vec::Vec<u8>) -> Self {
        Self {
            width,
            height,
            pixels: RefCell::new(pixels),
            format: TextureFormat::Rgba,
            stride: None,
        }
    }

    /// Preserve raw standard-image storage for admission by the rasterizer.
    /// Keeping validation at render entry preserves cancellation precedence.
    pub fn with_storage(
        width: usize,
        height: usize,
        stride: i64,
        format: TextureFormat,
        pixels: std::vec::Vec<u8>,
    ) -> Self {
        Self {
            width,
            height,
            pixels: RefCell::new(pixels),
            format,
            stride: Some(stride),
        }
    }

    pub(crate) fn channels(&self) -> usize {
        if self.format == TextureFormat::Alpha {
            1
        } else {
            4
        }
    }

    /// rasterTextureStorage: divide available bytes before comparing the
    /// stride so a malicious row stride cannot overflow a multiplication.
    pub(crate) fn validate_storage(&self) -> Result<usize, String> {
        if self.width == 0 || self.height == 0 || self.width > 16384 || self.height > 16384 {
            return Err("invalid native isometric texture storage".into());
        }
        let row_bytes = self.width * self.channels();
        let stride = self.stride.unwrap_or(row_bytes as i64);
        if stride < row_bytes as i64 {
            return Err("invalid native isometric texture storage".into());
        }
        let pixels = self.pixels.borrow().len();
        if pixels < row_bytes
            || self.height > 1 && stride as u64 > ((pixels - row_bytes) / (self.height - 1)) as u64
        {
            return Err("short native isometric texture storage".into());
        }
        Ok(stride as usize)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Material {
    pub color: Nrgba,
    pub roughness: f64,
    pub metalness: f64,
    pub emissive: Nrgba,
    pub texture: Option<Rc<Texture>>,
    pub vector: Option<Rc<crate::img::vector_surface::VectorSurface>>,
    pub unlit: bool,
    pub multiply: bool,
    pub svg_contour: bool,
    pub svg_solid_texture: bool,
    pub gradient: String,
}

pub type MaterialRef = Rc<RefCell<Material>>;

pub fn material(m: Material) -> MaterialRef {
    Rc::new(RefCell::new(m))
}

#[derive(Debug, Default)]
pub struct OpacityGroup {
    pub opacity: f64,
}

#[derive(Debug, Default)]
pub struct PaintOwner {
    pub opaque: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Triangle {
    pub v: [Vertex; 3],
    pub material: Option<MaterialRef>,
    pub cast_shadow: bool,
    pub shadow_opacity: Option<f64>,
    pub shadow_fill_alpha: Option<u8>,
    pub shadow_ground: Option<f64>,
    pub depth_bias: f64,
    pub no_depth_write: bool,
    pub opacity_group: Option<Rc<RefCell<OpacityGroup>>>,
    pub paint_owner: Option<Rc<RefCell<PaintOwner>>>,
    pub svg_coverage_only: bool,
}

pub const RASTER_MAX_TRIANGLES: usize = 1_000_000;
pub const RASTER_COORDINATE_LIMIT: f64 = 1e9;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Camera {
    pub right: Vec,
    pub up: Vec,
    pub direction: Vec,
    pub center_x: f64,
    pub center_y: f64,
    pub center_depth: f64,
    pub scale: f64,
    pub width: i64,
    pub height: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RasterPoint {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Camera {
    /// rasterCamera.project; the halves are exact (contraction unobservable).
    pub fn project(&self, p: Vec) -> RasterPoint {
        RasterPoint {
            x: (ndot(p, self.right) - self.center_x) * self.scale + self.width as f64 / 2.0,
            y: self.height as f64 / 2.0 - (ndot(p, self.up) - self.center_y) * self.scale,
            z: ndot(p, self.direction) - self.center_depth,
        }
    }

    pub fn world(&self, x: f64, y: f64, z: f64) -> Vec {
        nadd(
            nadd(
                nmul(self.right, (x - self.width as f64 / 2.0) / self.scale + self.center_x),
                nmul(self.up, (self.height as f64 / 2.0 - y) / self.scale + self.center_y),
            ),
            nmul(self.direction, z + self.center_depth),
        )
    }
}

pub fn native_view_direction() -> Vec {
    nv(0.16919264451761212, 0.7567450038061342, 0.6314355456066684)
}

/// rasterFit: fit the occupied vertices with the fixed orthographic camera.
pub fn raster_fit(ts: &[Triangle], direction: Vec, width: i64, height: i64, padding: f64) -> Camera {
    let direction = raster_unit(direction);
    let right = raster_unit(ncross(nv(0.0, 1.0, 0.0), direction));
    let up = raster_unit(ncross(direction, right));
    let (mut min_x, mut min_y, mut min_z) = (f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y, mut max_z) = (f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for t in ts {
        for v in &t.v {
            let p = v.position;
            let (x, y, z) = (ndot(p, right), ndot(p, up), ndot(p, direction));
            min_x = go_min(min_x, x);
            max_x = go_max(max_x, x);
            min_y = go_min(min_y, y);
            max_y = go_max(max_y, y);
            min_z = go_min(min_z, z);
            max_z = go_max(max_z, z);
        }
    }
    if min_x == f64::INFINITY {
        (min_x, min_y, min_z, max_x, max_y, max_z) = (-1.0, -1.0, -1.0, 1.0, 1.0, 1.0);
    }
    let span = go_max(0.02, go_max(max_y - min_y, (max_x - min_x) * height as f64 / width as f64)) * padding;
    Camera {
        right,
        up,
        direction,
        center_x: (min_x + max_x) / 2.0,
        center_y: (min_y + max_y) / 2.0,
        center_depth: (min_z + max_z) / 2.0,
        scale: height as f64 / span,
        width,
        height,
    }
}

/// rasterPaint: a material in linear light.
#[derive(Clone, Debug, Default)]
pub struct RasterPaint {
    pub base: Vec,
    pub emission: Vec,
    pub alpha: f64,
    pub roughness: f64,
    pub metalness: f64,
    pub texture: bool,
    pub unlit: bool,
    pub multiply: bool,
}

pub fn default_material() -> Material {
    Material { color: Nrgba::new(240, 243, 249, 255), roughness: 0.35, ..Default::default() }
}

pub fn raster_material(m: Option<&Material>) -> RasterPaint {
    let fallback = default_material();
    let m = m.unwrap_or(&fallback);
    let channel = |c: u8| raster_linear(c as f64 / 255.0);
    let emission = |c: u8| raster_linear(c as f64 / 255.0) * m.emissive.a as f64 / 255.0;
    RasterPaint {
        base: nv(channel(m.color.r), channel(m.color.g), channel(m.color.b)),
        emission: nv(emission(m.emissive.r), emission(m.emissive.g), emission(m.emissive.b)),
        alpha: m.color.a as f64 / 255.0,
        roughness: go_max(0.05, raster_clamp(m.roughness)),
        metalness: raster_clamp(m.metalness),
        texture: m.texture.is_some(),
        unlit: m.unlit,
        multiply: m.multiply,
    }
}

pub fn raster_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        return c / 12.92;
    }
    gomath::pow((c + 0.055) / 1.055, 2.4)
}

pub fn raster_srgb(c: f64) -> f64 {
    let c = raster_clamp(c);
    if c <= 0.0031308 {
        return c * 12.92;
    }
    // ARM64 raster.go:871: FNMSUBD.
    go_fnmsub(1.055, gomath::pow(c, 1.0 / 2.4), 0.055)
}

// ---------------------------------------------------------------- lighting.go

pub fn light_directions() -> [Vec; 3] {
    [
        raster_unit(nv(-1.0, 1.8, 1.0)),
        raster_unit(nv(10.0, 12.0, -16.0)),
        raster_unit(nv(12.0, 5.0, 12.0)),
    ]
}
const LIGHT_STRENGTHS: [f64; 3] = [0.55, 0.16, 0.07];
const LIGHT_COLORS: [Vec; 3] = [
    Vec { x: 1.0, y: 1.0, z: 1.0 },
    Vec { x: 0.93, y: 0.96, z: 1.0 },
    Vec { x: 1.0, y: 1.0, z: 1.0 },
];

/// A silhouette normal may carry a residual of either sign; keep the lit side.
pub fn lighting_normal(normal: Vec, direction: Vec) -> Vec {
    let normal = raster_unit(normal);
    if ndot(normal, direction) < -1e-12 {
        return nmul(normal, -1.0);
    }
    normal
}

/// Raster.shade with an occlusion term (SVG passes 0: its shadow map is empty).
pub fn shade(p: &RasterPaint, camera_direction: Vec, position: Vec, normal: Vec, occlusion: f64) -> (f64, f64, f64) {
    let _ = position;
    if p.unlit {
        return (raster_srgb(p.base.x), raster_srgb(p.base.y), raster_srgb(p.base.z));
    }
    let normal = lighting_normal(normal, camera_direction);
    let h = raster_clamp(normal.y * 0.5 + 0.5);
    // ARM64 lighting.go:28.
    let ambient = go_fmadd(0.12, h, 0.32);
    let mut illumination = nv(ambient, ambient, ambient);
    let mut spec = Vec::default();
    for (i, l) in light_directions().iter().enumerate() {
        let mut diffuse = go_max(0.0, ndot(normal, *l)) * LIGHT_STRENGTHS[i];
        if i == 0 {
            diffuse *= 1.0 - occlusion;
        }
        illumination = nadd(illumination, nmul(LIGHT_COLORS[i], diffuse));
        let half = raster_unit(nadd(*l, camera_direction));
        let power = go_max(2.0, 2.0 / (p.roughness * p.roughness) - 2.0);
        // ARM64 lighting.go:40.
        let s = gomath::pow(go_max(0.0, ndot(normal, half)), power) * diffuse * go_fmadd(p.metalness, 0.10, 0.025);
        spec = nadd(spec, nmul(LIGHT_COLORS[i], s));
    }
    // ARM64 lighting.go:43–44.
    let diffuse_factor = go_fmsub(p.metalness, 0.08, 1.0);
    (
        raster_srgb(go_fmadd(p.base.x * illumination.x, diffuse_factor, spec.x) + p.emission.x),
        raster_srgb(go_fmadd(p.base.y * illumination.y, diffuse_factor, spec.y) + p.emission.y),
        raster_srgb(go_fmadd(p.base.z * illumination.z, diffuse_factor, spec.z) + p.emission.z),
    )
}
