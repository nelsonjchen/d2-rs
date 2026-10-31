// SPDX-License-Identifier: MPL-2.0
//! FPDF v0.12.0 UTF-8 font records (`fpdf.go` putfonts/generateCIDFontMap,
//! `utf8fontfile.go` parseHEADTable/parseHHEATable/parseOS2Table/parsePOSTTable).
//! Upstream MIT and font-subsetter notices are in THIRD_PARTY_NOTICES.md.

use super::{ExportError, push_pdf_object_at};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug)]
pub(super) struct PdfFontObjects {
    pub type0: usize,
    cid_font: usize,
    to_unicode: usize,
    cid_info: usize,
    descriptor: usize,
    cid_to_gid: usize,
    font_file: usize,
}

pub(super) fn allocate_pdf_font_objects(next: &mut usize) -> PdfFontObjects {
    let first = *next;
    *next += 7;
    PdfFontObjects {
        type0: first,
        cid_font: first + 1,
        to_unicode: first + 2,
        cid_info: first + 3,
        descriptor: first + 4,
        cid_to_gid: first + 5,
        font_file: first + 6,
    }
}

#[derive(Debug)]
pub(super) struct PdfFontData {
    resource_id: String,
    base_name: &'static str,
    subset: Vec<u8>,
    cid_to_gid: Vec<u8>,
    widths: String,
    ascent: i32,
    descent: i32,
    cap_height: i32,
    bbox: [i32; 4],
    missing_width: u16,
    flags: u32,
    stem_v: i32,
    italic_angle: i32,
}

impl PdfFontData {
    pub(super) fn resource_id(&self) -> &str {
        &self.resource_id
    }
}

pub(super) fn build_pdf_font_data(
    characters: &str,
    bold: bool,
) -> Result<PdfFontData, ExportError> {
    let original = if bold {
        d2_rs_text::measure::bundled_source_sans_pro_bold_bytes()
    } else {
        d2_rs_text::measure::bundled_source_sans_pro_regular_bytes()
    };
    // makeSubsetRange(57) initializes map VALUES to zero, not their keys.
    // GenerateCutFont consumes the values, so unused fonts contain only rune 0.
    let used = characters.chars().chain(['\0']).collect::<BTreeSet<_>>();
    let cutset = used.iter().collect::<String>();
    let subset = d2_rs_text::subset_font::utf8_cut_font_fpdf(original, &cutset)
        .ok_or_else(|| ExportError::Pdf("could not subset bundled Source Sans Pro font".into()))?;
    let subset_face = ttf_parser::Face::parse(&subset, 0)
        .map_err(|error| ExportError::Pdf(format!("invalid embedded font: {error}")))?;
    let mut cid_to_gid = vec![0; 256 * 256 * 2];
    for &character in &used {
        let cid = character as usize;
        if cid != 0 && cid < 65536 {
            if let Some(gid) = subset_face.glyph_index(character) {
                cid_to_gid[cid * 2..cid * 2 + 2].copy_from_slice(&gid.0.to_be_bytes());
            }
        }
    }
    drop(subset_face);
    let face = ttf_parser::Face::parse(original, 0)
        .map_err(|error| ExportError::Pdf(format!("invalid bundled font: {error}")))?;
    let scale = 1000.0 / f64::from(face.units_per_em());
    let scaled = |v: i16| (f64::from(v) * scale) as i32;
    let hhea = face
        .raw_face()
        .table(ttf_parser::Tag::from_bytes(b"hhea"))
        .ok_or_else(|| ExportError::Pdf("bundled font has no hhea table".into()))?;
    let read_i16 =
        |data: &[u8], offset: usize| i16::from_be_bytes([data[offset], data[offset + 1]]);
    let ascent = scaled(read_i16(hhea, 4));
    let descent = scaled(read_i16(hhea, 6));
    let bbox = face.global_bounding_box();
    let bbox = [
        scaled(bbox.x_min),
        scaled(bbox.y_min),
        scaled(bbox.x_max),
        scaled(bbox.y_max),
    ];
    let weight = face.weight().to_number();
    let italic_angle = face.italic_angle() as i32;
    let flags = 4
        | if weight >= 600 { 262144 } else { 0 }
        | if italic_angle != 0 { 64 } else { 0 }
        | u32::from(face.is_monospaced());
    let cap_height = scaled(face.capital_height().unwrap_or(read_i16(hhea, 4)));
    let missing_width = face
        .glyph_hor_advance(ttf_parser::GlyphId(0))
        .unwrap_or_default();
    let missing_width = (f64::from(missing_width) * scale).round() as u16;
    let stem_v = 50 + (f64::from(weight) / 65.0).powi(2) as i32;
    // def.go:generateFontID hashes the complete JSON font definition before
    // usedRunes is populated. Preserve Go's struct field order, including the
    // zero-valued fields; Cw[0] is the number of nonzero mapped characters.
    let mut char_widths = vec![0i32; 65536];
    let mut char_count = 0;
    for cid in 1..65535u32 {
        let Some(character) = char::from_u32(cid) else {
            continue;
        };
        let Some(glyph) = face.glyph_index(character).filter(|g| g.0 != 0) else {
            continue;
        };
        let advance = face.glyph_hor_advance(glyph).unwrap_or_default();
        let width = if advance >= 32768 {
            0
        } else {
            (f64::from(advance) * scale).round() as i32
        };
        char_widths[cid as usize] = if width == 0 { 65535 } else { width };
        char_count += 1;
    }
    char_widths[0] = char_count;
    let underline = face.underline_metrics().unwrap_or(ttf_parser::LineMetrics {
        position: 0,
        thickness: 0,
    });
    let up = (f64::from(underline.position) * scale).round() as i32;
    let ut = (f64::from(underline.thickness) * scale).round() as i32;
    let base_name = if bold { "utf8sourceB" } else { "utf8source" };
    let [xmin, ymin, xmax, ymax] = bbox;
    let cw = serde_json::to_string(&char_widths).expect("integer widths serialize");
    let registered_name = if bold { "sourceB" } else { "source" };
    let definition = format!(
        "{{\"Tp\":\"UTF8\",\"Name\":\"{registered_name}\",\"Desc\":{{\"Ascent\":{ascent},\"Descent\":{descent},\"CapHeight\":{cap_height},\"Flags\":{flags},\"FontBBox\":{{\"Xmin\":{xmin},\"Ymin\":{ymin},\"Xmax\":{xmax},\"Ymax\":{ymax}}},\"ItalicAngle\":{italic_angle},\"StemV\":{stem_v},\"MissingWidth\":{missing_width}}},\"Up\":{up},\"Ut\":{ut},\"Cw\":{cw},\"Enc\":\"\",\"Diff\":\"\",\"File\":\"\",\"Size1\":0,\"Size2\":0,\"OriginalSize\":0,\"N\":0,\"DiffN\":0}}"
    );
    let resource_id = super::pdf_document::sha1_hex(definition.as_bytes());
    let last_rune = used.last().copied().unwrap_or('\0') as u32;
    let widths = cid_widths((1..=last_rune.min(65535)).filter_map(|cid| {
        let character = char::from_u32(cid)?;
        if cid > 255 && !used.contains(&character) {
            return None;
        }
        let glyph = face.glyph_index(character)?;
        if glyph.0 == 0 {
            return None;
        }
        let advance = face.glyph_hor_advance(glyph)?;
        // parseHMTXTable treats negative signed advances as zero.
        let width = if advance >= 32768 {
            0
        } else {
            (f64::from(advance) * scale).round() as i32
        };
        Some((cid, width))
    }));
    Ok(PdfFontData {
        resource_id,
        base_name,
        subset,
        cid_to_gid,
        widths,
        ascent,
        descent,
        cap_height,
        bbox,
        missing_width,
        flags,
        stem_v,
        italic_angle,
    })
}

