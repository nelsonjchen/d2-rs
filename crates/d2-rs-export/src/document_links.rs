// SPDX-License-Identifier: MPL-2.0
//! lib/pdf GetFillRGB: the page fill behind a paged board's transparent frame.
use super::*;

/// lib/pdf.GetFillRGB resolves page fill against the catalog, independently of
/// the transparent scene frame. Per-board root fills survive the raster clear.
pub fn page_fill(graph: &Graph, options: &RenderOptions) -> Result<[u8; 3], ExportError> {
    let target = d2_rs_render::target_root_metadata(graph, options)
        .map_err(|e| ExportError::Pdf(e.to_string()))?;
    let fill = target["fill"].as_str().unwrap_or_default();
    go_get_fill_rgb(options.theme_id.unwrap_or(0), fill)
}

/// lib/pdf/pdf.go `GetFillRGB`: theme codes resolve against the catalog,
/// `color.Name2RGB` names short-circuit unless they map to the zero RGB
/// (black), and everything else must pass `color.Hex2RGB`. Gradients, CSS
/// functions and short hex forms therefore keep Go's results and errors.
fn go_get_fill_rgb(theme_id: i64, fill: &str) -> Result<[u8; 3], ExportError> {
    let lowered = String::from_utf8(d2_rs_core::ir::go_lowercase(fill.as_bytes()))
        .unwrap_or_else(|_| fill.to_ascii_lowercase());
    if fill.is_empty() || lowered == "transparent" {
        return Ok([255; 3]);
    }
    let resolved = if d2_rs_target::theme::is_theme_color(fill) {
        // d2themescatalog.Find returns the zero theme for unknown IDs, whose
        // empty palette value then fails Hex2RGB.
        theme::palette(theme_id)
            .and_then(|palette| theme::resolve_color(palette, fill))
            .unwrap_or_default()
    } else {
        // lib/color/color.go namedRgbMap spells this entry "muintcream".
        let rgb = match lowered.as_str() {
            "mintcream" => None,
            "muintcream" => Some([245, 255, 250]),
            name => d2_rs_math::svg_color::css_named_rgb(name),
        };
        if let Some(rgb) = rgb.filter(|rgb| *rgb != [0; 3]) {
            return Ok(rgb);
        }
        fill
    };
    go_hex2rgb(resolved)
}

/// lib/color/color.go `Hex2RGB`.
fn go_hex2rgb(hex: &str) -> Result<[u8; 3], ExportError> {
    let digits = match hex.as_bytes() {
        [b'#', _, _, _, ..] => &hex[1..],
        _ => return Err(ExportError::Pdf(format!("cannot parse hex color {hex}"))),
    };
    // strconv.ParseUint(hex, 16, 32): bare hex digits only, at most 32 bits.
    let quoted = serde_json::to_string(digits).unwrap_or_default();
    if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ExportError::Pdf(format!(
            "strconv.ParseUint: parsing {quoted}: invalid syntax"
        )));
    }
    let values = u64::from_str_radix(digits, 16)
        .ok()
        .filter(|value| *value <= u64::from(u32::MAX))
        .ok_or_else(|| {
            ExportError::Pdf(format!(
                "strconv.ParseUint: parsing {quoted}: value out of range"
            ))
        })?;
    Ok([(values >> 16) as u8, (values >> 8) as u8, values as u8])
}

#[cfg(test)]
mod go_fill_rgb_tests {
    use super::*;

    #[test]
    fn get_fill_rgb_follows_lib_pdf_and_lib_color() {
        assert_eq!(go_get_fill_rgb(0, ""), Ok([255; 3]));
        assert_eq!(go_get_fill_rgb(0, "TransParent"), Ok([255; 3]));
        assert_eq!(go_get_fill_rgb(0, "Red"), Ok([255, 0, 0]));
        assert_eq!(go_get_fill_rgb(0, "#F7F8FE"), Ok([0xf7, 0xf8, 0xfe]));
        // Hex2RGB parses the digits as one integer: "#fff" is 0x000fff.
        assert_eq!(go_get_fill_rgb(0, "#fff"), Ok([0x00, 0x0f, 0xff]));
        // Name2RGB's zero result (black) falls through to Hex2RGB.
        assert!(go_get_fill_rgb(0, "black").is_err());
        assert!(go_get_fill_rgb(0, "mintcream").is_err());
        assert!(go_get_fill_rgb(0, "#123456789").is_err());
        assert_eq!(
            go_get_fill_rgb(0, "linear-gradient(#000000, #ffffff)"),
            Err(ExportError::Pdf(
                "cannot parse hex color linear-gradient(#000000, #ffffff)".into()
            ))
        );
        let n7 = theme::resolve_color(theme::palette(0).unwrap(), "N7").unwrap();
        assert_eq!(go_get_fill_rgb(0, "N7"), go_hex2rgb(n7));
    }
}
