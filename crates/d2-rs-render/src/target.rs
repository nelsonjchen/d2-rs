// SPDX-License-Identifier: MPL-2.0
//! Native rendering of public D2 v0.9.0 target diagrams. Target coordinates,
//! routes and exported styles are authoritative; this path performs no layout.
//! SDK integration remains incomplete; this API is under qualification.
//! Source: d2target/d2target.go and d2renderers/d2svg/d2svg.go at the pinned revision.
use super::*;
use d2_rs_core::SourceSqlText;
use serde::Deserialize;
#[path = "d2svg/mod.rs"]
pub(crate) mod d2svg;
mod decode;
pub(super) mod from_graph;
use decode::{GoSlice, UrlUser};
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(transparent)]
struct GoFloat(f64);
impl Serialize for GoFloat {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if !self.0.is_finite() {
            return Err(serde::ser::Error::custom(
                "unsupported nonfinite target float",
            ));
        }
        let text = d2_rs_core::go_json_float(self.0);
        serde_json::value::RawValue::from_string(text)
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

fn go_float_is_zero(v: &GoFloat) -> bool {
    v.0 == 0.0
}
fn empty_string(v: &Option<String>) -> bool {
    v.as_deref().is_none_or(str::is_empty)
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(default)]
struct HashUrl {
    #[serde(rename = "Scheme")]
    scheme: String,
    #[serde(rename = "Opaque")]
    opaque: String,
    #[serde(rename = "User")]
    user: Option<UrlUser>,
    #[serde(rename = "Host")]
    host: String,
    #[serde(rename = "Path")]
    path: String,
    #[serde(rename = "RawPath")]
    raw_path: String,
    #[serde(rename = "OmitHost")]
    omit_host: bool,
    #[serde(rename = "ForceQuery")]
    force_query: bool,
    #[serde(rename = "RawQuery")]
    raw_query: String,
    #[serde(rename = "Fragment")]
    fragment: String,
    #[serde(rename = "RawFragment")]
    raw_fragment: String,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(default)]
struct HashIntPoint {
    x: i64,
    y: i64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(default)]
struct HashPoint {
    x: GoFloat,
    y: GoFloat,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(default)]
pub(crate) struct TargetShape {
    id: String,
    #[serde(rename = "type")]
    shape_type: String,
    #[serde(skip_serializing_if = "GoSlice::is_empty")]
    classes: GoSlice<String>,
    pos: HashIntPoint,
    width: i64,
    height: i64,
    opacity: GoFloat,
    #[serde(rename = "strokeDash")]
    stroke_dash: GoFloat,
    #[serde(rename = "strokeWidth")]
    stroke_width: i64,
    #[serde(rename = "borderRadius")]
    border_radius: i64,
    fill: String,
    #[serde(rename = "fillPattern", skip_serializing_if = "empty_string")]
    fill_pattern: Option<String>,
    stroke: String,
    animated: bool,
    shadow: bool,
    #[serde(rename = "3d")]
    three_dee: bool,
    multiple: bool,
    #[serde(rename = "double-border")]
    double_border: bool,
    tooltip: String,
    link: String,
    #[serde(rename = "prettyLink", skip_serializing_if = "empty_string")]
    pretty_link: Option<String>,
    icon: Option<HashUrl>,
    #[serde(rename = "iconBorderRadius", skip_serializing_if = "i64_is_zero")]
    icon_border_radius: i64,
    #[serde(rename = "iconPosition")]
    icon_position: String,
    blend: bool,
    fields: Option<GoSlice<TargetClassField>>,
    methods: Option<GoSlice<TargetClassMethod>>,
    columns: Option<GoSlice<TargetSqlColumn>>,
    #[serde(rename = "contentAspectRatio", skip_serializing_if = "Option::is_none")]
    content_aspect_ratio: Option<GoFloat>,
    label: String,
    #[serde(rename = "fontSize")]
    font_size: i64,
    #[serde(rename = "fontFamily")]
    font_family: String,
    language: String,
    color: String,
    italic: bool,
    bold: bool,
    underline: bool,
    #[serde(rename = "labelWidth")]
    label_width: i64,
    #[serde(rename = "labelHeight")]
    label_height: i64,
    #[serde(rename = "labelFill", skip_serializing_if = "empty_string")]
    label_fill: Option<String>,
    #[serde(rename = "labelPosition", skip_serializing_if = "empty_string")]
    label_position: Option<String>,
    #[serde(rename = "tooltipPosition", skip_serializing_if = "empty_string")]
    tooltip_position: Option<String>,
    #[serde(rename = "zIndex")]
    z_index: i64,
    level: i64,
    #[serde(
        rename = "primaryAccentColor",
        skip_serializing_if = "String::is_empty"
    )]
    primary_accent_color: String,
    #[serde(
        rename = "secondaryAccentColor",
        skip_serializing_if = "String::is_empty"
    )]
    secondary_accent_color: String,
    #[serde(
        rename = "neutralAccentColor",
        skip_serializing_if = "String::is_empty"
    )]
    neutral_accent_color: String,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(default)]
