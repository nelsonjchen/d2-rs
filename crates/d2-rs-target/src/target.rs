// SPDX-License-Identifier: MPL-2.0
//! The d2target.Diagram JSON boundary read by d2isometric (d2target/d2target.go
//! at the pinned master). Field order and `omitempty` follow the Go struct
//! tags, so a decoded and re-encoded value matches Go's encoding/json output;
//! Go's BuildScene performs the same round trip to own its copy.

use serde::{Deserialize, Serialize};

fn is_false(value: &bool) -> bool {
    !*value
}
fn is_zero_i64(value: &i64) -> bool {
    *value == 0
}
fn is_zero_f64(value: &f64) -> bool {
    *value == 0.0
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Diagram {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<Config>,
    #[serde(rename = "isFolderOnly")]
    pub is_folder_only: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(rename = "fontFamily", skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(rename = "monoFontFamily", skip_serializing_if = "Option::is_none")]
    pub mono_font_family: Option<String>,
    pub shapes: Option<Vec<Shape>>,
    pub connections: Option<Vec<Connection>>,
    pub root: Shape,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legend: Option<Legend>,
    #[serde(skip_serializing_if = "empty_boards")]
    pub layers: Option<Vec<Option<Diagram>>>,
    #[serde(skip_serializing_if = "empty_boards")]
    pub scenarios: Option<Vec<Option<Diagram>>>,
    #[serde(skip_serializing_if = "empty_boards")]
    pub steps: Option<Vec<Option<Diagram>>>,
}

fn empty_boards(boards: &Option<Vec<Option<Diagram>>>) -> bool {
    boards.as_ref().is_none_or(Vec::is_empty)
}

impl Diagram {
    pub fn shapes(&self) -> &[Shape] {
        self.shapes.as_deref().unwrap_or_default()
    }
    pub fn connections(&self) -> &[Connection] {
        self.connections.as_deref().unwrap_or_default()
    }
    pub fn has_children(&self) -> bool {
        [&self.layers, &self.scenarios, &self.steps]
            .iter()
            .any(|boards| !empty_boards(boards))
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub sketch: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isometric: Option<bool>,
    #[serde(rename = "themeID")]
    pub theme_id: Option<i64>,
    #[serde(rename = "darkThemeID")]
    pub dark_theme_id: Option<i64>,
    pub pad: Option<i64>,
    pub center: Option<bool>,
    #[serde(rename = "layoutEngine")]
    pub layout_engine: Option<String>,
    #[serde(rename = "themeOverrides", skip_serializing_if = "Option::is_none")]
    pub theme_overrides: Option<ThemeOverrides>,
    #[serde(rename = "darkThemeOverrides", skip_serializing_if = "Option::is_none")]
    pub dark_theme_overrides: Option<ThemeOverrides>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct ThemeOverrides {
    pub n1: Option<String>,
    pub n2: Option<String>,
    pub n3: Option<String>,
    pub n4: Option<String>,
    pub n5: Option<String>,
    pub n6: Option<String>,
    pub n7: Option<String>,
    pub b1: Option<String>,
    pub b2: Option<String>,
    pub b3: Option<String>,
    pub b4: Option<String>,
    pub b5: Option<String>,
    pub b6: Option<String>,
    pub aa2: Option<String>,
    pub aa4: Option<String>,
    pub aa5: Option<String>,
    pub ab4: Option<String>,
    pub ab5: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Legend {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(skip_serializing_if = "slice_empty")]
    pub shapes: Option<Vec<Shape>>,
    #[serde(skip_serializing_if = "slice_empty")]
    pub connections: Option<Vec<Connection>>,
}

fn slice_empty<T>(value: &Option<Vec<T>>) -> bool {
    value.as_ref().is_none_or(Vec::is_empty)
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Point {
    pub x: i64,
    pub y: i64,
}

/// lib/geo.Point.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct GeoPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Text {
    pub label: String,
    #[serde(rename = "fontSize")]
    pub font_size: i64,
    #[serde(rename = "fontFamily")]
    pub font_family: String,
    pub language: String,
    pub color: String,
    pub italic: bool,
    pub bold: bool,
    pub underline: bool,
    #[serde(rename = "labelWidth")]
    pub label_width: i64,
    #[serde(rename = "labelHeight")]
    pub label_height: i64,
    #[serde(rename = "labelFill", skip_serializing_if = "String::is_empty")]
    pub label_fill: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ClassField {
    pub name: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub visibility: String,
    pub underline: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ClassMethod {
    pub name: String,
    #[serde(rename = "return")]
    pub return_: String,
    pub visibility: String,
    pub underline: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct SqlColumn {
    pub name: Text,
    #[serde(rename = "type")]
    pub type_: Text,
    pub constraint: Option<Vec<String>>,
    pub reference: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Shape {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(skip_serializing_if = "slice_empty")]
    pub classes: Option<Vec<String>>,
    pub pos: Point,
    pub width: i64,
    pub height: i64,
    pub opacity: f64,
    #[serde(rename = "strokeDash")]
    pub stroke_dash: f64,
    #[serde(rename = "strokeWidth")]
    pub stroke_width: i64,
    #[serde(rename = "borderRadius")]
    pub border_radius: i64,
    pub fill: String,
    #[serde(rename = "fillPattern", skip_serializing_if = "String::is_empty")]
    pub fill_pattern: String,
    pub stroke: String,
    pub animated: bool,
    pub shadow: bool,
    #[serde(rename = "3d")]
    pub three_dee: bool,
    pub multiple: bool,
    #[serde(rename = "double-border")]
    pub double_border: bool,
    pub tooltip: String,
    pub link: String,
    #[serde(rename = "prettyLink", skip_serializing_if = "String::is_empty")]
    pub pretty_link: String,
    pub icon: Option<Url>,
    #[serde(rename = "iconBorderRadius", skip_serializing_if = "is_zero_i64")]
    pub icon_border_radius: i64,
    #[serde(rename = "iconPosition")]
    pub icon_position: String,
    pub blend: bool,
    // Embedded Class and SQLTable.
    pub fields: Option<Vec<ClassField>>,
    pub methods: Option<Vec<ClassMethod>>,
    pub columns: Option<Vec<SqlColumn>>,
    #[serde(rename = "contentAspectRatio", skip_serializing_if = "Option::is_none")]
    pub content_aspect_ratio: Option<f64>,
    #[serde(flatten)]
    pub text: Text,
    #[serde(rename = "labelPosition", skip_serializing_if = "String::is_empty")]
    pub label_position: String,
    #[serde(rename = "tooltipPosition", skip_serializing_if = "String::is_empty")]
    pub tooltip_position: String,
    #[serde(rename = "zIndex")]
    pub z_index: i64,
    pub level: i64,
    #[serde(
        rename = "primaryAccentColor",
        skip_serializing_if = "String::is_empty"
    )]
    pub primary_accent_color: String,
    #[serde(
        rename = "secondaryAccentColor",
        skip_serializing_if = "String::is_empty"
    )]
    pub secondary_accent_color: String,
    #[serde(
        rename = "neutralAccentColor",
        skip_serializing_if = "String::is_empty"
    )]
    pub neutral_accent_color: String,
}

