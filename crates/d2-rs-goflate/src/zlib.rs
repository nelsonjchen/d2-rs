// SPDX-License-Identifier: MPL-2.0
// Translated from Go 1.27.0 src/compress/zlib/writer.go.
// Copyright 2009 The Go Authors. All rights reserved. BSD-3-Clause; see
// THIRD_PARTY_NOTICES.md.

//! zlib (RFC 1950) writer wrapping the flate compressor (`zlib/writer.go`).
//! Preset dictionaries are not translated (D2 passes none).

use std::io::{self, Write};

use crate::adler32::Digest;
use crate::deflate::{self, BEST_COMPRESSION, HUFFMAN_ONLY};
use crate::huffman_bit_writer::clone_err;

/// Go `zlib.Writer`.
pub struct Writer<W: Write> {
    /// The destination until the header is written (Go `z.w`).
    w: Option<W>,
    level: i32,
    compressor: Option<deflate::Writer<W>>,
    digest: Digest,
    err: Option<io::Error>,
    wrote_header: bool,
}

impl<W: Write> Writer<W> {
    /// `NewWriter`: DefaultCompression.
    pub fn new(w: W) -> Self {
        Self::with_level(w, deflate::DEFAULT_COMPRESSION).expect("default level is valid")
    }

    /// `NewWriterLevel`.
    pub fn with_level(w: W, level: i32) -> io::Result<Self> {
        if !(HUFFMAN_ONLY..=BEST_COMPRESSION).contains(&level) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("zlib: invalid compression level: {level}"),
            ));
        }
        Ok(Self {
            w: Some(w),
            level,
            compressor: None,
            digest: Digest::new(),
            err: None,
            wrote_header: false,
        })
    }

    fn err_result(&self) -> io::Result<()> {
        match &self.err {
            Some(e) => Err(clone_err(e)),
            None => Ok(()),
        }
    }

    /// `writeHeader` writes the ZLIB header.
    fn write_header(&mut self) -> io::Result<()> {
        self.wrote_header = true;
        let mut w = self.w.take().expect("zlib writer destination");
        // ZLIB has a two-byte header (as documented in RFC 1950).
        // CINFO 7 (32 KiB window), CM 8 (deflate).
        let mut scratch = [0x78u8, 0];
        // FLEVEL: 0=fastest, 1=fast, 2=default, 3=best.
        scratch[1] = match self.level {
            -2 | 0 | 1 => 0 << 6,
            2..=5 => 1 << 6,
            6 | -1 => 2 << 6,
            7..=9 => 3 << 6,
            _ => unreachable!(),
        };
        // FDICT is never set (no dictionary). The final five FCHECK bits
        // form a mod-31 checksum.
        scratch[1] += (31 - u16::from_be_bytes(scratch) % 31) as u8;
        w.write_all(&scratch)?;
        // Go: flate.NewWriterDict(z.w, z.level, nil); a nil dictionary
        // leaves the window empty.
        self.compressor = Some(deflate::Writer::new(w, self.level)?);
        self.digest = Digest::new();
        Ok(())
    }

    /// `if !z.wroteHeader { z.err = z.writeHeader() }`.
    fn ensure_header(&mut self) {
        if !self.wrote_header {
            self.err = self.write_header().err();
        }
    }

    /// `Write` writes a compressed form of p to the underlying writer.
    pub fn write(&mut self, p: &[u8]) -> io::Result<usize> {
        self.ensure_header();
        self.err_result()?;
        if p.is_empty() {
            return Ok(0);
        }
        let c = self.compressor.as_mut().expect("compressor");
        match c.write(p) {
            Ok(n) => {
                self.digest.write(p);
                Ok(n)
            }
            Err(e) => {
                let r = clone_err(&e);
                self.err = Some(e);
                Err(r)
            }
        }
    }

    /// `Flush` flushes the Writer to its underlying writer.
    pub fn flush(&mut self) -> io::Result<()> {
        self.ensure_header();
        self.err_result()?;
        if let Err(e) = self.compressor.as_mut().expect("compressor").flush() {
            self.err = Some(e);
        }
        self.err_result()
    }

    /// `Close` closes the Writer, flushing any unwritten data to the
    /// underlying writer, and appends the big-endian Adler-32 trailer.
    pub fn close(&mut self) -> io::Result<()> {
        self.ensure_header();
        self.err_result()?;
        let c = self.compressor.as_mut().expect("compressor");
        if let Err(e) = c.close() {
            self.err = Some(e);
            return self.err_result();
        }
        let checksum = self.digest.sum32();
        // ZLIB (RFC 1950) is big-endian, unlike GZIP (RFC 1952).
        let w = c.get_mut();
        if let Err(e) = w.write_all(&checksum.to_be_bytes()) {
            self.err = Some(e);
        }
        self.err_result()
    }

    /// Consume the writer, returning the underlying writer.
    pub fn into_inner(self) -> W {
        match self.compressor {
            Some(c) => c.into_inner(),
            None => self.w.expect("zlib writer destination"),
        }
    }
}
