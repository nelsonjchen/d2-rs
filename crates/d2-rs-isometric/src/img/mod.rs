// SPDX-License-Identifier: MPL-2.0
//! d2renderers/d2isometric/d2isometricimg: native tessellation, mesh types and SVG writer.
//! Whole-board static SVG assembly and dependency/sequence route painting.

pub mod arrow_visibility;
pub mod backends;
pub mod context;
pub mod framing;
pub mod geometry;
pub mod geometry_core;
pub mod geometry_routes;
pub mod gradient_paints;
pub mod ground_receivers;
pub mod hierarchy_boards;
pub mod hierarchy_headers;
pub mod hierarchy_support;
pub mod markdown_cards;
pub mod mesh;
pub mod native_edges;
pub mod native_page;
pub mod native_shapes;
pub mod native_solids;
pub mod native_structured;
pub mod native_style;
pub mod opacity_groups;
pub mod page;
pub mod render;
pub mod rich_labels;
pub mod route_labels;
pub mod source_panels;
pub mod surface_document;
pub mod surface_icons;
pub mod svg;
pub mod svg_order;
pub mod svg_visibility;
pub mod text;
mod text_raster;
pub mod vector_surface;

mod vector_surface_color;
pub mod vector_surface_svg;
mod vector_surface_text;

mod math;

#[cfg(test)]
mod geometry_api_tests;

pub mod route_bridges;
pub mod route_caption_mask;
pub mod route_caption_style;
pub mod route_lanes;
pub mod route_style;
pub mod routes;
pub mod sequence_routes;

pub mod gif;
pub mod gif_capture;
pub mod gif_loop;
pub mod raster;
pub mod timeline;

#[cfg(test)]
mod content_api_tests;
