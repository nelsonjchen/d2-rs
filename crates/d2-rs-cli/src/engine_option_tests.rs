// SPDX-License-Identifier: MPL-2.0
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!(
                "engine-option-tests-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn frame(path: &Path) -> Result<serde_json::Value, String> {
    let encoded = std::fs::read(path).map_err(|e| e.to_string())?;
    if encoded.get(..8) != Some(b"\x89PNG\r\n\x1a\n") {
        return Err("invalid PNG signature".into());
    }
    let mut offset = 8;
    let mut ended = false;
    while offset < encoded.len() {
        let header = encoded
            .get(offset..offset + 8)
            .ok_or("truncated PNG header")?;
        let length = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
        let kind = &header[4..];
        let next = offset
            .checked_add(length)
            .and_then(|n| n.checked_add(12))
            .ok_or("PNG length overflow")?;
        let chunk = encoded.get(offset + 4..next).ok_or("truncated PNG chunk")?;
        let mut crc = flate2::Crc::new();
        crc.update(&chunk[..chunk.len() - 4]);
        if crc.sum() != u32::from_be_bytes(chunk[chunk.len() - 4..].try_into().unwrap()) {
            return Err("PNG checksum mismatch".into());
        }
        if kind == b"IEND" {
            if length != 0 || next != encoded.len() {
                return Err("invalid IEND or trailing PNG data".into());
            }
            ended = true;
        }
        offset = next;
    }
    if !ended {
        return Err("missing PNG IEND".into());
    }
    let mut decoder = png::Decoder::new(std::io::Cursor::new(encoded));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    if reader.info().color_type != png::ColorType::Rgb
        || reader.info().bit_depth != png::BitDepth::Eight
        || reader.info().interlaced
    {
        return Err("expected non-interlaced opaque RGB8 PNG".into());
    }
    let mut bytes = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or("missing PNG buffer size")?
    ];
    let info = reader.next_frame(&mut bytes).map_err(|e| e.to_string())?;
    reader.finish().map_err(|e| e.to_string())?;
    bytes.truncate(info.buffer_size());
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => bytes,
        png::ColorType::Rgb => bytes
            .chunks_exact(3)
            .flat_map(|v| [v[0], v[1], v[2], 255])
            .collect(),
        png::ColorType::Grayscale => bytes.into_iter().flat_map(|v| [v, v, v, 255]).collect(),
        png::ColorType::GrayscaleAlpha => bytes
            .chunks_exact(2)
            .flat_map(|v| [v[0], v[0], v[0], v[1]])
            .collect(),
        png::ColorType::Indexed => return Err("indexed PNG was not expanded".into()),
    };
    if info.width == 0
        || info.height == 0
        || rgba.len() != info.width as usize * info.height as usize * 4
    {
        return Err("incomplete PNG frame".into());
    }
    let digest = ring::digest::digest(&ring::digest::SHA256, &rgba);
    let hash = digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(serde_json::json!({"width": info.width, "height": info.height, "rgba_sha256": hash}))
}

fn outputs(output: &Path) -> Result<BTreeMap<String, serde_json::Value>, String> {
    fn walk(
        root: &Path,
        folder: &Path,
        result: &mut BTreeMap<String, serde_json::Value>,
    ) -> Result<(), String> {
        for item in std::fs::read_dir(folder).map_err(|e| e.to_string())? {
            let path = item.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                walk(root, &path, result)?;
            } else if path.file_name().is_some_and(|name| {
                name == crate::board_output::BOARD_OUTPUT_MANIFEST_NAME
            }) {
                // Master's board output manifest; its digests cover PNG
                // encoder bytes, which are not this pixel contract.
                continue;
            } else {
                if path.extension().is_none_or(|ext| ext != "png") {
                    return Err("unexpected non-PNG output".into());
                }
                result.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    frame(&path)?,
                );
            }
        }
        Ok(())
    }
    let folder = output.with_extension("");
    if output.is_file() && folder.exists() {
        return Err("ambiguous file and board outputs".into());
    }
    let mut result = BTreeMap::new();
    if output.is_file() {
        result.insert(String::new(), frame(output)?);
    } else if folder.is_dir() {
        walk(&folder, &folder, &mut result)?;
    } else {
        return Err("missing PNG output".into());
    }
    if result.is_empty() {
        return Err("empty PNG inventory".into());
    }
    Ok(result)
}

#[test]
fn original_go_bundled_engine_options_match_every_png_board() {
    let fixtures: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/engine-options-source-v090.json"))
    .unwrap();
    assert_eq!(fixtures["rows"].as_array().unwrap().len(), 55);
    let scratch = Scratch::new();
    let mut boards = 0;
    for row in fixtures["rows"].as_array().unwrap() {
        let folder = scratch.0.join(row["name"].as_str().unwrap());
        std::fs::create_dir(&folder).unwrap();
        let input = folder.join("input.d2");
        let output = folder.join("output.png");
        std::fs::write(&input, row["source"].as_str().unwrap()).unwrap();
        let mut args = vec![
            format!("--layout={}", row["layout"].as_str().unwrap()),
            "--timeout=30".to_owned(),
        ];
        args.extend(serde_json::from_value::<Vec<String>>(row["args"].clone()).unwrap());
        args.extend([
            input.to_string_lossy().into_owned(),
            output.to_string_lossy().into_owned(),
        ]);
        assert!(
            crate::run_document(&args)
                .unwrap_or_else(|e| panic!("{}: {e}", row["name"]))
                .is_empty()
        );
        let actual = outputs(&output).unwrap_or_else(|e| panic!("{}: {e}", row["name"]));
        boards += actual.len();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            row["expected"],
            "{}",
            row["name"]
        );
    }
    assert_eq!(boards, 66);
}

#[test]
fn png_inventory_gate_rejects_missing_empty_corrupt_extra_and_ambiguous_artifacts() {
    let scratch = Scratch::new();
    let output = scratch.0.join("output.png");
    assert!(outputs(&output).is_err());
    std::fs::create_dir(output.with_extension("")).unwrap();
    assert!(outputs(&output).is_err());
    std::fs::write(output.with_extension("").join("extra.txt"), b"unexpected").unwrap();
    assert!(outputs(&output).is_err());
    std::fs::remove_dir_all(output.with_extension("")).unwrap();
    std::fs::write(&output, b"corrupt PNG").unwrap();
    assert!(outputs(&output).is_err());
    std::fs::write(
        &output,
        d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/unavailable-image-v090.png"),
    )
    .unwrap();
    assert_eq!(outputs(&output).unwrap().len(), 1);
    let valid = std::fs::read(&output).unwrap();
    std::fs::write(&output, &valid[..valid.len() - 12]).unwrap();
    assert!(outputs(&output).is_err());
    let mut trailing = valid.clone();
    trailing.extend_from_slice(b"unexpected trailing data");
    std::fs::write(&output, trailing).unwrap();
    assert!(outputs(&output).is_err());
    let mut bad_crc = valid.clone();
    bad_crc[29] ^= 1;
    std::fs::write(&output, bad_crc).unwrap();
    assert!(outputs(&output).is_err());
    std::fs::write(&output, valid).unwrap();
    std::fs::create_dir(output.with_extension("")).unwrap();
    assert!(outputs(&output).is_err());
}
