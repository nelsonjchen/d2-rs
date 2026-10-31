// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
// YCbCr conversion: Copyright 2009 The Go Authors. All rights reserved.
// BSD-3-Clause conditions and disclaimer are retained in THIRD_PARTY_NOTICES.md.
// Source: d2raster/image.go rasterImage/vectorImage and effects.go renderEffectNode.
use super::*;
use d2_rs_math::{svg_path::Command, svg_scene::Scene};
fn matrix(l: [f64; 6], r: [f64; 6]) -> [f64; 6] {
    d2_rs_math::svg_transform::mul(l, r)
}
fn affine(m: [f64; 6]) -> Affine64 {
    Affine64 {
        sx: m[0],
        ky: m[1],
        kx: m[2],
        sy: m[3],
        tx: m[4],
        ty: m[5],
    }
}
fn commands(values: &[Command]) -> SvgPath {
    let point = |p: [f64; 2]| SvgPoint { x: p[0], y: p[1] };
    let commands = values
        .iter()
        .map(|c| match c.kind {
            0 => SvgPathCommand::MoveTo(point(c.points[0])),
            1 => SvgPathCommand::LineTo(point(c.points[0])),
            2 => SvgPathCommand::QuadraticTo(point(c.points[0]), point(c.points[1])),
            3 => {
                SvgPathCommand::CubicTo(point(c.points[0]), point(c.points[1]), point(c.points[2]))
            }
            4 => SvgPathCommand::ArcTo {
                radius_x: c.radii[0],
                radius_y: c.radii[1],
                rotation: c.rotation,
                large_arc: c.large_arc,
                sweep: c.sweep,
                end: point(c.points[0]),
            },
            _ => SvgPathCommand::Close,
        })
        .collect();
    SvgPath {
        commands,
        intrinsic_clips: Vec::new(),
        intrinsic_transforms: None,
        scene_transform: None,
        intrinsic_viewport: None,
        image_viewport: None,
        paint_opacities: None,
        raster_image: None,
    }
}

