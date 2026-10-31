// SPDX-License-Identifier: BSD-3-Clause
//! Go 1.27 HTML decoding and the URL predicate reachable from D2 extendLinks.
//! Native validation of Go net/url Parse/ParseRequestURI as reached by D2.
// Copyright 2009, 2024, 2025 The Go Authors. All rights reserved.
// See THIRD_PARTY_NOTICES.md for the Go BSD license.
pub fn html_unescape(input: &[u8]) -> Vec<u8> {
    d2_rs_text::markdown::html::unescape_html(input)
}
fn host_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || b"!$&'()*+,;=:[]<>\"-_.~".contains(&c)
}
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Plain,
    Host,
    Zone,
}
fn unescape(bytes: &[u8], mode: Mode) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'%' {
            let escape_error = || {
                format!(
                    "invalid URL escape {}",
                    quote(&bytes[i..bytes.len().min(i + 3)])
                )
            };
            let hex = bytes.get(i + 1..i + 3).ok_or_else(escape_error)?;
            if !hex.iter().all(u8::is_ascii_hexdigit) {
                return Err(escape_error());
            }
            let digit = |c: u8| {
                if c.is_ascii_digit() {
                    c - b'0'
                } else {
                    c.to_ascii_lowercase() - b'a' + 10
                }
            };
            let v = digit(hex[0]) * 16 + digit(hex[1]);
            if mode == Mode::Host && v < 128 && v != b'%' {
                return Err(escape_error());
            }
            if mode == Mode::Zone && v != b'%' && v != b' ' && !host_char(v) {
                return Err(escape_error());
            }
            out.push(v);
            i += 3;
        } else {
            if mode != Mode::Plain && c < 128 && !host_char(c) {
                return Err(format!(
                    "invalid character {} in host name",
                    quote(&bytes[i..i + 1])
                ));
            }
            out.push(c);
            i += 1;
        }
    }
    Ok(out)
}
fn port(bytes: &[u8]) -> bool {
    bytes.is_empty() || (bytes[0] == b':' && bytes[1..].iter().all(u8::is_ascii_digit))
}
fn quote(bytes: &[u8]) -> String {
    crate::parser::quote_input(bytes)
}
fn host(scheme: &[u8], host: &[u8]) -> Result<(), String> {
    if let Some(open) = host.iter().rposition(|b| *b == b'[') {
        if open != 0 {
            return Err("invalid IP-literal".into());
        }
        let close = host
            .iter()
            .rposition(|b| *b == b']')
            .ok_or("missing ']' in host")?;
        let suffix = &host[close + 1..];
        if !port(suffix) {
            return Err(format!("invalid port {} after host", quote(suffix)));
        }
        unescape(suffix, Mode::Host)?;
        let name = &host[1..close];
        let name = if let Some(zone) = name.windows(3).position(|w| w == b"%25") {
            let mut prefix = unescape(&name[..zone], Mode::Host)?;
            prefix.extend(unescape(&name[zone..], Mode::Zone)?);
            prefix
        } else {
            unescape(name, Mode::Host)?
        };
        match ip_literal(&name) {
            Ok(true) => return Ok(()),
            Ok(false) => return Err("invalid IP-literal".into()),
            Err(cause) => return Err(format!("invalid host: {cause}")),
        }
    }
    if let Some(first) = host.iter().position(|c| *c == b':') {
        let last = host.iter().rposition(|c| *c == b':').unwrap();
        let strict = scheme.eq_ignore_ascii_case(b"http") || scheme.eq_ignore_ascii_case(b"https");
        let suffix = &host[if strict { first } else { last }..];
        if !port(suffix) {
            return Err(format!("invalid port {} after host", quote(suffix)));
        }
    }
    unescape(host, Mode::Host)?;
    Ok(())
}
fn authority(scheme: &[u8], authority: &[u8]) -> Result<bool, String> {
    let (user, name) = if let Some(i) = authority.iter().rposition(|c| *c == b'@') {
        (Some(&authority[..i]), &authority[i + 1..])
    } else {
        (None, authority)
    };
    host(scheme, name)?;
    if let Some(user) = user {
        if !user
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || b"-._:~!$&'()*+,;=%@".contains(c))
        {
            return Err("net/url: invalid userinfo".into());
        }
        // ParseAuthority unescapes username and password separately; an
        // incomplete escape before ':' is quoted without the password bytes.
        if let Some(i) = user.iter().position(|c| *c == b':') {
            unescape(&user[..i], Mode::Plain)?;
            unescape(&user[i + 1..], Mode::Plain)?;
        } else {
            unescape(user, Mode::Plain)?;
        }
    }
    Ok(!name.is_empty())
}
/// URL validation and the byte fields reached by D2 toPrettyLink.
#[derive(Debug, Default)]
pub struct Parsed {
    pub remote: bool,
    pub has_host: bool,
    pub scheme: Vec<u8>,
    pub host: Vec<u8>,
    pub raw_path: Vec<u8>,
}
pub fn parse(input: &[u8], via_request: bool) -> Result<Parsed, String> {
    let (raw, fragment) = if !via_request && let Some(i) = input.iter().position(|c| *c == b'#') {
        (&input[..i], &input[i + 1..])
    } else {
        (input, &[][..])
    };
    let result =
        parse_inner(raw, via_request).map_err(|cause| format!("parse {}: {cause}", quote(raw)))?;
    unescape(fragment, Mode::Plain).map_err(|cause| format!("parse {}: {cause}", quote(input)))?;
    Ok(result)
}
fn parse_inner(raw: &[u8], via_request: bool) -> Result<Parsed, String> {
    if raw.iter().any(|c| *c < 32 || *c == 127) {
        return Err("net/url: invalid control character in URL".into());
    }
    if raw.is_empty() && via_request {
        return Err("empty url".into());
    }
    if raw == b"*" {
        return Ok(Parsed::default());
    }
    let mut scheme = &[][..];
    let mut rest = raw;
    for (i, c) in raw.iter().copied().enumerate() {
        if c.is_ascii_alphabetic() {
            continue;
        }
        if c.is_ascii_digit() || b"+-.".contains(&c) {
            if i == 0 {
                break;
            }
            continue;
        }
        if c == b':' {
            if i == 0 {
                return Err("missing protocol scheme".into());
            }
            scheme = &raw[..i];
            rest = &raw[i + 1..];
        }
        break;
    }
    rest = rest.split(|c| *c == b'?').next().unwrap();
    if !rest.starts_with(b"/") {
        if !scheme.is_empty() {
            return Ok(Parsed {
                remote: true,
                has_host: false,
                scheme: scheme.to_ascii_lowercase(),
                ..Parsed::default()
            });
        }
        if via_request {
            return Err("invalid URI for request".into());
        }
        if rest.split(|c| *c == b'/').next().unwrap().contains(&b':') {
            return Err("first path segment in URL cannot contain colon".into());
        }
    }
    let mut has_host = false;
    let mut parsed_host = Vec::new();
    if (!scheme.is_empty() || !via_request && !rest.starts_with(b"///")) && rest.starts_with(b"//")
    {
        let auth = &rest[2..];
        let i = auth.iter().position(|c| *c == b'/').unwrap_or(auth.len());
        has_host = authority(scheme, &auth[..i])?;
        let name = auth[..i].rsplit(|c| *c == b'@').next().unwrap();
        parsed_host = unescape(name, Mode::Plain)?;
        rest = &auth[i..];
    }
    let path = unescape(rest, Mode::Plain)?;
    Ok(Parsed {
        remote: !scheme.is_empty() || path.starts_with(b"/"),
        has_host,
        scheme: scheme.to_ascii_lowercase(),
        host: parsed_host,
        raw_path: if escape_path(&path) == rest {
            Vec::new()
        } else {
            rest.to_vec()
        },
    })
}
// net/url.setPath preserves the authored spelling only when it differs from
// escape(decoded, encodePath). Go 1.27's generated path table leaves these
// reserved characters unescaped, while '?' and '#' require percent encoding.
fn escape_path(path: &[u8]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = Vec::new();
    for &byte in path {
        if byte.is_ascii_alphanumeric() || b"-_.~$&+,/:;=@".contains(&byte) {
            out.push(byte);
        } else {
            out.extend([
                b'%',
                HEX[usize::from(byte >> 4)],
                HEX[usize::from(byte & 15)],
            ]);
        }
    }
    out
}

