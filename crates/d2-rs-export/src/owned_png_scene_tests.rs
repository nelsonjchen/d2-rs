// SPDX-License-Identifier: MPL-2.0
use super::*;
use base64::Engine as _;

fn image(group: &usvg::Group) -> Option<&usvg::Image> {
    for node in group.children() {
        match node {
            usvg::Node::Image(image) => return Some(image),
            usvg::Node::Group(child) => {
                if let Some(found) = image(child) {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn complete_source_vector_images_use_visible_ink_rounding_origin() {
    compare_source_frames(
        d2_rs_testdata::oracle_fixture!("tests/fixtures/vector-image-origin-source-v090.json"),
        12,
    );
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn complete_source_vector_images_clip_transformed_viewports() {
    compare_source_frames(
        d2_rs_testdata::oracle_fixture!("tests/fixtures/vector-image-viewport-source-v090.json"),
        20,
    );
}

fn compare_source_frames(source: &str, count: usize) {
    let fixture: serde_json::Value = serde_json::from_str(source).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), count);
    let payload = base64::engine::general_purpose::STANDARD.encode(b"<svg xmlns='http://www.w3.org/2000/svg' width='64' height='64'><rect width='64' height='64' fill='red'/></svg>");
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='192' height='1280'><image href='data:image/svg+xml;base64,{payload}' width='64' height='64'/></svg>"
    );
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).unwrap();
    let image = image(tree.root()).expect("retained image viewport");
    let mut cache = d2_rs_math::svg_asset_cache::AssetCache::default();
    cache.resolve_unavailable("missing");
    let mut failures = Vec::new();
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let width = case["Width"].as_u64().unwrap() as usize;
        let height = case["Height"].as_u64().unwrap() as usize;
        assert!(width > 0 && height > 0);
        let placement = crate::svg_precision::SceneImagePlacement {
            matrix: serde_json::from_value(case["Matrix"].clone()).unwrap(),
            image_box: serde_json::from_value(case["Box"].clone()).unwrap(),
            aspect: crate::png_image::ImageAspect::parse(
                case["Slice"]
                    .as_bool()
                    .unwrap_or(false)
                    .then_some("xMidYMid slice"),
            )
            .unwrap(),
        };
        let native = crate::svg_precision::NativeImage {
            vector: &cache.vectors["image:unavailable"],
            resources: &cache.document.resources,
            jpeg_pngs: &std::collections::BTreeMap::new(),
        };
        let mut pixels = vec![255; width * height * 4];
        assert_eq!(
            draw(
                image,
                native,
                Some(&placement),
                &mut pixels,
                width,
                height,
                1.,
                1.
            ),
            Some(true)
        );
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(case["png"].as_str().unwrap())
            .unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        let mut expected = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut expected).unwrap();
        expected.truncate(info.buffer_size());
        assert_eq!((info.width as usize, info.height as usize), (width, height));
        let expected = match info.color_type {
            png::ColorType::Rgb => expected
                .chunks_exact(3)
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect::<Vec<_>>(),
            png::ColorType::Rgba => expected,
            other => panic!("unexpected source PNG color {other:?}"),
        };
        assert_eq!(expected.len(), pixels.len());
        #[cfg(feature = "diagnostic-traces")]
        if let Ok(folder) = std::env::var("D2RS_VECTOR_ORIGIN_CAPTURE") {
            let folder = std::path::Path::new(&folder);
            assert!(
                folder.is_absolute()
                    && folder.starts_with(
                        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                            .parent()
                            .unwrap()
                            .parent()
                            .unwrap()
                            .join(".reference")
                    )
            );
            std::fs::create_dir_all(folder).unwrap();
            let mut bytes = Vec::new();
            let mut encoder = png::Encoder::new(&mut bytes, width as u32, height as u32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
            assert!(bytes.len() <= 16 * 1024 * 1024);
            std::fs::write(folder.join(format!("{name}.png")), bytes).unwrap();
        }
        let changed = pixels
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count();
        if changed != 0 {
            failures.push((name, changed));
        }
    }
    assert!(
        failures.is_empty(),
        "complete-frame differences: {failures:?}"
    );
}