// Source effects.go renderEffectNode/compositeLayerOver intersects layer and
// destination bounds before applying masks/opacity/source-over. Storage here
// retains full-canvas indexing, but untouched pixels outside the prepared ink
// bounds must not require full-canvas compositing passes for every image node.
fn layer_rows(
    bounds: Option<[f64; 4]>,
    width: usize,
    height: usize,
) -> impl Iterator<Item = std::ops::Range<usize>> {
    let [left, top, right, bottom] = bounds.unwrap_or([0.; 4]);
    let left = left.max(0.).min(width as f64) as usize;
    let right = right.max(0.).min(width as f64) as usize;
    let top = top.max(0.).min(height as f64) as usize;
    let bottom = bottom.max(0.).min(height as f64) as usize;
    (top..bottom).map(move |y| (y * width + left) * 4..(y * width + right) * 4)
}
fn render(
    scene: &Scene,
    index: usize,
    parent: [f64; 6],
    resources: &std::collections::BTreeMap<String, Option<d2_rs_math::svg_image::RasterAsset>>,
    jpeg_pngs: &std::collections::BTreeMap<String, Vec<u8>>,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    parent_origin: Point64,
    prepared: &[Option<[f64; 4]>],
) -> bool {
    let node = &scene.nodes[index];
    let transform = matrix(parent, node.transform);
    if node.opacity == 0. {
        return true;
    }
    if node.clip.is_none() && node.opacity == 1. {
        return draw_content(
            scene,
            index,
            transform,
            resources,
            jpeg_pngs,
            pixels,
            width,
            height,
            parent_origin,
            prepared,
        );
    }
    let origin = if node.clip.is_some() || node.opacity != 1. {
        let Some(b) = prepared[index] else {
            return true;
        };
        Point64 {
            x: b[0].max(parent_origin.x),
            y: b[1].max(parent_origin.y),
        }
    } else {
        parent_origin
    };
    let mut layer = vec![0u8; pixels.len()];
    if !draw_content(
        scene, index, transform, resources, jpeg_pngs, &mut layer, width, height, origin, prepared,
    ) {
        return false;
    }
    if let Some(clip) = &node.clip {
        let m = affine(matrix(transform, clip.transform));
        let Some(paths) =
            collect_raw_subpaths(&commands(&clip.commands), flatten_tolerance64(m), m)
        else {
            return false;
        };
        let mapped: Vec<_> = paths.into_iter().map(|p| map_centerline(p, m)).collect();
        let mut mask = vec![0u8; pixels.len()];
        if clip.fill_rule == 1 {
            paint_even_odd_paths(&mut mask, width, height, &mapped, |_, _| Some([255; 4]));
        } else {
            let mut raster = Rasterizer::new(width, height, 0.375);
            raster.set_reference_origin(origin);
            for (points, _) in &mapped {
                raster.add_fill_polyline(points, true);
            }
            raster.paint(&mut mask, [255; 4]);
        }
        for row in layer_rows(prepared[index], width, height) {
            for (p, m) in layer[row.clone()]
                .chunks_exact_mut(4)
                .zip(mask[row].chunks_exact(4))
            {
                scale_premultiplied(p, m[3]);
            }
        }
    }
    let opacity = (node.opacity * 255.).round() as u8;
    for row in layer_rows(prepared[index], width, height) {
        for (dst, p) in pixels[row.clone()]
            .chunks_exact_mut(4)
            .zip(layer[row].chunks_exact_mut(4))
        {
            scale_premultiplied(p, opacity);
            blend_premultiplied_pixel(dst, p.try_into().unwrap());
        }
    }
    true
}
/// Render owned PNG/GIF images and admitted vector paints. Other codecs retain
/// the existing backend until their source decoding path is connected.
pub(super) fn draw(
    image: &usvg::Image,
    native: crate::svg_precision::NativeImage<'_>,
    placement: Option<&crate::svg_precision::SceneImagePlacement>,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    opacity: f32,
) -> Option<bool> {
    for node in &native.vector.scene.nodes {
        if let Some(p) = &node.primitive {
            if (p.geometry.kind == 4
                && !native
                    .resources
                    .get(&p.geometry.asset)
                    .and_then(Option::as_ref)
                    .is_some_and(|a| {
                        matches!(
                            a.mime.as_str(),
                            "image/png" | "image/gif" | "image/webp" | "image/jpeg"
                        )
                    }))
                || (p.geometry.kind != 4 && !matches!(p.geometry.kind, 1 | 2 | 3))
            {
                return None;
            }
        }
    }
    if !image.is_visible() {
        return Some(true);
    }
    let m = image.abs_transform();
    let s = f64::from(scale);
    let parsed_transform = [
        f64::from(m.sx) * s,
        f64::from(m.ky) * s,
        f64::from(m.kx) * s,
        f64::from(m.sy) * s,
        f64::from(m.tx) * s,
        f64::from(m.ty) * s,
    ];
    // Source vectorImage composes the original ancestor matrix with
    // AspectRatioMatrix(asset.ViewBox, primitive.Box, primitive.Aspect).
    // usvg's already-mapped image matrix has rounded both stages to f32.
    let transform = if let Some(placement) = placement {
        let Some(mapping) = placement
            .aspect
            .vector_matrix(native.vector.view_box, placement.image_box)
        else {
            return Some(false);
        };
        matrix(matrix([s, 0., 0., s, 0., 0.], placement.matrix), mapping)
    } else {
        parsed_transform
    };
    // Source vectorImage wraps the imported scene in the original image-box
    // viewport. Its bounds clip the imported scene's antialias-padded ink.
    let viewport_bounds = if let Some(placement) = placement {
        let [x, y, w, h] = placement.image_box;
        let bounds = point_bounds(
            [
                Point64 { x, y },
                Point64 { x: x + w, y },
                Point64 { x: x + w, y: y + h },
                Point64 { x, y: y + h },
            ]
            .into_iter(),
            matrix([s, 0., 0., s, 0., 0.], placement.matrix),
            0.,
            width,
            height,
        );
        let Some(bounds) = bounds else {
            return Some(true);
        };
        Some(bounds)
    } else {
        None
    };
    let mut prepared = vec![None; native.vector.scene.nodes.len()];
    prepare_bounds(
        &native.vector.scene,
        native.vector.scene.root?,
        transform,
        width,
        height,
        &mut prepared,
    );
    // Source vectorImage intersects imported ink and viewport bounds before
    // renderNode allocates its clipped offscreen layer. Float32 scanline
    // coordinates are relative to that visible layer, not the image-box corner.
    let Some(ink_bounds) = prepared[native.vector.scene.root?] else {
        return Some(true);
    };
    let visible_bounds = if let Some(bounds) = viewport_bounds {
        let visible = [
            bounds[0].max(ink_bounds[0]),
            bounds[1].max(ink_bounds[1]),
            bounds[2].min(ink_bounds[2]),
            bounds[3].min(ink_bounds[3]),
        ];
        if visible[0] >= visible[2] || visible[1] >= visible[3] {
            return Some(true);
        }
        visible
    } else {
        ink_bounds
    };
    let viewport_origin = viewport_bounds.map_or(Point64 { x: 0., y: 0. }, |_| Point64 {
        x: visible_bounds[0],
        y: visible_bounds[1],
    });
    let mut layer = vec![0u8; pixels.len()];
    if !render(
        &native.vector.scene,
        native.vector.scene.root?,
        transform,
        native.resources,
        native.jpeg_pngs,
        &mut layer,
        width,
        height,
        viewport_origin,
        &prepared,
    ) {
        return Some(false);
    }
    // Source imageBoxClip is an implicit, device-space four-edge NonZero
    // clip. AspectSlice may paint beyond the image box even when the imported
    // asset has its own intrinsic clip. Apply the outer viewport after rendering
    // all imported children, before the image's opacity and source-over.
    let viewport_mask = placement.map(|placement| {
        let [x, y, w, h] = placement.image_box;
        let m = affine(matrix([s, 0., 0., s, 0., 0.], placement.matrix));
        let points: Vec<_> = [
            Point64 { x, y },
            Point64 { x: x + w, y },
            Point64 { x: x + w, y: y + h },
            Point64 { x, y: y + h },
        ]
        .into_iter()
        .map(|point| map_point(point, m))
        .collect();
        let mut mask = vec![0u8; pixels.len()];
        let mut raster = Rasterizer::new(width, height, 0.375);
        raster.set_reference_origin(viewport_origin);
        raster.add_fill_polyline(&points, true);
        raster.paint(&mut mask, [255; 4]);
        mask
    });
    let alpha = (f64::from(opacity) * 255.).round() as u8;
    for row in layer_rows(Some(visible_bounds), width, height) {
        for offset in (row.start..row.end).step_by(4) {
            let p = &mut layer[offset..offset + 4];
            if let Some(mask) = &viewport_mask {
                scale_premultiplied(p, mask[offset + 3]);
            }
            scale_premultiplied(p, alpha);
            blend_premultiplied_pixel(&mut pixels[offset..offset + 4], p.try_into().unwrap());
        }
    }
    Some(true)
}

