// SPDX-License-Identifier: MPL-2.0
//! timeline.go and framing.go timelineCamera: ordered boards, one camera/palette.
use super::{
    context::{RenderContext, RenderError},
    framing::ProjectedExtent,
    gif::{BoundedWriter, cycle_frame_delay, frame_delta, index_frame, palette_lookup},
    gif_capture::{Capture, MAX_ANIMATION_PIXELS},
    render::{Options, build_native_scene_mode, normalize},
    svg::MAX_OUTPUT_BYTES,
};
use crate::{
    scene::{MAX_EDGES, MAX_NODES, build_scene},
    target::Diagram,
};
use d2_rs_core::CancellationToken;
use std::borrow::Cow;

pub const MAX_TIMELINE_BOARDS: usize = 128;
const MAX_TIMELINE_FRAMES: usize = 1000;

/// Render ordered boards to a looping GIF. The interval is signed nanoseconds,
/// preserving Go's duration admission. Source boards and tracks stay unchanged.
pub fn render_timeline(
    token: Option<&CancellationToken>,
    boards: &[Option<Diagram>],
    options: Option<&Options>,
    interval_ns: i64,
) -> Result<Vec<u8>, RenderError> {
    let mut o = normalize(options)?;
    if o.format != "gif"
        || boards.is_empty()
        || boards.len() > MAX_TIMELINE_BOARDS
        || interval_ns <= 0
        || interval_ns > 600_000_000_000
    {
        return Err(format!("isometric timeline requires GIF, 1..{MAX_TIMELINE_BOARDS} boards, and an interval in (0,10m]").into());
    }
    let context = RenderContext::new(token, o.timeout_ns);
    let interval = interval_ns as f64 / 1e9;
    let mut counts = Vec::with_capacity(boards.len());
    for (i, board) in boards.iter().enumerate() {
        context.check()?;
        let board = board
            .as_ref()
            .filter(|d| !d.is_folder_only)
            .ok_or_else(|| format!("isometric timeline board {i} has no drawable diagram"))?;
        if board.shapes().len() > MAX_NODES
            || board.connections().len() > MAX_EDGES
            || board.legend.as_ref().is_some_and(|l| {
                l.shapes.as_ref().map_or(0, Vec::len) > MAX_NODES
                    || l.connections.as_ref().map_or(0, Vec::len) > MAX_EDGES
            })
        {
            return Err(format!(
                "isometric timeline board {i} exceeds the source shape/connection limit"
            )
            .into());
        }
        counts.push(if timeline_board_animated(board) {
            (interval * 12.0).ceil().max(1.0) as usize
        } else {
            1
        });
    }
    let total: usize = counts.iter().sum();
    if total > MAX_TIMELINE_FRAMES
        || (total + 4 * boards.len()) as i64 * o.width * o.height > MAX_ANIMATION_PIXELS
    {
        return Err("isometric timeline exceeds frame/pixel budget; reduce --scale, interval, or board count".to_string().into());
    }
    let camera = timeline_camera(&context, boards, &o)
        .map_err(|e| context.error(format!("isometric timeline framing: {e}")))?;
    o.width = camera.width;
    o.height = camera.height;
    // At most one board's native scene is live during each sampling/render pass.
    let mut samples = vec![0; 128 * 128 * boards.len() * 4];
    for (i, board) in boards.iter().enumerate() {
        context.check()?;
        let capture = Capture::open_in_context(context.clone(), board.as_ref(), &o, Some(camera))
            .map_err(|e| context.error(format!("isometric timeline board {i}: {e}")))?;
        for phase in 0..4 {
            let seconds = phase as f64 * interval / 4.0;
            let frame = capture.frame_image_at(seconds, seconds, true)?;
            for y in 0..64 {
                for x in 0..64 {
                    let source =
                        (y * frame.height() / 64) * frame.stride + (x * frame.width() / 64) * 4;
                    let target = ((i * 128 + phase / 2 * 64 + y) * 128 + phase % 2 * 64 + x) * 4;
                    samples[target..target + 4].copy_from_slice(&frame.pix[source..source + 4]);
                }
            }
        }
    }
    let palette = d2_rs_export::gif_median_cut_palette(&samples);
    let lookup = palette_lookup(&context, &palette)?;
    let channels: Vec<_> = palette
        .iter()
        .flat_map(|c| c[..3].iter().copied())
        .collect();
    let mut writer = BoundedWriter {
        context: &context,
        bytes: Vec::new(),
        remaining: MAX_OUTPUT_BYTES,
    };
    {
        let mut encoder =
            gif::Encoder::new(&mut writer, o.width as u16, o.height as u16, &channels)
                .map_err(|e| context.error(e.to_string()))?;
        // image/gif writer.go writeHeader emits NETSCAPE2.0 only for
        // multiple frames. A single held board has no loop extension.
        if total > 1 {
            encoder
                .set_repeat(gif::Repeat::Infinite)
                .map_err(|e| context.error(e.to_string()))?;
        }
        let delay = (interval_ns as f64 / 10_000_000.0).round().max(1.0) as usize;
        let mut previous: Option<Vec<u8>> = None;
        for (board, &count) in boards.iter().zip(&counts) {
            let capture =
                Capture::open_in_context(context.clone(), board.as_ref(), &o, Some(camera))?;
            for i in 0..count {
                let seconds = i as f64 * interval / count as f64;
                let frame = capture.frame_image_at(seconds, seconds, true)?;
                let indexed = index_frame(&context, &frame, &lookup)?;
                let (left, top, width, height, pixels) = match &previous {
                    Some(before) => frame_delta(before, &indexed, frame.width(), frame.height()),
                    None => (0, 0, frame.width(), frame.height(), indexed.clone()),
                };
                encoder
                    .write_frame(&gif::Frame {
                        left: left as u16,
                        top: top as u16,
                        width: width as u16,
                        height: height as u16,
                        delay: cycle_frame_delay(i, count, delay),
                        dispose: gif::DisposalMethod::Keep,
                        buffer: Cow::Owned(pixels),
                        ..Default::default()
                    })
                    .map_err(|e| context.error(e.to_string()))?;
                previous = Some(indexed);
            }
        }
        encoder
            .into_inner()
            .map_err(|e| context.error(e.to_string()))?;
    }
    context.check()?;
    Ok(writer.bytes)
}

