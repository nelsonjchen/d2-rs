// SPDX-License-Identifier: MPL-2.0
//! d2ir.validateConfigs and the pinned d2themescatalog.Find membership boundary.
use super::{Compiler, edge_creation::error};
use d2_rs_core::{ast::ByteString, ir::*, theme};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

const CONFIG_THEME_CODES: &[&str] = &[
    "N1", "N2", "N3", "N4", "N5", "N6", "N7", "B1", "B2", "B3", "B4", "B5", "B6", "AA2", "AA4",
    "AA5", "AB4", "AB5",
];

// Pinned D2 lib/color/color.go::NamedColors. The source validator also
// accepts only #RGB and #RRGGBB hexadecimal forms.
pub(super) const D2_NAMED_COLORS: &str = concat!(
    "currentcolor transparent aliceblue antiquewhite aqua aquamarine azure beige bisque black ",
    "blanchedalmond blue blueviolet brown burlywood cadetblue chartreuse chocolate coral ",
    "cornflowerblue cornsilk crimson cyan darkblue darkcyan darkgoldenrod darkgray darkgrey ",
    "darkgreen darkkhaki darkmagenta darkolivegreen darkorange darkorchid darkred darksalmon ",
    "darkseagreen darkslateblue darkslategray darkslategrey darkturquoise darkviolet deeppink ",
    "deepskyblue dimgray dimgrey dodgerblue firebrick floralwhite forestgreen fuchsia gainsboro ",
    "ghostwhite gold goldenrod gray grey green greenyellow honeydew hotpink indianred indigo ivory ",
    "khaki lavender lavenderblush lawngreen lemonchiffon lightblue lightcoral lightcyan ",
    "lightgoldenrodyellow lightgray lightgrey lightgreen lightpink lightsalmon lightseagreen ",
    "lightskyblue lightslategray lightslategrey lightsteelblue lightyellow lime limegreen linen ",
    "magenta maroon mediumaquamarine mediumblue mediumorchid mediumpurple mediumseagreen ",
    "mediumslateblue mediumspringgreen mediumturquoise mediumvioletred midnightblue mintcream ",
    "mistyrose moccasin navajowhite navy oldlace olive olivedrab orange orangered orchid ",
    "palegoldenrod palegreen paleturquoise palevioletred papayawhip peachpuff peru pink plum ",
    "powderblue purple rebeccapurple red rosybrown royalblue saddlebrown salmon sandybrown ",
    "seagreen seashell sienna silver skyblue slateblue slategray slategrey snow springgreen ",
    "steelblue tan teal thistle tomato turquoise violet wheat white whitesmoke yellow yellowgreen",
);

fn valid_theme_color(value: &str) -> bool {
    let is_hex = value.strip_prefix('#').is_some_and(|digits| {
        matches!(digits.len(), 3 | 6) && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    });
    is_hex
        || D2_NAMED_COLORS
            .split_whitespace()
            .any(|name| name.eq_ignore_ascii_case(value))
}

/// Configuration compiled from the root `vars.d2-config` map.
///
/// This is deliberately separate from layout-engine options: command-line and
/// environment values can override it, matching D2's `applyConfigs` boundary.
/// Theme rendering itself remains a target-layer concern, but preserving these
/// values lets native and WASM callers make the same precedence decision.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct DiagramConfig {
    /// Milliseconds per board of an animation, from `animate-interval`.
    pub animate_interval: Option<i64>,
    /// Output scale from `scale`; the automatic `-1` sentinel is absent.
    pub scale: Option<f64>,
    /// Whether SVG exports get an appendix, from `force-appendix`.
    pub force_appendix: Option<bool>,
    /// Whether sketch rendering was requested in source.
    pub sketch: Option<bool>,
    /// Whether isometric rendering was requested in source (master config API).
    pub isometric: Option<bool>,
    /// Light theme ID selected in source.
    pub theme_id: Option<i64>,
    /// Dark theme ID selected in source.
    pub dark_theme_id: Option<i64>,
    /// Outer rendering padding selected in source.
    pub pad: Option<i64>,
    /// Whether the rendered diagram is centered.
    pub center: Option<bool>,
    /// Layout engine selected in source.
    pub layout_engine: Option<String>,
    /// Theme color overrides keyed by their canonical uppercase codes.
    pub theme_overrides: std::collections::BTreeMap<String, String>,
    /// Dark-theme color overrides keyed by their canonical uppercase codes.
    pub dark_theme_overrides: std::collections::BTreeMap<String, String>,
    /// User-defined graph data from the source config, including TALA seeds.
    pub data: std::collections::BTreeMap<String, JsonValue>,
}

