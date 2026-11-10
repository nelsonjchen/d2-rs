// SPDX-License-Identifier: MPL-2.0
// D2 v0.9.0 d2cli/main.go, help.go and util-go/xmain/opts.go.
// The ordered flag declarations and help template come from the pinned source.

/// The Go D2 version d2-rs matches. Output metadata carries it where Go writes
/// its own version (SVG `data-d2-version`, PNG EXIF), so files stay
/// byte-identical with Go's.
use d2_rs_core::stable_sort::StableSort;

pub const VERSION: &str = "v0.9.0\n";

/// What `d2-rs --version`, `d2-rs version` and the help header report:
/// d2-rs's own release, where Go's `d2` reports D2's.
pub const CLI_VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"), "\n");

#[derive(Clone)]
struct Flag {
    kind: String,
    env: String,
    name: String,
    short: String,
    default: String,
    usage: String,
    hidden: bool,
    /// An environment variable read by a d2cli helper (rather than
    /// xmain.Opts) to choose the flag's default, e.g. privateNetworkEnvDefault.
    default_env: String,
}

/// The pinned declarations, initialized once per process: one invocation reads
/// them from several argument scans.
fn flags() -> &'static [Flag] {
    static FLAGS: std::sync::OnceLock<Vec<Flag>> = std::sync::OnceLock::new();
    FLAGS.get_or_init(|| include!("command_line_flags.rs"))
}

pub fn usage(detail: impl std::fmt::Display) -> String {
    format!("bad usage: {detail}\nRun with --help to see usage.")
}

fn quote(value: &str) -> String {
    d2_rs_core::quote_go_bytes(value.as_bytes())
}

fn boolean(value: &str) -> Option<bool> {
    match value {
        "1" | "true" | "TRUE" | "True" | "t" | "T" => Some(true),
        "0" | "false" | "FALSE" | "False" | "f" | "F" => Some(false),
        _ => None,
    }
}

// strconv.ParseInt(..., 0, 64), including base prefixes and Go underscore rules.
fn integer(value: &str, infer_base: bool) -> Result<i64, &'static str> {
    let (negative, unsigned) = if let Some(s) = value.strip_prefix('-') {
        (true, s)
    } else {
        (false, value.strip_prefix('+').unwrap_or(value))
    };
    let (base, digits, prefix) = if infer_base && unsigned.len() >= 2 && unsigned.starts_with('0') {
        match unsigned.as_bytes()[1] {
            b'x' | b'X' => (16, &unsigned[2..], true),
            b'b' | b'B' => (2, &unsigned[2..], true),
            b'o' | b'O' => (8, &unsigned[2..], true),
            _ => (8, &unsigned[1..], true),
        }
    } else {
        (10, unsigned, false)
    };
    if digits.is_empty() {
        return Err("invalid syntax");
    }
    let mut previous_digit = prefix;
    let mut count = 0;
    let mut magnitude = Some(0u64);
    for c in digits.chars() {
        if c == '_' && infer_base && previous_digit {
            previous_digit = false;
            continue;
        }
        let digit = c
            .to_digit(base)
            .filter(|_| c.is_ascii())
            .ok_or("invalid syntax")?;
        count += 1;
        previous_digit = true;
        magnitude = magnitude.and_then(|n| {
            n.checked_mul(u64::from(base))?
                .checked_add(u64::from(digit))
        });
    }
    if !previous_digit || count == 0 {
        return Err("invalid syntax");
    }
    let magnitude = magnitude.ok_or("value out of range")?;
    if negative {
        if magnitude == 1u64 << 63 {
            Ok(i64::MIN)
        } else {
            i64::try_from(magnitude)
                .map(|n| -n)
                .map_err(|_| "value out of range")
        }
    } else {
        i64::try_from(magnitude).map_err(|_| "value out of range")
    }
}

fn validate(flag: &Flag, value: &str) -> Result<(), String> {
    if flag.kind == "Int64Slice" {
        for item in value.split(',') {
            if let Err(reason) = integer(item, true) {
                return Err(invalid_value(flag, value, "ParseInt", item, reason));
            }
        }
        return Ok(());
    }
    let result = match flag.kind.as_str() {
        "Bool" => boolean(value)
            .map(|_| ())
            .ok_or(("ParseBool", "invalid syntax")),
        "Int64" => integer(value, true)
            .map(|_| ())
            .map_err(|reason| ("ParseInt", reason)),
        "Float64" => d2_rs_core::parse_d2_float_result(value)
            .map(|_| ())
            .map_err(|reason| ("ParseFloat", reason)),
        _ => Ok(()),
    };
    result.map_err(|(parser, reason)| invalid_value(flag, value, parser, value, reason))
}