pub(crate) struct TargetConnection {
    id: String,
    #[serde(skip_serializing_if = "GoSlice::is_empty")]
    classes: GoSlice<String>,
    src: String,
    #[serde(rename = "srcArrow")]
    src_arrow: String,
    #[serde(rename = "srcLabel", skip_serializing_if = "Option::is_none")]
    src_label: Option<TargetText>,
    dst: String,
    #[serde(rename = "dstArrow")]
    dst_arrow: String,
    #[serde(rename = "dstLabel", skip_serializing_if = "Option::is_none")]
    dst_label: Option<TargetText>,
    opacity: GoFloat,
    #[serde(rename = "strokeDash")]
    stroke_dash: GoFloat,
    #[serde(rename = "strokeWidth")]
    stroke_width: i64,
    stroke: String,
    #[serde(skip_serializing_if = "empty_string")]
    fill: Option<String>,
    #[serde(rename = "borderRadius", skip_serializing_if = "go_float_is_zero")]
    border_radius: GoFloat,
    label: String,
    #[serde(rename = "fontSize")]
    font_size: i64,
    #[serde(rename = "fontFamily")]
    font_family: String,
    language: String,
    color: String,
    italic: bool,
    bold: bool,
    underline: bool,
    #[serde(rename = "labelWidth")]
    label_width: i64,
    #[serde(rename = "labelHeight")]
    label_height: i64,
    #[serde(rename = "labelFill", skip_serializing_if = "String::is_empty")]
    label_fill: String,
    #[serde(rename = "labelPosition")]
    label_position: String,
    #[serde(rename = "labelPercentage")]
    label_percentage: GoFloat,
    link: String,
    #[serde(rename = "prettyLink", skip_serializing_if = "String::is_empty")]
    pretty_link: String,
    route: Option<GoSlice<Option<HashPoint>>>,
    #[serde(rename = "isCurve", skip_serializing_if = "is_false_hash")]
    is_curve: bool,
    animated: bool,
    tooltip: String,
    icon: Option<HashUrl>,
    #[serde(rename = "iconPosition", skip_serializing_if = "empty_string")]
    icon_position: Option<String>,
    #[serde(rename = "iconBorderRadius", skip_serializing_if = "go_float_is_zero")]
    icon_border_radius: GoFloat,
    #[serde(rename = "zIndex")]
    z_index: i64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct TargetText {
    label: String,
    font_size: i64,
    font_family: String,
    language: String,
    color: String,
    italic: bool,
    bold: bool,
    underline: bool,
    label_width: i64,
    label_height: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    label_fill: String,
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(default)]
pub(crate) struct TargetSqlColumn {
    name: TargetText,
    #[serde(rename = "type")]
    type_text: TargetText,
    constraint: Option<GoSlice<String>>,
    reference: String,
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(default)]
struct TargetClassField {
    name: String,
    #[serde(rename = "type")]
    type_text: String,
    visibility: String,
    underline: bool,
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(default)]
struct TargetClassMethod {
    name: String,
    #[serde(rename = "return")]
    return_type: String,
    visibility: String,
    underline: bool,
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct TargetConfig {
    sketch: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    isometric: Option<bool>,
    #[serde(rename = "themeID")]
    theme_id: Option<i64>,
    #[serde(rename = "darkThemeID")]
    dark_theme_id: Option<i64>,
    pad: Option<i64>,
    center: Option<bool>,
    layout_engine: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    theme_overrides: Option<TargetThemeOverrides>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dark_theme_overrides: Option<TargetThemeOverrides>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    #[serde(serialize_with = "decode::serialize_data")]
    data: BTreeMap<String, Value>,
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(default)]
pub struct TargetThemeOverrides {
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
/// Public v0.9.0 target, independent of source text or private SDK metadata.
/// Its field order is also the declaration order used by Go's target encoder.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TargetDiagram {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    config: Option<TargetConfig>,
    is_folder_only: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    font_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mono_font_family: Option<String>,
    shapes: Option<GoSlice<TargetShape>>,
    connections: Option<GoSlice<TargetConnection>>,
    root: TargetShape,
    #[serde(skip_serializing_if = "Option::is_none")]
    legend: Option<TargetLegend>,
    #[serde(skip_serializing_if = "empty_boards")]
    layers: Option<GoSlice<Option<TargetDiagram>>>,
    #[serde(skip_serializing_if = "empty_boards")]
    scenarios: Option<GoSlice<Option<TargetDiagram>>>,
    #[serde(skip_serializing_if = "empty_boards")]
    steps: Option<GoSlice<Option<TargetDiagram>>>,
}
fn empty_slice<T>(v: &Option<GoSlice<T>>) -> bool {
    v.as_ref().is_none_or(GoSlice::is_empty)
}
fn empty_boards(v: &Option<GoSlice<Option<TargetDiagram>>>) -> bool {
    v.as_ref().is_none_or(GoSlice::is_empty)
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(default)]
struct TargetLegend {
    #[serde(skip_serializing_if = "String::is_empty")]
    label: String,
    #[serde(skip_serializing_if = "empty_slice")]
    shapes: Option<GoSlice<TargetShape>>,
    #[serde(skip_serializing_if = "empty_slice")]
    connections: Option<GoSlice<TargetConnection>>,
}
impl TargetDiagram {
    /// Decode the public Go JSON boundary without first parsing into Value:
    /// duplicate fields, nulls and invalid Unicode retain encoding/json v1
    /// behavior. A null top-level pointer is distinct from an empty diagram.
    pub fn decode_json(input: &[u8]) -> Result<Option<Self>, serde_json::Error> {
        serde_json::from_str(&decode::normalize_json(input))
    }
    /// Validate the entire supplied target before selecting or dropping boards,
    /// following d2target.ValidateRenderTarget at the pinned master revision.
    pub fn validate_render_target(&self) -> Result<(), String> {
        d2svg::target_validation::validate(self)
    }
    /// d2svg link, structure and paint admission without rendering flat SVG.
    pub fn validate_native_svg(&self, options: &RenderOptions) -> Result<(), String> {
        d2svg::validate_native_svg(self, options)
    }
    /// Direct document admission, retaining absent versus empty overrides and
    /// exact integer padding. Used by the higher-level isometric dispatcher.
    pub fn validate_svg_document(&self, options: &crate::SvgRenderOptions) -> Result<(), String> {
        d2svg::validate_document(self, options)
    }
    /// Flat d2svg primitive. Use d2-rs-isometric::svg::render for mode dispatch.
    pub fn render_flat_svg(&self, options: &crate::SvgRenderOptions) -> Result<Vec<u8>, String> {
        d2svg::render::render(self, options)
    }
    /// d2animate.Wrap: combine supplied SVG frames using the target's bounds,
    /// fonts and theme. Absent padding follows D2's default padding.
    pub fn wrap_svg_frames(
        &self,
        frames: &[Vec<u8>],
        options: &crate::SvgRenderOptions,
        interval_ms: i64,
    ) -> Result<Vec<u8>, String> {
        d2svg::animate::wrap(self, frames, options, interval_ms)
    }
    /// Exact Diagram.Bytes input, including child boards in source traversal
    /// order. No layout, source reconstruction or SVG parsing participates.
    pub fn hash_input(&self) -> Result<Vec<u8>, serde_json::Error> {
        let mut bytes = Vec::new();
        bytes.extend(escape_go_hash_json(&serde_json::to_vec(&self.shapes)?));
        bytes.extend(escape_go_hash_json(&serde_json::to_vec(&self.connections)?));
        bytes.extend(escape_go_hash_json(&serde_json::to_vec(&self.root)?));
        if let Some(config) = &self.config {
            bytes.extend(escape_go_hash_json(&serde_json::to_vec(config)?));
        }
        for board in self
            .layers
            .iter()
            .chain(self.scenarios.iter())
            .chain(self.steps.iter())
            .flatten()
        {
            let board = board.as_ref().ok_or_else(|| {
                serde_json::Error::io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "null target board",
                ))
            })?;
            bytes.extend(board.hash_input()?);
        }
        Ok(bytes)
    }
    /// Child boards in layers, scenarios, steps order; nil entries are omitted
    /// here and rejected separately by complete target validation.
    pub fn children(&self) -> impl Iterator<Item = &TargetDiagram> {
        self.layers
            .iter()
            .chain(self.scenarios.iter())
            .chain(self.steps.iter())
            .flatten()
            .flatten()
    }
    /// Original GetBoard accepts explicit wrappers and unqualified child names.
    pub fn get_board(&self, path: &[String]) -> Option<&Self> {
        let Some((head, tail)) = path.split_first() else {
            return Some(self);
        };
        if tail.is_empty() && self.name == *head {
            return Some(self);
        }
        let group = match head.as_str() {
            "layers" => Some(&self.layers),
            "scenarios" => Some(&self.scenarios),
            "steps" => Some(&self.steps),
            _ => None,
        };
        if let Some(group) = group {
            if let Some((name, rest)) = tail.split_first() {
                if let Some(board) = group.iter().flatten().flatten().find(|b| b.name == *name) {
                    return board.get_board(rest);
                }
            }
        }
        self.children().find(|b| b.name == *head)?.get_board(tail)
    }
    /// Drops descendants before single-board hashing, as Go Render does.
    pub fn without_children(&self) -> Self {
        let mut d = self.clone();
        d.layers = None;
        d.scenarios = None;
        d.steps = None;
        d
    }

