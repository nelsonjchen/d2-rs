// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
// Copyright 2009 The Go Authors. All rights reserved.
// Go BSD-3-Clause conditions and disclaimer: THIRD_PARTY_NOTICES.md.
//! D2 rasterimage JPEG DecodeFirst/contextReader and Go bufio transport.
use crate::{
    LayoutError,
    jpeg_decode::{self, Image},
    jpeg_entropy::{Error, Input},
};
/// Native equivalent of bytes.Reader; EOF accompanies only an empty read.
pub struct Bytes<'a> {
    pub data: &'a [u8],
    pub offset: usize,
}
impl Input for Bytes<'_> {
    fn read(&mut self, dst: &mut [u8]) -> (usize, Option<Error>) {
        let n = dst.len().min(self.data.len() - self.offset);
        dst[..n].copy_from_slice(&self.data[self.offset..self.offset + n]);
        self.offset += n;
        (n, if n == 0 { Some(Error::Eof) } else { None })
    }
}
struct ContextInput<'a> {
    input: &'a mut dyn Input,
    poll: &'a dyn Fn() -> Result<(), LayoutError>,
}
fn cause(error: LayoutError) -> Error {
    if matches!(error, LayoutError::DeadlineExceeded) {
        Error::Deadline
    } else {
        Error::Cancelled
    }
}
impl Input for ContextInput<'_> {
    fn read(&mut self, dst: &mut [u8]) -> (usize, Option<Error>) {
        if let Err(e) = (self.poll)() {
            return (0, Some(cause(e)));
        }
        let len = dst.len().min(32 << 10);
        let (n, error) = self.input.read(&mut dst[..len]);
        if let Err(e) = (self.poll)() {
            return (n, Some(cause(e)));
        }
        (n, error)
    }
}
struct Buffered<'a> {
    input: ContextInput<'a>,
    bytes: [u8; 4096],
    start: usize,
    end: usize,
    error: Option<Error>,
}
impl Buffered<'_> {
    fn fill(&mut self) {
        if self.start > 0 {
            self.bytes.copy_within(self.start..self.end, 0);
            self.end -= self.start;
            self.start = 0;
        }
        assert!(self.end < 4096);
        for _ in 0..100 {
            let (n, error) = self.input.read(&mut self.bytes[self.end..]);
            self.end += n;
            if error.is_some() {
                self.error = error;
                return;
            }
            if n > 0 {
                return;
            }
        }
    }
    fn peek(&mut self, n: usize) -> Option<&[u8]> {
        while self.end - self.start < n && self.end - self.start < 4096 && self.error.is_none() {
            self.fill();
        }
        if self.end - self.start < n {
            self.error.take();
            None
        } else {
            Some(&self.bytes[self.start..self.start + n])
        }
    }
}
impl Input for Buffered<'_> {
    fn read(&mut self, dst: &mut [u8]) -> (usize, Option<Error>) {
        if dst.is_empty() {
            return (
                0,
                if self.end > self.start {
                    None
                } else {
                    self.error.take()
                },
            );
        }
        if self.start == self.end {
            if self.error.is_some() {
                return (0, self.error.take());
            }
            if dst.len() >= 4096 {
                let (n, error) = self.input.read(dst);
                return (n, error);
            }
            self.start = 0;
            self.end = 0;
            let (n, error) = self.input.read(&mut self.bytes);
            self.error = error;
            if n == 0 {
                return (0, self.error.take());
            }
            self.end = n;
        }
        let n = dst.len().min(self.end - self.start);
        dst[..n].copy_from_slice(&self.bytes[self.start..self.start + n]);
        self.start += n;
        (n, None)
    }
}
/// Source JPEG first-image decode. `data` is metadata input; `input` is the
/// functional encoded-byte reader, normally Bytes over that same immutable data.
pub fn decode(
    data: &[u8],
    input: &mut dyn Input,
    poll: &dyn Fn() -> Result<(), LayoutError>,
) -> Result<Image, String> {
    poll().map_err(|e| e.to_string())?;
    let orientation =
        crate::raster_orientation::jpeg_with_poll(data, poll).map_err(|e| e.to_string())?;
    let mut buffered = Buffered {
        input: ContextInput { input, poll },
        bytes: [0; 4096],
        start: 0,
        end: 0,
        error: None,
    };
    // Pinned standard formats are registered GIF, JPEG, PNG, WebP. Preserve
    // preceding GIF lookahead for short JPEG streams before selecting JPEG.
    buffered.peek(6);
    let jpeg = buffered.peek(2).is_some_and(|bytes| bytes == [255, 216]);
    let result = if jpeg {
        jpeg_decode::decode_input(&mut buffered)
    } else {
        buffered.peek(8);
        buffered.peek(15);
        Err("image: unknown format".to_owned())
    };
    let image = match result {
        Ok(image) => image,
        Err(error) => return Err(poll().err().map_or(error, |e| e.to_string())),
    };
    poll().map_err(|e| e.to_string())?;
    if orientation <= 1 {
        return Ok(image);
    }
    let (width, height) = if orientation >= 5 {
        (image.height, image.width)
    } else {
        (image.width, image.height)
    };
    let mut rgba = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let (sx, sy) = match orientation {
                2 => (image.width - 1 - x, y),
                3 => (image.width - 1 - x, image.height - 1 - y),
                4 => (x, image.height - 1 - y),
                5 => (y, x),
                6 => (y, image.height - 1 - x),
                7 => (image.width - 1 - y, image.height - 1 - x),
                8 => (image.width - 1 - y, x),
                _ => (x, y),
            };
            rgba.push(image.rgba[sy * image.width + sx]);
        }
    }
    Ok(Image {
        width,
        height,
        rgba,
    })
}
