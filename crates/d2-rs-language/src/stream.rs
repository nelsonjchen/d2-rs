// SPDX-License-Identifier: MPL-2.0
//! Streaming d2parser.Parse boundary, including bufio/BOM error behavior.
use crate::{
    ParseOptions, ParseOutput,
    parser::{Parser, RuneReader, decode_rune},
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    Eof,
    Other(String),
}
/// Go-compatible byte-reader outcome. Unlike std::io::Read, a provider may
/// return bytes together with an error. Native Rust readers work directly via
/// the blanket implementation; virtual providers may implement this interface.
pub trait ByteReader {
    fn read_bytes(&mut self, destination: &mut [u8]) -> (usize, Option<ReadError>);
}
impl<R: std::io::Read> ByteReader for R {
    fn read_bytes(&mut self, destination: &mut [u8]) -> (usize, Option<ReadError>) {
        match self.read(destination) {
            Ok(0) => (0, Some(ReadError::Eof)),
            Ok(n) => (n, None),
            Err(e) => (0, Some(ReadError::Other(e.to_string()))),
        }
    }
}
struct Buffered<'a> {
    reader: &'a mut dyn ByteReader,
    bytes: [u8; 4096],
    start: usize,
    end: usize,
    error: Option<ReadError>,
    empty_limit: bool,
}
impl Buffered<'_> {
    fn available(&self) -> &[u8] {
        &self.bytes[self.start..self.end]
    }
    fn fill(&mut self) {
        if self.start > 0 {
            self.bytes.copy_within(self.start..self.end, 0);
            self.end -= self.start;
            self.start = 0;
        }
        assert!(self.end < self.bytes.len(), "full reader buffer");
        let mut empty_reads = 0;
        loop {
            let (n, error) = self.reader.read_bytes(&mut self.bytes[self.end..]);
            assert!(n <= self.bytes.len() - self.end, "invalid reader count");
            self.end += n;
            if error.is_some() {
                self.error = error;
                return;
            }
            if n > 0 {
                return;
            }
            empty_reads += 1;
            if self.empty_limit && empty_reads == 100 {
                self.error = Some(ReadError::Other(
                    "multiple Read calls return no data or error".into(),
                ));
                return;
            }
        }
    }
    fn ensure(&mut self, n: usize) {
        while self.available().len() < n && self.error.is_none() {
            self.fill()
        }
    }
    fn finish(&mut self) -> Result<Option<char>, String> {
        match self.error.take().unwrap_or(ReadError::Eof) {
            ReadError::Eof => Ok(None),
            ReadError::Other(e) => Err(e),
        }
    }
}
struct StreamRunes<'a> {
    buffer: Buffered<'a>,
    utf16: bool,
}
impl<'a> StreamRunes<'a> {
    fn new(reader: &'a mut dyn ByteReader) -> Self {
        let mut buffer = Buffered {
            reader,
            bytes: [0; 4096],
            start: 0,
            end: 0,
            error: None,
            empty_limit: true,
        };
        buffer.ensure(2);
        // A short bufio.Peek returns readErr(), which clears the stored error.
        if buffer.available().len() < 2 {
            buffer.error = None;
        }
        let utf16 = buffer.available().starts_with(&[0xff, 0xfe]);
        if utf16 {
            buffer.start += 2;
            // Go copies br.Buffered() into a MultiReader and resets br. Its
            // pending error is discarded, even when returned with BOM bytes.
            buffer.error = None;
            // transform.Reader retries empty reads internally without bufio's
            // 100-read limit. Keep this distinction after the BOM switch.
            buffer.empty_limit = false;
        }
        Self { buffer, utf16 }
    }
}
fn full_rune(bytes: &[u8]) -> bool {
    let Some(&first) = bytes.first() else {
        return false;
    };
    let width = match first {
        0..=127 => 1,
        194..=223 => 2,
        224..=239 => 3,
        240..=244 => 4,
        _ => return true,
    };
    if bytes.len() >= width {
        return true;
    }
    for (i, &byte) in bytes.iter().enumerate().skip(1) {
        let (min, max) = match (i, first) {
            (1, 224) => (160, 191),
            (1, 237) => (128, 159),
            (1, 240) => (144, 191),
            (1, 244) => (128, 143),
            _ => (128, 191),
        };
        if byte < min || byte > max {
            return true;
        }
    }
    false
}
impl RuneReader for StreamRunes<'_> {
    fn read_rune(&mut self) -> Result<Option<char>, String> {
        if !self.utf16 {
            while !full_rune(self.buffer.available()) && self.buffer.error.is_none() {
                self.buffer.fill()
            }
            if self.buffer.available().is_empty() {
                return self.buffer.finish();
            }
            let (rune, width) = decode_rune(self.buffer.available());
            self.buffer.start += width;
            return Ok(Some(rune));
        }
        self.buffer.ensure(2);
        let available = self.buffer.available().len();
        if available == 0 {
            return self.buffer.finish();
        }
        if available < 2 {
            if matches!(self.buffer.error, Some(ReadError::Other(_))) {
                return self.buffer.finish();
            }
            self.buffer.start += 1;
            return Ok(Some('\u{fffd}'));
        }
        let first = u16::from_le_bytes(self.buffer.available()[..2].try_into().unwrap());
        let mut consumed = 2;
        let mut rune = u32::from(first);
        if (0xd800..=0xdfff).contains(&first) {
            self.buffer.ensure(4);
            if self.buffer.available().len() >= 4 {
                let second = u16::from_le_bytes(self.buffer.available()[2..4].try_into().unwrap());
                if (0xdc00..=0xdfff).contains(&second) {
                    consumed = 4;
                    rune = if first <= 0xdbff {
                        0x10000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00)
                    } else {
                        0xfffd
                    };
                }
            } else if matches!(self.buffer.error, Some(ReadError::Other(_))) {
                return self.buffer.finish();
            }
        }
        self.buffer.start += consumed;
        Ok(Some(char::from_u32(rune).unwrap_or('\u{fffd}')))
    }
}
/// Parse a native or virtual byte stream, preserving partial maps and ordered
/// read errors. The caller owns and closes the reader, as with d2parser.Parse.
pub fn parse_stream(
    path: impl Into<String>,
    reader: &mut dyn ByteReader,
    options: Option<ParseOptions<'_>>,
) -> ParseOutput {
    parse_stream_with_context(path, reader, options, None).without_context()
}