    pub fn is_folder_only(&self) -> bool {
        self.is_folder_only
    }
}

fn position(s: &str) -> LabelPosition {
    serde_json::from_value(Value::String(s.into())).unwrap_or_default()
}
fn font(s: &str) -> String {
    match s {
        "MONO" => "mono",
        "HAND_DRAWN" => "hand-drawn",
        _ => s,
    }
    .into()
}
fn shape(s: &str) -> Result<ShapeKind, String> {
    match s {
        "rectangle" => Ok(ShapeKind::Rectangle),
        "sql_table" => Ok(ShapeKind::SqlTable),
        "stored_data" => Ok(ShapeKind::StoredData),
        "c4-person" => Ok(ShapeKind::C4Person),
        s => serde_json::from_value(Value::String(s.to_owned()))
            .map_err(|_| format!("unsupported target shape {s:?}")),
    }
}
/// net/url (Go 1.27) encoding modes used by URL.String.
#[derive(Clone, Copy, PartialEq, Eq)]
enum UrlEncoding {
    Path,
    Host,
    Fragment,
}

/// net/url shouldEscape (gen_encoding_table.go).
fn url_should_escape(c: u8, mode: UrlEncoding) -> bool {
    if c.is_ascii_alphanumeric() {
        return false;
    }
    if mode == UrlEncoding::Host
        && matches!(
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
        )
    {
        return false;
    }
    match c {
        b'-' | b'_' | b'.' | b'~' => return false,
        b'$' | b'&' | b'+' | b',' | b'/' | b':' | b';' | b'=' | b'?' | b'@' => match mode {
            UrlEncoding::Path => return c == b'?',
            UrlEncoding::Fragment => return false,
            UrlEncoding::Host => {}
        },
        _ => {}
    }
    !(mode == UrlEncoding::Fragment && matches!(c, b'!' | b'(' | b')' | b'*'))
}

/// net/url escape (no query-component mode is needed here).
fn url_escape(s: &str, mode: UrlEncoding) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len());
    for &c in s.as_bytes() {
        if url_should_escape(c, mode) {
            out.push('%');
            out.push(HEX[(c >> 4) as usize] as char);
            out.push(HEX[(c & 15) as usize] as char);
        } else {
            out.push(c as char);
        }
    }
    out
}

/// net/url validEncoded.
fn url_valid_encoded(s: &str, mode: UrlEncoding) -> bool {
    s.bytes().all(|c| {
        matches!(
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
                | b'@'
                | b'['
                | b']'
                | b'%'
        ) || !url_should_escape(c, mode)
    })
}