fn invalid_value(flag: &Flag, value: &str, parser: &str, parsed: &str, reason: &str) -> String {
    let name = if flag.short.is_empty() {
        format!("--{}", flag.name)
    } else {
        format!("-{}, --{}", flag.short, flag.name)
    };
    usage(format!(
        "failed to parse flags: invalid argument {} for {} flag: strconv.{parser}: parsing {}: {reason}",
        quote(value),
        quote(&name),
        quote(parsed)
    ))
}

fn defaults(flags: &[Flag]) -> Result<Vec<String>, String> {
    flags
        .iter()
        .map(|flag| {
            if !flag.default_env.is_empty() {
                // d2cli.privateNetworkEnvDefault: exact spellings only.
                return match std::env::var(&flag.default_env)
                    .unwrap_or_default()
                    .as_str()
                {
                    "" | "0" | "false" => Ok("false".to_owned()),
                    "1" | "true" => Ok("true".to_owned()),
                    value => Err(usage(format!(
                        "invalid environment variable {}. Expected bool. Found \"{value}\".",
                        flag.default_env
                    ))),
                };
            }
            let value = std::env::var(&flag.env).ok().filter(|v| !v.is_empty());
            let Some(value) = value else {
                return Ok(flag.default.clone());
            };
            let parsed = match flag.kind.as_str() {
                "Bool" if matches!(value.as_str(), "true" | "1") => "true".to_owned(),
                "Bool" if matches!(value.as_str(), "false" | "0") => "false".to_owned(),
                "Bool" if flag.env == "DEBUG" => {
                    eprintln!("warn: Invalid DEBUG flag value ignored");
                    return Ok(flag.default.clone());
                }
                "Bool" => {
                    return Err(usage(format!(
                        "invalid environment variable {}. Expected bool. Found \"{}\".",
                        flag.env, value
                    )));
                }
                "Int64" => integer(&value, false)
                    .map(|n| n.to_string())
                    .map_err(|reason| {
                        usage(format!(
                            "invalid environment variable {}. Expected int64. Found \"{}\".",
                            flag.env,
                            if reason == "value out of range" {
                                if value.starts_with('-') {
                                    i64::MIN
                                } else {
                                    i64::MAX
                                }
                            } else {
                                0
                            }
                        ))
                    })?,
                "Float64" => {
                    let n = d2_rs_core::parse_d2_float_result(&value).map_err(|reason| {
                        let found = if reason == "value out of range" {
                            if value.starts_with('-') {
                                "-Inf"
                            } else {
                                "+Inf"
                            }
                        } else {
                            "0"
                        };
                        usage(format!(
                            "invalid environment variable {}. Expected float64. Found \"{found}\".",
                            flag.env
                        ))
                    })?;
                    n.to_string()
                }
                _ => value,
            };
            Ok(parsed)
        })
        .collect()
}

fn initialized() -> Result<(Vec<Flag>, Vec<String>), String> {
    let mut flags = flags().to_vec();
    let mut values = defaults(&flags)?;
    // xmain.Bool returns before registering the flag when DEBUG is invalid;
    // Run warns and continues. The absent declaration affects parsing/help.
    if std::env::var("DEBUG").ok().is_some_and(|value| {
        !value.is_empty() && !matches!(value.as_str(), "0" | "1" | "true" | "false")
    }) {
        let index = flags.iter().position(|flag| flag.name == "debug").unwrap();
        flags.remove(index);
        values.remove(index);
    }
    Ok((flags, values))
}

fn help_with(flags: &[Flag], values: &[String], program: &str) -> Vec<u8> {
    let visible = flags
        .iter()
        .zip(values)
        .filter(|(flag, _)| !flag.hidden)
        .collect::<Vec<_>>();
    let text = flag_table(&visible);
    include!("command_line_help.rs")
        .replace("{program}", program)
        .replace("{version}", CLI_VERSION.trim_end())
        .replace("{flags}", &text)
        .into_bytes()
}