#[path = "owned_solid_primitive.rs"]
mod owned_solid_primitive;
use owned_solid_primitive::draw_solid;

fn union(a: Option<[f64; 4]>, b: Option<[f64; 4]>) -> Option<[f64; 4]> {
    match (a, b) {
        (Some(a), Some(b)) => Some([
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]),
        (a, None) => a,
        (None, b) => b,
    }
}
fn point_bounds(
    points: impl Iterator<Item = Point64>,
    m: [f64; 6],
    pad: f64,
    width: usize,
    height: usize,
) -> Option<[f64; 4]> {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for p in points {
        let p = map_point(p, affine(m));
        b = [b[0].min(p.x), b[1].min(p.y), b[2].max(p.x), b[3].max(p.y)];
    }
    if !b[0].is_finite() {
        return None;
    }
    let b = [
        (b[0] - pad).floor().max(0.),
        (b[1] - pad).floor().max(0.),
        (b[2] + pad).ceil().min(width as f64),
        (b[3] + pad).ceil().min(height as f64),
    ];
    (b[0] < b[2] && b[1] < b[3]).then_some(b)
}
fn prepare_bounds(
    scene: &Scene,
    index: usize,
    parent: [f64; 6],
    width: usize,
    height: usize,
    out: &mut [Option<[f64; 4]>],
) -> Option<[f64; 4]> {
    let n = &scene.nodes[index];
    let m = matrix(parent, n.transform);
    let mut b = None;
    if let Some(p) = &n.primitive {
        b = if p.geometry.kind == 4 {
            let [x, y, w, h] = p.geometry.bounds;
            point_bounds(
                [
                    Point64 { x, y },
                    Point64 { x: x + w, y },
                    Point64 { x: x + w, y: y + h },
                    Point64 { x, y: y + h },
                ]
                .into_iter(),
                m,
                0.,
                width,
                height,
            )
        } else {
            owned_solid_primitive::bounds(p, m, width, height)
        };
    }
    for &child in &n.children {
        b = union(b, prepare_bounds(scene, child, m, width, height, out));
    }
    if let Some(clip) = &n.clip {
        let m = matrix(m, clip.transform);
        let paths = collect_raw_subpaths(
            &commands(&clip.commands),
            flatten_tolerance64(affine(m)),
            affine(m),
        );
        let c = paths.and_then(|paths| {
            point_bounds(paths.into_iter().flat_map(|p| p.0), m, 1., width, height)
        });
        b = match (b, c) {
            (Some(b), Some(c)) => {
                let b = [
                    b[0].max(c[0]),
                    b[1].max(c[1]),
                    b[2].min(c[2]),
                    b[3].min(c[3]),
                ];
                (b[0] < b[2] && b[1] < b[3]).then_some(b)
            }
            _ => None,
        };
    }
    out[index] = b;
    b
}

