// SPDX-License-Identifier: MPL-2.0
//! D2 lib/pdf + FPDF v0.12.0 page/resource serialization. MIT notices retained.
use super::{
    ExportError, PdfAnnotation, PdfAnnotationDestination, PdfRasterPage,
    pdf_font::{allocate_pdf_font_objects, build_pdf_font_data, write_pdf_font},
    pdf_text_width, push_pdf_object_at, validate_pdf_raster,
};

/// FPDF putinfo uses local wall time and omits the zone suffix.
pub(super) fn timestamp() -> Result<String, ExportError> {
    if let Some(seconds) = source_date_epoch()? {
        return Ok(super::pdf_timestamp_at_utc(seconds)[..16].to_owned());
    }
    #[cfg(unix)]
    {
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as libc::time_t;
        let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
        // SAFETY: localtime_r initializes this valid output pointer on success;
        // the input lives through the call and the result is checked first.
        if !unsafe { libc::localtime_r(&seconds, local.as_mut_ptr()) }.is_null() {
            let local = unsafe { local.assume_init() };
            return Ok(format!(
                "D:{:04}{:02}{:02}{:02}{:02}{:02}",
                local.tm_year + 1900,
                local.tm_mon + 1,
                local.tm_mday,
                local.tm_hour,
                local.tm_min,
                local.tm_sec
            ));
        }
    }
    // Hosts without a native timezone interface use UTC. Keep the same FPDF
    // date syntax; clock/zone selection is not a fixture-specific override.
    Ok(super::pdf_timestamp_utc()[..16].to_owned())
}

// Explicit opt-in for reproducible documents. Without it, preserve FPDF's
// local wall clock. UTC makes the injected instant independent of host TZ.
pub(super) fn source_date_epoch() -> Result<Option<u64>, ExportError> {
    match std::env::var("SOURCE_DATE_EPOCH") {
        Ok(value) => parse_source_date_epoch(&value).map(Some),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(_) => Err(ExportError::Pdf("SOURCE_DATE_EPOCH must be UTF-8".into())),
    }
}

fn parse_source_date_epoch(value: &str) -> Result<u64, ExportError> {
    value
        .parse::<u64>()
        .ok()
        .filter(|seconds| {
            !value.is_empty()
                && value.bytes().all(|byte| byte.is_ascii_digit())
                && *seconds <= 253_402_300_799
        })
        .ok_or_else(|| {
            ExportError::Pdf(
                "SOURCE_DATE_EPOCH must be decimal Unix seconds in 0..=253402300799".into(),
            )
        })
}

pub(super) fn sha1_hex(data: &[u8]) -> String {
    sha1(data).iter().map(|b| format!("{b:02x}")).collect()
}

fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let mut message = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());
    for block in message.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        for (slot, value) in h.iter_mut().zip([a, b, c, d, e]) {
            *slot = slot.wrapping_add(value);
        }
    }
    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

struct Image {
    id: String,
    width: u32,
    height: u32,
    rgb: Vec<u8>,
    alpha: Option<Vec<u8>>,
    object: usize,
}
impl Image {
    fn new(page: &PdfRasterPage) -> Self {
        let (rgb, alpha) = page.image_streams();
        // def.go:generateImageID: raw bytes followed by LE int64/float64 fields.
        let mut data = rgb.clone();
        data.extend_from_slice(alpha.as_deref().unwrap_or_default());
        data.extend_from_slice(&0i64.to_le_bytes());
        data.extend_from_slice(&f64::from(page.width).to_le_bytes());
        data.extend_from_slice(&f64::from(page.height).to_le_bytes());
        data.extend_from_slice(b"DeviceRGB");
        data.extend_from_slice(&8i64.to_le_bytes());
        data.extend_from_slice(b"FlateDecode");
        data.extend_from_slice(
            format!(
                "/Predictor 15 /Colors 3 /BitsPerComponent 8 /Columns {}",
                page.width
            )
            .as_bytes(),
        );
        data.extend_from_slice(&1f64.to_le_bytes());
        data.extend_from_slice(&72f64.to_le_bytes());
        Self {
            id: sha1_hex(&data),
            width: page.width,
            height: page.height,
            rgb,
            alpha,
            object: 0,
        }
    }
    fn write(&self, pdf: &mut Vec<u8>, offsets: &mut [usize]) {
        for (plane, bytes) in
            std::iter::once((0, &self.rgb)).chain(self.alpha.iter().map(|a| (1, a)))
        {
            let mask = if plane == 0 && self.alpha.is_some() {
                format!("/SMask {} 0 R\n", self.object + 1)
            } else {
                String::new()
            };
            let (color, colors) = if plane == 0 {
                ("DeviceRGB", 3)
            } else {
                ("DeviceGray", 1)
            };
            let mut body = format!("<</Type /XObject\n/Subtype /Image\n/Width {}\n/Height {}\n/ColorSpace /{color}\n/BitsPerComponent 8\n/Filter /FlateDecode\n/DecodeParms <</Predictor 15 /Colors {colors} /BitsPerComponent 8 /Columns {}>>\n{mask}/Length {}>>\nstream\n", self.width, self.height, self.width, bytes.len()).into_bytes();
            body.extend_from_slice(bytes);
            body.extend_from_slice(b"\nendstream");
            push_pdf_object_at(pdf, offsets, self.object + plane, &body);
        }
    }
}