impl DiagramConfig {
    /// applyGraphData clones source nil []interface{} values into allocated
    /// empty slices. Config keeps null JSON; public Graph.Data keeps [].
    pub fn graph_data(&self) -> std::collections::BTreeMap<String, JsonValue> {
        self.data
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    if value.is_null() {
                        JsonValue::Array(Vec::new())
                    } else {
                        value.clone()
                    },
                )
            })
            .collect()
    }

    /// Parses TALA's seed list from source data. Store the result (including
    /// errors) on `Graph::tala_seed_data`; only an actual TALA core invocation
    /// validates it. Source data overrides caller seed options, unlike
    /// ordinary render configuration.
    pub fn tala_seeds(&self) -> Result<Option<Vec<i64>>, String> {
        let Some(raw) = self.data.get("tala-seeds") else {
            return Ok(None);
        };
        // compileConfig's null JSON value represents a typed nil []interface{}
        // inside Go. Source data is still a slice for seedsFromData, unlike
        // an actual nil interface supplied through the raw graph API.
        if raw.is_null() {
            return Ok(Some(Vec::new()));
        }
        let JsonValue::Array(values) = raw else {
            return Err("tala-seeds must be a list of signed 64-bit integers".into());
        };
        // Original seedsFromData checks the entry bound before parsing any
        // members, so an oversized list wins over a malformed first member.
        if values.len() > d2_rs_core::LayoutOptions::MAX_SEED_ENTRIES {
            return Err("tala accepts at most 64 seed entries".into());
        }
        values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let text = value.as_str().map(ToOwned::to_owned).unwrap_or_else(|| value.to_string());
                text.parse::<i64>().map_err(|_| {
                    format!("invalid tala seed at index {index} ({text}): must be a signed 64-bit integer")
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }
}

fn scalar_text(compiler: &Compiler, node: NodeId) -> Option<String> {
    let Node::Scalar(scalar) = compiler.arena.node(node) else {
        return None;
    };
    Some(
        ByteString::from(scalar.value.scalar_bytes())
            .to_text()
            .into_owned(),
    )
}

fn config_data_value(compiler: &Compiler, field: NodeId) -> Option<JsonValue> {
    if let Some(primary) = compiler.arena.primary(field) {
        return scalar_text(compiler, primary).map(JsonValue::String);
    }
    let Node::Field(field_node) = compiler.arena.node(field) else {
        return None;
    };
    let composite = field_node.composite?;
    let Node::Array(array) = compiler.arena.node(composite) else {
        // Go's `var arr []interface{}` remains nil for a map composite.
        return Some(JsonValue::Null);
    };
    let values: Vec<_> = array
        .values
        .iter()
        .flatten()
        .filter(|&value| matches!(compiler.arena.node(*value), Node::Scalar(_)))
        .map(|value| {
            JsonValue::String(
                ByteString::from(crate::ir::format(&compiler.arena, *value))
                    .to_text()
                    .into_owned(),
            )
        })
        .collect();
    Some(if values.is_empty() {
        JsonValue::Null
    } else {
        JsonValue::Array(values)
    })
}

fn bool_value(value: &str) -> Option<bool> {
    d2_rs_core::parse_d2_bool(value)
}

fn config_map(compiler: &Compiler, root: NodeId) -> Option<NodeId> {
    let vars = compiler
        .arena
        .get_field(root, &[d2_rs_core::ast::raw_string_box("vars", true)])?;
    let vars = compiler.arena.map(vars)?;
    let config = compiler
        .arena
        .get_field(vars, &[d2_rs_core::ast::raw_string_box("d2-config", true)])?;
    compiler.arena.map(config)
}

fn config_field(compiler: &Compiler, config: NodeId, name: &str) -> Option<NodeId> {
    compiler
        .arena
        .get_field(config, &[d2_rs_core::ast::raw_string_box(name, true)])
}

fn override_map(
    compiler: &Compiler,
    config: NodeId,
    name: &str,
) -> std::collections::BTreeMap<String, String> {
    let Some(field) = config_field(compiler, config, name) else {
        return std::collections::BTreeMap::new();
    };
    let Some(map) = compiler.arena.map(field) else {
        return std::collections::BTreeMap::new();
    };
    let Node::Map(map_node) = compiler.arena.node(map) else {
        return std::collections::BTreeMap::new();
    };
    map_node
        .fields
        .clone()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|field| {
            let Node::Field(field_node) = compiler.arena.node(field) else {
                return None;
            };
            let value = compiler
                .arena
                .primary(field)
                .and_then(|node| scalar_text(compiler, node))?;
            Some((field_node.name.scalar_string().to_ascii_uppercase(), value))
        })
        .collect()
}

