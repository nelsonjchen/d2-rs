// Copyright (c) 2009-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned mathjax-go 94d80186bfd7 tex/mml_token.go and mml/definitions.go.
use super::*;

#[derive(Clone, Debug, PartialEq)]
enum Value {
    Text(String),
    Boolean(bool),
}
impl Value {
    fn text(&self) -> &str {
        match self {
            Self::Text(s) => s,
            Self::Boolean(true) => "true",
            Self::Boolean(false) => "false",
        }
    }
}

struct Token {
    kind: String,
    text: String,
    // Preserve assignment order and duplicate keep names separately, as
    // applySourceObject overwrites properties without deduplicating keep.
    attributes: Vec<(String, Value)>,
    keep: Vec<String>,
}

fn token_kind(kind: &str) -> bool {
    matches!(
        kind,
        "mi" | "mn" | "mo" | "mtext" | "mspace" | "ms" | "mglyph"
    )
}
fn allowed(kind: &str, name: &str) -> bool {
    matches!(
        name,
        "fontfamily" | "fontsize" | "fontweight" | "fontstyle" | "color" |
        "background" | "id" | "class" | "href" | "style" |
        "mathbackground" | "mathcolor" | "mathsize" | "dir" | "mathvariant" |
        // Attributes.HasDefault includes the factory's math globals.
        "scriptlevel" | "displaystyle" | "display" | "maxwidth" | "overflow" |
        "altimg" | "alttext" | "cdgroup" | "scriptsizemultiplier" | "scriptminsize" |
        "infixlinebreakstyle" | "lineleading" | "linebreakmultchar" |
        "indentshift" | "indentalign" | "indenttarget" | "indentalignfirst" |
        "indentshiftfirst" | "indentalignlast" | "indentshiftlast"
    ) || kind == "mspace" && matches!(name, "width" | "height" | "depth" | "linebreak")
        || kind == "ms" && matches!(name, "lquote" | "rquote")
        || kind == "mglyph"
            && matches!(
                name,
                "alt" | "src" | "index" | "width" | "height" | "valign"
            )
        || kind == "mo"
            && matches!(
                name,
                "form"
                    | "fence"
                    | "separator"
                    | "lspace"
                    | "rspace"
                    | "stretchy"
                    | "symmetric"
                    | "maxsize"
                    | "minsize"
                    | "largeop"
                    | "movablelimits"
                    | "accent"
                    | "linebreak"
                    | "lineleading"
                    | "linebreakstyle"
                    | "indentalign"
                    | "indentshift"
                    | "indenttarget"
                    | "indentalignfirst"
                    | "indentshiftfirst"
                    | "indentalignlast"
                    | "indentshiftlast"
            )
}

fn attributes(kind: &str, raw: &str) -> Result<(Vec<(String, Value)>, Vec<String>), String> {
    static ATTRIBUTE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        let space = r"[\t\n\v\f\r \x{00A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}]";
        regex::Regex::new(&format!(
            r#"^([a-zA-Z]+){space}*={space}*('[^']*'|"[^"]*"|[^ ,]*){space}*,?{space}*"#
        ))
        .unwrap()
    });
    let mut raw = raw.trim_start_matches(|c| Parser::prime_space(&c));
    let mut result: Vec<(String, Value)> = Vec::new();
    let mut keep = Vec::new();
    while !raw.is_empty() {
        let capture = ATTRIBUTE
            .captures(raw)
            .ok_or_else(|| format!("Invalid MathML attribute: {raw}"))?;
        let name = &capture[1];
        // Recognition precedes ignoring an empty value.
        if !allowed(kind, name) {
            return Err(format!("{name} is not a recognized attribute for {kind}"));
        }
        let mut value = &capture[2];
        if value.len() >= 2
            && matches!(value.as_bytes()[0], b'\'' | b'"')
            && value.as_bytes().last() == value.as_bytes().first()
            && !value[1..value.len() - 1].contains(['\n', '\r', '\u{2028}', '\u{2029}'])
        {
            value = &value[1..value.len() - 1];
        }
        if !value.is_empty() {
            let value = if value.eq_ignore_ascii_case("true") {
                Value::Boolean(true)
            } else if value.eq_ignore_ascii_case("false") {
                Value::Boolean(false)
            } else {
                Value::Text(value.into())
            };
            if let Some((_, old)) = result.iter_mut().find(|(key, _)| key == name) {
                *old = value;
            } else {
                result.push((name.into(), value));
            }
            keep.push(name.into());
        }
        raw = &raw[capture[0].len()..];
    }
    Ok((result, keep))
}