/// Streaming ParseContext; preserves the same BOM/read behavior as Parse.
pub fn parse_stream_with_context(
    path: impl Into<String>,
    reader: &mut dyn ByteReader,
    options: Option<ParseOptions<'_>>,
    context: Option<crate::parser::ParseContext>,
) -> crate::ParseContextOutput {
    let options = options.unwrap_or_default();
    let runes = StreamRunes::new(reader);
    let utf16 = options.utf16_positions || runes.utf16;
    let mut parser = Parser::new(path.into(), runes, utf16);
    parser.context = context;
    if let Some(errors) = options.errors.as_deref() {
        parser.errors = errors.clone()
    }
    let map = parser.parse_map(true);
    if let Some(errors) = options.errors {
        *errors = parser.errors.clone()
    }
    let error = parser
        .context_error
        .map(crate::ParseContextError::Context)
        .or_else(|| {
            (!parser.errors.is_empty()).then_some(crate::ParseContextError::Parse(parser.errors))
        });
    crate::ParseContextOutput { map, error }
}

#[cfg(any(
    target_os = "macos",
    all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )
))]
mod native;
#[cfg(any(
    target_os = "macos",
    all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )
))]
mod os_errors;
#[cfg(any(
    target_os = "macos",
    all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )
))]
pub use native::open_native;
