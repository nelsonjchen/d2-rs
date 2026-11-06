// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Public byte-input Parse boundary, matching d2parser.Parse for in-memory input.
use crate::parser::{ParseContext, ParseError, Parser, Utf8Reader};
use d2_rs_core::ast::Map;
use d2_rs_core::ast::{Child, Key, KeyPath};

#[derive(Default)]
pub struct ParseOptions<'a> {
    pub utf16_positions: bool,
    /// Existing errors are retained and new errors are appended in source order.
    pub errors: Option<&'a mut ParseError>,
}

#[derive(Debug)]
pub struct ParseOutput {
    /// Partial maps remain available when parsing reports errors.
    pub map: Map,
    pub error: Option<ParseError>,
}

/// A context cause takes precedence over accumulated syntax diagnostics.
#[derive(Debug)]
pub enum ParseContextError {
    Parse(ParseError),
    Context(d2_rs_core::LayoutError),
}
impl std::fmt::Display for ParseContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(error) => error.fmt(f),
            Self::Context(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for ParseContextError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(match self {
            Self::Parse(error) => error,
            Self::Context(error) => error,
        })
    }
}
#[derive(Debug)]
pub struct ParseContextOutput {
    pub map: Map,
    pub error: Option<ParseContextError>,
}
impl ParseContextOutput {
    pub(crate) fn without_context(self) -> ParseOutput {
        ParseOutput {
            map: self.map,
            error: self.error.map(|error| match error {
                ParseContextError::Parse(error) => error,
                ParseContextError::Context(_) => unreachable!("no parser context installed"),
            }),
        }
    }
}

