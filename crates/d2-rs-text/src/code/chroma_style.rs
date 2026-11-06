// SPDX-License-Identifier: MIT
// Chroma v2.27.0 TokenType numbering (types.go), StyleEntry parsing and
// inheritance (style.go ParseStyleEntry/Inherit/Style.Get/get), Colour
// formatting (colour.go ParseColour/String) and the SVG formatter's
// StyleEntryToSVG (formatters/svg/svg.go). Copyright (C) 2017 Alec Thomas.
// Rust translation Copyright 2026 Nelson Chen. See assets/chroma/COPYING.
//! Exact Chroma style lookups, keyed by numeric TokenType as in the source.
use std::{collections::BTreeMap, sync::OnceLock};

/// types.go TokenType constants: (enumer name, value).
pub const TOKEN_TYPES: &[(&str, i32)] = &[
    ("Background", -1),
    ("PreWrapper", -2),
    ("Line", -3),
    ("LineNumbers", -4),
    ("LineNumbersTable", -5),
    ("LineHighlight", -6),
    ("LineTable", -7),
    ("LineTableTD", -8),
    ("LineLink", -9),
    ("CodeLine", -10),
    ("Error", -11),
    ("Other", -12),
    ("None", -13),
    ("Ignore", -14),
    ("EOFType", 0),
    ("Keyword", 1000),
    ("KeywordConstant", 1001),
    ("KeywordDeclaration", 1002),
    ("KeywordNamespace", 1003),
    ("KeywordPseudo", 1004),
    ("KeywordReserved", 1005),
    ("KeywordType", 1006),
    ("Name", 2000),
    ("NameAttribute", 2001),
    ("NameClass", 2002),
    ("NameConstant", 2003),
    ("NameDecorator", 2004),
    ("NameEntity", 2005),
    ("NameException", 2006),
    ("NameKeyword", 2007),
    ("NameLabel", 2008),
    ("NameNamespace", 2009),
    ("NameOperator", 2010),
    ("NameOther", 2011),
    ("NamePseudo", 2012),
    ("NameProperty", 2013),
    ("NameTag", 2014),
    ("NameBuiltin", 2100),
    ("NameBuiltinPseudo", 2101),
    ("NameVariable", 2200),
    ("NameVariableAnonymous", 2201),
    ("NameVariableClass", 2202),
    ("NameVariableGlobal", 2203),
    ("NameVariableInstance", 2204),
    ("NameVariableMagic", 2205),
    ("NameFunction", 2300),
    ("NameFunctionMagic", 2301),
    ("Literal", 3000),
    ("LiteralDate", 3001),
    ("LiteralOther", 3002),
    ("LiteralString", 3100),
    ("LiteralStringAffix", 3101),
    ("LiteralStringAtom", 3102),
    ("LiteralStringBacktick", 3103),
    ("LiteralStringBoolean", 3104),
    ("LiteralStringChar", 3105),
    ("LiteralStringDelimiter", 3106),
    ("LiteralStringDoc", 3107),
    ("LiteralStringDouble", 3108),
    ("LiteralStringEscape", 3109),
    ("LiteralStringHeredoc", 3110),
    ("LiteralStringInterpol", 3111),
    ("LiteralStringName", 3112),
    ("LiteralStringOther", 3113),
    ("LiteralStringRegex", 3114),
    ("LiteralStringSingle", 3115),
    ("LiteralStringSymbol", 3116),
    ("LiteralNumber", 3200),
    ("LiteralNumberBin", 3201),
    ("LiteralNumberFloat", 3202),
    ("LiteralNumberHex", 3203),
    ("LiteralNumberInteger", 3204),
    ("LiteralNumberIntegerLong", 3205),
    ("LiteralNumberOct", 3206),
    ("LiteralNumberByte", 3207),
    ("Operator", 4000),
    ("OperatorWord", 4001),
    ("OperatorReserved", 4002),
    ("Punctuation", 5000),
    ("Comment", 6000),
    ("CommentHashbang", 6001),
    ("CommentMultiline", 6002),
    ("CommentSingle", 6003),
    ("CommentSpecial", 6004),
    ("CommentPreproc", 6100),
    ("CommentPreprocFile", 6101),
    ("Generic", 7000),
    ("GenericDeleted", 7001),
    ("GenericEmph", 7002),
    ("GenericError", 7003),
    ("GenericHeading", 7004),
    ("GenericInserted", 7005),
    ("GenericOutput", 7006),
    ("GenericPrompt", 7007),
    ("GenericStrong", 7008),
    ("GenericSubheading", 7009),
    ("GenericTraceback", 7010),
    ("GenericUnderline", 7011),
    ("Text", 8000),
    ("TextWhitespace", 8001),
    ("TextSymbol", 8002),
    ("TextPunctuation", 8003),
];

