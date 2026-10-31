// SPDX-License-Identifier: MPL-2.0
//! Native port of D2's d2raster package (d2scene document to pixels) at the
//! pinned master. See SOURCE.md for the Go-file-to-module map.
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

pub use pixels::{NrgbaImage, Rect};
pub use raster::{render, render_bands, FrameOptions, RenderError};

pub use png::{encode_png_default, normalize_first_frame_png};
