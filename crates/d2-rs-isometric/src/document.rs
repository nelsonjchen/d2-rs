// SPDX-License-Identifier: MPL-2.0
//! d2svg/isometric.go document wrapper, shared by native CLI and WASM.
use std::collections::BTreeSet;

#[allow(clippy::too_many_arguments)]
pub fn wrap_svg(
    svg: &str,
    hash: &str,
    pad: i64,
    scale: Option<f64>,
    center: bool,
    no_xml: bool,
    omit: bool,
    version: &str,
) -> Result<Vec<u8>, String> {
    let opening = svg
        .find('>')
        .ok_or("native isometric output has an invalid SVG envelope")?
        + 1;
    let dimension = |name: &str| -> Result<i64, String> {
        let marker = format!("{name}=\"");
        let start = svg[..opening]
            .find(&marker)
            .ok_or("read isometric SVG dimensions")?
            + marker.len();
        svg[start..]
            .split('"')
            .next()
            .unwrap_or("")
            .parse()
            .map_err(|_| "read isometric SVG dimensions".into())
    };
    let (w, h) = (dimension("width")?, dimension("height")?);
    let invalid = || format!("padding {pad} produces invalid SVG dimensions");
    let twice = pad.checked_mul(2).ok_or_else(invalid)?;
    let (w, h) = (
        w.checked_add(twice)
            .filter(|n| *n > 0)
            .ok_or_else(invalid)?,
        h.checked_add(twice)
            .filter(|n| *n > 0)
            .ok_or_else(invalid)?,
    );
    let mut body = svg[opening..]
        .strip_suffix("</svg>")
        .ok_or("native isometric output has an invalid SVG envelope")?
        .to_owned();
    let ids = body
        .match_indices("id=\"")
        .filter_map(|(i, _)| body[i + 4..].split('"').next())
        .collect::<BTreeSet<_>>();
    let mut replacements = Vec::new();
    for id in ids {
        for (from, to) in [
            (
                format!("id=\"{id}\""),
                format!("id=\"{hash}-isometric-{id}\""),
            ),
            (
                format!("url(#{id})"),
                format!("url(#{hash}-isometric-{id})"),
            ),
            (
                format!("href=\"#{id}\""),
                format!("href=\"#{hash}-isometric-{id}\""),
            ),
        ] {
            replacements.push((from, to));
        }
    }
    for (from, to) in replacements {
        body = body.replace(&from, &to);
    }
    body = body.replacen(
        "<rect width=\"100%\" height=\"100%\"",
        &format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{w}\" height=\"{h}\"",
            -pad, -pad
        ),
        1,
    );
    let dims = if let Some(scale) = scale {
        let (sw, sh) = ((w as f64 * scale).ceil(), (h as f64 * scale).ceil());
        if !sw.is_finite() || !sh.is_finite() || sw > i64::MAX as f64 || sh > i64::MAX as f64 {
            return Err("isometric SVG scale produces invalid dimensions".into());
        }
        format!(" width=\"{sw:.0}\" height=\"{sh:.0}\"")
    } else {
        String::new()
    };
    let xml = if no_xml {
        ""
    } else {
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>"
    };
    let version = if omit {
        String::new()
    } else {
        format!(" data-d2-version=\"{}\"", version.trim())
    };
    let alignment = if center { "xMidYMid" } else { "xMinYMin" };
    let output = format!(
        "{xml}<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"{version} preserveAspectRatio=\"{alignment} meet\" viewBox=\"0 0 {w} {h}\"{dims}><svg class=\"{hash} d2-svg d2-isometric\" width=\"{w}\" height=\"{h}\" viewBox=\"{} {} {w} {h}\" role=\"img\">{body}</svg></svg>",
        -pad, -pad
    );
    if output.len() > 32 << 20 {
        return Err("isometric SVG document exceeds output byte limit".into());
    }
    Ok(output.into_bytes())
}