pub const SHAPE_RECTANGLE: &str = "rectangle";
pub const SHAPE_SQUARE: &str = "square";
pub const SHAPE_PAGE: &str = "page";
pub const SHAPE_PARALLELOGRAM: &str = "parallelogram";
pub const SHAPE_DOCUMENT: &str = "document";
pub const SHAPE_CYLINDER: &str = "cylinder";
pub const SHAPE_QUEUE: &str = "queue";
pub const SHAPE_PACKAGE: &str = "package";
pub const SHAPE_STEP: &str = "step";
pub const SHAPE_CALLOUT: &str = "callout";
pub const SHAPE_STORED_DATA: &str = "stored_data";
pub const SHAPE_PERSON: &str = "person";
pub const SHAPE_C4_PERSON: &str = "c4-person";
pub const SHAPE_DIAMOND: &str = "diamond";
pub const SHAPE_OVAL: &str = "oval";
pub const SHAPE_CIRCLE: &str = "circle";
pub const SHAPE_HEXAGON: &str = "hexagon";
pub const SHAPE_CLOUD: &str = "cloud";
pub const SHAPE_TEXT: &str = "text";
pub const SHAPE_CODE: &str = "code";
pub const SHAPE_CLASS: &str = "class";
pub const SHAPE_SQL_TABLE: &str = "sql_table";
pub const SHAPE_IMAGE: &str = "image";
pub const SHAPE_SEQUENCE_DIAGRAM: &str = "sequence_diagram";
pub const SHAPE_HIERARCHY: &str = "hierarchy";