impl Parser {
    // tex/parser.go readArgument: shared by MmlToken and BeginEnd.
    pub(super) fn read_argument(&mut self, name: &str) -> Result<String, String> {
        while self
            .chars
            .get(self.index)
            .copied()
            .is_some_and(internal_text::space)
        {
            self.index += 1;
        }
        match self.chars.get(self.index).copied() {
            None => Err(format!("Missing argument for \\{name}")),
            Some('}') => Err("Extra close brace or missing open brace".into()),
            Some('{') => self
                .braced_text()
                .ok_or_else(|| "Missing close brace".into()),
            Some('\\') => {
                self.index += 1;
                let cs = self.capture_control_sequence();
                Ok(format!(
                    "\\{}",
                    if matches!(cs.as_str(), "\n" | "\r" | "\u{2028}" | "\u{2029}") {
                        " "
                    } else {
                        &cs
                    }
                ))
            }
            Some(c) => {
                self.index += 1;
                Ok(c.to_string())
            }
        }
    }

    fn read_mml_token(&mut self, name: &str) -> Result<Token, String> {
        let kind = self.read_argument(name)?;
        while self
            .chars
            .get(self.index)
            .copied()
            .is_some_and(internal_text::space)
        {
            self.index += 1;
        }
        let raw = if self.chars.get(self.index) == Some(&'[') {
            self.padded_brackets()
        } else {
            Ok(String::new())
        }
        .map_err(|message| {
            if message == "Could not find closing ']' for argument" {
                format!("{message} to \\{name}")
            } else {
                message
            }
        })?;
        // Text is read before kind/attribute validation. It is literal,
        // including command text, and mspace discards it at zero arity.
        let text = self.read_argument(name)?;
        if !token_kind(&kind) {
            return Err(format!("{kind} is not a token element"));
        }
        let (attributes, keep) = attributes(&kind, &raw)?;
        Ok(Token {
            text: if kind == "mspace" {
                String::new()
            } else {
                text
            },
            kind,
            attributes,
            keep,
        })
    }

    pub(super) fn mml_token(&mut self, name: &str) -> Option<Atom> {
        let token = match self.read_mml_token(name) {
            Ok(token) => token,
            Err(message) => return Some(Atom::ErrorDisplay(message)),
        };
        let Token {
            kind,
            text,
            attributes,
            keep,
        } = token;
        if kind == "mspace" {
            let mut dimensions = [Dimension::relative(0.); 3];
            let mut background = None;
            for (key, value) in &attributes {
                match key.as_str() {
                    "width" => dimensions[0] = length::dimension(value.text()),
                    "height" => dimensions[1] = length::dimension(value.text()),
                    "depth" => dimensions[2] = length::dimension(value.text()),
                    "mathbackground" => background = Some(value.text().into()),
                    "linebreak" => {}
                    // The shared SVG attribute/font/style paths do not yet
                    // lower arbitrary authored token properties. Refuse those
                    // after parsing/validation, never silently discard them.
                    _ => return None,
                }
            }
            return Some(Atom::RuleSpace {
                dimensions,
                background,
                mathsize: 1.,
            });
        }
        if kind == "mo"
            && text.len() > 1
            && text.chars().all(|c| c.is_ascii_alphabetic())
            && !keep.is_empty()
        {
            let mut left = None;
            let mut right = None;
            for (key, value) in attributes {
                let target = match key.as_str() {
                    "lspace" => &mut left,
                    "rspace" => &mut right,
                    _ => return None,
                };
                *target = Some(length::dimension(value.text()));
            }
            let body = self.nonambient_font(Atom::OperatorToken(
                text.chars().map(|c| Atom::Glyph(c as u32)).collect(),
            ));
            return Some(Atom::MathMLSpace {
                body: Box::new(body),
                left,
                right,
            });
        }
        if !attributes.is_empty() || kind != "mi" {
            return None;
        }
        let single = text.chars().count() == 1;
        let token = self.nonambient_font(Atom::Identifier(
            text.chars()
                .map(|c| {
                    if single {
                        automatic_italic_atom(c)
                    } else {
                        Atom::Glyph(c as u32)
                    }
                })
                .collect(),
        ));
        Some(
            if matches!(&token, Atom::Identifier(v) if automatic_operator(v)) {
                Atom::AutoOperator(Box::new(token))
            } else {
                token
            },
        )
    }
}

#[cfg(test)]
mod contracts {
    use super::*;