/// D2 shortens only a noncanonical RawPath longer than thirty bytes. Both
/// ten-byte slices deliberately retain Go string bytes, including split UTF-8.
pub fn shortened_url_label(input: &[u8]) -> Option<Vec<u8>> {
    let parsed = parse(input, true).ok()?;
    if !parsed.has_host || parsed.raw_path.len() <= 30 {
        return None;
    }
    let mut label = parsed.scheme;
    label.extend_from_slice(b"://");
    label.extend_from_slice(&parsed.host);
    label.extend_from_slice(&parsed.raw_path[..10]);
    label.extend_from_slice(b"...");
    label.extend_from_slice(&parsed.raw_path[parsed.raw_path.len() - 10..]);
    Some(label)
}

/// Match Go extendLinks: html.UnescapeString, Parse, then scheme/absolute path.
pub fn is_remote(input: &[u8]) -> bool {
    parse(&html_unescape(input), false).is_ok_and(|url| url.remote)
}
// Go net/netip ParseAddr error precedence, retaining bytes and unparsed suffixes.
fn ip_error(input: &[u8], message: &str, at: &[u8]) -> String {
    let mut out = format!("ParseAddr({}): {message}", quote(input));
    if !at.is_empty() {
        out.push_str(&format!(" (at {})", quote(at)));
    }
    out
}
fn ipv4(input: &[u8], s: &[u8]) -> Result<(), String> {
    let (mut value, mut position, mut digits) = (0u32, 0, 0);
    for (i, c) in s.iter().copied().enumerate() {
        if c.is_ascii_digit() {
            if digits == 1 && value == 0 {
                return Err(ip_error(
                    input,
                    "IPv4 field has octet with leading zero",
                    b"",
                ));
            }
            value = value * 10 + u32::from(c - b'0');
            digits += 1;
            if value > 255 {
                return Err(ip_error(input, "IPv4 field has value >255", b""));
            }
        } else if c == b'.' {
            if i == 0 || i == s.len() - 1 || s[i - 1] == b'.' {
                return Err(ip_error(
                    input,
                    "IPv4 field must have at least one digit",
                    &s[i..],
                ));
            }
            if position == 3 {
                return Err(ip_error(input, "IPv4 address too long", b""));
            }
            position += 1;
            value = 0;
            digits = 0;
        } else {
            return Err(ip_error(input, "unexpected character", &s[i..]));
        }
    }
    if position < 3 {
        return Err(ip_error(input, "IPv4 address too short", b""));
    }
    Ok(())
}
fn ip_literal(input: &[u8]) -> Result<bool, String> {
    match input.iter().find(|c| matches!(c, b'.' | b':' | b'%')) {
        Some(b'.') => {
            ipv4(input, input)?;
            return Ok(false);
        }
        Some(b'%') => return Err(ip_error(input, "missing IPv6 address", b"")),
        None => return Err(ip_error(input, "unable to parse IP", b"")),
        _ => (),
    }
    let fail = |message: &str, at: &[u8]| ip_error(input, message, at);
    let mut s = input;
    if let Some(i) = s.iter().position(|c| *c == b'%') {
        if i + 1 == s.len() {
            return Err(fail("zone must be a non-empty string", b""));
        }
        s = &s[..i];
    }
    let mut ellipsis = false;
    if s.starts_with(b"::") {
        ellipsis = true;
        s = &s[2..];
        if s.is_empty() {
            return Ok(true);
        }
    }
    let mut used = 0;
    while used < 16 {
        let off = s.iter().take_while(|c| c.is_ascii_hexdigit()).count();
        if off > 4 {
            return Err(fail("each group must have 4 or less digits", s));
        }
        if off == 0 {
            return Err(fail(
                "each colon-separated field must have at least one digit",
                s,
            ));
        }
        if s.get(off) == Some(&b'.') {
            if !ellipsis && used != 12 {
                return Err(fail(
                    "embedded IPv4 address must replace the final 2 fields of the address",
                    s,
                ));
            }
            if used + 4 > 16 {
                return Err(fail(
                    "too many hex fields to fit an embedded IPv4 at the end of the address",
                    s,
                ));
            }
            ipv4(input, s)?;
            s = b"";
            used += 4;
            break;
        }
        used += 2;
        s = &s[off..];
        if s.is_empty() {
            break;
        }
        if s[0] != b':' {
            return Err(fail("unexpected character, want colon", s));
        }
        if s.len() == 1 {
            return Err(fail("colon must be followed by more characters", s));
        }
        s = &s[1..];
        if s[0] == b':' {
            if ellipsis {
                return Err(fail("multiple :: in address", s));
            }
            ellipsis = true;
            s = &s[1..];
            if s.is_empty() {
                break;
            }
        }
    }
    if !s.is_empty() {
        return Err(fail("trailing garbage after address", s));
    }
    if used < 16 && !ellipsis {
        return Err(fail("address string too short", b""));
    }
    if used == 16 && ellipsis {
        return Err(fail(
            "the :: must expand to at least one field of zeros",
            b"",
        ));
    }
    Ok(true)
}

