// SPDX-License-Identifier: MPL-2.0
//! Rust side of the fontface scoreboard; mirrors tools/fontface-probe.
//!
//! usage: fontface-probe [<ttf dir> <Noto .ttf.br>] < requests.jsonl > records.jsonl

use d2_rs_fontface::sfnt::{self, Hinting, Segment};
use d2_rs_fontface::{
    COLRv1ColorLine, COLRv1Paint, Error, Nrgba, ParsedFace, ShapeFace, ShapeLimits,
    ShapingWorkspace, fixed, register_bundled_face, registered_bundled_face,
    registered_bundled_noto_color_emoji,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

const NOTO: &str = "NotoColorEmoji";

fn load(name: &str, ttf_dir: &Path, noto: &Path) -> Result<ParsedFace, String> {
    if name == NOTO {
        let compressed = std::fs::read(noto).map_err(|e| e.to_string())?;
        let mut data = Vec::new();
        brotli_decompressor::Decompressor::new(&compressed[..], 4096)
            .read_to_end(&mut data)
            .map_err(|e| e.to_string())?;
        let source = registered_bundled_noto_color_emoji(&data, 0)
            .map_err(|e| format!("register bundled Noto Color Emoji: true {e}"))?
            .ok_or("register bundled Noto Color Emoji: false <nil>")?;
        return source.clone_read_only().map_err(|e| e.to_string());
    }
    let data = std::fs::read(ttf_dir.join(format!("{name}.ttf"))).map_err(|e| e.to_string())?;
    let digest: [u8; 32] = Sha256::digest(&data).into();
    register_bundled_face(&data, digest).map_err(|e| e.to_string())?;
    let source = registered_bundled_face(&data, 0)
        .map_err(|e| format!("registered bundled face {name}: true {e}"))?
        .ok_or_else(|| format!("registered bundled face {name}: false <nil>"))?;
    source.clone_read_only().map_err(|e| e.to_string())
}

fn err_text<T>(result: &Result<T, Error>) -> Value {
    match result {
        Ok(_) => Value::Null,
        Err(error) => json!(error.to_string()),
    }
}

fn sfnt_err<T>(result: &Result<T, sfnt::Error>) -> Value {
    match result {
        Ok(_) => Value::Null,
        Err(error) => json!(error.to_string()),
    }
}

fn segment_rows(segments: &[Segment]) -> Vec<Value> {
    segments
        .iter()
        .map(|s| {
            json!([
                s.op as u32,
                s.args[0].x,
                s.args[0].y,
                s.args[1].x,
                s.args[1].y,
                s.args[2].x,
                s.args[2].y
            ])
        })
        .collect()
}

fn rect(r: &fixed::Rectangle26_6) -> Value {
    json!([r.min.x, r.min.y, r.max.x, r.max.y])
}

fn color(c: Nrgba) -> Value {
    json!([c.r, c.g, c.b, c.a])
}

fn line(line: &COLRv1ColorLine) -> Value {
    Value::Array(
        line.stops
            .iter()
            .map(|s| json!([s.offset, color(s.color)]))
            .collect(),
    )
}

fn paint(p: &COLRv1Paint) -> Value {
    match p {
        COLRv1Paint::Layers(l) => json!({"layers": l.paints.iter().map(paint).collect::<Vec<_>>()}),
        COLRv1Paint::Solid(s) => json!({"solid": color(s.color)}),
        COLRv1Paint::LinearGradient(g) => {
            json!({"linear": [line(&g.color_line), g.x0, g.y0, g.x1, g.y1, g.x2, g.y2]})
        }
        COLRv1Paint::RadialGradient(g) => json!({"radial": [line(&g.color_line), g.x0, g.y0,
            g.radius0, g.x1, g.y1, g.radius1]}),
        COLRv1Paint::Glyph(g) => json!({"glyph": g.glyph_id, "paint": paint(&g.paint)}),
        COLRv1Paint::Transform(t) => {
            let m = t.matrix;
            json!({"transform": [m.xx, m.yx, m.xy, m.yy, m.dx, m.dy], "paint": paint(&t.paint)})
        }
        COLRv1Paint::Composite(c) => json!({"composite": c.mode as u8, "source": paint(&c.source),
            "backdrop": paint(&c.backdrop)}),
    }
}

fn glyph_values(face: &ParsedFace, glyph: u32) -> Value {
    let mut values = serde_json::Map::new();
    let segments = face.outline().load_glyph(glyph as u16, fixed::i(64));
    values.insert(
        "segments".into(),
        segments
            .as_ref()
            .map_or(Value::Null, |s| json!(segment_rows(s))),
    );
    values.insert("segmentsError".into(), sfnt_err(&segments));
    let advance = face
        .outline()
        .glyph_advance(glyph as u16, fixed::i(64), Hinting::None);
    values.insert(
        "advance".into(),
        json!(advance.as_ref().copied().unwrap_or(0)),
    );
    values.insert("advanceError".into(), sfnt_err(&advance));
    for size in [fixed::i(64), fixed::i(15)] {
        let bounds = face.glyph_render_bounds(glyph, size);
        let (r, ink) = bounds.as_ref().copied().unwrap_or_default();
        values.insert(
            format!("bounds{size}"),
            json!([rect(&r), ink, err_text(&bounds)]),
        );
    }
    let layers = face.colr0_glyph_layers(glyph);
    let rows: Vec<Value> = layers
        .as_ref()
        .ok()
        .and_then(|l| l.as_ref())
        .map(|l| {
            l.iter()
                .map(|l| json!([l.glyph_id, color(l.color), l.foreground]))
                .collect()
        })
        .unwrap_or_default();
    values.insert(
        "colr0".into(),
        json!([rows, matches!(&layers, Ok(Some(_))), err_text(&layers)]),
    );
    if face.is_bundled_noto_color_emoji() {
        let plan = face.compile_bundled_noto_color_emoji_colrv1_plan(glyph);
        let plan_json = match &plan {
            Ok(Some(plan)) => json!({
                "root": paint(&plan.root),
                "clip": plan.clip.map(|c| json!([c.x_min, c.y_min, c.x_max, c.y_max])),
                "usage": [plan.usage.paint_nodes, plan.usage.max_depth, plan.usage.max_layers,
                    plan.usage.max_gradient_stops],
            }),
            _ => Value::Null,
        };
        values.insert(
            "plan".into(),
            json!([plan_json, matches!(plan, Ok(Some(_))), err_text(&plan)]),
        );
    }
    Value::Object(values)
}

fn font_values(face: &ParsedFace) -> Value {
    let outline = face.outline();
    let mut values = serde_json::Map::new();
    values.insert("numGlyphs".into(), json!(outline.num_glyphs()));
    values.insert("unitsPerEm".into(), json!(outline.units_per_em()));
    for size in [fixed::i(64), fixed::i(15), 1000] {
        for (index, hinting) in [(0, Hinting::None), (2, Hinting::Full)] {
            let m = outline.metrics(size, hinting);
            values.insert(
                format!("metrics{size}/{index}"),
                json!([
                    [
                        m.height,
                        m.ascent,
                        m.descent,
                        m.x_height,
                        m.cap_height,
                        m.caret_slope.0,
                        m.caret_slope.1
                    ],
                    null
                ]),
            );
            values.insert(
                format!("bounds{size}/{index}"),
                json!([rect(&outline.bounds(size, hinting)), null]),
            );
        }
    }
    let mut runes = Vec::new();
    for r in (0u32..0x3000).filter_map(char::from_u32) {
        let g = outline.glyph_index(r);
        if g.as_ref().is_ok_and(|&g| g != 0) || g.is_err() {
            runes.push(json!([
                u32::from(r),
                g.as_ref().copied().unwrap_or(0),
                sfnt_err(&g)
            ]));
        }
    }
    values.insert("cmap".into(), Value::Array(runes));
    let mut supported = Vec::new();
    for r in (0x20u32..0x2800).filter_map(char::from_u32) {
        let ok = face.supports_renderable_rune(r);
        if ok.as_ref().is_ok_and(|&ok| ok) || ok.is_err() {
            supported.push(json!([
                u32::from(r),
                ok.as_ref().copied().unwrap_or(false),
                err_text(&ok)
            ]));
        }
    }
    values.insert("supports".into(), Value::Array(supported));
    let mut kerns = Vec::new();
    for a in 0x21u8..0x7f {
        for b in 0x21u8..0x7f {
            let ga = outline.glyph_index(char::from(a)).unwrap_or(0);
            let gb = outline.glyph_index(char::from(b)).unwrap_or(0);
            let k = outline.kern(ga, gb, fixed::i(64), Hinting::None);
            let kern = k.as_ref().copied().unwrap_or(0);
            if kern != 0 || matches!(k, Err(e) if e != sfnt::Error::NotFound) {
                kerns.push(json!([a, b, kern, sfnt_err(&k)]));
            }
        }
    }
    values.insert("kern".into(), Value::Array(kerns));
    Value::Object(values)
}

fn main() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (ttf_dir, noto) = match args.as_slice() {
        [ttf, noto] => (PathBuf::from(ttf), PathBuf::from(noto)),
        [] => (
            manifest.join("../d2-rs-text/assets"),
            manifest.join("../d2-rs-export/assets/NotoColorEmoji-COLRv1-v2.051.ttf.br"),
        ),
        _ => {
            eprintln!("usage: fontface-probe [<ttf dir> <Noto .ttf.br>]");
            std::process::exit(2);
        }
    };
    let mut faces: HashMap<String, ParsedFace> = HashMap::new();
    let limits = ShapeLimits {
        runes: 100_000,
        faces: 64,
        coverage_checks: 50_000_000,
        runs: 100_000,
        glyphs: 10_000_000,
    };
    let mut workspace = ShapingWorkspace::new();
    let mut seen = HashSet::new();
    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    let mut emit = |value: Value| {
        serde_json::to_writer(&mut out, &value).unwrap();
        out.write_all(b"\n").unwrap();
    };
    for line in std::io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let key = request["key"].as_str().unwrap();
        let kind = request["kind"].as_str().unwrap_or("");
        if kind == "glyphs" || kind == "font" {
            let name = request["font"].as_str().unwrap();
            if !faces.contains_key(name) {
                match load(name, &ttf_dir, &noto) {
                    Ok(face) => {
                        faces.insert(name.to_owned(), face);
                    }
                    Err(error) => {
                        emit(json!({"key": key, "error": error}));
                        continue;
                    }
                }
            }
            let face = &faces[name];
            if kind == "glyphs" {
                for glyph in 0..face.outline().num_glyphs() {
                    emit(
                        json!({"key": format!("{key}:{glyph}"), "values": glyph_values(face, glyph as u32)}),
                    );
                }
            } else {
                emit(json!({"key": key, "values": font_values(face)}));
            }
            continue;
        }
        let mut names = vec![request["font"].as_str().unwrap().to_owned()];
        if request["fallback"].as_bool().unwrap() {
            names.push(NOTO.to_owned());
        }
        let mut load_error = None;
        for name in &names {
            if !faces.contains_key(name) {
                match load(name, &ttf_dir, &noto) {
                    Ok(face) => {
                        faces.insert(name.clone(), face);
                    }
                    Err(error) => {
                        load_error = Some(error);
                        break;
                    }
                }
            }
        }
        if let Some(error) = load_error {
            emit(json!({"key": key, "error": error}));
            continue;
        }
        let shape_faces: Vec<ShapeFace<'_>> = names
            .iter()
            .map(|name| ShapeFace {
                id: name,
                face: &faces[name],
            })
            .collect();
        let size = request["size"].as_i64().unwrap() as i32;
        let shaped = match workspace.shape_text_transient(
            &(),
            request["text"].as_str().unwrap(),
            size,
            &shape_faces,
            limits,
        ) {
            Ok(shaped) => shaped,
            Err(error) => {
                emit(json!({"key": key, "error": error.to_string()}));
                continue;
            }
        };
        let glyphs: Vec<Value> = shaped
            .glyphs
            .iter()
            .map(|g| {
                json!({
                    "id": g.id, "face": g.face, "x": g.position_x, "y": g.position_y,
                    "advance": g.advance,
                    "ink": [g.ink.min.x, g.ink.min.y, g.ink.max.x, g.ink.max.y],
                    "hasInk": g.has_ink, "empty": g.empty, "source": g.source,
                    "sourceIndex": g.source_index,
                })
            })
            .collect();
        emit(json!({"key": key, "shaped": {
            "glyphs": glyphs, "advance": shaped.advance, "runes": shaped.runes,
            "coverageChecks": shaped.coverage_checks, "runs": shaped.runs,
        }}));
        for g in &shaped.glyphs {
            if !g.has_ink {
                continue;
            }
            let name = &names[g.face];
            let key = format!("outline:{name}:{}", g.id);
            if !seen.insert(key.clone()) {
                continue;
            }
            match shape_faces[g.face]
                .face
                .outline()
                .load_glyph(g.id as u16, fixed::i(64))
            {
                Err(error) => emit(json!({"key": key, "error": error.to_string()})),
                Ok(segments) if segments.is_empty() => emit(json!({"key": key})),
                Ok(segments) => emit(json!({"key": key, "segments": segment_rows(&segments)})),
            }
        }
    }
}