pub const THREE_DEE_OFFSET: i64 = 15;
pub const MULTIPLE_OFFSET: i64 = 10;
pub const INNER_BORDER_OFFSET: i64 = 5;

impl Shape {
    /// Shape.GetFontColor.
    pub fn font_color(&self) -> String {
        if self.type_ == SHAPE_CLASS || self.type_ == SHAPE_SQL_TABLE {
            if !crate::theme::is_theme_color(&self.text.color) {
                return self.text.color.clone();
            }
            return self.stroke.clone();
        }
        if !self.text.color.is_empty() {
            return self.text.color.clone();
        }
        "N1".to_owned()
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Connection {
    pub id: String,
    #[serde(skip_serializing_if = "slice_empty")]
    pub classes: Option<Vec<String>>,
    pub src: String,
    #[serde(rename = "srcArrow")]
    pub src_arrow: String,
    #[serde(rename = "srcLabel", skip_serializing_if = "Option::is_none")]
    pub src_label: Option<Text>,
    pub dst: String,
    #[serde(rename = "dstArrow")]
    pub dst_arrow: String,
    #[serde(rename = "dstLabel", skip_serializing_if = "Option::is_none")]
    pub dst_label: Option<Text>,
    pub opacity: f64,
    #[serde(rename = "strokeDash")]
    pub stroke_dash: f64,
    #[serde(rename = "strokeWidth")]
    pub stroke_width: i64,
    pub stroke: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub fill: String,
    #[serde(rename = "borderRadius", skip_serializing_if = "is_zero_f64")]
    pub border_radius: f64,
    #[serde(flatten)]
    pub text: Text,
    #[serde(rename = "labelPosition")]
    pub label_position: String,
    #[serde(rename = "labelPercentage")]
    pub label_percentage: f64,
    pub link: String,
    #[serde(rename = "prettyLink", skip_serializing_if = "String::is_empty")]
    pub pretty_link: String,
    pub route: Option<Vec<Option<GeoPoint>>>,
    #[serde(rename = "isCurve", skip_serializing_if = "is_false")]
    pub is_curve: bool,
    pub animated: bool,
    pub tooltip: String,
    pub icon: Option<Url>,
    #[serde(rename = "iconPosition", skip_serializing_if = "String::is_empty")]
    pub icon_position: String,
    #[serde(rename = "iconBorderRadius", skip_serializing_if = "is_zero_f64")]
    pub icon_border_radius: f64,
    #[serde(rename = "zIndex")]
    pub z_index: i64,
}

impl Connection {
    pub fn route(&self) -> &[Option<GeoPoint>] {
        self.route.as_deref().unwrap_or_default()
    }
}

/// Owned net/url.Userinfo. Credentials are private in Go's JSON boundary.
/// Cloning retains credentials; serializing and deserializing intentionally do not.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Userinfo {
    #[serde(skip)]
    username: String,
    #[serde(skip)]
    password: Option<String>,
}

impl Userinfo {
    /// net/url.User: no password, including when the username is empty.
    pub fn new(username: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: None,
        }
    }

    /// net/url.UserPassword: an explicitly empty password remains present.
    pub fn with_password(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: Some(password.into()),
        }
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn password(&self) -> Option<&str> {
        self.password.as_deref()
    }

    /// Decode an already separated authority userinfo component.
    /// This is not a URL parser. A plus sign is a literal plus.
    pub fn from_encoded(value: &str) -> Option<Self> {
        match value.split_once(':') {
            Some((username, password)) => Some(Self::with_password(
                unescape(username)?,
                unescape(password)?,
            )),
            None => Some(Self::new(unescape(value)?)),
        }
    }