fn font_command(id: &str) -> String {
    format!("BT /F{id} 14.00 Tf ET\n")
}
fn rgb_command(rgb: [u8; 3], gray_op: &str, rgb_op: &str) -> String {
    if rgb[0] == rgb[1] && rgb[0] == rgb[2] {
        format!("{:.3} {gray_op}", f64::from(rgb[0]) / 255.0)
    } else {
        format!(
            "{:.3} {:.3} {:.3} {rgb_op}",
            f64::from(rgb[0]) / 255.0,
            f64::from(rgb[1]) / 255.0,
            f64::from(rgb[2]) / 255.0
        )
    }
}
fn light(rgb: [u8; 3]) -> bool {
    (0.299 * f64::from(rgb[0]).powi(2)
        + 0.587 * f64::from(rgb[1]).powi(2)
        + 0.114 * f64::from(rgb[2]).powi(2))
    .sqrt()
        > 130.0
}
fn literal_bytes(data: &[u8]) -> Vec<u8> {
    let mut result = vec![b'('];
    for &byte in data {
        match byte {
            b'\\' | b'(' | b')' => {
                result.push(b'\\');
                result.push(byte);
            }
            b'\r' => result.extend_from_slice(b"\\r"),
            _ => result.push(byte),
        }
    }
    result.push(b')');
    result
}
fn cell_text(
    out: &mut Vec<u8>,
    value: &str,
    x: f64,
    height: f64,
    text_color: &str,
    fill_color: &str,
) {
    if value.is_empty() {
        return;
    }
    let use_color = text_color != fill_color;
    if use_color {
        out.extend_from_slice(format!("q {text_color} ").as_bytes());
    }
    // FPDF's cMargin is one tenth of its 28.35pt default margin, not 2.84.
    out.extend_from_slice(
        format!(
            "BT {:.2} {:.2} Td ",
            x + 2.835,
            height - (0.5 * 72.0 + 0.3 * 14.0)
        )
        .as_bytes(),
    );
    let text = value
        .encode_utf16()
        .flat_map(u16::to_be_bytes)
        .collect::<Vec<_>>();
    out.extend_from_slice(&literal_bytes(&text));
    out.extend_from_slice(b"Tj ET");
    if use_color {
        out.extend_from_slice(b" Q");
    }
    out.push(b'\n');
}

