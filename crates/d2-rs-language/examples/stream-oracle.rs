// SPDX-License-Identifier: MPL-2.0
use d2_rs_language::{
    ParseOptions, parse_stream,
    parser::ParseError,
    stream::{ByteReader, ReadError},
};
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Default, Deserialize)]
#[serde(default)]
pub struct Request {
    load: bool,
    hex: String,
    chunk: usize,
    error_at: Option<usize>,
    combined: bool,
    zero_reads: usize,
    utf16: bool,
    seed: bool,
}
struct FaultReader {
    data: Vec<u8>,
    position: usize,
    ended: bool,
    chunk: usize,
    error_at: Option<usize>,
    combined: bool,
    zero_reads: usize,
}
impl ByteReader for FaultReader {
    fn read_bytes(&mut self, dst: &mut [u8]) -> (usize, Option<ReadError>) {
        if self.zero_reads > 0 {
            self.zero_reads -= 1;
            return (0, None);
        }
        if self.ended {
            return (0, Some(ReadError::Eof));
        }
        if self.error_at == Some(self.position) {
            self.ended = true;
            return (0, Some(ReadError::Other("read fault".into())));
        }
        if self.position == self.data.len() {
            self.ended = true;
            return (0, Some(ReadError::Eof));
        }
        let mut n = (self.data.len() - self.position).min(dst.len());
        if self.chunk > 0 {
            n = n.min(self.chunk)
        }
        if let Some(cutoff) = self.error_at {
            n = n.min(cutoff - self.position)
        }
        dst[..n].copy_from_slice(&self.data[self.position..self.position + n]);
        self.position += n;
        if self.combined {
            if self.error_at == Some(self.position) {
                self.ended = true;
                return (n, Some(ReadError::Other("read fault".into())));
            }
            if self.position == self.data.len() {
                self.ended = true;
                return (n, Some(ReadError::Eof));
            }
        }
        (n, None)
    }
}
pub fn evaluate(r: Request) -> Value {
    let bytes = r
        .hex
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect();
    let mut reader = FaultReader {
        data: bytes,
        position: 0,
        ended: false,
        chunk: r.chunk,
        error_at: r.error_at,
        combined: r.combined,
        zero_reads: r.zero_reads,
    };
    let mut accumulator = ParseError::default();
    if r.seed {
        accumulator.errors = Some(vec![d2_rs_core::ast::Error {
            message: "existing error".into(),
            ..Default::default()
        }]);
    }
    if r.load {
        let mut state = d2_rs_language::compiler::imports::ImportState {
            utf16_positions: r.utf16,
            ..Default::default()
        };
        let mut reader = Some(reader);
        let (working, open_error) = state.load_stream("stream.d2", &mut accumulator, |_| {
            Ok(Box::new(reader.take().unwrap()))
        });
        return json!({"ok":working.is_some(),"working":working,"open_error":open_error,"accumulator":accumulator,"cached":!state.parsed.is_empty()});
    }
    let output = parse_stream(
        "stream.d2",
        &mut reader,
        Some(ParseOptions {
            utf16_positions: r.utf16,
            errors: Some(&mut accumulator),
        }),
    );
    json!({"map":output.map,"error":output.error.as_ref().map_or_else(String::new,ToString::to_string),"structured":output.error,"accumulator":accumulator})
}
fn main() {
    let requests: Vec<Request> = serde_json::from_reader(std::io::stdin()).unwrap();
    serde_json::to_writer(
        std::io::stdout(),
        &requests.into_iter().map(evaluate).collect::<Vec<_>>(),
    )
    .unwrap();
}