#[derive(Debug)]
pub struct ParseHelperError {
    pub message: String,
    pub cause: Option<ParseError>,
}
impl std::fmt::Display for ParseHelperError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ParseHelperError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause.as_ref().map(|e| e as &dyn std::error::Error)
    }
}
fn helper<T>(
    input: &[u8],
    name: &str,
    parse: impl FnOnce(&mut Parser<Utf8Reader<'_>>) -> Option<T>,
) -> Result<T, ParseHelperError> {
    let mut parser = Parser::new(String::new(), Utf8Reader::new(input), false);
    let value = parse(&mut parser);
    if !parser.errors.is_empty() {
        return Err(ParseHelperError {
            message: format!(
                "failed to parse {name} {}: {}",
                crate::parser::quote_input(input),
                parser.errors
            ),
            cause: Some(parser.errors),
        });
    }
    value.ok_or_else(|| ParseHelperError {
        message: format!("empty {name}: {}", crate::parser::quote_input(input)),
        cause: None,
    })
}
/// Parse a key prefix, retaining upstream's trailing-input behavior.
pub fn parse_key(input: &[u8]) -> Result<KeyPath, ParseHelperError> {
    helper(input, "key", |p| p.parse_key())
}
pub fn parse_map_key(input: &[u8]) -> Result<Key, ParseHelperError> {
    helper(input, "map key", |p| p.parse_map_key())
}
pub fn parse_value(input: &[u8]) -> Result<Child, ParseHelperError> {
    helper(input, "value", |p| p.parse_value().unbox())
}

/// Parse an in-memory D2 file. A little-endian UTF-16 BOM enables decoding and
/// UTF-16 positions. UTF-8 and big-endian UTF-16 BOMs are not stripped by Go Parse.
pub fn parse(
    path: impl Into<String>,
    input: &[u8],
    options: Option<ParseOptions<'_>>,
) -> ParseOutput {
    parse_with_context(path, input, options, None).without_context()
}

/// Parse with a cooperative context, retaining the partial AST on cancellation.
pub fn parse_with_context(
    path: impl Into<String>,
    input: &[u8],
    options: Option<ParseOptions<'_>>,
    context: Option<ParseContext>,
) -> ParseContextOutput {
    let options = options.unwrap_or_default();
    let utf16_bom = input.starts_with(&[0xff, 0xfe]);
    let decoded;
    let bytes = if utf16_bom {
        decoded = decode_utf16_le(&input[2..]);
        decoded.as_bytes()
    } else {
        input
    };
    let mut parser = Parser::new(
        path.into(),
        Utf8Reader::new(bytes),
        options.utf16_positions || utf16_bom,
    );
    parser.context = context;
    if let Some(errors) = options.errors.as_deref() {
        parser.errors = errors.clone();
    }
    let map = parser.parse_map(true);
    if let Some(errors) = options.errors {
        *errors = parser.errors.clone();
    }
    let error = parser
        .context_error
        .map(ParseContextError::Context)
        .or_else(|| (!parser.errors.is_empty()).then_some(ParseContextError::Parse(parser.errors)));
    ParseContextOutput { map, error }
}

// x/text v0.41.0 unicode.utf16Decoder rules. In particular any surrogate followed
// by a low surrogate consumes both words, even when DecodeRune returns RuneError.
fn decode_utf16_le(mut bytes: &[u8]) -> String {
    let mut text = String::new();
    while bytes.len() >= 2 {
        let first = u16::from_le_bytes([bytes[0], bytes[1]]) as u32;
        let mut consumed = 2;
        let mut rune = first;
        if (0xd800..=0xdfff).contains(&first) && bytes.len() >= 4 {
            let second = u16::from_le_bytes([bytes[2], bytes[3]]) as u32;
            if (0xdc00..=0xdfff).contains(&second) {
                consumed = 4;
                rune = if first <= 0xdbff {
                    0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00)
                } else {
                    0xfffd
                };
            }
        }
        text.push(char::from_u32(rune).unwrap_or('\u{fffd}'));
        bytes = &bytes[consumed..];
    }
    if !bytes.is_empty() {
        text.push('\u{fffd}');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    use d2_rs_core::ast::Error;
    #[cfg(feature = "oracle-fixtures")]
    use serde::Deserialize;
    #[cfg(feature = "oracle-fixtures")]
    use serde_json::json;
    #[cfg(feature = "oracle-fixtures")]
    #[derive(Deserialize)]
    struct Request {
        hex: String,
        mode: String,
        utf16: bool,
        options: String,
        path: String,
    }
    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn pinned_go_public_parser_regressions() {
        let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/public-parse.json"
        ))
        .unwrap();
        let requests: Vec<Request> = serde_json::from_value(fixture["requests"].clone()).unwrap();
        let rows:Vec<_>=requests.into_iter().map(|r|{
        let bytes:Vec<u8>=r.hex.as_bytes().as_chunks::<2>().0.iter().map(|h|u8::from_str_radix(std::str::from_utf8(h).unwrap(),16).unwrap()).collect();
        if r.mode != "public_parse" {
            let value = match r.mode.as_str() {
                "public_key" => crate::parse_key(&bytes).map(|n|json!(n)),
                "public_mapkey" => crate::parse_map_key(&bytes).map(|n|json!(n)),
                "public_value" => crate::parse_value(&bytes).map(|n|json!(d2_rs_core::ast::ValueBox::from_child(Some(n)))),
                _=>panic!("unknown mode"),
            };
            return match value {
                Ok(node)=>json!({"node":node,"error":"","cause":null}),
                Err(error)=>json!({"node":null,"error":error.to_string(),"cause":error.cause}),
            };
        }
        let mut initial=match r.options.as_str(){
            "empty"=>Some(ParseError{errors:Some(Vec::new()),..Default::default()}),
            "prior"=>Some(ParseError{errors:Some(vec![Error{message:"existing diagnostic".into(),..Default::default()}]),..Default::default()}),
            _=>None,
        };
        let options=if r.options=="none"{None}else{Some(ParseOptions{utf16_positions:r.utf16,errors:initial.as_mut()})};
        let output=parse(r.path,&bytes,options);
        let display=output.error.as_ref().map_or_else(String::new,ToString::to_string);
        json!({"map":output.map,"error":display,"structured":output.error,"accumulator":initial})
    }).collect();

        for (index, (actual, expected)) in rows
            .iter()
            .zip(fixture["reference"].as_array().unwrap())
            .enumerate()
        {
            assert_eq!(
                actual, expected,
                "public parser case {index}: {}",
                fixture["requests"][index]
            );
        }
        assert_eq!(rows.len(), fixture["reference"].as_array().unwrap().len());
    }

    #[test]
    fn pinned_go_pattern_index_fault_after_substitution() {
        // Pinned parse.go:1262 still panics; d2js wrapWASMCall recovers it.
        let result = parse("pattern-fuzz.d2", b"a: x*${x}a*", None);
        let error = result.error.unwrap();
        assert_eq!(
            error.fault,
            Some(crate::parser::ParserFault::SliceBounds { start: 2, end: 1 })
        );
        assert!(error.errors.as_ref().is_none_or(Vec::is_empty));
        assert_eq!(
            error.to_string(),
            "panic recovered: runtime error: slice bounds out of range [2:1]"
        );
        assert!(parse("healthy.d2", b"a -> b", None).error.is_none());
    }

    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn pinned_upstream_testparse_corpus() {
        #[derive(Deserialize)]
        struct Corpus {
            cases: Vec<Case>,
        }
        #[derive(Deserialize)]
        struct Case {
            name: String,
            path: String,
            hex: String,
            #[serde(default)]
            utf16: bool,
            reference: serde_json::Value,
        }

        let corpus: Corpus = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/upstream-parser.json"
        ))
        .unwrap();
        for case in corpus.cases {
            let bytes: Vec<u8> = case
                .hex
                .as_bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .map(|hex| u8::from_str_radix(std::str::from_utf8(hex).unwrap(), 16).unwrap())
                .collect();
            let output = parse(
                case.path,
                &bytes,
                Some(ParseOptions {
                    utf16_positions: case.utf16,
                    errors: None,
                }),
            );
            let display = output
                .error
                .as_ref()
                .map_or_else(String::new, ToString::to_string);
            let actual = json!({
                "map": output.map,
                "error": display,
                "structured": output.error,
                "accumulator": null,
            });
            assert_eq!(
                actual, case.reference,
                "pinned upstream parser case {} (utf16={})",
                case.name, case.utf16
            );
        }
    }
}