#[path = "owned_gradient_paint.rs"]
mod owned_gradient_paint;

fn draw_content(
    scene: &Scene,
    index: usize,
    transform: [f64; 6],
    resources: &std::collections::BTreeMap<String, Option<d2_rs_math::svg_image::RasterAsset>>,
    jpeg_pngs: &std::collections::BTreeMap<String, Vec<u8>>,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    origin: Point64,
    prepared: &[Option<[f64; 4]>],
) -> bool {
    let node = &scene.nodes[index];
    if let Some(p) = &node.primitive {
        let g = &p.geometry;
        if g.kind != 4 {
            if !draw_solid(p, affine(transform), pixels, width, height, origin) {
                return false;
            }
        } else {
            let Some(Some(asset)) = resources.get(&g.asset) else {
                return false;
            };
            let names = [
                "none", "xMinYMin", "xMidYMin", "xMaxYMin", "xMinYMid", "xMidYMid", "xMaxYMid",
                "xMinYMax", "xMidYMax", "xMaxYMax",
            ];
            let Some(name) = names.get(g.aspect[0] as usize) else {
                return false;
            };
            let raw = format!("{name} {}", if g.aspect[1] == 1 { "slice" } else { "meet" });
            let Some(aspect) = crate::png_image::ImageAspect::parse(Some(&raw)) else {
                return false;
            };
            let placement = crate::svg_precision::SceneImagePlacement {
                matrix: transform,
                image_box: g.bounds,
                aspect,
            };
            let normalized;
            let data = if asset.mime == "image/gif" {
                let Ok(bytes) = crate::first_gif_frame_png(&asset.data) else {
                    return false;
                };
                normalized = bytes;
                normalized.as_slice()
            } else if asset.mime == "image/jpeg" {
                let Some(bytes) = jpeg_pngs.get(&g.asset) else {
                    return false;
                };
                bytes.as_slice()
            } else if asset.mime == "image/webp" {
                let Ok(Some(bytes)) = owned_webp_png(&asset.data) else {
                    return false;
                };
                normalized = bytes;
                normalized.as_slice()
            } else {
                asset.data.as_slice()
            };
            if !crate::png_image::draw_scene(data, pixels, width, height, &placement) {
                return false;
            }
        }
    }
    for &child in &node.children {
        if !render(
            scene, child, transform, resources, jpeg_pngs, pixels, width, height, origin, prepared,
        ) {
            return false;
        }
    }
    true
}