/// FPDF generateCIDFontMap: group consecutive widths, splitting repeated runs,
/// then merge adjacent short runs in the same order as untypedKeyMap.
fn cid_widths(widths: impl Iterator<Item = (u32, i32)>) -> String {
    struct Run {
        start: u32,
        values: Vec<i32>,
        interval: bool,
    }
    let mut runs: Vec<Run> = Vec::new();
    let mut previous_cid = None;
    let mut previous_width = -1;
    let mut interval = false;
    for (cid, width) in widths {
        if previous_cid == cid.checked_sub(1) && !runs.is_empty() {
            let run = runs.last_mut().unwrap();
            if width == previous_width {
                if width == run.values[0] {
                    run.values.push(width);
                } else {
                    run.values.pop();
                    runs.push(Run {
                        start: cid - 1,
                        values: vec![previous_width, width],
                        interval: false,
                    });
                }
                interval = true;
                runs.last_mut().unwrap().interval = true;
            } else {
                if interval {
                    runs.push(Run {
                        start: cid,
                        values: vec![width],
                        interval: false,
                    });
                } else {
                    run.values.push(width);
                }
                interval = false;
            }
        } else {
            runs.push(Run {
                start: cid,
                values: vec![width],
                interval: false,
            });
            interval = false;
        }
        previous_cid = Some(cid);
        previous_width = width;
    }
    let mut merged: Vec<Run> = Vec::new();
    let mut next_key = None;
    let mut is_interval = false;
    for run in runs {
        let count = run.values.len() + usize::from(run.interval);
        if next_key == Some(run.start) && !is_interval && (!run.interval || count < 4) {
            merged
                .last_mut()
                .unwrap()
                .values
                .extend_from_slice(&run.values);
        } else {
            merged.push(Run {
                start: run.start,
                values: run.values.clone(),
                interval: false,
            });
        }
        next_key = Some(run.start + run.values.len() as u32);
        is_interval = run.interval && count > 3;
    }
    let mut text = String::from("/W [");
    for run in merged {
        if run.values.iter().all(|v| *v == run.values[0]) {
            text.push_str(&format!(
                " {} {} {}",
                run.start,
                run.start + run.values.len() as u32 - 1,
                run.values[0]
            ));
        } else {
            text.push_str(&format!(
                " {} [ {} ]\n",
                run.start,
                run.values
                    .iter()
                    .map(i32::to_string)
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
    }
    text.push_str(" ]");
    text
}

fn stream(pdf: &mut Vec<u8>, offsets: &mut [usize], id: usize, dictionary: &str, data: &[u8]) {
    let mut body = format!("{dictionary}\nstream\n").into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(b"\nendstream");
    push_pdf_object_at(pdf, offsets, id, &body);
}

pub(super) fn write_pdf_font(
    pdf: &mut Vec<u8>,
    offsets: &mut [usize],
    ids: PdfFontObjects,
    font: &PdfFontData,
) {
    let name = font.base_name;
    push_pdf_object_at(pdf, offsets, ids.type0, format!("<</Type /Font\n/Subtype /Type0\n/BaseFont /{name}\n/Encoding /Identity-H\n/DescendantFonts [{} 0 R]\n/ToUnicode {} 0 R>>", ids.cid_font, ids.to_unicode).as_bytes());
    let default_width = if font.missing_width != 0 {
        format!("/DW {}\n", font.missing_width)
    } else {
        String::new()
    };
    push_pdf_object_at(pdf, offsets, ids.cid_font, format!("<</Type /Font\n/Subtype /CIDFontType2\n/BaseFont /{name}\n/CIDSystemInfo {} 0 R\n/FontDescriptor {} 0 R\n{default_width}{}\n/CIDToGIDMap {} 0 R>>", ids.cid_info, ids.descriptor, font.widths, ids.cid_to_gid).as_bytes());
    let cmap = b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo\n<</Registry (Adobe)\n/Ordering (UCS)\n/Supplement 0\n>> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n1 beginbfrange\n<0000> <FFFF> <0000>\nendbfrange\nendcmap\nCMapName currentdict /CMap defineresource pop\nend\nend";
    stream(
        pdf,
        offsets,
        ids.to_unicode,
        &format!("<</Length {}>>", cmap.len()),
        cmap,
    );
    push_pdf_object_at(
        pdf,
        offsets,
        ids.cid_info,
        b"<</Registry (Adobe)\n/Ordering (UCS)\n/Supplement 0>>",
    );
    push_pdf_object_at(pdf, offsets, ids.descriptor, format!("<</Type /FontDescriptor /FontName /{name}\n /Ascent {} /Descent {} /CapHeight {} /Flags {}/FontBBox [{} {} {} {}]  /ItalicAngle {} /StemV {} /MissingWidth {}/FontFile2 {} 0 R>>", font.ascent, font.descent, font.cap_height, font.flags, font.bbox[0], font.bbox[1], font.bbox[2], font.bbox[3], font.italic_angle, font.stem_v, font.missing_width, ids.font_file).as_bytes());
    let compressed_map = d2_rs_goflate::zlib_compress(&font.cid_to_gid, 1);
    stream(
        pdf,
        offsets,
        ids.cid_to_gid,
        &format!("<</Length {}/Filter /FlateDecode>>", compressed_map.len()),
        &compressed_map,
    );
    let compressed_font = d2_rs_goflate::zlib_compress(&font.subset, 1);
    stream(
        pdf,
        offsets,
        ids.font_file,
        &format!(
            "<</Length {}\n/Filter /FlateDecode\n/Length1 {}\n>>",
            compressed_font.len(),
            font.subset.len()
        ),
        &compressed_font,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_font_records_match_fpdf() {
        let cases: serde_json::Value =
            serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/pdf-fonts-fpdf.json")).unwrap();
        for case in cases.as_array().unwrap() {
            for (style, bold) in [("regular", false), ("bold", true)] {
                let expected = &case[style];
                let mut next = expected["first_object"].as_u64().unwrap() as usize;
                let ids = allocate_pdf_font_objects(&mut next);
                let data = build_pdf_font_data(expected["text"].as_str().unwrap(), bold).unwrap();
                let expected_subset = base64::engine::general_purpose::STANDARD
                    .decode(expected["subset"].as_str().unwrap())
                    .unwrap();
                assert!(
                    data.subset == expected_subset,
                    "{} {style} subset: lengths {}/{}, first difference {:?}",
                    case["name"],
                    data.subset.len(),
                    expected_subset.len(),
                    data.subset
                        .iter()
                        .zip(&expected_subset)
                        .position(|(a, b)| a != b)
                );
                let mut output = Vec::new();
                let mut offsets = vec![0; next];
                write_pdf_font(&mut output, &mut offsets, ids, &data);
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(expected["objects"].as_str().unwrap())
                    .unwrap();
                assert!(
                    output == bytes,
                    "{} {style}: length {}/{}, first difference {:?}",
                    case["name"],
                    output.len(),
                    bytes.len(),
                    output.iter().zip(&bytes).position(|(a, b)| a != b)
                );
            }
        }
    }
}