/// Extracts source configuration after IR substitutions and validation.
#[must_use]
pub fn extract_config(compiler: &Compiler, root: NodeId) -> Option<DiagramConfig> {
    let config = config_map(compiler, root)?;
    let mut output = DiagramConfig {
        theme_overrides: override_map(compiler, config, "theme-overrides"),
        dark_theme_overrides: override_map(compiler, config, "dark-theme-overrides"),
        ..Default::default()
    };
    let text = |name: &str| {
        config_field(compiler, config, name)
            .and_then(|field| compiler.arena.primary(field))
            .and_then(|node| scalar_text(compiler, node))
    };
    output.animate_interval = text("animate-interval").map(|value| {
        // compileConfig discards the parse result's validity: validation
        // already rejected anything else.
        parse_animate_interval(&value).unwrap_or(0)
    });
    output.scale = text("scale")
        .map(|value| d2_rs_core::parse_d2_float(&value).unwrap_or(0.0))
        .filter(|scale| *scale != -1.0);
    output.force_appendix = text("force-appendix").map(|value| bool_value(&value).unwrap_or(false));
    output.sketch = text("sketch").and_then(|value| bool_value(&value));
    output.isometric = text("isometric").and_then(|value| bool_value(&value));
    output.center = text("center").and_then(|value| bool_value(&value));
    output.theme_id = text("theme-id").and_then(|value| value.parse().ok());
    output.dark_theme_id = text("dark-theme-id").and_then(|value| value.parse().ok());
    output.pad = text("pad").and_then(|value| value.parse().ok());
    output.layout_engine = text("layout-engine");
    if let Some(field) = config_field(compiler, config, "data")
        && let Some(map) = compiler.arena.map(field)
        && let Node::Map(map_node) = compiler.arena.node(map)
    {
        for entry in map_node.fields.clone().unwrap_or_default() {
            let Node::Field(entry_node) = compiler.arena.node(entry) else {
                continue;
            };
            if let Some(value) = config_data_value(compiler, entry) {
                output
                    .data
                    .insert(entry_node.name.scalar_string().to_owned(), value);
            }
        }
    }
    Some(output)
}

