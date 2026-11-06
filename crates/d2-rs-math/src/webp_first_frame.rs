// SPDX-License-Identifier: MPL-2.0
//! D2 internal/rasterimage/firstframe.go inspectWebP and extractWebPFrame:
//! the WebP container pass behind imageasset's raster Config and
//! DecodeFirst. Animated WebP resources resolve to their first ANMF frame
//! on the logical canvas; later frames are never decoded.
use d2_rs_core::{CancellationToken, LayoutError};

fn context(c: Option<&CancellationToken>) -> Result<(), String> {
    match c.and_then(CancellationToken::error) {
        Some(LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}

/// firstframe.go animationPlan for a WebP resource.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebPPlan {
    /// The VP8X logical canvas, when the container declares one.
    pub canvas: Option<(usize, usize)>,
    pub animated: bool,
    /// The first frame's canvas rectangle `(x0, y0, x1, y1)`.
    pub frame: (usize, usize, usize, usize),
    /// ANMF "do not blend": the frame replaces the canvas (draw.Src).
    pub source_blend: bool,
    /// The first frame as a standalone static WebP, when materialized.
    pub first_frame: Vec<u8>,
}

fn read_u24(data: &[u8]) -> usize {
    usize::from(data[0]) | usize::from(data[1]) << 8 | usize::from(data[2]) << 16
}

fn write_u24(destination: &mut [u8], value: usize) {
    destination[0] = value as u8;
    destination[1] = (value >> 8) as u8;
    destination[2] = (value >> 16) as u8;
}

/// webPChunk: the chunk type, payload, and padded end offset.
fn chunk(data: &[u8], offset: usize, end: usize) -> Result<(&[u8], &[u8], usize), String> {
    if end - offset < 8 {
        return Err("truncated WebP chunk header".into());
    }
    let length = u64::from(u32::from_le_bytes(
        data[offset + 4..offset + 8].try_into().unwrap(),
    ));
    let padded = length + length % 2;
    let chunk_end = offset as u64 + 8 + padded;
    if chunk_end > end as u64 {
        return Err("truncated WebP chunk data".into());
    }
    let start = offset + 8;
    Ok((
        &data[offset..offset + 4],
        &data[start..start + length as usize],
        chunk_end as usize,
    ))
}

fn append_chunk(destination: &mut Vec<u8>, kind: &[u8], payload: &[u8]) {
    destination.extend_from_slice(kind);
    destination.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    destination.extend_from_slice(payload);
    if !payload.len().is_multiple_of(2) {
        destination.push(0);
    }
}

/// inspectWebP.
pub fn inspect(
    data: &[u8],
    materialize: bool,
    cancel: Option<&CancellationToken>,
) -> Result<WebPPlan, String> {
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WEBP" {
        return Err("invalid WebP RIFF header".into());
    }
    let declared_end = u64::from(u32::from_le_bytes(data[4..8].try_into().unwrap())) + 8;
    if declared_end != data.len() as u64 || declared_end < 12 {
        return Err("invalid WebP RIFF length".into());
    }
    let end = declared_end as usize;
    let mut plan = WebPPlan::default();
    let (mut saw_vp8x, mut animation_flag) = (false, false);
    let mut anim_chunks = 0;
    let mut anim_payload_length = None;
    let mut first_anmf: Option<&[u8]> = None;
    let mut image_chunks = 0;
    let mut offset = 12;
    while offset < end {
        context(cancel)?;
        let (kind, payload, next) = chunk(data, offset, end)?;
        match kind {
            b"VP8X" => {
                if saw_vp8x || payload.len() != 10 {
                    return Err("invalid WebP VP8X chunk".into());
                }
                saw_vp8x = true;
                animation_flag = payload[0] & 0x02 != 0;
                let width = read_u24(&payload[4..7]) + 1;
                let height = read_u24(&payload[7..10]) + 1;
                plan.canvas = Some((width, height));
            }
            b"ANIM" => {
                anim_chunks += 1;
                if anim_chunks == 1 {
                    anim_payload_length = Some(payload.len());
                }
            }
            b"ANMF" => {
                first_anmf.get_or_insert(payload);
            }
            b"VP8 " | b"VP8L" => image_chunks += 1,
            _ => {}
        }
        offset = next;
    }
    if animation_flag {
        let Some(first_anmf) =
            first_anmf.filter(|_| saw_vp8x && anim_chunks == 1 && anim_payload_length == Some(6))
        else {
            return Err("incomplete animated WebP container".into());
        };
        let (frame, source_blend, encoded) = extract_frame(first_anmf, materialize, cancel)?;
        let (canvas_width, canvas_height) = plan.canvas.unwrap_or_default();
        if frame.2 > canvas_width || frame.3 > canvas_height {
            return Err("WebP first frame falls outside its logical canvas".into());
        }
        plan.animated = true;
        plan.frame = frame;
        plan.source_blend = source_blend;
        plan.first_frame = encoded;
        return Ok(plan);
    }
    if image_chunks != 1 {
        return Err(format!(
            "expected exactly one static WebP image bitstream, got {image_chunks}"
        ));
    }
    Ok(plan)
}