/// Complete exported net/url.URL fields for a validated graph attribute.
/// RawPath/RawFragment retain noncanonical authored escapes, as in Go setPath/
/// setFragment; this is independent of shortened-link presentation.
pub fn graph_url_metadata(input: &[u8]) -> Result<serde_json::Value, String> {
    use d2_rs_core::ast::ByteString;
    use serde_json::json;
    let parsed = parse(input, false)?;
    let (raw, fragment) = input
        .iter()
        .position(|c| *c == b'#')
        .map_or((input, &[][..]), |i| (&input[..i], &input[i + 1..]));
    let fragment_decoded = unescape(fragment, Mode::Plain)?;
    let escape_fragment = |bytes: &[u8]| {
        let mut result = Vec::new();
        let hex = b"0123456789ABCDEF";
        for &c in bytes {
            if c.is_ascii_alphanumeric() || b"-_.~$&+,/:;=?@!()*".contains(&c) {
                result.push(c);
            } else {
                result.extend([b'%', hex[(c >> 4) as usize], hex[(c & 15) as usize]]);
            }
        }
        result
    };
    let mut rest = raw;
    if !parsed.scheme.is_empty() {
        rest = &rest[rest.iter().position(|c| *c == b':').unwrap() + 1..];
    }
    let mut query = &[][..];
    let mut force_query = false;
    if let Some(i) = rest.iter().position(|c| *c == b'?') {
        query = &rest[i + 1..];
        force_query = i + 1 == rest.len();
        rest = &rest[..i];
    }
    let mut opaque = &[][..];
    let mut path = &[][..];
    let mut user = false;
    let mut omit_host = false;
    if !parsed.scheme.is_empty() && !rest.starts_with(b"/") {
        opaque = rest;
    } else {
        if (!parsed.scheme.is_empty() || !rest.starts_with(b"///")) && rest.starts_with(b"//") {
            let authority = &rest[2..];
            let end = authority
                .iter()
                .position(|c| *c == b'/')
                .unwrap_or(authority.len());
            user = authority[..end].contains(&b'@');
            rest = &authority[end..];
        } else {
            omit_host = !parsed.scheme.is_empty() && rest.starts_with(b"/");
        }
        path = rest;
    }
    Ok(
        json!({"Scheme":ByteString::from(parsed.scheme),"Opaque":ByteString::from(opaque),
        "User":if user {json!({})}else{serde_json::Value::Null},"Host":ByteString::from(parsed.host),
        "Path":ByteString::from(unescape(path,Mode::Plain)?),"RawPath":ByteString::from(parsed.raw_path),
        "OmitHost":omit_host,"ForceQuery":force_query,"RawQuery":ByteString::from(query),
        "Fragment":ByteString::from(fragment_decoded.clone()),
        "RawFragment":ByteString::from(if escape_fragment(&fragment_decoded)==fragment {Vec::new()}else{fragment.to_vec()})}),
    )
}
