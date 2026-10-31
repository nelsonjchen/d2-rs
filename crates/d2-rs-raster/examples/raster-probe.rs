// SPDX-License-Identifier: MPL-2.0
//! Rust side of `scripts/scene-scoreboard.py --tier raster`.
//!
//! Reads the scoreboard's JSON request lines on stdin. Each names `go`, the
//! Go probe's output directory for one board directory, and `out`. For every
//! `<name>.scene.json` there it renders Go's Document with tools/scene-probe's
//! frameOptions and writes `<out>/<name>.png`, or `<out>/<name>.err` with
//! the error text.

use std::io::BufRead;
use std::path::Path;

use d2_rs_raster::{FrameOptions, NrgbaImage, render};
use d2_rs_scene::{Document, Nrgba};

fn frame_options() -> FrameOptions {
    FrameOptions {
        scale: 1.0,
        time: 0,
        background: Some(Nrgba::new(255, 255, 255, 255)),
        max_width: 32_768,
        max_height: 32_768,
        max_pixels: 64 * 1024 * 1024,
        max_nodes: 1_000_000,
        max_depth: 1_024,
        max_path_commands: 10_000_000,
        max_text_runes_per_run: 100_000,
        max_font_faces_per_text: 64,
        max_text_coverage_checks: 10_000_000,
        max_text_shaping_runs: 100_000,
        max_animation_tracks: 1_000_000,
        max_animation_keyframes: 10_000_000,
        max_assets: 4_096,
        max_asset_bytes: 512 * 1024 * 1024,
        max_decoded_asset_bytes: 512 * 1024 * 1024,
        max_import_depth: 1_024,
        max_offscreen_bytes: 512 * 1024 * 1024,
        max_even_odd_clip_work: 250_000_000,
        max_scanline_work: 4_000_000_000,
    }
}

/// image/png's encoding choice for an NRGBA frame: 8-bit RGB when opaque.
fn encode_png(frame: &NrgbaImage) -> Vec<u8> {
    let (width, height) = (frame.width(), frame.height());
    let opaque = frame.pix.chunks_exact(4).all(|p| p[3] == 0xff);
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width as u32, height as u32);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_color(if opaque {
            png::ColorType::Rgb
        } else {
            png::ColorType::Rgba
        });
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().expect("png header");
        let data: Vec<u8> = if opaque {
            frame
                .pix
                .chunks_exact(4)
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect()
        } else {
            frame.pix.clone()
        };
        writer.write_image_data(&data).expect("png data");
    }
    out
}

fn render_dir(go: &Path, out: &Path) {
    let Ok(entries) = std::fs::read_dir(go) else {
        return;
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".scene.json"))
        .collect();
    names.sort();
    std::fs::create_dir_all(out).expect("create output directory");
    for name in names {
        let stem = name.trim_end_matches(".scene.json");
        // Inline-API captures retain the original test's resource ceilings
        // and background alongside the Document. Corpus captures use the
        // unchanged production profile above.
        let options_path = go.join(format!("{stem}.options.json"));
        let options = if options_path.is_file() {
            captured_options(&std::fs::read(options_path).expect("read options"))
        } else {
            frame_options()
        };
        let data = std::fs::read(go.join(&name)).expect("read scene");
        let mut document: Document = match serde_json::from_slice(&data) {
            Ok(document) => document,
            Err(err) => {
                std::fs::write(
                    out.join(format!("{stem}.err")),
                    format!("rust probe: decode scene: {err}"),
                )
                .unwrap();
                continue;
            }
        };
        document.load_asset_files().expect("load asset files");
        let started = std::time::Instant::now();
        match render(&document, &options) {
            Ok(frame) => {
                std::fs::write(out.join(format!("{stem}.png")), encode_png(&frame)).unwrap()
            }
            Err(err) => std::fs::write(out.join(format!("{stem}.err")), err.0).unwrap(),
        }
        if std::env::var_os("RASTER_PROBE_TIMING").is_some() {
            eprintln!("{} {:?}", go.join(&name).display(), started.elapsed());
        }
    }
}

fn captured_options(data: &[u8]) -> FrameOptions {
    let value: serde_json::Value = serde_json::from_slice(data).expect("decode options");
    let mut options = FrameOptions::default();
    options.scale = value["Scale"].as_f64().expect("Scale");
    options.time = value["Time"].as_i64().expect("Time");
    if !value["Background"].is_null() {
        options.background =
            Some(serde_json::from_value(value["Background"].clone()).expect("Background"));
    }
    macro_rules! limits {
        ($($field:ident => $key:literal),* $(,)?) => {$(options.$field = value[$key].as_i64().expect($key);)*};
    }
    limits! {
        max_width => "MaxWidth", max_height => "MaxHeight", max_pixels => "MaxPixels",
        max_nodes => "MaxNodes", max_depth => "MaxDepth", max_path_commands => "MaxPathCommands",
        max_text_runes_per_run => "MaxTextRunesPerRun", max_font_faces_per_text => "MaxFontFacesPerText",
        max_text_coverage_checks => "MaxTextCoverageChecks", max_text_shaping_runs => "MaxTextShapingRuns",
        max_animation_tracks => "MaxAnimationTracks", max_animation_keyframes => "MaxAnimationKeyframes",
        max_assets => "MaxAssets", max_asset_bytes => "MaxAssetBytes",
        max_decoded_asset_bytes => "MaxDecodedAssetBytes", max_import_depth => "MaxImportDepth",
        max_offscreen_bytes => "MaxOffscreenBytes", max_even_odd_clip_work => "MaxEvenOddClipWork",
        max_scanline_work => "MaxScanlineWork",
    }
    options
}

fn main() {
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line.expect("read request");
        if line.trim().is_empty() {
            continue;
        }
        let request: serde_json::Value = serde_json::from_str(&line).expect("request JSON");
        let go = request["go"].as_str().expect("go directory");
        let out = request["out"].as_str().expect("out directory");
        render_dir(Path::new(go), Path::new(out));
    }
}
