// SPDX-License-Identifier: MPL-2.0
//! render.go CaptureFrames, native.go capture and geometry.go Frame.
//! Exact scene time and packet time are separate; callers own every PNG.
use super::{
    context::{RenderContext, RenderError},
    geometry_core::MeshGeometry,
    geometry_routes::path_point,
    mesh::{Texture, nv},
    raster::StaticFrame,
    render::{Options, normalize},
    svg::NativeScene,
};
use crate::{scene::build_scene, target::Diagram};
use d2_rs_core::CancellationToken;
use std::{cell::OnceCell, rc::Rc};

pub const FRAME_COUNT: usize = 100;
pub const CYCLE_SECONDS: f64 = 25.0 / 3.0;
pub const MAX_ANIMATION_PIXELS: i64 = 100_000_000;

pub type CallbackError = Box<dyn std::error::Error + Send + Sync>;
#[derive(Debug)]
pub enum CaptureError {
    Render(RenderError),
    Callback(CallbackError),
}
impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Render(e) => e.fmt(f),
            Self::Callback(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for CaptureError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(match self {
            Self::Render(e) => e,
            Self::Callback(e) => e.as_ref(),
        })
    }
}
impl From<RenderError> for CaptureError {
    fn from(e: RenderError) -> Self {
        Self::Render(e)
    }
}
impl From<String> for CaptureError {
    fn from(e: String) -> Self {
        Self::Render(e.into())
    }
}

/// A bounded, serial capture owns the mesh and never mutates its source diagram.
pub struct Capture {
    pub(crate) context: RenderContext,
    pub(crate) scene: NativeScene,
    /// The non-animated scene's static raster (nativeScene.raster), built on
    /// first use and shared by every traffic frame. Reset with the scene.
    pub(crate) static_frame: OnceCell<StaticFrame>,
}

impl Capture {
    pub fn open(
        token: Option<&CancellationToken>,
        diagram: Option<&Diagram>,
        options: Option<&Options>,
    ) -> Result<Self, RenderError> {
        let o = normalize(options)?;
        Self::open_normalized(token, diagram, &o)
    }

    fn open_normalized(
        token: Option<&CancellationToken>,
        diagram: Option<&Diagram>,
        o: &Options,
    ) -> Result<Self, RenderError> {
        token
            .map(RenderContext::from_token)
            .unwrap_or_default()
            .check()?;
        let context = RenderContext::new(token, o.timeout_ns);
        Self::open_in_context(context, diagram, o, None)
    }

    pub(crate) fn open_in_context(
        context: RenderContext,
        diagram: Option<&Diagram>,
        o: &Options,
        camera: Option<super::mesh::Camera>,
    ) -> Result<Self, RenderError> {
        context.check()?;
        let diagram = diagram.ok_or_else(|| "isometric: nil diagram".to_string())?;
        let source = build_scene(diagram, Some(&o.render))?;
        context.check()?;
        let scene = super::render::build_native_scene_with_camera(
            &source, o.width, o.height, o, false, &context, camera,
        )
        .map_err(|e| context.error(format!("initialize native isometric renderer: {e}")))?;
        context.check()?;
        Ok(Self {
            context,
            scene,
            static_frame: OnceCell::new(),
        })
    }

    pub fn frame(&self, seconds: f64, traffic: bool) -> Result<Vec<u8>, RenderError> {
        let image = self.frame_image_at(seconds, seconds, traffic)?;
        let bytes = d2_rs_raster::encode_png_default(&image)
            .map_err(|e| self.context.error(format!("encode isometric PNG: {e}")))?;
        if bytes.len() > 32 << 20 {
            return Err("isometric PNG exceeds output byte limit".to_string().into());
        }
        self.context.check()?;
        Ok(bytes)
    }

    pub fn frame_image_at(
        &self,
        seconds: f64,
        packet_seconds: f64,
        traffic: bool,
    ) -> Result<d2_rs_raster::NrgbaImage, RenderError> {
        self.context.check()?;
        let frame = self
            .scene
            .frame_at(
                &self.context,
                &self.static_frame,
                seconds,
                packet_seconds,
                traffic,
            )
            .map_err(|e| {
                self.context
                    .error(format!("render isometric frame at {seconds:.4}s: {e}"))
            })?;
        self.context.check()?;
        if frame.rect != d2_rs_raster::Rect::new(0, 0, self.scene.width, self.scene.height) {
            return Err("native isometric renderer returned unexpected frame bounds"
                .to_string()
                .into());
        }
        Ok(frame)
    }
}