// Source x/image/webp preserves VP8 4:2:0 planes; image/color.YCbCr.RGBA
// converts nearest chroma to sixteen-bit channels before raster interpolation.
pub(crate) fn owned_webp_png(data: &[u8]) -> Result<Option<Vec<u8>>, String> {
    use std::io::{BufReader, Cursor};
    let mut decoder = image_webp::WebPDecoder::new(BufReader::new(Cursor::new(data)))
        .map_err(|e| e.to_string())?;
    if !decoder.is_lossy() {
        return crate::webp_frame_png(data, false);
    }
    let mut offset = 12usize;
    let mut vp8 = None;
    while offset.checked_add(8).is_some_and(|end| end <= data.len()) {
        let size = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let start = offset + 8;
        let end = start
            .checked_add(size)
            .filter(|end| *end <= data.len())
            .ok_or_else(|| "truncated WebP chunk".to_owned())?;
        if &data[offset..offset + 4] == b"VP8 " {
            vp8 = Some(&data[start..end]);
            break;
        }
        offset = end
            .checked_add(size & 1)
            .ok_or_else(|| "WebP chunk overflow".to_owned())?;
    }
    let frame = image_webp::vp8::Vp8Decoder::decode_frame(Cursor::new(
        vp8.ok_or_else(|| "missing VP8 chunk".to_owned())?,
    ))
    .map_err(|e| e.to_string())?;
    let (width, height) = decoder.dimensions();
    if width != u32::from(frame.width) || height != u32::from(frame.height) {
        return Err("WebP frame dimensions differ".into());
    }
    // The existing decoder handles compressed alpha and all four alpha filters.
    // Its RGB conversion is deliberately unused.
    let alpha = if decoder.has_alpha() {
        let mut bytes = vec![0; decoder.output_buffer_size().ok_or("WebP size overflow")?];
        decoder.read_image(&mut bytes).map_err(|e| e.to_string())?;
        Some(bytes)
    } else {
        None
    };
    let stride = (width as usize).div_ceil(16) * 16;
    let chroma_stride = stride / 2;
    let mut bytes = Vec::with_capacity(width as usize * height as usize * 8);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let yy = i32::from(frame.ybuf[y * stride + x]) * 0x10101;
            let ci = (y / 2) * chroma_stride + x / 2;
            let cb = i32::from(frame.ubuf[ci]) - 128;
            let cr = i32::from(frame.vbuf[ci]) - 128;
            let channels = [
                ((yy + 91881 * cr) >> 8).clamp(0, 65535) as u16,
                ((yy - 22554 * cb - 46802 * cr) >> 8).clamp(0, 65535) as u16,
                ((yy + 116130 * cb) >> 8).clamp(0, 65535) as u16,
                alpha.as_ref().map_or(65535, |a| {
                    u16::from(a[(y * width as usize + x) * 4 + 3]) * 257
                }),
            ];
            for channel in channels {
                bytes.extend_from_slice(&channel.to_be_bytes());
            }
        }
    }
    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Sixteen);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&bytes).map_err(|e| e.to_string())?;
    drop(writer);
    Ok(Some(output))
}

#[cfg(test)]
#[path = "owned_png_scene_tests.rs"]
mod tests;
