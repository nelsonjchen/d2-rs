// Copyright 2010, 2011 The Go Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause
// Native translation of pinned Go 1.27.0 net/http/internal/sniff.go and the
// built-in extension lookup in mime/type.go. See THIRD_PARTY_NOTICES.md.
// D2's imgbundler treats these bytes as opaque; decoding images is a separate
// contract used by raster exports.
#[path = "image_mime_data.rs"]
mod data;
#[path = "image_mime_registry.rs"]
mod registry;

enum Signature {
    Html(&'static [u8]),
    Exact(&'static [u8], &'static str),
    Masked(&'static [u8], &'static [u8], bool, &'static str),
    Mp4,
    Text,
}

pub(super) fn sniff(href: &str, bytes: &[u8]) -> String {
    let path = if href.starts_with("http://") || href.starts_with("https://") {
        let path = super::parse_hash_url(Some(href))
            .map(|url| url.path)
            .unwrap_or_default();
        unescape_path(&path).unwrap_or_default()
    } else {
        href.to_owned()
    };
    // Go path.Ext only examines the last slash-delimited path element. Remote
    // query strings/fragments are excluded, and escaped path bytes are decoded.
    let base = path.rsplit('/').next().unwrap_or_default();
    if let Some(dot) = base.rfind('.') {
        if let Some(mime) = registry::lookup(&base[dot..]) {
            return mime;
        }
    }
    detect(bytes).to_owned()
}

fn unescape_path(path: &str) -> Option<String> {
    let mut bytes = path.bytes();
    let mut decoded = Vec::with_capacity(path.len());
    while let Some(byte) = bytes.next() {
        decoded.push(if byte == b'%' {
            let high = (bytes.next()? as char).to_digit(16)?;
            let low = (bytes.next()? as char).to_digit(16)?;
            (high * 16 + low) as u8
        } else {
            byte
        });
    }
    Some(String::from_utf8_lossy(&decoded).into_owned())
}

pub(super) fn detect(bytes: &[u8]) -> &'static str {
    let bytes = &bytes[..bytes.len().min(512)];
    let first = bytes
        .iter()
        .position(|b| !matches!(b, b'\t' | b'\n' | 0x0c | b'\r' | b' '))
        .unwrap_or(bytes.len());
    for signature in data::SIGNATURES {
        match signature {
            Signature::Html(pattern) => {
                let body = &bytes[first..];
                if body.len() > pattern.len()
                    && pattern
                        .iter()
                        .zip(body)
                        .all(|(a, b)| *a == if a.is_ascii_uppercase() { b & 0xdf } else { *b })
                    && matches!(body[pattern.len()], b' ' | b'>')
                {
                    return "text/html; charset=utf-8";
                }
            }
            Signature::Exact(pattern, mime) if bytes.starts_with(pattern) => return mime,
            Signature::Masked(mask, pattern, skip_ws, mime) => {
                let body = if *skip_ws { &bytes[first..] } else { bytes };
                if body.len() >= pattern.len()
                    && mask.len() == pattern.len()
                    && body
                        .iter()
                        .zip(*mask)
                        .zip(*pattern)
                        .all(|((b, m), p)| b & m == *p)
                {
                    return mime;
                }
            }
            Signature::Mp4 if bytes.len() >= 12 => {
                let size = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
                if size <= bytes.len()
                    && size.is_multiple_of(4)
                    && &bytes[4..8] == b"ftyp"
                    && (8..size)
                        .step_by(4)
                        .any(|start| start != 12 && &bytes[start..start + 3] == b"mp4")
                {
                    return "video/mp4";
                }
            }
            Signature::Text
                if !bytes[first..]
                    .iter()
                    .any(|b| matches!(b, 0x00..=0x08 | 0x0b | 0x0e..=0x1a | 0x1c..=0x1f)) =>
            {
                return "text/plain; charset=utf-8";
            }
            _ => {}
        }
    }
    "application/octet-stream"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_precedes_bytes_and_uses_decoded_remote_path() {
        assert_eq!(
            sniff("http://example.test/icon%2eSVG?name=file.png#x", b""),
            "image/svg+xml"
        );
        assert_eq!(
            sniff("http://example.test/icon.png?type=.svg", b"<svg/>"),
            "image/png"
        );
        assert_eq!(sniff("local.png", b"<svg/>"), "image/png");
        assert_eq!(
            sniff("http://example.svg/no-extension", b"<svg/>"),
            "text/plain; charset=utf-8"
        );
        assert_eq!(sniff("empty", b""), "text/plain; charset=utf-8");
    }

    #[test]
    fn sniff_window_whitespace_and_tag_terminators_follow_source() {
        assert_eq!(detect(b"\r\n <hTmL>"), "text/html; charset=utf-8");
        assert_eq!(detect(b"<htmlx>"), "text/plain; charset=utf-8");
        assert_eq!(detect(b"<html\t>"), "text/plain; charset=utf-8");
        assert_eq!(detect(b" \t<?xml"), "text/xml; charset=utf-8");
        assert_eq!(detect(b"GIF8"), "text/plain; charset=utf-8");
        let mut bytes = vec![b'A'; 512];
        bytes.push(0);
        assert_eq!(detect(&bytes), "text/plain; charset=utf-8");
        bytes[511] = 0;
        assert_eq!(detect(&bytes), "application/octet-stream");
    }

    #[test]
    fn mp4_box_bounds_and_minor_version_are_checked() {
        assert_eq!(detect(b"\0\0\0\x10ftypmp42\0\0\0\0"), "video/mp4");
        assert_eq!(
            detect(b"\0\0\0\x10ftypxxxxmp42"),
            "application/octet-stream"
        );
        assert_eq!(
            detect(b"\0\0\0\x14ftypmp42\0\0\0\0"),
            "application/octet-stream"
        );
    }
}