/// Extracts source configuration and validates the late `compileConfig`
/// checks that D2 performs after IR and graph compilation.
pub fn extract_config_checked(
    compiler: &Compiler,
    root: NodeId,
) -> Result<Option<DiagramConfig>, Vec<d2_rs_core::ast::Error>> {
    for name in ["theme-overrides", "dark-theme-overrides"] {
        let Some(config) = config_map(compiler, root) else {
            return Ok(None);
        };
        let Some(field) = config_field(compiler, config, name) else {
            continue;
        };
        let Some(map) = compiler.arena.map(field) else {
            continue;
        };
        let Node::Map(map_node) = compiler.arena.node(map) else {
            continue;
        };
        let mut errors = Vec::new();
        for entry in map_node.fields.clone().unwrap_or_default() {
            let Node::Field(entry_node) = compiler.arena.node(entry) else {
                continue;
            };
            let code = entry_node.name.scalar_string().to_owned();
            let message = if !CONFIG_THEME_CODES.contains(&code.to_ascii_uppercase().as_str()) {
                Some(format!("\"{code}\" is not a valid theme code"))
            } else {
                let color = compiler
                    .arena
                    .primary(entry)
                    .and_then(|primary| scalar_text(compiler, primary))
                    .unwrap_or_default();
                (!valid_theme_color(&color)).then(|| {
                    format!(
                        "expected \"{code}\" to be a valid named color (\"orange\") or a hex code (\"#f0ff3a\")"
                    )
                })
            };
            if let Some(message) = message {
                let ast =
                    d2_rs_core::ast::Child::Key(compiler.arena.last_primary_key(entry).unwrap());
                errors.push(error(&ast, message));
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
    }
    Ok(extract_config(compiler, root))
}

impl Compiler {
    pub(super) fn validate_configs(&mut self, configs: Option<NodeId>) {
        let Some(configs) = configs else { return };
        let Some(map) = self.arena.map(configs) else {
            return;
        };
        let parent = self.arena.parent_map(configs).unwrap();
        let root = self.arena.parent_map(parent).unwrap();
        let Node::Field(config) = self.arena.node(configs) else {
            unreachable!()
        };
        let name = ByteString::from(config.name.scalar_bytes())
            .to_text()
            .into_owned();
        if self.arena.board_kind(root).is_empty() {
            self.config_error(configs, format!("\"{name}\" can only appear at root vars"));
            return;
        }
        let Node::Map(m) = self.arena.node(map) else {
            unreachable!()
        };
        let fields = m.fields.clone().unwrap_or_default();
        for field in fields {
            let Node::Field(f) = self.arena.node(field) else {
                unreachable!()
            };
            let name = ByteString::from(f.name.scalar_bytes())
                .to_text()
                .into_owned();
            let mut value = String::new();
            let map_property = matches!(
                name.as_str(),
                "theme-overrides" | "dark-theme-overrides" | "data"
            );
            if let Some(primary) = f.primary {
                let Node::Scalar(s) = self.arena.node(primary) else {
                    unreachable!()
                };
                value = ByteString::from(s.value.scalar_bytes())
                    .to_text()
                    .into_owned();
            } else if !map_property {
                self.config_error(field, format!("\"{name}\" needs a value"));
                continue;
            }
            let composite = self.arena.map(field).is_some();
            match name.as_str() {
                "animate-interval" => {
                    if parse_animate_interval(&value).is_none() || composite {
                        self.config_error(
                            field,
                            format!(
                                "expected a non-negative whole number of milliseconds or a duration in whole milliseconds for \"animate-interval\", got {}",
                                d2_rs_core::quote_go_bytes(value.as_bytes())
                            ),
                        );
                    }
                }
                "scale" => {
                    let valid = d2_rs_core::parse_d2_float(&value).is_some_and(|scale| {
                        scale.is_finite() && (scale > 0.0 || scale == -1.0)
                    });
                    if !valid || composite {
                        self.config_error(
                            field,
                            format!(
                                "expected a finite positive scale or -1 for automatic sizing, got {}",
                                d2_rs_core::quote_go_bytes(value.as_bytes())
                            ),
                        );
                    }
                }
                "sketch" | "isometric" | "center" | "force-appendix" => {
                    if (name == "force-appendix" && composite) || !matches!(
                        value.as_str(),
                        "1" | "t"
                            | "T"
                            | "TRUE"
                            | "true"
                            | "True"
                            | "0"
                            | "f"
                            | "F"
                            | "FALSE"
                            | "false"
                            | "False"
                    ) {
                        self.config_error(
                            field,
                            format!("expected a boolean for \"{name}\", got \"{value}\""),
                        );
                    }
                }
                "theme-overrides" | "dark-theme-overrides" | "data" => {
                    if self.arena.map(field).is_none() {
                        self.config_error(field, format!("\"{name}\" needs a map"));
                    }
                }
                "theme-id" | "dark-theme-id" | "pad" => {
                    let Ok(number) = value.parse::<i64>() else {
                        self.config_error(
                            field,
                            format!("expected an integer for \"{name}\", got \"{value}\""),
                        );
                        continue;
                    };
                    // Membership only: the complete theme data/rendering API is separate.
                    if name != "pad" && theme::find(number).is_none() {
                        self.config_error(field, format!("{number} is not a valid theme ID"));
                    }
                }
                "layout-engine" => {}
                _ => self.config_error(field, format!("\"{name}\" is not a valid config")),
            }
        }
    }
    fn config_error(&mut self, node: NodeId, message: String) {
        let ast = self.arena.last_ref(node).unwrap().ast().unwrap();
        self.errors.push(error(&ast, message));
    }
}

/// d2ir.ParseAnimateInterval: the source interval in milliseconds. A bare
/// integer uses milliseconds, matching the CLI flag.
#[must_use]
pub fn parse_animate_interval(value: &str) -> Option<i64> {
    let mut text = value.to_owned();
    if value.parse::<i64>().is_ok() {
        text.push_str("ms");
    }
    let duration = go_parse_duration(&text)?;
    if duration < 0 || duration % 1_000_000 != 0 {
        return None;
    }
    // ParseDuration truncates below a nanosecond. Only an actual zero should
    // turn an animation off.
    if duration == 0 && text.bytes().any(|byte| matches!(byte, b'1'..=b'9')) {
        return None;
    }
    Some(duration / 1_000_000)
}

/// Go time.ParseDuration, in nanoseconds.
fn go_parse_duration(original: &str) -> Option<i64> {
    const LIMIT: u64 = 1 << 63;
    let mut s = original.as_bytes();
    let mut negative = false;
    if let Some(&sign @ (b'-' | b'+')) = s.first() {
        negative = sign == b'-';
        s = &s[1..];
    }
    if s == b"0" {
        return Some(0);
    }
    if s.is_empty() {
        return None;
    }
    let mut total: u64 = 0;
    while !s.is_empty() {
        if !(s[0] == b'.' || s[0].is_ascii_digit()) {
            return None;
        }
        // leadingInt
        let mut whole: u64 = 0;
        let mut index = 0;
        while index < s.len() && s[index].is_ascii_digit() {
            if whole > LIMIT / 10 {
                return None;
            }
            whole = whole * 10 + u64::from(s[index] - b'0');
            if whole > LIMIT {
                return None;
            }
            index += 1;
        }
        let consumed_whole = index > 0;
        s = &s[index..];
        // leadingFraction
        let mut fraction: u64 = 0;
        let mut scale = 1.0f64;
        let mut consumed_fraction = false;
        if s.first() == Some(&b'.') {
            s = &s[1..];
            let mut index = 0;
            let mut overflow = false;
            while index < s.len() && s[index].is_ascii_digit() {
                let digit = u64::from(s[index] - b'0');
                index += 1;
                if overflow {
                    continue;
                }
                if fraction > (LIMIT - 1) / 10 {
                    overflow = true;
                    continue;
                }
                let next = fraction * 10 + digit;
                if next > LIMIT {
                    overflow = true;
                    continue;
                }
                fraction = next;
                scale *= 10.0;
            }
            consumed_fraction = index > 0;
            s = &s[index..];
        }
        if !consumed_whole && !consumed_fraction {
            return None;
        }
        let unit_end = s
            .iter()
            .position(|byte| *byte == b'.' || byte.is_ascii_digit())
            .unwrap_or(s.len());
        if unit_end == 0 {
            return None;
        }
        let unit: u64 = match &s[..unit_end] {
            b"ns" => 1,
            b"us" | b"\xc2\xb5s" | b"\xce\xbcs" => 1_000,
            b"ms" => 1_000_000,
            b"s" => 1_000_000_000,
            b"m" => 60_000_000_000,
            b"h" => 3_600_000_000_000,
            _ => return None,
        };
        s = &s[unit_end..];
        if whole > LIMIT / unit {
            return None;
        }
        whole *= unit;
        if fraction > 0 {
            whole += (fraction as f64 * (unit as f64 / scale)) as u64;
            if whole > LIMIT {
                return None;
            }
        }
        total += whole;
        if total > LIMIT {
            return None;
        }
    }
    if negative {
        return Some((total as i64).wrapping_neg());
    }
    if total > LIMIT - 1 {
        return None;
    }
    Some(total as i64)
}

#[cfg(test)]
mod animate_interval_tests {
    use super::parse_animate_interval;

    #[test]
    fn parses_like_d2ir() {
        for (value, expected) in [
            ("0", Some(0)),
            ("250", Some(250)),
            ("+5", Some(5)),
            ("1.5s", Some(1500)),
            ("2m", Some(120_000)),
            ("1h30m", Some(5_400_000)),
            ("0s", Some(0)),
            ("0.0001ns", None),
            ("1us", None),
            ("1000us", Some(1)),
            ("-1", None),
            ("-0", Some(0)),
            ("1.5", None),
            ("ms", None),
            ("", None),
            ("10x", None),
            ("1e3", None),
        ] {
            assert_eq!(parse_animate_interval(value), expected, "{value:?}");
        }
    }
}