/// xmain.Opts.Defaults over one flag set: measure the two sentinel-delimited
/// columns, then Fprintln separates each fragment with a space; multiline
/// usage is indented.
fn flag_table(flags: &[(&Flag, &String)]) -> String {
    let columns: Vec<_> = flags
        .iter()
        .map(|(flag, _)| {
            let mut label = if flag.short.is_empty() {
                format!("      --{}", flag.name)
            } else {
                format!("  -{}, --{}", flag.short, flag.name)
            };
            label.push_str(match flag.kind.as_str() {
                "String" => " string",
                "Int64" => " int",
                "Float64" => " float",
                "Int64Slice" => " int64Slice",
                _ => "",
            });
            let env = if flag.env.is_empty() {
                String::new()
            } else {
                format!("${}", flag.env)
            };
            (label, env)
        })
        .collect();
    let max_label = columns
        .iter()
        .map(|(label, _)| label.len() + 1)
        .max()
        .unwrap_or(0);
    let max_combined = columns
        .iter()
        .map(|(label, env)| label.len() + env.len() + 2)
        .max()
        .unwrap_or(0);
    let mut text = String::new();
    for ((flag, value), (label, env)) in flags.iter().zip(columns) {
        let value = match flag.kind.as_str() {
            "String" => quote(value),
            "Int64Slice" => format!("[{value}]"),
            _ => (*value).clone(),
        };
        let description = format!("{} (default {value})", flag.usage)
            .replace('\n', &format!("\n{}", " ".repeat(max_combined + 3)));
        text.push_str(&format!(
            "{label} {} {env} {} {description}\n",
            " ".repeat(max_label - label.len()),
            " ".repeat(max_combined - max_label - env.len() - 1)
        ));
    }
    text
}

/// The Defaults() table of one bundled layout plugin's own xmain.Opts
/// (d2plugin plugin_{dagre,elk,tala}.go), whose flags are the hidden
/// `<engine>-` declarations of the pinned CLI.
pub fn layout_flags(engine: &str) -> String {
    let flags = flags();
    let prefix = format!("{engine}-");
    let selected = flags
        .iter()
        .filter(|flag| flag.hidden && flag.name.starts_with(&prefix))
        .map(|flag| (flag, &flag.default))
        .collect::<Vec<_>>();
    flag_table(&selected)
}

pub fn help() -> Result<Vec<u8>, String> {
    let (flags, values) = initialized()?;
    Ok(help_with(&flags, &values, &program()))
}

fn program() -> String {
    std::env::args()
        .next()
        .and_then(|name| {
            std::path::Path::new(&name)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "d2-rs".into())
}

// lib/font.Sfnt2Woff validates aligned table checksums in tag order before
// encoding. Version has no font output, but must still perform that validation.
fn validate_font_data(bytes: &[u8]) -> Result<(), String> {
    fn part(bytes: &[u8], start: usize, len: usize) -> Result<&[u8], String> {
        bytes
            .get(start..start.saturating_add(len))
            .ok_or_else(|| "font table is outside the supplied bytes".to_owned())
    }
    fn u32_at(bytes: &[u8], start: usize) -> Result<u32, String> {
        Ok(u32::from_be_bytes(
            part(bytes, start, 4)?.try_into().unwrap(),
        ))
    }
    let count = u16::from_be_bytes(part(bytes, 4, 2)?.try_into().unwrap());
    let mut entries = Vec::new();
    for index in 0..usize::from(count) {
        let entry = part(bytes, 12 + index * 16, 16)?;
        entries.push((
            part(entry, 0, 4)?,
            u32_at(entry, 4)?,
            u32_at(entry, 8)? as usize,
            u32_at(entry, 12)?,
        ));
    }
    entries.stable_sort_by_key(|entry| entry.0);
    for &(tag, expected, offset, len) in &entries {
        if tag != b"head" {
            let aligned = len.wrapping_add(3) & !3;
            let actual = part(bytes, offset, aligned as usize)?
                .chunks_exact(4)
                .fold(0u32, |sum, word| {
                    sum.wrapping_add(u32::from_be_bytes(word.try_into().unwrap()))
                });
            if actual != expected {
                return Err(format!(
                    "checksum error in table: {}",
                    String::from_utf8_lossy(tag)
                ));
            }
        }
    }
    part(bytes, 0, 12)?;
    for &(tag, _, offset, len) in &entries {
        let data = part(bytes, offset, len as usize)?;
        if tag == b"head" {
            part(data, 0, 12)?;
        }
    }
    Ok(())
}

pub enum Boundary {
    Output(Vec<u8>),
    Document(Vec<String>),
}

/// Rewrite pflag shorthand clusters (`-l tala`, `-sw`, `-t=1`) into the long
/// `--name=value` spellings the render parser accepts. Unknown shorthands,
/// long flags, `-` and everything after `--` pass through untouched so the
/// existing parser reports them.
pub fn expand_shorthands(args: &[String]) -> Vec<String> {
    let flags = flags();
    let mut output = Vec::with_capacity(args.len());
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        index += 1;
        if argument == "--" {
            output.extend(args[index - 1..].iter().cloned());
            break;
        }
        if argument == "-" || argument.starts_with("--") || !argument.starts_with('-') {
            output.push(argument.clone());
            continue;
        }
        let mut expanded = Vec::new();
        let mut shorts = &argument[1..];
        let mut recognized = true;
        while !shorts.is_empty() {
            let width = shorts.chars().next().map_or(1, char::len_utf8);
            let (name, rest) = shorts.split_at(width);
            let Some(flag) = flags.iter().find(|f| f.short == name) else {
                recognized = false;
                break;
            };
            if let Some(value) = rest.strip_prefix('=') {
                expanded.push(format!("--{}={value}", flag.name));
                shorts = "";
            } else if flag.kind == "Bool" {
                expanded.push(format!("--{}=true", flag.name));
                shorts = rest;
            } else if !rest.is_empty() {
                expanded.push(format!("--{}={rest}", flag.name));
                shorts = "";
            } else if let Some(value) = args.get(index) {
                expanded.push(format!("--{}={value}", flag.name));
                index += 1;
                shorts = "";
            } else {
                recognized = false;
                break;
            }
        }
        if recognized {
            output.extend(expanded);
        } else {
            output.push(argument.clone());
        }
    }
    output
}