    /// net/url.Userinfo.String, using encodeUserPassword for each component.
    pub fn string(&self) -> String {
        let mut value = escape(&self.username, Encoding::UserPassword);
        if let Some(password) = &self.password {
            value.push(':');
            value.push_str(&escape(password, Encoding::UserPassword));
        }
        value
    }
}

/// net/url.URL as encoding/json writes it (exported fields, declaration
/// order). Userinfo has only unexported fields and encodes as `{}`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Url {
    #[serde(rename = "Scheme")]
    pub scheme: String,
    #[serde(rename = "Opaque")]
    pub opaque: String,
    #[serde(rename = "User")]
    pub user: Option<Userinfo>,
    #[serde(rename = "Host")]
    pub host: String,
    #[serde(rename = "Path")]
    pub path: String,
    #[serde(rename = "RawPath")]
    pub raw_path: String,
    #[serde(rename = "OmitHost")]
    pub omit_host: bool,
    #[serde(rename = "ForceQuery")]
    pub force_query: bool,
    #[serde(rename = "RawQuery")]
    pub raw_query: String,
    #[serde(rename = "Fragment")]
    pub fragment: String,
    #[serde(rename = "RawFragment")]
    pub raw_fragment: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Encoding {
    Path,
    Host,
    Fragment,
    UserPassword,
}

/// net/url shouldEscape for the modes URL.String uses.
fn should_escape(c: u8, mode: Encoding) -> bool {
    if c.is_ascii_alphanumeric() {
        return false;
    }
    if mode == Encoding::Host {
        return !matches!(
            c,
            b'!' | b'$'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b'+'
                | b','
                | b';'
                | b'='
                | b':'
                | b'['
                | b']'
                | b'<'
                | b'>'
                | b'"'
        ) && !matches!(c, b'-' | b'_' | b'.' | b'~');
    }
    match c {
        b'-' | b'_' | b'.' | b'~' => false,
        b'$' | b'&' | b'+' | b',' | b'/' | b':' | b';' | b'=' | b'?' | b'@' => match mode {
            Encoding::Path => c == b'?',
            Encoding::UserPassword => matches!(c, b'@' | b'/' | b'?' | b':'),
            Encoding::Fragment => false,
            Encoding::Host => unreachable!(),
        },
        _ => !(mode == Encoding::Fragment && matches!(c, b'!' | b'(' | b')' | b'*')),
    }
}

fn escape(s: &str, mode: Encoding) -> String {
    let mut out = String::with_capacity(s.len());
    for &c in s.as_bytes() {
        if should_escape(c, mode) {
            out.push_str(&format!("%{c:02X}"));
        } else {
            out.push(c as char);
        }
    }
    out
}