/// Emit exact-time PNGs in order. Admission precedes scene/resource work;
/// callback failures retain their typed cause, and cancellation is checked
/// immediately after each callback, including the last one.
pub fn capture_frames(
    token: Option<&CancellationToken>,
    diagram: Option<&Diagram>,
    options: Option<&Options>,
    seconds: &[f64],
    traffic: bool,
    mut emit: Option<&mut dyn FnMut(usize, Vec<u8>) -> Result<(), CallbackError>>,
) -> Result<(), CaptureError> {
    let o = normalize(options)?;
    if seconds.is_empty() || seconds.len() > FRAME_COUNT + 4 || emit.is_none() {
        return Err(format!("capture needs 1..{} times and a callback", FRAME_COUNT + 4).into());
    }
    if o.width * o.height * seconds.len() as i64 > MAX_ANIMATION_PIXELS {
        return Err("capture aggregate pixels exceed limit".to_string().into());
    }
    if seconds
        .iter()
        .any(|t| !t.is_finite() || !(0.0..=86400.0).contains(t))
    {
        return Err("capture time must be finite and in [0,86400]"
            .to_string()
            .into());
    }
    let capture = Capture::open_normalized(token, diagram, &o)?;
    for (i, &time) in seconds.iter().enumerate() {
        let png = capture.frame(time, traffic)?;
        emit.as_mut().unwrap()(i, png).map_err(CaptureError::Callback)?;
        capture.context.check()?;
    }
    Ok(())
}

impl NativeScene {
    fn frame_at(
        &self,
        context: &RenderContext,
        static_frame: &OnceCell<StaticFrame>,
        seconds: f64,
        packet_seconds: f64,
        traffic: bool,
    ) -> Result<d2_rs_raster::NrgbaImage, String> {
        if !seconds.is_finite() || seconds < 0.0 {
            return Err("isometric frame time must be finite and nonnegative".into());
        }
        let mut dynamic = MeshGeometry::default();
        dynamic.context = context.clone();
        if traffic {
            if !packet_seconds.is_finite() || packet_seconds < 0.0 {
                return Err("isometric packet time must be finite and nonnegative".into());
            }
            let mut phase = packet_seconds / CYCLE_SECONDS;
            phase -= phase.floor();
            if phase < 1e-12 || 1.0 - phase < 1e-12 {
                phase = 0.0;
            }
            for packet in &self.packets {
                for i in 0..2 {
                    let mut t = (phase + i as f64 / 2.0) % 1.0;
                    if packet.reverse && (!packet.forward || i % 2 != 0) {
                        t = 1.0 - t;
                    }
                    let point = path_point(&packet.points, &packet.lengths, t);
                    if let Some(material) = &packet.material {
                        dynamic.sphere(point, nv(0.045, 0.045, 0.045), material, 10, 8);
                    }
                }
            }
            if let Some(error) = dynamic.err {
                return Err(error);
            }
        }
        let mut triangles = None;
        if traffic && !self.animated_nodes.is_empty() {
            let ts = triangles.get_or_insert_with(|| self.triangles.clone());
            let phase = seconds - seconds.floor();
            let pulse = 1.0 - (2.0 * phase - 1.0).abs();
            for node in &self.animated_nodes {
                for triangle in &mut ts[node.first..node.last] {
                    // ARM64 source_panels.go:293 emits FMSUBD (the
                    // scale product rounds before the pulse multiplication).
                    for vertex in &mut triangle.v {
                        vertex.position.z = d2_rs_render::go_float::go_fmsub(
                            4.0 * self.pixel_scale,
                            pulse,
                            vertex.position.z,
                        );
                    }
                    let m = triangle.material.as_ref().map(|m| m.borrow());
                    if !triangle.cast_shadow
                        && m.as_ref().is_none_or(|m| !m.unlit)
                        && (m.as_ref().is_none_or(|m| m.texture.is_none())
                            || !triangle.no_depth_write)
                    {
                        triangle.cast_shadow = pulse > 0.0;
                        triangle.shadow_opacity = Some(pulse);
                    }
                }
            }
        }
        if traffic {
            for panel in self.panels.iter().filter(|p| p.animated) {
                context.check_string()?;
                let pixels = super::backends::raster_surface_at(
                    &panel.document,
                    panel.width as usize,
                    panel.height as usize,
                    seconds,
                )?;
                let texture = Rc::new(Texture::new(
                    panel.width as usize,
                    panel.height as usize,
                    pixels,
                ));
                let ts = triangles.get_or_insert_with(|| self.triangles.clone());
                for triangle in &mut ts[panel.first..panel.last] {
                    let mut material = triangle
                        .material
                        .as_ref()
                        .map(|m| m.borrow().clone())
                        .unwrap_or_default();
                    material.texture = Some(texture.clone());
                    // Capture rasterizes this owned document; still vector surfaces
                    // remain untouched and are not needed by the raster material.
                    material.vector = None;
                    triangle.material = Some(super::mesh::material(material));
                }
            }
        }
        if triangles.is_some() {
            return super::raster::render_frame(
                self,
                triangles.as_deref(),
                &dynamic.triangles,
                context,
            );
        }
        // frameRaster returns the scene's retained raster; Raster.Frame draws
        // only the packets over a copy of its samples (geometry.go:720).
        let cached = match static_frame.get() {
            Some(cached) => cached,
            None => {
                let built = StaticFrame::new(self, context)?;
                static_frame.get_or_init(|| built)
            }
        };
        cached.frame(self, &dynamic.triangles, context)
    }
}