type Frame = ((usize, usize, usize, usize), bool, Vec<u8>);

/// extractWebPFrame.
fn extract_frame(
    payload: &[u8],
    materialize: bool,
    cancel: Option<&CancellationToken>,
) -> Result<Frame, String> {
    if payload.len() < 16 {
        return Err("truncated WebP ANMF header".into());
    }
    let x = read_u24(&payload[0..3]) * 2;
    let y = read_u24(&payload[3..6]) * 2;
    let width = read_u24(&payload[6..9]) + 1;
    let height = read_u24(&payload[9..12]) + 1;
    let frame = (x, y, x + width, y + height);
    let source_blend = payload[15] & 0x02 != 0;
    let mut alpha: Option<&[u8]> = None;
    let mut image: Option<(&[u8], &[u8])> = None;
    let mut offset = 16;
    while offset < payload.len() {
        context(cancel)?;
        let (kind, chunk_payload, next) = chunk(payload, offset, payload.len())?;
        match kind {
            b"ALPH" => {
                if alpha.is_some() {
                    return Err("multiple WebP ALPH chunks in first frame".into());
                }
                alpha = Some(chunk_payload);
            }
            b"VP8 " | b"VP8L" => {
                if image.is_some() {
                    return Err("multiple WebP bitstreams in first frame".into());
                }
                image = Some((kind, chunk_payload));
            }
            // The WebP container permits future or application-specific chunks.
            _ => {}
        }
        offset = next;
    }
    let Some((image_kind, image_payload)) =
        image.filter(|(kind, _)| alpha.is_none() || *kind == b"VP8 ")
    else {
        return Err("invalid WebP first-frame bitstream".into());
    };
    let (bitstream_width, bitstream_height) = bitstream_dimensions(image_kind, image_payload)?;
    if bitstream_width != width || bitstream_height != height {
        return Err(format!(
            "WebP first-frame header is {width}x{height} but its bitstream is {bitstream_width}x{bitstream_height}"
        ));
    }
    if !materialize {
        return Ok((frame, source_blend, Vec::new()));
    }
    let mut chunks = Vec::new();
    if let Some(alpha) = alpha {
        let mut vp8x = [0_u8; 10];
        vp8x[0] = 0x10;
        write_u24(&mut vp8x[4..7], width - 1);
        write_u24(&mut vp8x[7..10], height - 1);
        append_chunk(&mut chunks, b"VP8X", &vp8x);
        append_chunk(&mut chunks, b"ALPH", alpha);
    }
    append_chunk(&mut chunks, image_kind, image_payload);
    let mut result = Vec::with_capacity(12 + chunks.len());
    result.extend_from_slice(b"RIFF");
    result.extend_from_slice(&((chunks.len() + 4) as u32).to_le_bytes());
    result.extend_from_slice(b"WEBP");
    result.extend_from_slice(&chunks);
    Ok((frame, source_blend, result))
}

