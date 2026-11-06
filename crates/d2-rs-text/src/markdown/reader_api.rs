// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Goldmark reader interface subset required by native inline parsers.
use super::{block_reader::BlockReader, bytes::Bytes, reader::Reader, text::Segment};
pub trait ReaderCursor {
    fn source(&self) -> Bytes;
    fn preceding_character(&self) -> char;
    fn position(&self) -> (isize, Segment);
    fn set_position(&mut self, line: isize, pos: Segment);
    fn peek_line(&mut self) -> (Option<Bytes>, Segment);
    fn advance(&mut self, n: isize);
    fn advance_line(&mut self);
    fn advance_to_eol(&mut self);
}

/// Goldmark block parsers use the same cursor over complete input and
/// quote/container source segments; inline implementers remain unchanged.
pub trait BlockCursor: ReaderCursor {
    fn line_offset(&mut self) -> isize;
    fn advance_and_set_padding(&mut self, n: isize, padding: isize);
}
macro_rules! implement_block {
    ($type:ty) => {
        impl BlockCursor for $type {
            fn line_offset(&mut self) -> isize {
                <$type>::line_offset(self)
            }
            fn advance_and_set_padding(&mut self, n: isize, padding: isize) {
                <$type>::advance_and_set_padding(self, n, padding)
            }
        }
    };
}
implement_block!(Reader);
implement_block!(BlockReader<'_>);

macro_rules! implement {
    ($type:ty) => {
        impl ReaderCursor for $type {
            fn preceding_character(&self) -> char {
                <$type>::preceding_character(self)
            }
            fn source(&self) -> Bytes {
                <$type>::source(self)
            }
            fn position(&self) -> (isize, Segment) {
                <$type>::position(self)
            }
            fn set_position(&mut self, line: isize, pos: Segment) {
                <$type>::set_position(self, line, pos);
            }
            fn peek_line(&mut self) -> (Option<Bytes>, Segment) {
                <$type>::peek_line(self)
            }
            fn advance(&mut self, n: isize) {
                <$type>::advance(self, n);
            }
            fn advance_to_eol(&mut self) {
                <$type>::advance_to_eol(self);
            }
            fn advance_line(&mut self) {
                <$type>::advance_line(self);
            }
        }
    };
}
implement!(Reader);
implement!(BlockReader<'_>);