/// The positional arguments pflag leaves after parsing the pinned flag set:
/// known non-boolean flags consume their value, and `--` ends flag parsing.
pub fn positional_arguments(args: &[String]) -> Vec<String> {
    let flags = flags();
    let args = expand_shorthands(args);
    let mut positionals = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        index += 1;
        if argument == "--" {
            positionals.extend(args[index..].iter().cloned());
            break;
        }
        let Some(name) = argument.strip_prefix("--") else {
            positionals.push(argument.clone());
            continue;
        };
        if !name.contains('=')
            && flags
                .iter()
                .any(|flag| flag.name == name && flag.kind != "Bool")
        {
            index += 1;
        }
    }
    positionals
}

/// Read one option belonging to a bundled Dagre/ELK adapter. TALA seeds retain
/// their list/repetition handling at the existing source boundary.
pub fn engine_option(
    args: &[String],
    index: &mut usize,
) -> Result<Option<(String, String)>, String> {
    let Some(argument) = args[*index].strip_prefix("--") else {
        return Ok(None);
    };
    let (name, explicit) = argument
        .split_once('=')
        .map_or((argument, None), |(name, value)| (name, Some(value)));
    let Some(flag) = flags()
        .iter()
        .find(|flag| flag.hidden && flag.name == name && flag.kind != "Int64Slice")
    else {
        return Ok(None);
    };
    let value = if let Some(value) = explicit {
        value.to_owned()
    } else {
        *index += 1;
        args.get(*index).cloned().ok_or_else(|| {
            usage(format!(
                "failed to parse flags: flag needs an argument: --{name}"
            ))
        })?
    };
    validate(flag, &value)?;
    let value = if flag.kind == "Int64" {
        integer(&value, true).unwrap().to_string()
    } else {
        value
    };
    Ok(Some((name.to_owned(), value)))
}

pub fn set_engine_option(
    settings: &mut d2_rs_core::LayoutEngineOptions,
    name: &str,
    value: String,
) {
    match name {
        "dagre-nodesep" => settings.dagre_node_sep = Some(value.parse().unwrap()),
        "dagre-edgesep" => settings.dagre_edge_sep = Some(value.parse().unwrap()),
        "elk-algorithm" => settings.elk_algorithm = Some(value),
        "elk-nodeNodeBetweenLayers" => settings.elk_node_spacing = Some(value.parse().unwrap()),
        "elk-edgeNodeBetweenLayers" => {
            settings.elk_edge_node_spacing = Some(value.parse().unwrap())
        }
        "elk-edgeEdgeBetweenLayers" => {
            settings.elk_edge_edge_spacing = Some(value.parse().unwrap())
        }
        "elk-nodeSelfLoop" => settings.elk_self_loop_spacing = Some(value.parse().unwrap()),
        "elk-padding" => settings.elk_padding = Some(value),
        _ => unreachable!("validated bundled engine option"),
    }
}

