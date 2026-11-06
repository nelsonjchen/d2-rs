// SPDX-License-Identifier: MPL-2.0
//! lib/textmeasure/markdown_svg.go URL safety classification on master.
//! Safe links retain their original spelling; normalization is only a predicate.

fn has_scheme(value: &[u8]) -> bool {
    let Some(colon) = value.iter().position(|c| *c == b':') else {
        return false;
    };
    colon > 0
        && value[0].is_ascii_alphabetic()
        && value[1..colon]
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || b"+-.".contains(c))
}

fn percent_decode(value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(value.len());
    let mut at = 0;
    while at < value.len() {
        if value[at] == b'%'
            && let Some(digits) = value.get(at + 1..at + 3)
            && let (Some(hi), Some(lo)) = (
                (digits[0] as char).to_digit(16),
                (digits[1] as char).to_digit(16),
            )
        {
            out.push((hi * 16 + lo) as u8);
            at += 3;
        } else {
            out.push(value[at]);
            at += 1;
        }
    }
    out
}

/// A trimmed safe link for interactive Markdown, PDF and PPTX metadata.
/// Unsafe schemes and empty links produce an empty string.
pub fn safe_markdown_link(link: &str) -> &str {
    let link = link.trim();
    if link.is_empty() || is_dangerous_link(link.as_bytes()) {
        ""
    } else {
        link
    }
}

/// The same bounded HTML/percent/control normalization and Goldmark policy
/// used by D2 for ordinary links and interactive Markdown metadata.
pub fn is_dangerous_link(link: &[u8]) -> bool {
    let mut normalized = String::from_utf8_lossy(link).trim().as_bytes().to_vec();
    for _ in 0..8 {
        let decoded = percent_decode(&super::html::unescape_html(&normalized));
        let next = String::from_utf8_lossy(&decoded)
            .chars()
            .filter(|c| !c.is_control() && !c.is_whitespace())
            .collect::<String>()
            .into_bytes();
        if has_scheme(&next) || next == normalized {
            let prefix = |value: &[u8], start: &[u8]| {
                value
                    .get(..start.len())
                    .is_some_and(|v| v.eq_ignore_ascii_case(start))
            };
            if prefix(&next, b"data:image/") {
                return ![b"png;".as_slice(), b"gif;", b"jpeg;", b"webp;"]
                    .iter()
                    .any(|kind| prefix(&next[11..], kind));
            }
            return [b"javascript:".as_slice(), b"vbscript:", b"file:", b"data:"]
                .iter()
                .any(|scheme| prefix(&next, scheme));
        }
        normalized = next;
    }
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn upstream_dangerous_link_policy() {
        // lib/textmeasure/markdown_svg_test.go: TestDangerousLinkPolicy.
        for link in [
            "javascript:alert(1)",
            "JAVA\tSCRIPT:alert(1)",
            "vBsCrIpT:msgbox(1)",
            "data:text/html,<script>alert(1)</script>",
            "data:image/svg+xml,<svg onload=alert(1)>",
            "file:///etc/passwd",
            "j&#x61;vascript:alert(1)",
            "java%0Ascript:alert(1)",
            "java%250Ascript:alert(1)",
        ] {
            assert!(super::is_dangerous_link(link.as_bytes()), "{link}");
        }
        for link in [
            "https://example.com/a?x=1&y=2",
            "http://example.com",
            "mailto:security@example.com",
            "../docs/security",
            "root.layers.details",
            "vscode://file/example.go:10:2",
            "obsidian://open?vault=notes",
            "data:image/png;base64,iVBORw0KGgo=",
        ] {
            assert!(!super::is_dangerous_link(link.as_bytes()), "{link}");
        }
    }
}
