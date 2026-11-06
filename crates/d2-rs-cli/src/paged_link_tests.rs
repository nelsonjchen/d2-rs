// SPDX-License-Identifier: MPL-2.0
//! Original Go CLI PDF/PPTX navigation, metadata and complete frame contracts.
use super::*;
use serde_json::{Value, json};
use std::io::Read;

fn fixture() -> Value {
    serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/paged-links-v090.json"
    ))
    .unwrap()
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn scratch(name: &str) -> Scratch {
    let p = std::env::temp_dir().join(format!("d2-rs-paged-links-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&p).unwrap();
    Scratch(p)
}
fn text(z: &mut zip::ZipArchive<std::io::Cursor<Vec<u8>>>, path: &str) -> String {
    let mut value = String::new();
    z.by_name(path).unwrap().read_to_string(&mut value).unwrap();
    value
}
fn child<'a, 'i>(n: roxmltree::Node<'a, 'i>, name: &str) -> roxmltree::Node<'a, 'i> {
    n.children().find(|n| n.has_tag_name(name)).unwrap()
}
fn pptx_contract(bytes: Vec<u8>, count: usize) -> (Value, Value) {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut slides = Vec::new();
    let mut frames = Vec::new();
    let relns = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    assert_eq!(
        z.file_names()
            .filter(|n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml"))
            .count(),
        count
    );
    for i in 1..=count {
        let xml = text(&mut z, &format!("ppt/slides/slide{i}.xml"));
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let rel = text(&mut z, &format!("ppt/slides/_rels/slide{i}.xml.rels"));
        let rel = roxmltree::Document::parse(&rel).unwrap();
        let relationship = |id: &str| {
            rel.descendants()
                .find(|n| n.attribute("Id") == Some(id))
                .unwrap()
        };
        let mut links = Vec::new();
        let mut pictures = Vec::new();
        let mut images = Vec::new();
        for shape in doc.descendants().filter(|n| n.has_tag_name("sp")) {
            let nv = child(shape, "nvSpPr");
            let props = child(nv, "cNvPr");
            let Some(click) = props.children().find(|n| n.has_tag_name("hlinkClick")) else {
                continue;
            };
            let target = relationship(click.attribute((relns, "id")).unwrap());
            let xf = child(child(shape, "spPr"), "xfrm");
            let off = child(xf, "off");
            let ext = child(xf, "ext");
            links.push(json!({"destination":target.attribute("Target").unwrap(),"external":target.attribute("TargetMode")==Some("External"),"tooltip":click.attribute("tooltip").unwrap_or(""),"action":click.attribute("action").unwrap_or(""),"rect":[off.attribute("x").unwrap().parse::<i64>().unwrap(),off.attribute("y").unwrap().parse::<i64>().unwrap(),ext.attribute("cx").unwrap().parse::<i64>().unwrap(),ext.attribute("cy").unwrap().parse::<i64>().unwrap()]}));
        }
        for pic in doc.descendants().filter(|n| n.has_tag_name("pic")) {
            let xf = child(child(pic, "spPr"), "xfrm");
            let off = child(xf, "off");
            let ext = child(xf, "ext");
            pictures.push(json!({"x":off.attribute("x").unwrap(),"y":off.attribute("y").unwrap()}));
            pictures
                .push(json!({"cx":ext.attribute("cx").unwrap(),"cy":ext.attribute("cy").unwrap()}));
            let blip = child(child(pic, "blipFill"), "blip");
            let target = relationship(blip.attribute((relns, "embed")).unwrap())
                .attribute("Target")
                .unwrap();
            let name = target.strip_prefix("../").unwrap();
            let mut bytes = Vec::new();
            z.by_name(&format!("ppt/{name}"))
                .unwrap()
                .read_to_end(&mut bytes)
                .unwrap();
            let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
                .read_info()
                .unwrap();
            let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut pixels).unwrap();
            assert_eq!(info.color_type, png::ColorType::Rgba);
            pixels.truncate(info.buffer_size());
            let digest = ring::digest::digest(&ring::digest::SHA256, &pixels)
                .as_ref()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            images.push(json!({"width":info.width,"height":info.height,"rgba_sha256":digest}));
        }
        let nav = doc
            .descendants()
            .filter(|n| {
                n.has_tag_name("hlinkClick") && n.parent().is_some_and(|p| p.has_tag_name("rPr"))
            })
            .map(|n| {
                relationship(n.attribute((relns, "id")).unwrap())
                    .attribute("Target")
                    .unwrap()
            })
            .collect::<Vec<_>>();
        slides.push(json!({"text":doc.descendants().filter(|n| n.has_tag_name("t")).map(|n| n.text().unwrap_or("")).collect::<Vec<_>>(),"links":links,"navigation":nav,"pictures":pictures}));
        frames.push(json!(images));
    }
    (json!(slides), json!(frames))
}
#[test]
fn original_go_document_links_and_complete_powerpoint_frames() {
    let f = fixture();
    let dir = scratch("documents");
    for row in f["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["expected_kind"] == "document")
    {
        let input = dir.0.join("input.d2");
        std::fs::write(&input, row["source"].as_str().unwrap()).unwrap();
        for fmt in ["pdf", "pptx"] {
            let output = dir.0.join(format!("output.{fmt}"));
            let mut args = vec![
                format!("--layout={}", row["engine"].as_str().unwrap()),
                "--tala-seeds=0".into(),
            ];
            args.extend(
                row["flags"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned()),
            );
            args.extend([
                input.to_string_lossy().into_owned(),
                output.to_string_lossy().into_owned(),
            ]);
            assert!(run_document(&args).unwrap().is_empty());
            let bytes = std::fs::read(output).unwrap();
            if fmt == "pdf" {
                let text = String::from_utf8_lossy(&bytes);
                let actual = text
                    .split("/Rect [")
                    .skip(1)
                    .map(|s| {
                        s.split(']')
                            .next()
                            .unwrap()
                            .split_whitespace()
                            .map(|s| s.parse::<f64>().unwrap())
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                let expected = row["pdf"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .flat_map(|p| p["annotations"].as_array().unwrap())
                    .map(|a| {
                        a["rect"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|x| x.as_f64().unwrap())
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                assert_eq!(actual, expected, "{}", row["name"]);
            } else {
                let (actual, frames) =
                    pptx_contract(bytes, row["expected_pages"].as_u64().unwrap() as usize);
                assert_eq!(actual, row["pptx"], "{}", row["name"]);
                assert_eq!(frames, row["images"], "{} full frames", row["name"]);
            }
        }
    }
}
#[test]
fn original_go_missing_rendered_board_targets_reject_without_document() {
    let f = fixture();
    let dir = scratch("errors");
    for row in f["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["expected_kind"] == "error")
    {
        let input = dir.0.join("input.d2");
        std::fs::write(&input, row["source"].as_str().unwrap()).unwrap();
        for fmt in ["pdf", "pptx"] {
            let output = dir.0.join(format!("output.{fmt}"));
            let args = vec![
                format!("--layout={}", row["engine"].as_str().unwrap()),
                "--tala-seeds=0".into(),
                input.to_string_lossy().into_owned(),
                output.to_string_lossy().into_owned(),
            ];
            let error = run_document(&args).unwrap_err();
            assert!(
                error.ends_with(row["errors"][fmt].as_str().unwrap()),
                "{error}"
            );
            assert!(!output.exists());
        }
    }
}