/// Parse before version dispatch, as d2cli.Run does. Other invocations retain
/// their existing renderer/command parser. Consumed string values cannot turn
/// into version flags and `--` terminates flag parsing.
pub fn version_boundary(args: &[String]) -> Result<Boundary, String> {
    let (flags, defaults) = initialized()?;
    let mut values = defaults.clone();
    let mut positionals = Vec::new();
    let mut remaining = Vec::new();
    let mut index = 0;
    let mut ended = false;
    while index < args.len() {
        let argument = &args[index];
        if ended || argument == "-" || !argument.starts_with('-') {
            positionals.push(argument.clone());
            remaining.push(argument.clone());
            index += 1;
            continue;
        }
        if argument == "--" {
            ended = true;
            remaining.push(argument.clone());
            index += 1;
            continue;
        }
        if argument == "--help" {
            return Ok(Boundary::Output(help_with(&flags, &defaults, &program())));
        }
        let long = argument.starts_with("--");
        let mut shorts = argument.trim_start_matches('-');
        loop {
            let (name, explicit) = if long {
                shorts
                    .split_once('=')
                    .map_or((shorts, None), |(n, v)| (n, Some(v)))
            } else {
                let width = shorts.chars().next().unwrap().len_utf8();
                (&shorts[..width], shorts[width..].strip_prefix('='))
            };
            if long && name == "help" {
                return Ok(Boundary::Output(help_with(&flags, &defaults, &program())));
            }
            let found = flags.iter().position(|f| {
                if long {
                    f.name == name
                } else {
                    f.short == name
                }
            });
            let Some(slot) = found else {
                let detail = if long {
                    format!("unknown flag: --{name}")
                } else {
                    format!("unknown shorthand flag: '{}' in -{shorts}", name)
                };
                return Err(usage(format!("failed to parse flags: {detail}")));
            };
            let flag = &flags[slot];
            let mut tail = if long { "" } else { &shorts[name.len()..] };
            let value = if let Some(value) = explicit {
                tail = "";
                value.to_owned()
            } else if flag.kind == "Bool" {
                "true".to_owned()
            } else if !tail.is_empty() {
                let value = tail.to_owned();
                tail = "";
                value
            } else {
                index += 1;
                args.get(index).cloned().ok_or_else(|| {
                    usage(format!(
                        "failed to parse flags: flag needs an argument: {}",
                        if long {
                            format!("--{name}")
                        } else {
                            format!("'{name}' in -{shorts}")
                        }
                    ))
                })?
            };
            validate(flag, &value)?;
            values[slot] = value.clone();
            if flag.name != "version" {
                let forwarded = match flag.kind.as_str() {
                    "Int64" => integer(&value, true).unwrap().to_string(),
                    "Float64" => d2_rs_core::parse_d2_float_result(&value)
                        .unwrap()
                        .to_string(),
                    "Bool" => boolean(&value).unwrap().to_string(),
                    _ => value,
                };
                remaining.push(format!("--{}={forwarded}", flag.name));
            }
            if tail.is_empty() {
                break;
            }
            shorts = tail;
        }
        index += 1;
    }
    // d2cli.Run loads supplied faces after parsing, before subcommand dispatch.
    let mut fonts = Vec::new();
    for (flag, path) in flags
        .iter()
        .zip(&values)
        .filter(|(flag, path)| flag.name.starts_with("font-") && !path.is_empty())
    {
        let file = std::path::Path::new(path);
        let extension = file
            .extension()
            .map(|ext| format!(".{}", ext.to_string_lossy()))
            .unwrap_or_default();
        if extension != ".ttf" {
            return Err(usage(format!(
                "failed to load specified fonts: expected .ttf file but {path} has extension {extension}"
            )));
        }
        let bytes = std::fs::read(file).map_err(|error| {
            usage(format!(
                "failed to load specified fonts: failed to read font at {path}: {}",
                crate::local_asset_read_error(file, error)
            ))
        })?;
        eprintln!(
            "info: font {} loaded",
            file.file_name().unwrap_or_default().to_string_lossy()
        );
        fonts.push((&flag.name, bytes));
    }
    const FONT_LOAD_ORDER: [&str; 8] = [
        "font-regular",
        "font-italic",
        "font-bold",
        "font-semibold",
        "font-mono",
        "font-mono-italic",
        "font-mono-bold",
        "font-mono-semibold",
    ];
    fonts.stable_sort_by_key(|(name, _)| {
        FONT_LOAD_ORDER
            .iter()
            .position(|slot| slot == name)
            .unwrap()
    });
    for (_, bytes) in fonts {
        validate_font_data(&bytes).map_err(|error| {
            usage(format!(
                "failed to load specified fonts: failed to encode ttf to woff: {error}"
            ))
        })?;
    }
    if positionals.first().is_some_and(|p| p == "version") {
        if positionals.len() > 1 {
            return Err(usage("version subcommand accepts no arguments"));
        }
        return Ok(Boundary::Output(CLI_VERSION.as_bytes().to_vec()));
    }
    if positionals.is_empty() {
        let version = flags.iter().position(|f| f.name == "version").unwrap();
        return Ok(Boundary::Output(
            if boolean(&values[version]) == Some(true) {
                CLI_VERSION.as_bytes().to_vec()
            } else {
                help_with(&flags, &defaults, &program())
            },
        ));
    }
    Ok(Boundary::Document(remaining))
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    /// Historical version-dispatch cases retain their original expectation;
    /// master help is independently captured from the pinned release oracle.
    #[cfg(feature = "oracle-fixtures")]
    fn master_help(v090: &str) -> String {
        if v090.contains("Usage:\n") {
            d2_rs_testdata::oracle_fixture!("tests/fixtures/help-master-6a7b4837.txt").to_owned()
        } else {
            v090.to_owned()
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_go_version_flag_dispatch_and_errors() {
        let fixtures: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/version-flags-source-v090.json"
        ))
        .unwrap();
        for row in fixtures["rows"].as_array().unwrap() {
            let args: Vec<String> = serde_json::from_value(row["args"].clone()).unwrap();
            let result = version_boundary(&args);
            if row["dispatch"] == "document" {
                let Boundary::Document(document) = result.unwrap() else {
                    panic!("{} must reach document rendering", row["name"]);
                };
                assert!(
                    document.iter().all(|arg| !arg.starts_with("--version")),
                    "{}",
                    row["name"]
                );
                continue;
            }
            match result {
                Ok(Boundary::Output(bytes)) => {
                    assert_eq!(row["exit"], 0, "{}", row["name"]);
                    assert_eq!(
                        String::from_utf8(bytes).unwrap(),
                        // d2-rs reports its own version where Go reports
                        // D2's: the version line and the help header.
                        master_help(row["stdout"].as_str().unwrap())
                            .replace("{program}", &program())
                            .replacen(VERSION, CLI_VERSION, 1),
                        "{}",
                        row["name"]
                    );
                }
                Err(error) => {
                    let mut stderr = Vec::new();
                    crate::write_cli_error(&mut stderr, &error).unwrap();
                    assert_eq!(row["exit"], 1, "{}", row["name"]);
                    assert_eq!(
                        String::from_utf8(stderr).unwrap(),
                        row["stderr"],
                        "{}",
                        row["name"]
                    );
                }
                Ok(Boundary::Document(_)) => panic!("unexpected document: {}", row["name"]),
            }
        }
    }

    #[test]
    fn shorthand_flags_expand_like_pflag() {
        let args = |values: &[&str]| values.iter().map(|v| v.to_string()).collect::<Vec<_>>();
        assert_eq!(
            super::expand_shorthands(&args(&[
                "-l", "tala", "-t=1", "-sb", "in.d2", "-", "--", "-l"
            ])),
            args(&[
                "--layout=tala",
                "--theme=1",
                "--sketch=true",
                "--bundle=true",
                "in.d2",
                "-",
                "--",
                "-l"
            ])
        );
        assert_eq!(
            super::expand_shorthands(&args(&["-ltala"])),
            args(&["--layout=tala"])
        );
        // Unknown shorthands and negative numbers are left for the parser.
        assert_eq!(
            super::expand_shorthands(&args(&["-Z", "--dark-theme", "-1"])),
            args(&["-Z", "--dark-theme", "-1"])
        );
    }
}