/// net/url unescape for path/fragment modes (no '+' handling).
fn url_unescape(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = |b: u8| (b as char).to_digit(16);
            let (hi, lo) = (hex(*bytes.get(i + 1)?)?, hex(*bytes.get(i + 2)?)?);
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// URL.EscapedPath / EscapedFragment.
fn url_escaped(raw: &str, decoded: &str, mode: UrlEncoding) -> String {
    if !raw.is_empty()
        && url_valid_encoded(raw, mode)
        && url_unescape(raw).as_deref() == Some(decoded)
    {
        return raw.to_owned();
    }
    if mode == UrlEncoding::Path && decoded == "*" {
        return "*".to_owned();
    }
    url_escape(decoded, mode)
}

/// net/url URL.String (Go 1.27). The target's Userinfo is encoded as `{}`,
/// so a present user contributes only its `@`.
fn url(u: &HashUrl) -> String {
    let mut buf = String::new();
    if !u.scheme.is_empty() {
        buf.push_str(&u.scheme);
        buf.push(':');
    }
    if !u.opaque.is_empty() {
        buf.push_str(&u.opaque);
    } else {
        let has_user = u.user.is_some();
        if (!u.scheme.is_empty() || !u.host.is_empty() || has_user)
            && !(u.omit_host && u.host.is_empty() && !has_user)
        {
            if !u.host.is_empty() || !u.path.is_empty() || has_user {
                buf.push_str("//");
            }
            if has_user {
                buf.push('@');
            }
            if !u.host.is_empty() {
                buf.push_str(&url_escape(&u.host, UrlEncoding::Host));
            }
        }
        let mut path = url_escaped(&u.raw_path, &u.path, UrlEncoding::Path);
        if u.omit_host && u.host.is_empty() && !has_user && path.starts_with("//") {
            buf.push_str("%2F");
            path.remove(0);
        }
        if !path.is_empty() && !path.starts_with('/') && !u.host.is_empty() {
            buf.push('/');
        }
        if buf.is_empty()
            && path
                .split('/')
                .next()
                .is_some_and(|segment| segment.contains(':'))
        {
            buf.push_str("./");
        }
        buf.push_str(&path);
    }
    if u.force_query || !u.raw_query.is_empty() {
        buf.push('?');
        buf.push_str(&u.raw_query);
    }
    if !u.fragment.is_empty() {
        buf.push('#');
        buf.push_str(&url_escaped(
            &u.raw_fragment,
            &u.fragment,
            UrlEncoding::Fragment,
        ));
    }
    buf
}
fn member_name(name: &str, visibility: &str) -> String {
    format!(
        "{}{}",
        match visibility {
            "private" => "-",
            "protected" => "#",
            _ => "+",
        },
        name
    )
}
impl TargetText {
    fn source(&self) -> SourceSqlText {
        SourceSqlText {
            label: self.label.clone(),
            font_size: self.font_size as u32,
            font_family: self.font_family.clone(),
            language: self.language.clone(),
            color: self.color.clone(),
            italic: self.italic,
            bold: self.bold,
            underline: self.underline,
            label_width: self.label_width as u32,
            label_height: self.label_height as u32,
        }
    }
}
impl TargetShape {
    fn style(&self) -> SourceNodeStyle {
        SourceNodeStyle {
            fill: Some(self.fill.clone()),
            fill_pattern: self.fill_pattern.clone(),
            stroke: Some(self.stroke.clone()),
            stroke_width: Some(self.stroke_width.to_string()),
            opacity: Some(self.opacity.0.to_string()),
            stroke_dash: Some(self.stroke_dash.0.to_string()),
            animated: Some(self.animated.to_string()),
            border_radius: Some(self.border_radius.to_string()),
            shadow: Some(self.shadow.to_string()),
            three_dee: Some(self.three_dee.to_string()),
            multiple: Some(self.multiple.to_string()),
            double_border: Some(self.double_border.to_string()),
            font_color: Some(
                if matches!(self.shape_type.as_str(), "class" | "sql_table")
                    && resolve_color(theme::palette(0).unwrap(), &self.color).is_some()
                {
                    self.stroke.clone()
                } else {
                    self.color.clone()
                },
            ),
            bold: self.bold,
            italic: self.italic,
            underline: self.underline,
            font: Some(font(&self.font_family)),
            font_size: Some(self.font_size as u32),
            target_font_size: Some(self.font_size as u32),
            target_accents: Some(SourceTargetAccents {
                primary_accent_color: self.primary_accent_color.clone(),
                secondary_accent_color: self.secondary_accent_color.clone(),
                neutral_accent_color: self.neutral_accent_color.clone(),
            }),
            language: Some(self.language.clone()),
            markdown: self.language == "markdown",
            class_fields: self
                .fields
                .iter()
                .flatten()
                .map(|f| (member_name(&f.name, &f.visibility), f.type_text.clone()))
                .collect(),
            class_methods: self
                .methods
                .iter()
                .flatten()
                .map(|f| (member_name(&f.name, &f.visibility), f.return_type.clone()))
                .collect(),
            class_field_underlines: self.fields.iter().flatten().map(|f| f.underline).collect(),
            class_method_underlines: self.methods.iter().flatten().map(|f| f.underline).collect(),
            table_target_columns: self
                .columns
                .iter()
                .flatten()
                .map(|c| SourceSqlColumn {
                    name: c.name.source(),
                    type_text: c.type_text.source(),
                    constraint: c.constraint.as_ref().map(|v| v.to_vec()),
                    reference: c.reference.clone(),
                })
                .collect(),
            table_rows: self
                .columns
                .iter()
                .flatten()
                .map(|c| {
                    (
                        c.name.label.clone(),
                        c.type_text.label.clone(),
                        c.constraint
                            .iter()
                            .flatten()
                            .map(|s| match s.as_str() {
                                "primary_key" => "PK",
                                "foreign_key" => "FK",
                                "unique" => "UNQ",
                                s => s,
                            })
                            .collect::<Vec<_>>()
                            .join(", "),
                    )
                })
                .collect(),
            table_max_name_width: self
                .columns
                .iter()
                .flatten()
                .map(|c| c.name.label_width as u32)
                .max()
                .unwrap_or(0),
            link: Some(self.link.clone()),
            // PrettyLink is already exporter-owned in a public target. An
            // absent value is the Go zero string, not permission to infer Link.
            pretty_link: Some(self.pretty_link.clone().unwrap_or_default()),
            tooltip: Some(self.tooltip.clone()),
            tooltip_position: self.tooltip_position.clone(),
            icon: self.icon.as_ref().map(url),
            icon_border_radius: Some(self.icon_border_radius.to_string()),
            content_aspect_ratio_bits: self.content_aspect_ratio.as_ref().map(|f| f.0.to_bits()),
            ..Default::default()
        }
    }
}

/// Raw target paint metadata that cannot be reconstructed from an authored
/// graph: the diagram's font families.
pub(super) struct PaintContext<'a> {
    diagram: &'a TargetDiagram,
}
impl PaintContext<'_> {
    /// Diagram font metadata is independent of RenderOpts.Sketch in Go.
    pub(super) fn primary_family(&self) -> super::MarkdownFamily {
        let family = |name: Option<&str>, default| match name {
            Some("SourceCodePro") => super::BundledFamily::Mono,
            Some("HandDrawn") => super::BundledFamily::HandDrawn,
            Some("SourceSansPro") => super::BundledFamily::Sans,
            _ => default,
        };
        super::MarkdownFamily::Pair {
            primary: family(
                self.diagram.font_family.as_deref(),
                super::BundledFamily::Sans,
            ),
            mono: family(
                self.diagram.mono_font_family.as_deref(),
                super::BundledFamily::Mono,
            ),
            mono_label: false,
        }
    }
}

