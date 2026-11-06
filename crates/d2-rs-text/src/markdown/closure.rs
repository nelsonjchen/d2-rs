// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Goldmark v1.8.5 text/reader.go findClosureReader. MIT terms in notices.
use super::reader_api::ReaderCursor;
use super::{block_reader::BlockReader, reader::Reader, text::Segment};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct FindClosureOptions {
    pub code_span: bool,
    pub nesting: bool,
    pub newline: bool,
    pub advance: bool,
}

macro_rules! implement {
    ($type:ty) => {
        impl $type {
            pub fn find_closure(
                &mut self,
                opener: u8,
                closer: u8,
                options: FindClosureOptions,
            ) -> Option<Vec<Segment>> {
                find_closure(self, opener, closer, options)
            }
        }
    };
}
implement!(Reader);
implement!(BlockReader<'_>);

fn find_closure(
    r: &mut impl ReaderCursor,
    opener: u8,
    closer: u8,
    options: FindClosureOptions,
) -> Option<Vec<Segment>> {
    let mut opened = 1;
    let mut code_span_opener = 0;
    let original = r.position();
    let mut segments = Vec::new();
    let mut closed = false;
    'search: loop {
        let (bytes, seg) = r.peek_line();
        let Some(bytes) = bytes else {
            break;
        };
        let bytes = bytes.bytes();
        let mut i = 0;
        while i < bytes.len() {
            let byte = bytes[i];
            if options.code_span && code_span_opener != 0 && byte == b'`' {
                let start = i;
                while i < bytes.len() && bytes[i] == b'`' {
                    i += 1;
                }
                if i - start == code_span_opener {
                    code_span_opener = 0;
                }
                continue;
            } else if code_span_opener == 0
                && byte == b'\\'
                && i + 1 < bytes.len()
                && bytes[i + 1].is_ascii_punctuation()
            {
                i += 2;
                continue;
            } else if options.code_span && code_span_opener == 0 && byte == b'`' {
                let start = i;
                while i < bytes.len() && bytes[i] == b'`' {
                    i += 1;
                }
                code_span_opener = i - start;
                continue;
            } else if code_span_opener == 0 || !options.code_span {
                if byte == closer {
                    opened -= 1;
                    if opened == 0 {
                        segments.push(seg.with_stop(seg.start + i as isize - seg.padding));
                        r.advance(i as isize + 1);
                        closed = true;
                        break 'search;
                    }
                } else if byte == opener {
                    if !options.nesting {
                        break 'search;
                    }
                    opened += 1;
                }
            }
            i += 1;
        }
        if !options.newline {
            break;
        }
        r.advance_line();
        segments.push(seg);
    }
    if !options.advance {
        r.set_position(original.0, original.1);
    }
    closed.then_some(segments)
}