fn unescape(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// validEncoded: raw is an acceptable encoding of its decoded form.
fn valid_encoded(s: &str, mode: Encoding) -> bool {
    for &c in s.as_bytes() {
        match c {
            b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'=' | b':'
            | b'@' | b'[' | b']' => {}
            b'%' => {}
            _ if should_escape(c, mode) => return false,
            _ => {}
        }
    }
    true
}

impl Url {
    fn escaped_path(&self) -> String {
        if !self.raw_path.is_empty()
            && valid_encoded(&self.raw_path, Encoding::Path)
            && unescape(&self.raw_path).as_deref() == Some(self.path.as_str())
        {
            return self.raw_path.clone();
        }
        if self.path == "*" {
            return "*".to_owned();
        }
        escape(&self.path, Encoding::Path)
    }

    fn escaped_fragment(&self) -> String {
        if !self.raw_fragment.is_empty()
            && valid_encoded(&self.raw_fragment, Encoding::Fragment)
            && unescape(&self.raw_fragment).as_deref() == Some(self.fragment.as_str())
        {
            return self.raw_fragment.clone();
        }
        escape(&self.fragment, Encoding::Fragment)
    }

    /// URL.String.
    pub fn string(&self) -> String {
        let mut buf = String::new();
        if !self.scheme.is_empty() {
            buf.push_str(&self.scheme);
            buf.push(':');
        }
        if !self.opaque.is_empty() {
            buf.push_str(&self.opaque);
        } else {
            if !self.scheme.is_empty() || !self.host.is_empty() || self.user.is_some() {
                if self.omit_host && self.host.is_empty() && self.user.is_none() {
                    // Omit the empty host.
                } else {
                    if !self.host.is_empty() || !self.path.is_empty() || self.user.is_some() {
                        buf.push_str("//");
                    }
                    if let Some(user) = &self.user {
                        buf.push_str(&user.string());
                        buf.push('@');
                    }
                    if !self.host.is_empty() {
                        buf.push_str(&escape(&self.host, Encoding::Host));
                    }
                }
            }
            let path = self.escaped_path();
            if !path.is_empty() && !path.starts_with('/') && !self.host.is_empty() {
                buf.push('/');
            }
            if buf.is_empty() {
                // RFC 3986 §4.2: a first segment with a colon needs "./".
                if let Some(colon) = path.find(':') {
                    if path.find('/').is_none_or(|slash| colon < slash) {
                        buf.push_str("./");
                    }
                }
            }
            buf.push_str(&path);
        }
        if self.force_query || !self.raw_query.is_empty() {
            buf.push('?');
            buf.push_str(&self.raw_query);
        }
        if !self.fragment.is_empty() {
            buf.push('#');
            buf.push_str(&self.escaped_fragment());
        }
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_string_prefers_valid_raw_path() {
        let url = Url {
            scheme: "https".into(),
            host: "icons.terrastruct.com".into(),
            path: "/essentials/213-alarm.svg".into(),
            raw_path: "/essentials%2F213-alarm.svg".into(),
            ..Default::default()
        };
        assert_eq!(
            url.string(),
            "https://icons.terrastruct.com/essentials%2F213-alarm.svg"
        );
        let plain = Url {
            raw_path: String::new(),
            path: "/a b.svg".into(),
            ..url
        };
        assert_eq!(plain.string(), "https://icons.terrastruct.com/a%20b.svg");
    }

    #[test]
    fn userinfo_distinguishes_absent_and_empty_password() {
        let user = Userinfo::new("user");
        let empty_password = Userinfo::with_password("user", "");
        assert_eq!(user.password(), None);
        assert_eq!(empty_password.password(), Some(""));
        assert_eq!(user.string(), "user");
        assert_eq!(empty_password.string(), "user:");
        assert_eq!(Userinfo::new("").string(), "");
        assert_eq!(Userinfo::with_password("", "").string(), ":");
    }

    #[test]
    fn userinfo_escapes_components_and_keeps_opaque_json() {
        let user = Userinfo::with_password("a:b@ /?%é", "$&+,;=:/?@!");
        assert_eq!(
            user.string(),
            "a%3Ab%40%20%2F%3F%25%C3%A9:$&+,;=%3A%2F%3F%40%21"
        );
        assert_eq!(serde_json::to_string(&user).unwrap(), "{}");
        let decoded: Userinfo = serde_json::from_str("{}").unwrap();
        assert_eq!(decoded, Userinfo::default());
        assert_eq!(Userinfo::from_encoded(&user.string()).unwrap(), user);
        assert_eq!(Userinfo::from_encoded("a+b").unwrap().username(), "a+b");
        assert!(Userinfo::from_encoded("%XZ").is_none());
    }

    #[test]
    fn url_clone_owns_credentials_without_json_exposure() {
        let source = Url {
            scheme: "https".into(),
            host: "example.test".into(),
            path: "/icon.svg".into(),
            raw_query: "variant=1".into(),
            user: Some(Userinfo::with_password("user", "pass")),
            ..Default::default()
        };
        let mut owned = source.clone();
        assert_eq!(
            owned.string(),
            "https://user:pass@example.test/icon.svg?variant=1"
        );
        assert_ne!(
            source.user.as_ref().unwrap().username().as_ptr(),
            owned.user.as_ref().unwrap().username().as_ptr()
        );
        assert_ne!(
            source.user.as_ref().unwrap().password().unwrap().as_ptr(),
            owned.user.as_ref().unwrap().password().unwrap().as_ptr()
        );
        owned.user = Some(Userinfo::new("changed"));
        assert_eq!(source.user.as_ref().unwrap().username(), "user");
        let encoded = serde_json::to_value(&source).unwrap();
        assert_eq!(encoded["User"], serde_json::json!({}));
        let decoded: Url = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded.string(), "https://@example.test/icon.svg?variant=1");
    }
}
