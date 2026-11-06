// SPDX-License-Identifier: MPL-2.0
//! Rasterization: a Rust port of upstream D2's `d2renderers/d2raster` package,
//! which turns a `d2scene` document into pixels. The `diagnostic-traces`
//! feature enables raster traces; `oracle-fixtures` compiles tests that read
//! regenerated Go-oracle artifacts.
//!
//! Entry points: [`render`] (`d2raster.Render`) returns the straight-alpha
//! NRGBA frame and [`render_bands`] (`d2raster.RenderBands`) streams
//! consecutive full-width bands. Both take [`FrameOptions`].
//! [`normalize_first_frame_png`] and [`encode_png_default`] provide the
//! authored-image/default-PNG boundary used by isometric SVG surfaces.

mod animation;
mod colrv1;
mod effects;
mod filter;
mod first_frame;
mod fma;
mod geometry;
mod image;
mod paint;
pub mod pixels;
mod png;
mod raster;
mod regions;
pub mod scanline;
mod text;
mod vector_validation;

/// Scene types callers need to build frames without a direct d2-rs-scene
/// dependency.
pub use d2_rs_scene::{Document as SceneDocument, Nrgba};
pub use pixels::{NrgbaImage, Rect};
pub use raster::{FrameOptions, RenderError, render, render_bands};

/// Validate and plan a complete frame with the same resource limits and error
/// ordering as [`render`], without allocating the final pixel buffer.
/// Exporters can admit a typed scene before handing pixels to an encoder.
pub fn preflight(
    document: &d2_rs_scene::Document,
    options: &FrameOptions,
) -> Result<(), RenderError> {
    raster::prepare(document, options, 0)
        .map(|_| ())
        .map_err(RenderError)
}

/// [`preflight`] for [`render_bands`]: the same admission with `band_height`
/// row bands, as d2raster.RenderBands prepares before its first band.
pub fn preflight_bands(
    document: &d2_rs_scene::Document,
    options: &FrameOptions,
    band_height: i64,
) -> Result<(), RenderError> {
    raster::prepare(document, options, band_height)
        .map(|_| ())
        .map_err(RenderError)
}

pub use png::{encode_png_default, normalize_first_frame_png};