/// types.go StandardTypes keys (the CSS class names are not needed here).
const STANDARD_TYPES: &[i32] = &[
    -1, -2, -3, -4, -5, -6, -7, -8, -9, -10, 8000, 8001, -11, -12, //
    1000, 1001, 1002, 1003, 1004, 1005, 1006, //
    2000, 2001, 2100, 2101, 2002, 2003, 2004, 2005, 2006, 2300, 2301, 2013, 2008, 2009, 2011, 2014,
    2200, 2202, 2203, 2204, 2205, //
    3000, 3001, //
    3100, 3101, 3103, 3105, 3106, 3107, 3108, 3109, 3110, 3111, 3113, 3114, 3115, 3116, //
    3200, 3201, 3202, 3203, 3204, 3205, 3206, //
    4000, 4001, 4002, 5000, //
    6000, 6001, 6002, 6100, 6101, 6003, 6004, //
    7000, 7001, 7002, 7003, 7004, 7005, 7006, 7007, 7008, 7009, 7010, 7011,
];

/// TokenTypeString: the numeric TokenType for an enumer name.
pub fn token_type(name: &str) -> Option<i32> {
    TOKEN_TYPES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, v)| *v)
}

/// The interned enumer name for a token type name.
pub fn token_name(name: &str) -> Option<&'static str> {
    TOKEN_TYPES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(n, _)| *n)
}

/// TokenType.Category (Go integer division truncates toward zero).
pub fn category(t: i32) -> i32 {
    t / 1000 * 1000
}

/// TokenType.SubCategory.
pub fn sub_category(t: i32) -> i32 {
    t / 100 * 100
}

/// Trilean: 0 Pass, 1 Yes, 2 No.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Entry {
    /// Colour values are stored +1 as in Go, so 0 means unset.
    colour: u32,
    background: u32,
    border: u32,
    bold: u8,
    italic: u8,
    underline: u8,
    no_inherit: bool,
}

const YES: u8 = 1;
const NO: u8 = 2;

impl Entry {
    fn is_zero(&self) -> bool {
        self.colour == 0
            && self.background == 0
            && self.border == 0
            && self.bold == 0
            && self.italic == 0
            && self.underline == 0
            && !self.no_inherit
    }

    /// StyleEntry.Inherit: ancestors are given oldest to newest and walked
    /// backwards; only the receiver's own NoInherit stops the walk.
    fn inherit(self, ancestors: &[Entry]) -> Entry {
        let mut out = self;
        for ancestor in ancestors.iter().rev() {
            if out.no_inherit {
                return out;
            }
            if out.colour == 0 {
                out.colour = ancestor.colour;
            }
            if out.background == 0 {
                out.background = ancestor.background;
            }
            if out.border == 0 {
                out.border = ancestor.border;
            }
            if out.bold == 0 {
                out.bold = ancestor.bold;
            }
            if out.italic == 0 {
                out.italic = ancestor.italic;
            }
            if out.underline == 0 {
                out.underline = ancestor.underline;
            }
        }
        out
    }
}

/// colour.go normaliseColour + ParseColour. ANSI2RGB names are not used by
/// the pinned styles; any unparsable colour panics instead of degrading.
fn parse_colour(colour: &str) -> u32 {
    let mut c = colour.strip_prefix('#').unwrap_or(colour).to_owned();
    if c.len() == 3 {
        c = c.chars().flat_map(|ch| [ch, ch]).collect();
    }
    match u32::from_str_radix(&c, 16) {
        Ok(n) if !c.starts_with('+') => n + 1,
        _ => panic!("pinned Chroma colour {colour:?}"),
    }
}