pub(super) fn encode(
    pages: &[PdfRasterPage],
    annotations: &[Vec<PdfAnnotation>],
    titles: &[Vec<String>],
    nav: bool,
    default_light: bool,
    fills: &[[u8; 3]],
    timestamp: &str,
) -> Result<Vec<u8>, ExportError> {
    if pages.is_empty() || pages.len() != annotations.len() || pages.len() != titles.len() {
        return Err(ExportError::Pdf(
            "PDF page metadata counts must match".into(),
        ));
    }
    for page in pages {
        validate_pdf_raster(page)?;
    }
    let sizes = pages
        .iter()
        .zip(titles)
        .map(|(p, names)| {
            let header_width = if nav {
                56.0 + pdf_text_width(&names.join("  /  "), true)
            } else {
                0.0
            };
            (
                576f64.max(f64::from(p.width) / 2.0).max(header_width),
                576f64.max(f64::from(p.height) / 2.0) + if nav { 72.0 } else { 0.0 },
            )
        })
        .collect::<Vec<_>>();
    let mut characters = [String::new(), String::new()];
    for path in titles.iter().filter(|_| nav) {
        if let Some((last, ancestors)) = path.split_last() {
            for ancestor in ancestors {
                characters[0].push_str(ancestor);
                characters[0].push_str("  /  ");
            }
            characters[1].push_str(last);
        }
    }
    let regular = build_pdf_font_data(&characters[0], false)?;
    let bold = build_pdf_font_data(&characters[1], true)?;
    let regular_id = regular.resource_id();
    let bold_id = bold.resource_id();
    let mut next = 3 + 2 * pages.len();
    let regular_objects = allocate_pdf_font_objects(&mut next);
    let bold_objects = allocate_pdf_font_objects(&mut next);
    let mut images = pages.iter().map(Image::new).collect::<Vec<_>>();
    // Go's unsorted maps permit multiple byte orders. Use the stable catalog
    // order supported by FPDF; never randomize output or normalize the oracle.
    let mut order = (0..images.len()).collect::<Vec<_>>();
    order.sort_by_key(|&i| images[i].width);
    let mut unique = std::collections::BTreeMap::new();
    for &i in &order {
        let im = &mut images[i];
        im.object = *unique.entry(im.id.clone()).or_insert_with(|| {
            let number = next;
            next += 1 + usize::from(im.alpha.is_some());
            number
        });
    }
    let info = next;
    let catalog = next + 1;
    let transparent = pages.iter().any(|p| p.alpha.is_some());
    let version = if transparent { "1.4" } else { "1.3" };
    let mut pdf = format!("%PDF-{version}\n%µ¶\n").into_bytes();
    let mut offsets = vec![0; catalog + 1];
    let mut previous_draw = "0.000 G".to_owned();
    let mut previous_fill = "0.000 g".to_owned();
    for (i, page) in pages.iter().enumerate() {
        let (width, height) = sizes[i];
        let header = if nav { 72.0 } else { 0.0 };
        let iw = f64::from(page.width) / 2.0;
        let ih = f64::from(page.height) / 2.0;
        let ix = (width - iw) / 2.0;
        let iy = (height - header - ih) / 2.0;
        let fill = fills.get(i).copied().unwrap_or([255; 3]);
        let is_light = if fills.is_empty() {
            default_light
        } else {
            light(fill)
        };
        let ink = if is_light { [10, 15, 37] } else { [255; 3] };
        let text_color = rgb_command(ink, "g", "rg");
        let draw_color = rgb_command(ink, "G", "RG");
        let mut content = format!(
            "0 J\n0 j\n{:.2} w\n{}{previous_draw}\n{previous_fill}\n{}",
            if i == 0 { 2.0 } else { 1.0 },
            font_command(&bold_id),
            font_command(&bold_id)
        )
        .into_bytes();
        // lib/pdf.AddPDFPage paints the complete page even without navigation.
        // FPDF also carries that fill state into the next page's preamble.
        let fill_color = rgb_command(fill, "g", "rg");
        content.extend_from_slice(
            format!(
                "{fill_color}\n0.00 {height:.2} {width:.2} {:.2} re f\n",
                -height
            )
            .as_bytes(),
        );
        if nav {
            content.extend_from_slice(font_command(&regular_id).as_bytes());
            let mut x = 28.0;
            if let Some((last, ancestors)) = titles[i].split_last() {
                for name in ancestors {
                    cell_text(&mut content, name, x, height, &text_color, &fill_color);
                    x += pdf_text_width(name, false);
                    cell_text(&mut content, "  /  ", x, height, &text_color, &fill_color);
                    x += pdf_text_width("  /  ", false);
                }
                content.extend_from_slice(font_command(&bold_id).as_bytes());
                cell_text(&mut content, last, x, height, &text_color, &fill_color);
            }
        }
        previous_fill = fill_color;
        content.extend_from_slice(format!("q {iw:.5} 0 0 {ih:.5} {ix:.5} {iy:.5} cm /I{} Do Q\n1.00 w\n{draw_color}\n{:.2} {:.2} m {:.2} {:.2} l S \n", images[i].id, if nav {28.0} else {0.0}, height-header, width-if nav {28.0} else {0.0}, height-header).as_bytes());
        // AddPDFPage's SetFont before the next AddPage writes to this page.
        if i + 1 < pages.len() {
            content.extend_from_slice(font_command(&bold_id).as_bytes());
        }
        previous_draw = draw_color;
        let mut dictionary = format!(
            "<</Type /Page\n/Parent 1 0 R\n/MediaBox [0 0 {width:.2} {height:.2}]\n/Resources 2 0 R\n"
        );
        if !annotations[i].is_empty() {
            dictionary.push_str("/Annots [");
            for annotation in &annotations[i] {
                let (x0, x1, y0, y1) = if annotation.header {
                    let r = annotation.rect;
                    (
                        r.origin.x,
                        r.origin.x + r.size.width,
                        height - r.origin.y,
                        height - r.origin.y - r.size.height,
                    )
                } else {
                    let r = annotation.rect;
                    (
                        ix + r.origin.x.floor(),
                        ix + (r.origin.x + r.size.width).ceil(),
                        iy + ih - r.origin.y.floor(),
                        iy + ih - (r.origin.y + r.size.height).ceil(),
                    )
                };
                dictionary.push_str(&format!("<</Type /Annot /Subtype /Link /Rect [{x0:.2} {y0:.2} {x1:.2} {y1:.2}] /Border [0 0 0] "));
                match &annotation.destination {
                    PdfAnnotationDestination::Page(target) => dictionary.push_str(&format!(
                        "/Dest [{} 0 R /XYZ 0 {:.2} null]>>",
                        3 + 2 * target,
                        sizes[*target].1
                    )),
                    PdfAnnotationDestination::Uri(uri) => dictionary.push_str(&format!(
                        "/A <</S /URI /URI {}>>>>",
                        super::pdf_literal(uri)
                    )),
                }
            }
            dictionary.push_str("]\n");
        }
        if transparent {
            dictionary.push_str("/Group <</Type /Group /S /Transparency /CS /DeviceRGB>>\n");
        }
        dictionary.push_str(&format!("/Contents {} 0 R>>", 4 + 2 * i));
        push_pdf_object_at(&mut pdf, &mut offsets, 3 + 2 * i, dictionary.as_bytes());
        let compressed = d2_rs_goflate::zlib_compress(&content, 1);
        let mut body = format!(
            "<</Filter /FlateDecode /Length {}>>\nstream\n",
            compressed.len()
        )
        .into_bytes();
        body.extend_from_slice(&compressed);
        body.extend_from_slice(b"\nendstream");
        push_pdf_object_at(&mut pdf, &mut offsets, 4 + 2 * i, &body);
    }
    let kids = (0..pages.len())
        .map(|i| format!("{} 0 R ", 3 + 2 * i))
        .collect::<String>();
    push_pdf_object_at(
        &mut pdf,
        &mut offsets,
        1,
        format!(
            "<</Type /Pages\n/Kids [{kids}]\n/Count {}\n/MediaBox [0 0 595.28 841.89]\n>>",
            pages.len()
        )
        .as_bytes(),
    );
    write_pdf_font(&mut pdf, &mut offsets, regular_objects, &regular);
    write_pdf_font(&mut pdf, &mut offsets, bold_objects, &bold);
    let mut written = std::collections::BTreeSet::new();
    for &i in &order {
        if written.insert(images[i].object) {
            images[i].write(&mut pdf, &mut offsets);
        }
    }
    let mut resource = format!(
        "<<\n/ProcSet [/PDF /Text /ImageB /ImageC /ImageI]\n/Font <<\n/F{regular_id} {} 0 R\n/F{bold_id} {} 0 R\n>>\n/XObject <<\n",
        regular_objects.type0, bold_objects.type0
    );
    let mut resources = images.iter().collect::<Vec<_>>();
    resources.sort_by_key(|im| &im.id);
    // Registered image keys may share a content ID. FPDF still emits one entry
    // per registration; its PDF dictionary resolves the duplicate to one image.
    for im in resources {
        resource.push_str(&format!("/I{} {} 0 R\n", im.id, im.object));
    }
    resource.push_str(">>\n/ColorSpace <<\n>>\n>>");
    push_pdf_object_at(&mut pdf, &mut offsets, 2, resource.as_bytes());
    let mut info_body = b"<<\n/Producer (\xfe\xff\0F\0P\0D\0F\0 \x001\0.\x007)\n".to_vec();
    info_body.extend_from_slice(
        format!("/CreationDate ({timestamp})\n/ModDate ({timestamp})\n>>").as_bytes(),
    );
    push_pdf_object_at(&mut pdf, &mut offsets, info, &info_body);
    push_pdf_object_at(
        &mut pdf,
        &mut offsets,
        catalog,
        b"<<\n/Type /Catalog\n/Pages 1 0 R\n/Names <<\n/EmbeddedFiles << /Names [\n  \n] >>\n>>\n>>",
    );
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in offsets.iter().skip(1) {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(format!("trailer\n<<\n/Size {}\n/Root {catalog} 0 R\n/Info {info} 0 R\n>>\nstartxref\n{xref}\n%%EOF\n",offsets.len()).as_bytes());
    Ok(pdf)
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn convenience_apis_match_complete_source_documents() {
        use super::super::{
            DocumentLinkRegion, package_pdf_pages, package_pdf_pages_with_link_regions,
        };
        let test_name = "pdf_document::tests::convenience_apis_match_complete_source_documents";
        if std::env::var_os("D2_RS_PDF_CONVENIENCE_TEST").is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", test_name, "--nocapture"])
                .env("SOURCE_DATE_EPOCH", "946782245")
                .env("D2_RS_PDF_CONVENIENCE_TEST", "1")
                .output()
                .unwrap();
            println!("{}", String::from_utf8_lossy(&output.stdout));
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let fixtures: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/pdf-png-idat-fpdf-master.json"))
        .unwrap();
        let unhex = |value: &serde_json::Value| {
            value
                .as_str()
                .unwrap()
                .as_bytes()
                .chunks_exact(2)
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect::<Vec<_>>()
        };
        let mut failures = Vec::new();
        for fixture in fixtures.as_array().unwrap() {
            let pages = fixture["pages"]
                .as_array()
                .unwrap()
                .iter()
                .map(unhex)
                .collect::<Vec<_>>();
            // Go numbers same-width image objects in map order, so the hook
            // records every distinct document it produces as `pdf_variants`.
            let expected = fixture["pdf_variants"]
                .as_array()
                .map(|variants| variants.iter().map(unhex).collect::<Vec<_>>())
                .unwrap_or_else(|| vec![unhex(&fixture["pdf"])]);
            for (api, actual) in [
                ("pages", package_pdf_pages(&pages).unwrap()),
                (
                    "empty-links",
                    package_pdf_pages_with_link_regions(&pages, &vec![Vec::new(); pages.len()])
                        .unwrap(),
                ),
            ] {
                let name = format!("{api}/{}", fixture["name"].as_str().unwrap());
                let matches = expected.contains(&actual);
                println!(
                    "{name}: complete source match={matches} ({} / {} bytes)",
                    actual.len(),
                    expected[0].len()
                );
                if !matches {
                    failures.push(name);
                }
            }
        }
        // Same RGB PNG, with the shape link from run 37's complete Go witness.
        // The 4x2 image is centered on a 576pt page: this also checks the X/Y
        // translation from image-relative rectangles to PDF page coordinates.
        let pages = vec![unhex(&fixtures[0]["pages"][0])];
        let links = vec![vec![DocumentLinkRegion {
            rect: Rect {
                origin: Point { x: 3.0, y: 4.0 },
                size: Size {
                    width: 10.0,
                    height: 11.0,
                },
            },
            href: "https://example.test/(x)".into(),
        }]];
        let actual = package_pdf_pages_with_link_regions(&pages, &links).unwrap();
        let expected = d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/pdf-opaque-default-fpdf-master.pdf");
        let matches = actual == expected;
        println!(
            "links/opaque-default: complete source match={matches} ({} / {} bytes)",
            actual.len(),
            expected.len()
        );
        if !matches {
            failures.push("links/opaque-default".to_owned());
        }
        assert!(
            failures.is_empty(),
            "complete source mismatches: {failures:?}"
        );
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn reproducible_clock_public_apis() {
        use super::super::{
            DocumentLinkRegion, package_pdf_pages, package_pdf_pages_with_link_regions,
            package_pdf_pages_with_navigation,
        };
        // Isolate environment changes in subprocesses; other tests keep their
        // ordinary clocks and no process-global environment mutation is needed.
        let Ok(expected) = std::env::var("D2_RS_PDF_CLOCK_TEST") else {
            for (epoch, expected) in [
                ("946782245", "D:20000102030405"),
                ("951827696", "D:20000229123456"),
                ("-1", "error"),
                ("invalid", "error"),
                ("253402300800", "error"),
            ] {
                let output = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "pdf_document::tests::reproducible_clock_public_apis",
                    ])
                    .env("SOURCE_DATE_EPOCH", epoch)
                    .env("D2_RS_PDF_CLOCK_TEST", expected)
                    .env("TZ", "Pacific/Honolulu")
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{epoch}: {}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            return;
        };
        let fixtures: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/pdf-png-idat-fpdf-master.json"))
        .unwrap();
        let png = fixtures[0]["pages"][0]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect();
        let pages = vec![png];
        let links = vec![vec![DocumentLinkRegion {
            rect: super::super::Rect {
                origin: super::super::Point { x: 0.0, y: 0.0 },
                size: d2_rs_core::Size {
                    width: 1.0,
                    height: 1.0,
                },
            },
            href: "https://example.com".into(),
        }]];
        let outputs = [
            package_pdf_pages(&pages),
            package_pdf_pages_with_link_regions(&pages, &links),
            package_pdf_pages_with_navigation(
                &pages,
                &links,
                &[vec!["root".into()]],
                &[vec!["Root".into()]],
                true,
            ),
        ];
        for output in outputs {
            if expected == "error" {
                assert!(
                    output
                        .unwrap_err()
                        .to_string()
                        .contains("SOURCE_DATE_EPOCH")
                );
            } else {
                let bytes = output.unwrap();
                let text = String::from_utf8_lossy(&bytes);
                for key in ["CreationDate", "ModDate"] {
                    assert!(
                        text.contains(&format!("/{key} ({expected}")),
                        "missing injected {key}"
                    );
                }
            }
        }
    }

    #[test]
    fn reproducible_clock_validates_range_and_calendar_boundaries() {
        for (seconds, expected) in [
            ("0", "D:19700101000000+00'00'"),
            ("951827696", "D:20000229123456+00'00'"),
            ("253402300799", "D:99991231235959+00'00'"),
        ] {
            let parsed = super::parse_source_date_epoch(seconds).unwrap();
            assert_eq!(super::super::pdf_timestamp_at_utc(parsed), expected);
        }
        for value in [
            "",
            "-1",
            "+1",
            " 1",
            "1\n",
            "1.0",
            "١",
            "253402300800",
            "18446744073709551616",
        ] {
            assert!(super::parse_source_date_epoch(value).is_err(), "{value:?}");
        }
    }
    use super::*;
    use d2_rs_core::{Point, Rect, Size};

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn caller_rgb_pngs_match_complete_fpdf_documents() {
        let fixtures: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/pdf-png-idat-fpdf-master.json"))
        .unwrap();
        let unhex = |value: &serde_json::Value| {
            value
                .as_str()
                .unwrap()
                .as_bytes()
                .chunks_exact(2)
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect::<Vec<_>>()
        };
        let mut failures = Vec::new();
        for fixture in fixtures.as_array().unwrap() {
            let name = fixture["name"].as_str().unwrap();
            let pages = fixture["pages"]
                .as_array()
                .unwrap()
                .iter()
                .map(|page| super::super::decode_pdf_png(&unhex(page)).unwrap())
                .collect::<Vec<_>>();
            let actual = encode(
                &pages,
                &vec![Vec::new(); pages.len()],
                &vec![vec!["Root".to_owned()]; pages.len()],
                false,
                true,
                &vec![[255; 3]; pages.len()],
                "D:20000102030405",
            )
            .unwrap();
            let expected = fixture["pdf_variants"]
                .as_array()
                .map(|variants| variants.iter().map(unhex).collect::<Vec<_>>())
                .unwrap_or_else(|| vec![unhex(&fixture["pdf"])]);
            let matches = expected.contains(&actual);
            println!(
                "{name}: complete PDF match={matches} ({} / {} bytes)",
                actual.len(),
                expected[0].len()
            );
            if !matches {
                failures.push(name);
            }
        }
        assert!(
            failures.is_empty(),
            "complete source PDF mismatches: {failures:?}"
        );
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn all_png_package_apis_preserve_source_rgb_image_streams() {
        use super::super::{
            DocumentLinkRegion, decode_pdf_png, package_pdf_pages,
            package_pdf_pages_with_link_regions, package_pdf_pages_with_navigation,
        };
        let fixtures: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/pdf-png-idat-fpdf-master.json"))
        .unwrap();
        let unhex = |value: &serde_json::Value| {
            value
                .as_str()
                .unwrap()
                .as_bytes()
                .chunks_exact(2)
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect::<Vec<_>>()
        };
        // Extract the length-delimited image stream from the complete Go PDF,
        // independently of the PNG chunk parser and native PDF image encoder.
        let image_stream = |pdf: &[u8]| {
            let marker = b"/Subtype /Image";
            let start = pdf.windows(marker.len()).position(|w| w == marker).unwrap();
            let image = &pdf[start..];
            let offset = image.windows(7).position(|w| w == b"stream\n").unwrap() + 7;
            let header = std::str::from_utf8(&image[..offset]).unwrap();
            let length = header
                .split("/Length ")
                .nth(1)
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap()
                .trim_end_matches('>')
                .parse::<usize>()
                .unwrap();
            image[offset..offset + length].to_vec()
        };
        for fixture in fixtures.as_array().unwrap() {
            if fixture["name"] == "mixed" {
                continue;
            }
            let png = unhex(&fixture["pages"][0]);
            let expected = image_stream(&unhex(&fixture["pdf"]));
            let pages = vec![png.clone()];
            let links = vec![vec![DocumentLinkRegion {
                rect: Rect {
                    origin: Point { x: 0.0, y: 0.0 },
                    size: Size {
                        width: 1.0,
                        height: 1.0,
                    },
                },
                href: "https://example.test".to_owned(),
            }]];
            let titles = vec![vec!["root".to_owned()]];
            for actual in [
                package_pdf_pages(&pages).unwrap(),
                package_pdf_pages_with_link_regions(&pages, &links).unwrap(),
                package_pdf_pages_with_navigation(&pages, &links, &titles, &titles, true).unwrap(),
            ] {
                assert_eq!(image_stream(&actual), expected, "{}", fixture["name"]);
            }
            let mut corrupt = png;
            let start = corrupt.windows(4).position(|w| w == b"IDAT").unwrap();
            corrupt[start + 4] ^= 1;
            assert!(decode_pdf_png(&corrupt).is_err());
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_fpdf_documents_match_fixed_source_fixtures() {
        let expected: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/pdf-documents-fpdf-master.json"))
        .unwrap();
        for name in ["plain", "nav", "opaque", "opaque_default"] {
            let opaque = name.starts_with("opaque");
            let sizes = if opaque {
                vec![(4, 2)]
            } else {
                vec![(32, 1200), (8, 2), (12, 6), (8, 2)]
            };
            let mut pages = sizes
                .iter()
                .map(|&(width, height)| PdfRasterPage {
                    width,
                    height,
                    rgb: if opaque {
                        [60, 100, 180].repeat((width * height) as usize)
                    } else {
                        vec![0; (width * height * 3) as usize]
                    },
                    alpha: (!opaque).then(|| vec![0; (width * height) as usize]),
                    rgb_png_idat: None,
                })
                .collect::<Vec<_>>();
            if name == "opaque_default" {
                let inputs: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/pdf-png-idat-fpdf-master.json"))
                .unwrap();
                let png = inputs[0]["pages"][0]
                    .as_str()
                    .unwrap()
                    .as_bytes()
                    .chunks_exact(2)
                    .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                    .collect::<Vec<_>>();
                pages[0] = super::super::decode_pdf_png(&png).unwrap();
            }
            let titles = (0..sizes.len())
                .map(|i| {
                    if i == 0 {
                        vec!["Root (α) \\".to_owned()]
                    } else {
                        vec!["Root (α) \\".to_owned(), "Child café".to_owned()]
                    }
                })
                .collect::<Vec<_>>();
            let mut annotations = vec![Vec::new(); sizes.len()];
            for (i, list) in annotations.iter_mut().enumerate() {
                if name == "nav" && i > 0 {
                    list.push(PdfAnnotation {
                        rect: Rect {
                            origin: Point { x: 30.835, y: 29.0 },
                            size: Size {
                                width: pdf_text_width(&titles[i][0], false),
                                height: 14.0,
                            },
                        },
                        destination: PdfAnnotationDestination::Page(0),
                        header: true,
                    });
                }
                list.push(PdfAnnotation {
                    rect: Rect {
                        origin: Point { x: 3.0, y: 4.0 },
                        size: Size {
                            width: 10.0,
                            height: 11.0,
                        },
                    },
                    destination: if i == 0 {
                        PdfAnnotationDestination::Uri("https://example.test/(x)".into())
                    } else {
                        PdfAnnotationDestination::Page(0)
                    },
                    header: false,
                });
            }
            let fills = [[255; 3], [16, 16, 32], [255; 3], [255; 3]];
            let actual = encode(
                &pages,
                &annotations,
                &titles,
                name == "nav",
                true,
                &fills,
                "D:20000102030405",
            )
            .unwrap();
            let wanted = if name == "opaque_default" {
                d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/pdf-opaque-default-fpdf-master.pdf").to_vec()
            } else {
                expected[name]
                    .as_str()
                    .unwrap()
                    .as_bytes()
                    .chunks_exact(2)
                    .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                    .collect::<Vec<_>>()
            };
            if actual != wanted {
                // A fresh path is required: never overwrite a content-store alias.
                let path = std::env::temp_dir()
                    .join(format!("d2-rs-run37-{name}-{}.pdf", std::process::id()));
                std::fs::write(&path, &actual).unwrap();
                let first = actual.iter().zip(&wanted).position(|(a, b)| a != b);
                panic!(
                    "{name}: first difference {first:?}; lengths {} / {}; native witness {}",
                    actual.len(),
                    wanted.len(),
                    path.display()
                );
            }
        }
    }
}

/// Decode only the page streams in format tests, following each /Contents
/// reference and its declared length. Font/image streams are never mistaken
/// for drawing commands.
#[cfg(test)]
pub(super) fn decoded_page_streams(pdf: &[u8]) -> Vec<Vec<u8>> {
    use std::io::Read;
    let marker = b"/Contents ";
    pdf.windows(marker.len())
        .enumerate()
        .filter(|(_, w)| *w == marker)
        .map(|(pos, _)| {
            let start = pos + marker.len();
            let end = start
                + pdf[start..]
                    .iter()
                    .take_while(|b| b.is_ascii_digit())
                    .count();
            let number = std::str::from_utf8(&pdf[start..end]).unwrap();
            let object = format!("\n{number} 0 obj\n");
            let pos = pdf
                .windows(object.len())
                .position(|w| w == object.as_bytes())
                .unwrap()
                + object.len();
            let stream = b"\nstream\n";
            let data_start = pos
                + pdf[pos..]
                    .windows(stream.len())
                    .position(|w| w == stream)
                    .unwrap()
                + stream.len();
            let header = std::str::from_utf8(&pdf[pos..data_start]).unwrap();
            let length = header
                .split("/Length ")
                .nth(1)
                .unwrap()
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .unwrap()
                .parse::<usize>()
                .unwrap();
            let mut data = Vec::new();
            flate2::read::ZlibDecoder::new(&pdf[data_start..data_start + length])
                .read_to_end(&mut data)
                .unwrap();
            data
        })
        .collect()
}