    #[test]
    fn token_admission_keeps_source_error_precedence_and_attribute_grammar() {
        for (source, message) in [
            (r"\mmlToken{mrow}[bad]{x}", "mrow is not a token element"),
            (r"\mmlToken{mrow}[bad]", r"Missing argument for \mmlToken"),
            (
                r"\mmlToken{mi}[",
                r"Could not find closing ']' for argument to \mmlToken",
            ),
            (
                r"\mmlToken{mi}[}]{x}",
                "Extra close brace while looking for ']'",
            ),
            (
                r"\mmlToken{mi}[width=]{x}",
                "width is not a recognized attribute for mi",
            ),
            (
                r"\mmlToken{mspace}[Width=1em]{x}",
                "Width is not a recognized attribute for mspace",
            ),
            (
                r"\mmlToken{mspace}[width=1em,,height=1em]{x}",
                "Invalid MathML attribute: ,height=1em",
            ),
            (
                r"\mmlToken{mi}[id=ok garbage]{x}",
                "Invalid MathML attribute: garbage",
            ),
        ] {
            assert_eq!(
                render_exact(source),
                Ok(cd::render_error(message)),
                "{source}"
            );
        }
    }

    #[test]
    fn space_tokens_discard_literal_text_and_keep_signed_dimensions_and_removal() {
        for prefix in ["", r"\scriptstyle ", r"\scriptscriptstyle "] {
            for dimensions in ["1em,.4em,.2em", "-1em,-.4em,-.2em", "1pt,2px,3mu"] {
                let dimensions: Vec<_> = dimensions.split(',').collect();
                let token = format!(
                    r"\mmlToken{{mspace}}[width={} height={} depth={}]{{\undefined}}",
                    dimensions[0], dimensions[1], dimensions[2]
                );
                // MmlToken's MathML lengths retain mu units; Rule's GetDimen
                // normalizes mu first, so compare that dimension separately.
                if !dimensions[2].ends_with("mu") {
                    assert_eq!(
                        render_exact(&format!("{prefix}{token}")),
                        render_exact(&format!(
                            r"{prefix}\Space{{{}}}{{{}}}{{{}}}",
                            dimensions[0], dimensions[1], dimensions[2]
                        ))
                    );
                }
                assert_eq!(
                    render_exact(&format!(r"\scriptstyle\nonscript{token}x")),
                    render_exact(r"\scriptstyle x")
                );
            }
        }
        assert_eq!(
            render_exact(r"\mmlToken{mspace}[width=2em,width=1em,width=]{}"),
            render_exact(r"\Space{1em}{0em}{0em}")
        );
        for value in ["true", "TRUE", "False", ""] {
            assert_eq!(
                render_exact(&format!(r"\mmlToken{{mspace}}[width={value}]{{}}")),
                render_exact(r"\Space{0em}{0em}{0em}")
            );
        }
    }

    #[test]
    fn token_arguments_use_js_whitespace_control_sequence_values_and_registration() {
        for whitespace in ["\u{feff}", "\u{a0}", "\t", "\u{2028}"] {
            assert_eq!(
                render_exact(&format!(
                    "\\mmlToken{whitespace}{{mspace}}{whitespace}[width=1em]{whitespace}{{discarded}}"
                )),
                render_exact(r"\Space{1em}{0em}{0em}")
            );
        }
        assert_eq!(
            render_exact("\\mmlToken\u{85}{x}"),
            Ok(cd::render_error("\u{85} is not a token element"))
        );
        assert_eq!(
            render_exact(r"\mmlToken{mi}\x "),
            render_exact(r"\mmlToken{mi}{\x}")
        );
        assert_eq!(
            render_exact("\\mmlToken{mi}\\\n"),
            render_exact(r"\mmlToken{mi}{\ }")
        );
        assert_eq!(
            render_exact(r"\DeclareMathOperator{\mmlToken}{f}\mmlToken x"),
            render_exact(r"\DeclareMathOperator{\probe}{f}\probe x")
        );
        // Global defaults and mglyph's token defaults are accepted before
        // validation reaches the subsequent unknown attribute.
        assert_eq!(
            render_exact(r"\mmlToken{mi}[scriptlevel=1 bad=1]{x}"),
            Ok(cd::render_error("bad is not a recognized attribute for mi"))
        );
        assert_eq!(
            render_exact(r"\mmlToken{mglyph}[src=x bad=1]{y}"),
            Ok(cd::render_error(
                "bad is not a recognized attribute for mglyph"
            ))
        );
    }
}