/// colour.go Red/Green/Blue of a Colour (int32 c-1, so unset reads 0xffffff).
fn rgb(c: u32) -> [f64; 3] {
    let v = c.wrapping_sub(1);
    [(v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff].map(|x| f64::from(x as u8))
}

/// colour.go BrightenOrDarken/Brighten/Brightness/NewColour.
fn brighten_or_darken(c: u32, factor: f64) -> u32 {
    let [r, g, b] = rgb(c);
    let brightness = (r + g + b) / 255.0 / 3.0;
    let mut factor = if brightness < 0.5 { factor } else { -factor };
    let [r, g, b] = if factor < 0.0 {
        factor += 1.0;
        [r * factor, g * factor, b * factor]
    } else {
        // Unreached by the pinned github/catppuccin-mocha backgrounds (both
        // darken); Go ARM64 may fuse this multiply-add.
        [r, g, b].map(|x| (255.0 - x) * factor + x)
    };
    // Go uint8(float64) truncates for in-range values.
    ((r as u8 as u32) << 16 | (g as u8 as u32) << 8 | (b as u8 as u32)) + 1
}

/// colour.go Colour.String.
fn colour_string(c: u32) -> String {
    format!("#{:06x}", c - 1)
}

/// style.go ParseStyleEntry (the pinned styles contain only valid entries).
fn parse_entry(entry: &str) -> Entry {
    let mut out = Entry::default();
    for part in entry.split_whitespace() {
        match part {
            "italic" => out.italic = YES,
            "noitalic" => out.italic = NO,
            "bold" => out.bold = YES,
            "nobold" => out.bold = NO,
            "underline" => out.underline = YES,
            "nounderline" => out.underline = NO,
            "inherit" => out.no_inherit = false,
            "noinherit" => out.no_inherit = true,
            "bg:" => out.background = 0,
            _ if part.starts_with("bg:#") => out.background = parse_colour(&part[3..]),
            _ if part.starts_with("border:#") => out.border = parse_colour(&part[7..]),
            _ if part.starts_with('#') => out.colour = parse_colour(part),
            other => panic!("pinned Chroma style element {other:?}"),
        }
    }
    out
}

struct Style {
    entries: BTreeMap<i32, Entry>,
}

impl Style {
    fn parse(source: &str) -> Style {
        let doc = roxmltree::Document::parse(source).expect("pinned Chroma style");
        let entries = doc
            .descendants()
            .filter(|n| n.has_tag_name("entry"))
            .map(|n| {
                let name = n.attribute("type").expect("style entry type");
                let tt = token_type(name).unwrap_or_else(|| panic!("Chroma token type {name}"));
                (tt, parse_entry(n.attribute("style").unwrap_or("")))
            })
            .collect();
        Style { entries }
    }

    /// Style.get without a parent, including synthesise for LineHighlight
    /// and LineNumbers(Table).
    fn get_raw(&self, t: i32) -> Entry {
        let out = self.entries.get(&t).copied().unwrap_or_default();
        if out.is_zero() && matches!(t, -6..=-4) {
            let bg = self.get_raw(-1);
            return match t {
                -6 => Entry {
                    background: brighten_or_darken(bg.background, 0.1),
                    ..Entry::default()
                },
                _ => Entry {
                    colour: brighten_or_darken(bg.colour, 0.5),
                    ..Entry::default()
                },
            };
        }
        out
    }

    /// Style.Get.
    fn get(&self, t: i32) -> Entry {
        self.get_raw(t).inherit(&[
            self.get_raw(-1),
            self.get_raw(8000),
            self.get_raw(category(t)),
            self.get_raw(sub_category(t)),
        ])
    }
}

/// svg.StyleEntryToSVG.
fn entry_to_svg(e: &Entry) -> String {
    let mut styles = Vec::new();
    if e.colour != 0 {
        styles.push(format!("fill=\"{}\"", colour_string(e.colour)));
    }
    if e.bold == YES {
        styles.push("font-weight=\"bold\"".to_owned());
    }
    if e.italic == YES {
        styles.push("font-style=\"italic\"".to_owned());
    }
    if e.underline == YES {
        styles.push("text-decoration=\"underline\"".to_owned());
    }
    styles.join(" ")
}

fn style(dark: bool) -> &'static Style {
    static LIGHT: OnceLock<Style> = OnceLock::new();
    static DARK: OnceLock<Style> = OnceLock::new();
    if dark {
        DARK.get_or_init(|| Style::parse(include_str!("../../assets/chroma/catppuccin-mocha.xml")))
    } else {
        LIGHT.get_or_init(|| Style::parse(include_str!("../../assets/chroma/github.xml")))
    }
}

/// d2svg/code.go styleToSVG for styles.Get("github") (light) or
/// styles.Get("catppuccin-mocha") (dark): StyleEntryToSVG of every
/// StandardTypes entry whose inherited Style.Get is nonzero.
pub fn svg_styles(dark: bool) -> &'static BTreeMap<i32, String> {
    static LIGHT: OnceLock<BTreeMap<i32, String>> = OnceLock::new();
    static DARK: OnceLock<BTreeMap<i32, String>> = OnceLock::new();
    (if dark { &DARK } else { &LIGHT }).get_or_init(|| {
        let style = style(dark);
        STANDARD_TYPES
            .iter()
            .filter_map(|&t| {
                let entry = style.get(t);
                (!entry.is_zero()).then(|| (t, entry_to_svg(&entry)))
            })
            .collect()
    })
}

/// style.Get(chroma.Background).Background.String().
pub fn background(dark: bool) -> String {
    colour_string(style(dark).get(-1).background)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn categories_truncate_toward_zero_and_backgrounds_match_pinned_styles() {
        assert_eq!(category(-1), 0);
        assert_eq!(sub_category(3108), 3100);
        assert_eq!(category(2205), 2000);
        assert_eq!(background(false), "#f7f7f7");
        assert_eq!(background(true), "#1e1e2e");
        // github Error: "#f6f8fa bg:#82071e"; Text inherits nothing visible.
        assert_eq!(svg_styles(false)[&-11], "fill=\"#f6f8fa\"");
        assert_eq!(svg_styles(false)[&8000], "");
        assert_eq!(svg_styles(true)[&8000], "fill=\"#cdd6f4\"");
    }
}