pub fn timeline_board_animated(board: &Diagram) -> bool {
    let edge = |e: &crate::target::Connection| e.animated && e.opacity > 0.0 && e.stroke_width > 0;
    let shape = |s: &crate::target::Shape| s.animated && s.opacity > 0.0;
    board.connections().iter().any(edge)
        || board.shapes().iter().any(shape)
        || shape(&board.root)
        || board.legend.as_ref().is_some_and(|l| {
            l.connections
                .as_deref()
                .unwrap_or_default()
                .iter()
                .any(edge)
                || l.shapes.as_deref().unwrap_or_default().iter().any(shape)
        })
}

fn timeline_camera(
    context: &RenderContext,
    boards: &[Option<Diagram>],
    o: &Options,
) -> Result<super::mesh::Camera, RenderError> {
    let mut bounds = ProjectedExtent::default();
    let mut measure = o.clone();
    measure.fit_content = false;
    for board in boards {
        context.check()?;
        let source = build_scene(board.as_ref().unwrap(), Some(&o.render))?;
        let native = build_native_scene_mode(&source, o.width, o.height, &measure, false, context)
            .map_err(|e| context.error(e))?;
        bounds.animated_mesh(
            &native.triangles,
            &native.animated_nodes,
            native.pixel_scale,
        );
        bounds.traffic_routes(&native.packets);
    }
    Ok(bounds.camera(o.width, o.height, o.fit_content))
}