/// webPBitstreamDimensions.
fn bitstream_dimensions(kind: &[u8], payload: &[u8]) -> Result<(usize, usize), String> {
    match kind {
        b"VP8 " => {
            if payload.len() < 10 || payload[3..6] != [0x9d, 0x01, 0x2a] {
                return Err("invalid WebP VP8 first-frame header".into());
            }
            Ok((
                usize::from(u16::from_le_bytes([payload[6], payload[7]]) & 0x3fff),
                usize::from(u16::from_le_bytes([payload[8], payload[9]]) & 0x3fff),
            ))
        }
        _ => {
            if payload.len() < 5 || payload[0] != 0x2f {
                return Err("invalid WebP VP8L first-frame header".into());
            }
            let bits = u32::from_le_bytes(payload[1..5].try_into().unwrap());
            Ok((
                (bits & 0x3fff) as usize + 1,
                ((bits >> 14) & 0x3fff) as usize + 1,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn container(chunks: &[(&[u8], Vec<u8>)]) -> Vec<u8> {
        let mut body = Vec::new();
        for (kind, payload) in chunks {
            append_chunk(&mut body, kind, payload);
        }
        let mut data = b"RIFF".to_vec();
        data.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
        data.extend_from_slice(b"WEBP");
        data.extend_from_slice(&body);
        data
    }

    fn vp8l(width: usize, height: usize) -> Vec<u8> {
        let bits = (width as u32 - 1) | (height as u32 - 1) << 14;
        let mut payload = vec![0x2f];
        payload.extend_from_slice(&bits.to_le_bytes());
        payload
    }

    fn animated(canvas: (usize, usize), x: usize, y: usize, flags: u8) -> Vec<u8> {
        let mut vp8x = vec![0x12, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        write_u24(&mut vp8x[4..7], canvas.0 - 1);
        write_u24(&mut vp8x[7..10], canvas.1 - 1);
        let mut anmf = vec![0; 16];
        write_u24(&mut anmf[0..3], x / 2);
        write_u24(&mut anmf[3..6], y / 2);
        write_u24(&mut anmf[6..9], 3 - 1);
        write_u24(&mut anmf[9..12], 2 - 1);
        anmf[15] = flags;
        append_chunk(&mut anmf, b"VP8L", &vp8l(3, 2));
        container(&[
            (b"VP8X", vp8x),
            (b"ANIM", vec![0; 6]),
            (b"ANMF", anmf.clone()),
            (b"ANMF", anmf),
        ])
    }

    #[test]
    fn animated_containers_resolve_their_first_frame_like_firstframe_go() {
        let plan = inspect(&animated((7, 5), 2, 2, 0x02), true, None).unwrap();
        assert!(plan.animated);
        assert_eq!(plan.canvas, Some((7, 5)));
        assert_eq!(plan.frame, (2, 2, 5, 4));
        assert!(plan.source_blend);
        assert_eq!(plan.first_frame, container(&[(b"VP8L", vp8l(3, 2))]));
        assert_eq!(
            inspect(&animated((4, 3), 2, 2, 0), false, None).unwrap_err(),
            "WebP first frame falls outside its logical canvas"
        );
    }

    #[test]
    fn static_containers_keep_firstframe_go_errors() {
        assert_eq!(
            inspect(b"RIFF\0\0\0\0WEBP", false, None).unwrap_err(),
            "invalid WebP RIFF length"
        );
        let two = container(&[(b"VP8L", vp8l(1, 1)), (b"VP8L", vp8l(1, 1))]);
        assert_eq!(
            inspect(&two, false, None).unwrap_err(),
            "expected exactly one static WebP image bitstream, got 2"
        );
        assert!(
            !inspect(&container(&[(b"VP8L", vp8l(1, 1))]), false, None)
                .unwrap()
                .animated
        );
    }
}