impl TargetDiagram {
    fn lower_geometry(
        &self,
        allow_unknown_shape: bool,
    ) -> Result<
        (
            Graph,
            LayoutResult,
            BTreeMap<NodeId, String>,
            BTreeMap<NodeId, SourceNodeStyle>,
        ),
        String,
    > {
        if self
            .connections
            .iter()
            .flatten()
            .any(|c| c.route.iter().flatten().any(Option::is_none))
        {
            return Err("null target route point".into());
        }
        let mut graph = Graph::default();
        graph.target_routes_exported = true;
        let mut result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: BTreeMap::new(),
            sequence_lifelines: Vec::new(),
            boxes: BTreeMap::new(),
            routes: BTreeMap::new(),
            edge_labels: BTreeMap::new(),
            edge_label_percentage_commits: BTreeSet::new(),
            node_labels: BTreeMap::new(),
            report: d2_rs_core::LayoutReport {
                selected_seed: 0,
                selected_pass: 0,
                score: d2_rs_core::Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        let mut labels = BTreeMap::new();
        let mut styles = BTreeMap::new();
        let mut ids = BTreeMap::new();
        for s in self.shapes.iter().flatten() {
            let mut n = Node::new(
                &s.id,
                Size {
                    width: s.width as f64,
                    height: s.height as f64,
                },
            );
            n.shape = if allow_unknown_shape {
                shape(&s.shape_type).unwrap_or(ShapeKind::Rectangle)
            } else {
                shape(&s.shape_type)?
            };
            n.label_size = Some(Size {
                width: s.label_width as f64,
                height: s.label_height as f64,
            });
            n.font_size = Some(s.font_size as u32);
            n.label_position = s
                .label_position
                .as_deref()
                .map(position)
                .unwrap_or_default();
            n.icon_position = (!s.icon_position.is_empty()).then(|| position(&s.icon_position));
            n.has_icon = s.icon.is_some();
            n.is_3d = s.three_dee;
            n.is_multiple = s.multiple;
            let id = graph.add_node(n);
            ids.insert(s.id.clone(), id);
            labels.insert(id, s.label.clone());
            graph.set_node_label(id, &s.label);
            graph.set_node_has_language(id, !s.language.is_empty());
            graph.target_node_classes.insert(id, s.classes.to_vec());
            styles.insert(id, s.style());
            result.boxes.insert(
                id,
                Rect {
                    origin: Point {
                        x: s.pos.x as f64,
                        y: s.pos.y as f64,
                    },
                    size: Size {
                        width: s.width as f64,
                        height: s.height as f64,
                    },
                },
            );
        }
        graph.legend = self.legend.as_ref().map(|legend| d2_rs_core::Legend {
            label: legend.label.clone(),
            shapes: legend
                .shapes
                .iter()
                .flatten()
                .map(|s| d2_rs_core::LegendShape {
                    source_node: None,
                    font: Some(s.font_family.clone()),
                    label: s.label.clone(),
                    shape: ShapeKind::Rectangle,
                    fill: None,
                    stroke: None,
                })
                .collect(),
            connections: legend
                .connections
                .iter()
                .flatten()
                .map(|c| d2_rs_core::LegendConnection {
                    font: Some(c.font_family.clone()),
                    label: c.label.clone(),
                    ..Default::default()
                })
                .collect(),
            source_graph: None,
        });
        graph.root_style = Some(d2_rs_core::RootStyle {
            fill: Some(self.root.fill.clone()),
            stroke: Some(self.root.stroke.clone()),
            stroke_width: Some(self.root.stroke_width.to_string()),
            opacity: Some(self.root.opacity.0.to_string()),
            stroke_dash: Some(self.root.stroke_dash.0.to_string()),
            border_radius: Some(self.root.border_radius.to_string()),
            fill_pattern: self.root.fill_pattern.clone(),
            double_border: Some(self.root.double_border.to_string()),
            ..Default::default()
        });
        // Go indexes a map of Shape values; missing endpoint IDs resolve to
        // the zero Shape. Sequence lifeline end IDs intentionally have no
        // corresponding painted shape. Keep their zero stroke without adding
        // boxes, labels, hierarchy or layout work.
        for c in self.connections.iter().flatten() {
            for endpoint in [&c.src, &c.dst] {
                if !ids.contains_key(endpoint) {
                    let id = graph.add_node(Node::new(endpoint, Size::default()));
                    styles.insert(
                        id,
                        SourceNodeStyle {
                            stroke_width: Some("0".into()),
                            ..Default::default()
                        },
                    );
                    ids.insert(endpoint.clone(), id);
                }
            }
        }
        for c in self.connections.iter().flatten() {
            let source = *ids
                .get(&c.src)
                .ok_or_else(|| format!("target connection {}: missing source {}", c.id, c.src))?;
            let target = *ids.get(&c.dst).ok_or_else(|| {
                format!("target connection {}: missing destination {}", c.id, c.dst)
            })?;
            let id = graph.add_edge(d2_rs_core::Edge { source, target });
            graph.target_edge_classes.insert(id, c.classes.to_vec());
            graph.set_edge_arrows(
                id,
                d2_rs_core::EdgeArrows {
                    source: c.src_arrow != "none" && !c.src_arrow.is_empty(),
                    target: c.dst_arrow != "none" && !c.dst_arrow.is_empty(),
                },
            );
            graph.set_edge_arrowheads(
                id,
                d2_rs_core::EdgeArrowheads {
                    source: Some(c.src_arrow.clone()),
                    target: Some(c.dst_arrow.clone()),
                },
            );
            graph.set_edge_is_curve(id, c.is_curve);
            result.route_curves.insert(id, c.is_curve);
            result.routes.insert(
                id,
                c.route
                    .iter()
                    .flatten()
                    .flatten()
                    .map(|p| Point { x: p.x.0, y: p.y.0 })
                    .collect(),
            );
            let label = d2_rs_core::EdgeLabel {
                text: c.label.clone(),
                size: Size {
                    width: c.label_width as f64,
                    height: c.label_height as f64,
                },
                position: position(&c.label_position),
                percentage: c.label_percentage.0,
            };
            // Original target bounds/paint ignore dimensions when Label is
            // empty, even if a caller retained nonzero measured dimensions.
            if !c.label.is_empty() {
                graph.set_edge_label(id, Some(label.clone()));
                result.edge_labels.insert(id, label);
            }
            graph.set_edge_label_percentage_present(id, true);
            graph.set_edge_font_size(id, c.font_size as u32);
            graph.set_edge_label_bold(id, c.bold);
            graph.set_edge_style(
                id,
                EdgeStyle {
                    opacity: Some(c.opacity.0.to_string()),
                    stroke: Some(c.stroke.clone()),
                    stroke_width: Some(c.stroke_width.to_string()),
                    stroke_dash: Some(c.stroke_dash.0.to_string()),
                    border_radius: Some(c.border_radius.0.to_string()),
                    animated: Some(c.animated.to_string()),
                    font_color: Some(c.color.clone()),
                    font: Some(font(&c.font_family)),
                    italic: Some(c.italic.to_string()),
                    underline: Some(c.underline.to_string()),
                    language: Some(c.language.clone()),
                    fill: c.fill.clone(),
                    ..Default::default()
                },
            );
            graph.set_edge_link(id, Some(c.link.clone()));
            graph.set_edge_tooltip(id, Some(c.tooltip.clone()));
            graph.set_edge_icon(
                id,
                c.icon.as_ref().map(url),
                c.icon_position.as_deref().map(position),
                c.icon_border_radius.0,
            );
            let arrow_label = |text: &TargetText| d2_rs_core::ArrowheadLabel {
                text: text.label.clone(),
                size: Size {
                    width: text.label_width as f64,
                    height: text.label_height as f64,
                },
                color: (!text.color.is_empty()).then(|| text.color.clone()),
            };
            graph.set_edge_arrowhead_labels(
                id,
                d2_rs_core::EdgeArrowheadLabels {
                    source: c.src_label.as_ref().map(arrow_label),
                    target: c.dst_label.as_ref().map(arrow_label),
                },
            );
        }
        Ok((graph, result, labels, styles))
    }

    /// Original target-space bounding box, independent of canvas and legend.
    /// Unknown shape names use Go's default rectangle geometry in this query.
    pub fn bounding_box(&self) -> Result<[f64; 4], String> {
        if self.shapes.as_ref().is_none_or(|s| s.is_empty()) {
            return Ok([0.0; 4]);
        }
        let (graph, result, _labels, styles) = self.lower_geometry(true)?;
        let context = PaintContext { diagram: self };
        let (left, top, right, bottom) = bounds_with_markdown_family_and_legend(
            &graph,
            &result,
            &BTreeMap::new(),
            &styles,
            context.primary_family(),
            false,
        );
        let mut bounds = [left.floor(), top.floor(), right.ceil(), bottom.ceil()];
        // BoundingBox uses the full target box for every label, including
        // structured shapes. SVG paint's inner content box is not this query.
        for shape in self.shapes.iter().flatten().filter(|s| !s.label.is_empty()) {
            let position = shape
                .label_position
                .as_deref()
                .map(position)
                .unwrap_or_default();
            let rect = Rect {
                origin: Point {
                    x: shape.pos.x as f64,
                    y: shape.pos.y as f64,
                },
                size: Size {
                    width: shape.width as f64,
                    height: shape.height as f64,
                },
            };
            let mut label = if matches!(
                position,
                LabelPosition::Unset
                    | LabelPosition::UnlockedTop
                    | LabelPosition::UnlockedMiddle
                    | LabelPosition::UnlockedBottom
            ) {
                Rect {
                    origin: rect.origin,
                    size: Size {
                        width: shape.label_width as f64,
                        height: shape.label_height as f64,
                    },
                }
            } else {
                node_label_box(
                    rect,
                    ShapeKind::Rectangle,
                    position,
                    Size {
                        width: shape.label_width as f64,
                        height: shape.label_height as f64,
                    },
                    None,
                )
            };
            label = target_modifier_label_bounds(
                label,
                if shape.shape_type == "hexagon" {
                    ShapeKind::Hexagon
                } else {
                    ShapeKind::Rectangle
                },
                position,
                shape.three_dee,
            );
            let (x, y) = (label.origin.x.trunc(), label.origin.y.trunc());
            bounds[0] = bounds[0].min(x);
            bounds[1] = bounds[1].min(y);
            bounds[2] = bounds[2].max(x + shape.label_width as f64);
            bounds[3] = bounds[3].max(y + shape.label_height as f64);
        }
        Ok(bounds)
    }

    /// Original NestedBoundingBox; descendant boards contribute bounds only.
    pub fn nested_bounding_box(&self) -> Result<[f64; 4], String> {
        let mut bounds = self.bounding_box()?;
        for child in self
            .layers
            .iter()
            .flatten()
            .chain(self.scenarios.iter().flatten())
            .chain(self.steps.iter().flatten())
            .flatten()
        {
            let other = child.nested_bounding_box()?;
            bounds[0] = bounds[0].min(other[0]);
            bounds[1] = bounds[1].min(other[1]);
            bounds[2] = bounds[2].max(other[2]);
            bounds[3] = bounds[3].max(other[3]);
        }
        Ok(bounds)
    }

    /// Render with d2wasm.Render's render/animation error wrappers. The caller
    /// validates the original target and selects the board before this stage.
    pub fn render_wasm_svg(
        &self,
        options: &RenderOptions,
        sketch: bool,
        no_xml_tag: bool,
        interval_ms: u64,
        descendants: bool,
    ) -> Result<Vec<u8>, String> {
        let svg =
            d2svg::render_wasm_api(self, options, sketch, no_xml_tag, interval_ms, descendants)?;
        Ok(bundle_svg_images(&svg, &options.asset_data))
    }

    /// Render a supplied target directly. Native source compilation and layout
    /// are deliberately absent from this API.
    pub fn render_d2svg(
        &self,
        options: &RenderOptions,
        sketch: bool,
        no_xml_tag: bool,
        interval_ms: u64,
        descendants: bool,
    ) -> Result<Vec<u8>, String> {
        let svg = d2svg::render_wasm(self, options, sketch, no_xml_tag, interval_ms, descendants)?;
        Ok(bundle_svg_images(&svg, &options.asset_data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "oracle-fixtures")]
    fn fixtures() -> Value {
        serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/public-target-v090.json"
        ))
        .unwrap()
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_go_raw_json_decoder_contracts() {
        use sha2::{Digest, Sha256};
        let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/public-target-json-v090.json"
        ))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 2028);
        fn projection(value: Value) -> Value {
            if value.is_null() {
                return value;
            }
            let mut out = value.as_object().unwrap().clone();
            out.retain(|k, _| {
                matches!(
                    k.as_str(),
                    "name"
                        | "isFolderOnly"
                        | "description"
                        | "fontFamily"
                        | "monoFontFamily"
                        | "legend"
                        | "layers"
                        | "scenarios"
                        | "steps"
                )
            });
            for key in ["layers", "scenarios", "steps"] {
                if let Some(Value::Array(values)) = out.get_mut(key) {
                    for v in values {
                        *v = projection(v.take());
                    }
                }
            }
            Value::Object(out)
        }
        let mut decoded = 0;
        let mut rejected = 0;
        let mut nil = 0;
        let mut source_fault = 0;
        for row in cases {
            let name = row["name"].as_str().unwrap();
            let raw = if let Some(encoded) = row["raw_base64"].as_str() {
                base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .unwrap()
            } else {
                row["raw"].as_str().unwrap().as_bytes().to_vec()
            };
            let result = TargetDiagram::decode_json(&raw);
            match row["expected_outcome"].as_str().unwrap() {
                "decode-error" => {
                    assert!(result.is_err(), "{name}");
                    rejected += 1;
                }
                "nil" => {
                    assert!(result.unwrap().is_none(), "{name}");
                    nil += 1;
                }
                kind => {
                    let diagram = result.unwrap_or_else(|e| panic!("{name}: {e}")).unwrap();
                    let value = serde_json::to_value(&diagram).unwrap();
                    assert_eq!(projection(value), row["expected_projection"], "{name}");
                    if kind == "source-panic" {
                        assert!(diagram.hash_input().is_err(), "{name}");
                        source_fault += 1;
                        continue;
                    }
                    assert_eq!(kind, "decoded");
                    let bytes = diagram.hash_input().unwrap();
                    assert_eq!(
                        format!("{:x}", Sha256::digest(&bytes)),
                        row["expected_hash_input_sha256"].as_str().unwrap(),
                        "{name}"
                    );
                    assert_eq!(
                        target_hash_id_from_input(&bytes, None),
                        row["expected_hash"].as_str().unwrap(),
                        "{name}"
                    );
                    decoded += 1;
                }
            }
        }
        assert_eq!((decoded, rejected, nil, source_fault), (1352, 674, 1, 1));
    }
    #[test]
    fn target_nil_slices_and_nullable_config_are_not_compiler_defaults() {
        let nil: TargetDiagram = serde_json::from_str("{}").unwrap();
        let empty: TargetDiagram =
            serde_json::from_str(r#"{"shapes":[],"connections":[],"config":{}}"#).unwrap();
        let nil = String::from_utf8(nil.hash_input().unwrap()).unwrap();
        let empty = String::from_utf8(empty.hash_input().unwrap()).unwrap();
        assert!(nil.starts_with("nullnull"));
        assert!(empty.starts_with("[][]"));
        assert!(empty.ends_with(r#"{"sketch":null,"themeID":null,"darkThemeID":null,"pad":null,"center":null,"layoutEngine":null}"#));
        assert!(!nil.contains("themeID"));
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_get_board_accepts_wrapper_or_bare_name_without_source() {
        let data = fixtures();
        let row = data["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == "boards-clone")
            .unwrap();
        let tree: TargetDiagram = serde_json::from_value(row["diagram"].clone()).unwrap();
        let board = tree.get_board(&["layers".into(), "alt".into()]).unwrap();
        assert_eq!(board.name, "alt");
        assert_eq!(
            board.hash_input().unwrap(),
            tree.get_board(&["alt".into()])
                .unwrap()
                .hash_input()
                .unwrap()
        );
        assert!(tree.get_board(&["layers".into()]).is_none());
        assert!(tree.get_board(&["missing".into()]).is_none());
        assert_eq!(tree.without_children().children().count(), 0);
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_go_raw_legend_hashes_canvases_and_text_origins() {
        let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/target-legend-source-v090.json"
        ))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 33);
        for row in cases {
            let diagram: TargetDiagram = serde_json::from_value(row["diagram"].clone()).unwrap();
            let options = RenderOptions {
                pad: row["options"]["pad"].as_f64().unwrap(),
                theme_id: row["options"]["themeID"].as_i64(),
                ..Default::default()
            };
            let bytes = diagram
                .without_children()
                .render_d2svg(
                    &options,
                    row["options"]["sketch"].as_bool().unwrap_or(false),
                    false,
                    0,
                    false,
                )
                .unwrap();
            let svg = std::str::from_utf8(&bytes).unwrap();
            let doc = roxmltree::Document::parse(svg).unwrap();
            assert_eq!(
                doc.root_element().attribute("viewBox"),
                row["viewbox"].as_str(),
                "{}",
                row["name"]
            );
            let hash = row["main_hash"].as_str().unwrap();
            assert_eq!(
                doc.descendants()
                    .filter(|n| n.has_tag_name("svg")
                        && n.attribute("class")
                            .is_some_and(|c| c.split_ascii_whitespace().any(|c| c == hash)))
                    .count(),
                1,
                "{}",
                row["name"]
            );
            for expected in row["legend_text"].as_array().unwrap() {
                let node = doc
                    .descendants()
                    .rfind(|n| n.has_tag_name("text") && n.text() == expected["text"].as_str())
                    .unwrap();
                for (actual, key) in [("x", "x"), ("y", "y"), ("class", "cls")] {
                    assert_eq!(
                        node.attribute(actual),
                        expected[key].as_str(),
                        "{} {}",
                        row["name"],
                        expected["text"]
                    );
                }
            }
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_go_serialized_and_mutated_targets_keep_hashes_and_canvases() {
        let fixtures = fixtures();
        let cases = fixtures["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 32);
        for row in cases {
            let diagram: TargetDiagram = serde_json::from_value(row["diagram"].clone()).unwrap();
            let board = diagram.without_children();
            assert_eq!(
                target_hash_id_from_input(&board.hash_input().unwrap(), None),
                row["expected_hash"].as_str().unwrap(),
                "{}",
                row["name"]
            );
            let output = board
                .render_d2svg(
                    &RenderOptions {
                        pad: 20.,
                        ..Default::default()
                    },
                    false,
                    false,
                    0,
                    false,
                )
                .unwrap();
            let svg = std::str::from_utf8(&output).unwrap();
            let document = roxmltree::Document::parse(svg).unwrap();
            assert_eq!(
                document.root_element().attribute("viewBox"),
                row["expected_view_box"].as_str(),
                "{}",
                row["name"]
            );
            assert!(svg.contains(row["expected_hash"].as_str().unwrap()));
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn supplied_fractional_routes_do_not_repeat_export_quantization() {
        let cases: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/public-target-precision-v090.json"
        ))
        .unwrap();
        for row in cases.as_array().unwrap() {
            let diagram: TargetDiagram = serde_json::from_value(row["diagram"].clone()).unwrap();
            let output = diagram
                .render_d2svg(
                    &RenderOptions {
                        pad: 20.,
                        ..Default::default()
                    },
                    false,
                    false,
                    0,
                    false,
                )
                .unwrap();
            let svg = std::str::from_utf8(&output).unwrap();
            assert!(
                svg.contains(row["expected_coordinate"].as_str().unwrap()),
                "{}",
                row["name"]
            );
            let document = roxmltree::Document::parse(svg).unwrap();
            assert_eq!(
                document.root_element().attribute("viewBox"),
                row["expected_view_box"].as_str(),
                "{}",
                row["name"]
            );
            assert!(svg.contains(row["expected_hash"].as_str().unwrap()));
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_go_render_only_themes_classes_and_label_fills() {
        let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/public-target-paint-v090.json"
        ))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 88);
        for row in cases {
            let options = &row["options"];
            let colors = |key: &str| {
                options[key]
                    .as_object()
                    .map(|v| {
                        v.iter()
                            .map(|(k, v)| (k.to_ascii_uppercase(), v.as_str().unwrap().to_owned()))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let options = RenderOptions {
                pad: 20.,
                theme_id: options["themeID"].as_i64(),
                dark_theme_id: options["darkThemeID"].as_i64(),
                theme_overrides: colors("themeOverrides"),
                dark_theme_overrides: colors("darkThemeOverrides"),
                ..Default::default()
            };
            let diagram: TargetDiagram = serde_json::from_value(row["diagram"].clone()).unwrap();
            let bytes = diagram
                .without_children()
                .render_d2svg(&options, false, false, 0, false)
                .unwrap();
            let svg = std::str::from_utf8(&bytes).unwrap();
            let document = roxmltree::Document::parse(svg).unwrap();
            assert!(
                svg.contains(row["expected_hash"].as_str().unwrap()),
                "{}",
                row["name"]
            );
            assert_eq!(
                document.root_element().attribute("viewBox"),
                row["expected_view_box"].as_str(),
                "{}",
                row["name"]
            );
            let texts = document
                .descendants()
                .filter(|n| n.has_tag_name("text"))
                .map(|n| {
                    n.descendants()
                        .filter(|c| c.is_text())
                        .filter_map(|c| c.text())
                        .collect::<String>()
                })
                .collect::<Vec<_>>();
            assert_eq!(
                serde_json::to_value(texts).unwrap(),
                row["expected_texts"],
                "{}",
                row["name"]
            );
            for class in row["expected_classes"].as_array().unwrap() {
                assert!(
                    document
                        .descendants()
                        .any(|n| n.has_tag_name("g") && n.attribute("class") == class.as_str()),
                    "{}",
                    row["name"]
                );
            }
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_go_target_font_families_are_independent_of_sketch_geometry() {
        let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/public-target-fonts-v090.json"
        ))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 48);
        for row in cases {
            let diagram: TargetDiagram = serde_json::from_value(row["diagram"].clone()).unwrap();
            let bytes = diagram
                .without_children()
                .render_d2svg(
                    &RenderOptions {
                        pad: 20.0,
                        ..Default::default()
                    },
                    row["sketch"].as_bool().unwrap(),
                    false,
                    0,
                    false,
                )
                .unwrap();
            let svg = std::str::from_utf8(&bytes).unwrap();
            let document = roxmltree::Document::parse(svg).unwrap();
            assert!(
                svg.contains(row["expected_hash"].as_str().unwrap()),
                "{}",
                row["name"]
            );
            assert_eq!(
                document.root_element().attribute("viewBox"),
                row["expected_view_box"].as_str(),
                "{}",
                row["name"]
            );
            let texts = document
                .descendants()
                .filter(|n| n.has_tag_name("text"))
                .collect::<Vec<_>>();
            let content = texts
                .iter()
                .map(|n| {
                    n.descendants()
                        .filter(|c| c.is_text())
                        .filter_map(|c| c.text())
                        .collect::<String>()
                })
                .collect::<Vec<_>>();
            assert_eq!(
                serde_json::to_value(content).unwrap(),
                row["expected_texts"],
                "{}",
                row["name"]
            );
            if !row["structured_xy"].is_null() {
                let xy = texts
                    .iter()
                    .map(|n| [n.attribute("x"), n.attribute("y")])
                    .collect::<Vec<_>>();
                assert_eq!(
                    serde_json::to_value(xy).unwrap(),
                    row["structured_xy"],
                    "{}",
                    row["name"]
                );
            }
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_go_primary_and_mono_font_families_are_independent() {
        let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/public-target-mono-fonts-v090.json"
        ))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 180);
        for row in cases {
            let diagram: TargetDiagram = serde_json::from_value(row["diagram"].clone()).unwrap();
            let bytes = diagram
                .without_children()
                .render_d2svg(
                    &RenderOptions {
                        pad: 20.0,
                        ..Default::default()
                    },
                    row["sketch"].as_bool().unwrap(),
                    false,
                    0,
                    false,
                )
                .unwrap();
            let svg = std::str::from_utf8(&bytes).unwrap();
            let document = roxmltree::Document::parse(svg).unwrap();
            assert!(
                svg.contains(row["expected_hash"].as_str().unwrap()),
                "{}",
                row["name"]
            );
            assert_eq!(
                document.root_element().attribute("viewBox"),
                row["expected_view_box"].as_str(),
                "{}",
                row["name"]
            );
            let texts = document
                .descendants()
                .filter(|n| n.has_tag_name("text"))
                .collect::<Vec<_>>();
            let content = texts
                .iter()
                .map(|n| {
                    n.descendants()
                        .filter(|c| c.is_text())
                        .filter_map(|c| c.text())
                        .collect::<String>()
                })
                .collect::<Vec<_>>();
            assert_eq!(
                serde_json::to_value(content).unwrap(),
                row["expected_texts"],
                "{}",
                row["name"]
            );
            if !row["structured_xy"].is_null() {
                let xy = texts
                    .iter()
                    .map(|n| [n.attribute("x"), n.attribute("y")])
                    .collect::<Vec<_>>();
                assert_eq!(
                    serde_json::to_value(xy).unwrap(),
                    row["structured_xy"],
                    "{}",
                    row["name"]
                );
            }
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn sequence_lifelines_paint_without_inventing_visible_endpoint_shapes() {
        let data = fixtures();
        let row = data["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == "sequence-clone")
            .unwrap();
        let tree: TargetDiagram = serde_json::from_value(row["diagram"].clone()).unwrap();
        let svg = tree
            .render_d2svg(&RenderOptions::default(), false, false, 0, false)
            .unwrap();
        let document = roxmltree::Document::parse(std::str::from_utf8(&svg).unwrap()).unwrap();
        let shapes = document
            .descendants()
            .filter(|n| n.has_tag_name("g") && n.attribute("class") == Some("shape"))
            .count();
        assert_eq!(shapes, tree.shapes.as_ref().unwrap().len());
        assert_eq!(shapes, 2);
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod admission_api_tests;
