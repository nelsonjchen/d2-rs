// SPDX-License-Identifier: BSD-3-Clause
// Copyright 2016 The Go Authors. All rights reserved.
// ReaderAt-backed prefix of x/image/font/sfnt/postscript.go's cffParser.
use super::{BoundedReader, Point26_6, ProbeTables, Segment, SegmentOp, Segments};
use std::io::{self, Read, Seek};

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "sfnt: invalid CFF table")
}

struct Parser<'a, R> {
    source: &'a mut BoundedReader<R>,
    base: i64,
    offset: i64,
    end: i64,
}

// Retain the offsets that x/image's glyphData uses when executing a glyph.
// The bounded probe will use these for the same charged subroutine reads.
#[allow(dead_code)]
#[derive(Debug)]
pub(super) struct GlyphData {
    pub(super) locations: Vec<i64>,
    global_subrs: Vec<i64>,
    single_subrs: Vec<i64>,
    multi_subrs: Option<Vec<Vec<i64>>>,
    fd_select: Option<FdSelect>,
}

#[allow(dead_code)]
#[derive(Debug)]
struct FdSelect {
    format: u8,
    num_ranges: u16,
    offset: i64,
}

// Instruction ranges are re-read on return, just as t2CReturn does. Keeping
// ranges instead of buffers also accounts for each read against the scan cap.
#[allow(dead_code)]
struct Type2Instructions {
    bytes: Vec<u8>,
    offset: i64,
    position: usize,
    calls: Vec<(i64, i64)>,
}

#[allow(dead_code)]
impl Type2Instructions {
    // psInterpreter.parseNumber in the Type 2 context; None means the next
    // byte is an operator and leaves it for the interpreter.
    fn number(&mut self) -> io::Result<Option<i32>> {
        let rest = self.bytes.get(self.position..).ok_or_else(invalid)?;
        let Some(&first) = rest.first() else {
            return Ok(None);
        };
        let (value, length) = match first {
            28 => (
                i32::from(i16::from_be_bytes(
                    rest.get(1..3).ok_or_else(invalid)?.try_into().unwrap(),
                )),
                3,
            ),
            32..=246 => (i32::from(first) - 139, 1),
            247..=250 => (
                (i32::from(first) - 247) * 256 + i32::from(*rest.get(1).ok_or_else(invalid)?) + 108,
                2,
            ),
            251..=254 => (
                -(i32::from(first) - 251) * 256
                    - i32::from(*rest.get(1).ok_or_else(invalid)?)
                    - 108,
                2,
            ),
            255 => {
                let fixed =
                    i32::from_be_bytes(rest.get(1..5).ok_or_else(invalid)?.try_into().unwrap());
                ((fixed >> 16) + (1 & (fixed >> 15)), 5)
            }
            _ => return Ok(None),
        };
        self.position += length;
        Ok(Some(value))
    }

    fn call<R: Read + Seek>(
        &mut self,
        source: &mut BoundedReader<R>,
        subrs: &[i64],
        operand: i32,
    ) -> io::Result<()> {
        if self.calls.len() == 10 || subrs.is_empty() {
            return Err(invalid());
        }
        let count = subrs.len() - 1;
        let bias = if count < 1240 {
            107
        } else if count < 33900 {
            1131
        } else {
            32768
        };
        let index = operand.wrapping_add(bias);
        let index = usize::try_from(index).map_err(|_| invalid())?;
        if index >= count {
            return Err(invalid());
        }
        let start = subrs[index];
        let length = subrs[index + 1]
            .checked_sub(start)
            .filter(|&n| (0..=65536).contains(&n))
            .ok_or_else(invalid)?;
        let remaining = self
            .bytes
            .len()
            .checked_sub(self.position)
            .ok_or_else(invalid)?;
        self.calls
            .push((self.offset + self.position as i64, remaining as i64));
        self.bytes = source.view(start, length)?;
        self.offset = start;
        self.position = 0;
        Ok(())
    }

    fn return_from_call<R: Read + Seek>(
        &mut self,
        source: &mut BoundedReader<R>,
    ) -> io::Result<()> {
        let (offset, length) = self.calls.pop().ok_or_else(invalid)?;
        self.bytes = source.view(offset, length)?;
        self.offset = offset;
        self.position = 0;
        Ok(())
    }
}

// The path state follows psType2CharstringsData in x/image's postscript.go.
#[derive(Default)]
struct Outline {
    x: i32,
    y: i32,
    first_x: i32,
    first_y: i32,
    segments: Segments,
}

impl Outline {
    fn point(&self) -> Point26_6 {
        Point26_6 {
            x: self.x,
            y: self.y,
        }
    }
    fn advance(&mut self, x: i32, y: i32) {
        self.x = self.x.wrapping_add(x);
        self.y = self.y.wrapping_add(y);
    }
    fn close(&mut self) {
        if self.x != self.first_x || self.y != self.first_y {
            self.segments.push(Segment {
                op: SegmentOp::LineTo,
                args: [
                    Point26_6 {
                        x: self.first_x,
                        y: self.first_y,
                    },
                    Point26_6::default(),
                    Point26_6::default(),
                ],
            });
        }
    }
    fn moveto(&mut self, x: i32, y: i32) {
        self.close();
        self.advance(x, y);
        self.segments.push(Segment {
            op: SegmentOp::MoveTo,
            args: [self.point(), Point26_6::default(), Point26_6::default()],
        });
        self.first_x = self.x;
        self.first_y = self.y;
    }
    fn lineto(&mut self, x: i32, y: i32) {
        self.advance(x, y);
        self.segments.push(Segment {
            op: SegmentOp::LineTo,
            args: [self.point(), Point26_6::default(), Point26_6::default()],
        });
    }
    fn cube(&mut self, a: [i32; 6]) {
        self.advance(a[0], a[1]);
        let p = self.point();
        self.advance(a[2], a[3]);
        let q = self.point();
        self.advance(a[4], a[5]);
        self.segments.push(Segment {
            op: SegmentOp::CubeTo,
            args: [p, q, self.point()],
        });
    }
}

// Validate the Type 2 program as x/image's LoadGlyph does. The probe needs
// its reads and errors, not the resulting outline coordinates.
#[allow(dead_code)]
pub(super) fn probe_type2<R: Read + Seek>(
    source: &mut BoundedReader<R>,
    data: &GlyphData,
    glyph: u16,
) -> io::Result<()> {
    type2_outline(source, data, glyph).map(|_| ())
}

#[allow(dead_code)]
pub(super) fn type2_outline<R: Read + Seek>(
    source: &mut BoundedReader<R>,
    data: &GlyphData,
    glyph: u16,
) -> io::Result<Segments> {
    let index = usize::from(glyph);
    let (&start, &end) = data
        .locations
        .get(index)
        .zip(data.locations.get(index + 1))
        .ok_or_else(invalid)?;
    let length = end
        .checked_sub(start)
        .filter(|&n| (0..=65536).contains(&n))
        .ok_or_else(invalid)?;
    let mut program = Type2Instructions {
        bytes: source.view(start, length)?,
        offset: start,
        position: 0,
        calls: Vec::new(),
    };
    let mut stack: Vec<i32> = Vec::new();
    let mut outline = Outline::default();
    let mut seen_width = false;
    let mut hints = 0usize;
    let mut ended = false;
    let mut selected_fd = None;
    while program.position < program.bytes.len() {
        if let Some(value) = program.number()? {
            if stack.len() == 48 {
                return Err(invalid());
            }
            stack.push(value);
            continue;
        }
        let op = program.bytes[program.position];
        program.position += 1;
        if op == 10 || op == 29 {
            let operand = stack.pop().ok_or_else(invalid)?;
            let subrs = if op == 29 {
                &data.global_subrs
            } else if let Some(multi) = &data.multi_subrs {
                let fd = match selected_fd {
                    Some(fd) => fd,
                    None => {
                        let fd = data
                            .fd_select
                            .as_ref()
                            .ok_or_else(invalid)?
                            .lookup(source, glyph)?;
                        selected_fd = Some(fd);
                        fd
                    }
                };
                multi.get(fd).ok_or_else(invalid)?
            } else {
                &data.single_subrs
            };
            program.call(source, subrs, operand)?;
            continue;
        }
        if op == 11 {
            program.return_from_call(source)?;
            continue;
        }
        if op == 12 {
            let escaped = *program.bytes.get(program.position).ok_or_else(invalid)?;
            program.position += 1;
            let count = match escaped {
                34 => 7,
                36 => 9,
                _ => return Err(invalid()),
            };
            if stack.len() < count {
                return Err(invalid());
            }
            // Go's flex operators read from the bottom of the argument stack.
            if escaped == 34 {
                outline.cube([stack[0], 0, stack[1], stack[2], stack[3], 0]);
                outline.cube([stack[4], 0, stack[5], stack[2].wrapping_neg(), stack[6], 0]);
            } else {
                outline.cube([stack[0], stack[1], stack[2], stack[3], stack[4], 0]);
                let dy = stack[1]
                    .wrapping_neg()
                    .wrapping_sub(stack[3])
                    .wrapping_sub(stack[7]);
                outline.cube([stack[5], 0, stack[6], stack[7], stack[8], dy]);
            }
            stack.truncate(stack.len() - count);
            continue;
        }
        let n = stack.len();
        if !seen_width && matches!(op, 1 | 3 | 4 | 14 | 18..=23) {
            let expected = match op {
                4 | 22 => Some(1),
                21 => Some(2),
                14 => Some(0),
                _ => None,
            };
            if expected.map_or(n % 2 == 1, |v| n == v + 1) {
                stack.remove(0);
            }
            seen_width = true;
        }
        let n = stack.len();
        match op {
            1 | 3 | 18 | 23 => {
                if !n.is_multiple_of(2) {
                    return Err(invalid());
                }
                hints += n / 2;
                if hints > 256 {
                    return Err(invalid());
                }
            }
            19 | 20 => {
                if !n.is_multiple_of(2) {
                    return Err(invalid());
                }
                hints += n / 2;
                if hints > 256 {
                    return Err(invalid());
                }
                program.position = program
                    .position
                    .checked_add(hints.div_ceil(8))
                    .filter(|&p| p <= program.bytes.len())
                    .ok_or_else(invalid)?;
            }
            4 | 22 if n == 1 => (),
            21 if n == 2 => (),
            5 if seen_width && n >= 2 && n.is_multiple_of(2) => (),
            6 | 7 if seen_width && n >= 1 => (),
            8 if seen_width && n >= 6 && n.is_multiple_of(6) => (),
            24 if seen_width && n >= 8 && n % 6 == 2 => (),
            25 if seen_width && n >= 8 && n.is_multiple_of(2) => (),
            26 | 27 | 30 | 31 if seen_width && n >= 4 && n % 4 <= 1 => (),
            14 if n == 0
                && program.position == program.bytes.len()
                && program.calls.iter().all(|&(_, len)| len == 0) =>
            {
                ended = true
            }
            _ => return Err(invalid()),
        }
        match op {
            4 => outline.moveto(0, stack[0]),
            21 => outline.moveto(stack[0], stack[1]),
            22 => outline.moveto(stack[0], 0),
            5 => {
                for pair in stack.chunks_exact(2) {
                    outline.lineto(pair[0], pair[1]);
                }
            }
            6 | 7 => {
                for (i, &v) in stack.iter().enumerate() {
                    if (i % 2 == 0) == (op == 6) {
                        outline.lineto(v, 0);
                    } else {
                        outline.lineto(0, v);
                    }
                }
            }
            8 => {
                for curve in stack.chunks_exact(6) {
                    outline.cube(curve.try_into().unwrap());
                }
            }
            24 => {
                let end = n - 2;
                for curve in stack[..end].chunks_exact(6) {
                    outline.cube(curve.try_into().unwrap());
                }
                outline.lineto(stack[end], stack[end + 1]);
            }
            25 => {
                let end = n - 6;
                for pair in stack[..end].chunks_exact(2) {
                    outline.lineto(pair[0], pair[1]);
                }
                outline.cube(stack[end..].try_into().unwrap());
            }
            26 | 27 | 30 | 31 => {
                let swap = op == 30 || op == 31;
                let mut vertical = op == 26 || op == 30;
                let mut i = 0;
                if n % 4 == 1 && !swap {
                    if vertical {
                        outline.x = outline.x.wrapping_add(stack[0]);
                    } else {
                        outline.y = outline.y.wrapping_add(stack[0]);
                    }
                    i = 1;
                }
                while i < n {
                    let (mut xa, mut ya) = (stack[i], 0);
                    let (xb, yb) = (stack[i + 1], stack[i + 2]);
                    let (mut xc, mut yc) = (stack[i + 3], 0);
                    i += 4;
                    if vertical {
                        std::mem::swap(&mut xa, &mut ya);
                    }
                    if swap && i + 1 == n {
                        yc = stack[i];
                        i += 1;
                    }
                    if swap != vertical {
                        std::mem::swap(&mut xc, &mut yc);
                    }
                    outline.cube([xa, ya, xb, yb, xc, yc]);
                    if swap {
                        vertical = !vertical;
                    }
                }
            }
            14 => outline.close(),
            _ => (),
        }
        stack.clear();
    }
    if ended {
        Ok(outline.segments)
    } else {
        Err(invalid())
    }
}

#[allow(dead_code)]
impl FdSelect {
    // x/image's fdSelect.lookup reads each binary-search candidate separately.
    fn lookup<R: Read + Seek>(
        &self,
        source: &mut BoundedReader<R>,
        glyph: u16,
    ) -> io::Result<usize> {
        match self.format {
            0 => Ok(usize::from(
                source.view(self.offset + i64::from(glyph), 1)?[0],
            )),
            3 => {
                let (mut lo, mut hi) = (0, i64::from(self.num_ranges));
                while lo < hi {
                    let i = (lo + hi) / 2;
                    let data = source.view(self.offset + 3 * i, 5)?;
                    let start = u16::from_be_bytes([data[0], data[1]]);
                    let end = u16::from_be_bytes([data[3], data[4]]);
                    if glyph < start {
                        hi = i;
                    } else if glyph >= end {
                        lo = i + 1;
                    } else {
                        return Ok(usize::from(data[2]));
                    }
                }
                Err(io::Error::new(io::ErrorKind::NotFound, "sfnt: not found"))
            }
            _ => Err(io::Error::new(io::ErrorKind::NotFound, "sfnt: not found")),
        }
    }
}

impl<R: Read + Seek> Parser<'_, R> {
    fn seek_from_base(&mut self, offset: i32) -> io::Result<()> {
        let offset = i64::from(offset);
        if offset < 0 || offset > self.end - self.base {
            return Err(invalid());
        }
        self.offset = self.base + offset;
        Ok(())
    }

    fn private_dict(&mut self, offset: i32, length: i32) -> io::Result<Vec<i64>> {
        let mut subrs = 0;
        if length != 0 {
            if offset <= 0
                || length < 0
                || i64::from(offset) + i64::from(length) > self.end - self.base
            {
                return Err(invalid());
            }
            self.seek_from_base(offset)?;
            subrs = dict(&self.read(i64::from(length))?, true)?.subrs;
        }
        if subrs != 0 {
            self.seek_from_base(offset.wrapping_add(subrs))?;
            let (count, size) = self.index_header()?;
            if count > 40_000 {
                return Err(invalid());
            }
            return self.index_locations(count, size);
        }
        Ok(Vec::new())
    }

    fn read(&mut self, n: i64) -> io::Result<Vec<u8>> {
        let end = self
            .offset
            .checked_add(n)
            .filter(|&end| n >= 0 && end <= self.end)
            .ok_or_else(invalid)?;
        let bytes = self.source.view(self.offset, n)?;
        self.offset = end;
        Ok(bytes)
    }

    fn index_header(&mut self) -> io::Result<(usize, usize)> {
        let b = self.read(2)?;
        let count = usize::from(u16::from_be_bytes([b[0], b[1]]));
        if count == 0 {
            return Ok((0, 0));
        }
        let size = usize::from(self.read(1)?[0]);
        if !(1..=4).contains(&size) {
            return Err(invalid());
        }
        Ok((count, size))
    }

    fn index_locations(&mut self, count: usize, size: usize) -> io::Result<Vec<i64>> {
        if count == 0 {
            return Ok(Vec::new());
        }
        let bytes = self.read(((count + 1) * size) as i64)?;
        let mut result = Vec::with_capacity(count + 1);
        let mut previous = 0;
        for (i, chunk) in bytes.chunks_exact(size).enumerate() {
            let loc = chunk.iter().fold(0u32, |n, b| (n << 8) | u32::from(*b));
            let loc = loc.checked_sub(1).ok_or_else(invalid)?;
            if (i == 0 && loc != 0)
                || (i != 0 && loc <= previous)
                || i64::from(loc) > self.end - self.offset
            {
                return Err(invalid());
            }
            result.push(self.offset + i64::from(loc));
            previous = loc;
        }
        Ok(result)
    }
}

// The DICT interpreter uses the same operand stack and operator arities as
// x/image's psInterpreter in the Top DICT context.
#[derive(Default, Debug, PartialEq)]
struct TopDict {
    char_strings: i32,
    private_length: i32,
    private_offset: i32,
    cid: bool,
    fd_array: i32,
    fd_select: i32,
    subrs: i32,
}

fn top_dict(bytes: &[u8]) -> io::Result<TopDict> {
    dict(bytes, false)
}

fn dict(bytes: &[u8], private: bool) -> io::Result<TopDict> {
    let mut result = TopDict::default();
    let mut stack = Vec::<i32>::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        i += 1;
        let number = match b {
            28 => {
                let a = bytes.get(i..i + 2).ok_or_else(invalid)?;
                i += 2;
                Some(i16::from_be_bytes([a[0], a[1]]) as i32)
            }
            29 => {
                let a = bytes.get(i..i + 4).ok_or_else(invalid)?;
                i += 4;
                Some(i32::from_be_bytes([a[0], a[1], a[2], a[3]]))
            }
            30 => {
                let mut real = String::new();
                'real: loop {
                    let byte = *bytes.get(i).ok_or_else(invalid)?;
                    i += 1;
                    for nib in [byte >> 4, byte & 15] {
                        if nib == 15 {
                            break 'real;
                        }
                        if nib == 13 || real.len() + 2 > 64 {
                            return Err(invalid());
                        }
                        real.push_str(match nib {
                            0..=9 => {
                                ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"][nib as usize]
                            }
                            10 => ".",
                            11 => "E",
                            12 => "E-",
                            14 => "-",
                            _ => unreachable!(),
                        });
                    }
                }
                {
                    let value = real.parse::<f32>().map_err(|_| invalid())?;
                    if !value.is_finite() {
                        return Err(invalid());
                    }
                    Some(value.to_bits() as i32)
                }
            }
            32..=246 => Some(i32::from(b) - 139),
            247..=254 => {
                let next = i32::from(*bytes.get(i).ok_or_else(invalid)?);
                i += 1;
                Some(if b < 251 {
                    i32::from(b - 247) * 256 + next + 108
                } else {
                    -i32::from(b - 251) * 256 - next - 108
                })
            }
            _ => None,
        };
        if let Some(n) = number {
            if stack.len() == 48 {
                return Err(invalid());
            }
            stack.push(n);
            continue;
        }
        let escaped = b == 12;
        let op = if escaped {
            let op = *bytes.get(i).ok_or_else(invalid)?;
            i += 1;
            op
        } else {
            b
        };
        let pops = match (private, escaped, op) {
            (true, false, 6..=9) | (true, true, 12 | 13) => -1,
            (true, false, 10 | 11 | 19..=21) | (true, true, 9..=11 | 14 | 17..=19) => 1,
            (false, false, 0..=4 | 13 | 15..=17) => 1,
            (false, false, 5 | 14) => -1,
            (false, false, 18) => 2,
            (false, true, 0..=6 | 8 | 20..=22 | 31..=38) => 1,
            (false, true, 7 | 23) => -1,
            (false, true, 30) => 3,
            _ => return Err(invalid()),
        };
        if pops >= 0 && stack.len() < pops as usize {
            return Err(invalid());
        }
        let n = stack.len();
        match (private, escaped, op) {
            (true, false, 19) => result.subrs = stack[n - 1],
            (false, false, 17) => result.char_strings = stack[n - 1],
            (false, false, 18) => {
                result.private_length = stack[n - 2];
                result.private_offset = stack[n - 1];
            }
            (false, true, 30) => result.cid = true,
            (false, true, 36) => result.fd_array = stack[n - 1],
            (false, true, 37) => result.fd_select = stack[n - 1],
            _ => (),
        }
        if pops < 0 {
            stack.clear();
        } else {
            stack.truncate(n - pops as usize);
        }
    }
    Ok(result)
}

// Parse the CFF tables needed by the bounded coverage probe.
pub(super) fn probe_prefix<R: Read + Seek>(
    source: &mut BoundedReader<R>,
    tables: &ProbeTables,
    glyphs: u16,
) -> io::Result<GlyphData> {
    let (base, length) = tables.table(u32::from_be_bytes(*b"CFF "));
    let mut p = Parser {
        source,
        base: i64::from(base),
        offset: i64::from(base),
        end: i64::from(base) + i64::from(length),
    };
    let header = p.read(4)?;
    if header[..3] != [1, 0, 4] {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "sfnt: unsupported CFF version",
        ));
    }
    let (count, size) = p.index_header()?;
    if count != 1 {
        return Err(invalid());
    }
    let locations = p.index_locations(count, size)?;
    p.offset = locations[1];
    let (count, size) = p.index_header()?;
    if count != 1 {
        return Err(invalid());
    }
    let locations = p.index_locations(count, size)?;
    let dict = p.read(locations[1] - locations[0])?;
    let dict = top_dict(&dict)?;

    // String INDEX: Go skips all but the final offset without reading it.
    let (count, size) = p.index_header()?;
    if count != 0 {
        let skip = (count * size) as i64;
        p.offset = p
            .offset
            .checked_add(skip)
            .filter(|&n| n <= p.end)
            .ok_or_else(invalid)?;
        let last = p.read(size as i64)?;
        let loc = last.iter().fold(0u32, |n, b| (n << 8) | u32::from(*b));
        let loc = loc.checked_sub(1).ok_or_else(invalid)?;
        p.offset = p
            .offset
            .checked_add(i64::from(loc))
            .filter(|&n| n <= p.end)
            .ok_or_else(invalid)?;
    }

    let (count, size) = p.index_header()?;
    if count > 40_000 {
        return Err(invalid());
    }
    let global_subrs = p.index_locations(count, size)?;

    if dict.char_strings < 0 || i64::from(dict.char_strings) > p.end - i64::from(base) {
        return Err(invalid());
    }
    p.offset = i64::from(base) + i64::from(dict.char_strings);
    let (count, size) = p.index_header()?;
    if count == 0 || count != usize::from(glyphs) {
        return Err(invalid());
    }
    let locations = p.index_locations(count, size)?;
    let (single_subrs, multi_subrs, fd_select) = if !dict.cid {
        (
            p.private_dict(dict.private_offset, dict.private_length)?,
            None,
            None,
        )
    } else {
        p.seek_from_base(dict.fd_select)?;
        let format = p.read(1)?[0];
        let num_ranges = match format {
            0 if p.end - p.offset >= i64::from(glyphs) => 0,
            3 => {
                let b = p.read(2)?;
                let ranges = u16::from_be_bytes([b[0], b[1]]);
                if p.end - p.offset < 3 * i64::from(ranges) + 2 {
                    return Err(invalid());
                }
                ranges
            }
            _ => return Err(invalid()),
        };
        let fd_select = FdSelect {
            format,
            num_ranges,
            offset: p.offset,
        };
        p.seek_from_base(dict.fd_array)?;
        let (count, size) = p.index_header()?;
        if count > 256 {
            return Err(invalid());
        }
        let locations = p.index_locations(count, size)?;
        let mut private = Vec::with_capacity(count);
        for pair in locations.windows(2) {
            let entry = top_dict(&p.read(pair[1] - pair[0])?)?;
            private.push((entry.private_offset, entry.private_length));
        }
        let mut subrs = Vec::with_capacity(count);
        for (offset, length) in private {
            subrs.push(p.private_dict(offset, length)?);
        }
        (Vec::new(), Some(subrs), Some(fd_select))
    };
    Ok(GlyphData {
        locations,
        global_subrs,
        single_subrs,
        multi_subrs,
        fd_select,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn top_dict_reads_operands_and_validates_stack() {
        // CharStrings=100, Private=(20, 200), ROS marks CID, FDArray=30.
        let parsed = top_dict(&[
            239, 17, 159, 247, 92, 18, 139, 140, 141, 12, 30, 169, 12, 36,
        ])
        .unwrap();
        assert_eq!(parsed.char_strings, 100);
        assert_eq!((parsed.private_length, parsed.private_offset), (20, 200));
        assert!(parsed.cid);
        assert_eq!(parsed.fd_array, 30);
        assert!(top_dict(&[18]).is_err());
        assert!(top_dict(&[12]).is_err());
        assert!(top_dict(&[30, 0x1d]).is_err());
        assert_eq!(dict(&[144, 19], true).unwrap().subrs, 5);
        assert!(dict(&[19], true).is_err());
        assert!(dict(&[17], true).is_err());
    }

    #[test]
    fn private_dict_charges_local_subroutine_index_reads() {
        // Private DICT at offset 1 points five bytes forward to its Subrs INDEX.
        let data = vec![0, 144, 19, 0, 0, 0, 0, 1, 1, 1, 2, 11];
        let mut reader = BoundedReader::new(Cursor::new(data), 20);
        let mut parser = Parser {
            source: &mut reader,
            base: 0,
            offset: 0,
            end: 12,
        };
        assert_eq!(parser.private_dict(1, 2).unwrap(), vec![11, 12]);
        assert_eq!(reader.bytes_read(), 7);

        // Two format-3 ranges: [0, 4) -> 2 and [4, 9) -> 3.
        let mut reader = BoundedReader::new(Cursor::new(vec![0, 0, 2, 0, 4, 3, 0, 9]), 20);
        let select = FdSelect {
            format: 3,
            num_ranges: 2,
            offset: 0,
        };
        assert_eq!(select.lookup(&mut reader, 2).unwrap(), 2);
        assert_eq!(reader.bytes_read(), 10);
    }

    #[test]
    fn type2_subroutine_return_rereads_remaining_instructions() {
        let mut reader = BoundedReader::new(Cursor::new(vec![1, 2, 3, 4, 11, 14]), 20);
        let mut program = Type2Instructions {
            bytes: vec![1, 2, 3, 4],
            offset: 0,
            position: 2,
            calls: Vec::new(),
        };
        program.call(&mut reader, &[4, 5], -107).unwrap();
        assert_eq!(program.bytes, [11]);
        program.return_from_call(&mut reader).unwrap();
        assert_eq!(program.bytes, [3, 4]);
        assert_eq!(reader.bytes_read(), 3);
        assert!(program.return_from_call(&mut reader).is_err());
        assert!(program.call(&mut reader, &[4, 5], -106).is_err());
    }

    #[test]
    fn type2_numbers_round_fixed_values_and_leave_operators() {
        let mut program = Type2Instructions {
            bytes: vec![255, 0xff, 0xff, 0x80, 0, 28, 0xff, 0xfe, 247, 0, 29],
            offset: 0,
            position: 0,
            calls: Vec::new(),
        };
        assert_eq!(program.number().unwrap(), Some(0));
        assert_eq!(program.number().unwrap(), Some(-2));
        assert_eq!(program.number().unwrap(), Some(108));
        assert_eq!(program.number().unwrap(), None);
        assert_eq!(program.position, 10);
    }

    #[test]
    fn type2_probe_charges_subroutine_reads_and_requires_endchar() {
        // Move, call local subroutine, then end. The subroutine draws a line.
        let bytes = vec![139, 139, 21, 32, 10, 14, 140, 140, 5, 11];
        let data = GlyphData {
            locations: vec![0, 6],
            global_subrs: vec![],
            single_subrs: vec![6, 10],
            multi_subrs: None,
            fd_select: None,
        };
        let mut reader = BoundedReader::new(Cursor::new(bytes.clone()), 30);
        let outline = type2_outline(&mut reader, &data, 0).unwrap();
        assert_eq!(reader.bytes_read(), 11);
        assert_eq!(
            outline
                .iter()
                .map(|s| (s.op, s.args[0].x, s.args[0].y))
                .collect::<Vec<_>>(),
            vec![
                (SegmentOp::MoveTo, 0, 0),
                (SegmentOp::LineTo, 1, 1),
                (SegmentOp::LineTo, 0, 0)
            ]
        );
        let mut invalid_bytes = bytes;
        invalid_bytes[5] = 11;
        let mut reader = BoundedReader::new(Cursor::new(invalid_bytes), 30);
        assert!(probe_type2(&mut reader, &data, 0).is_err());
    }

    #[test]
    fn prefix_charges_views_and_rejects_bad_index() {
        // Header, Name, Top DICT, empty String and Subrs, one CharString.
        let data = vec![
            1, 0, 4, 4, 0, 1, 1, 1, 2, 42, 0, 1, 1, 1, 3, 160, 17, 0, 0, 0, 0, 0, 1, 1, 1, 2, 14,
        ];
        let tables = ProbeTables {
            entries: vec![(u32::from_be_bytes(*b"CFF "), 0, data.len() as u32)],
            is_postscript: true,
            final_offset: data.len() as u32,
        };
        let mut reader = BoundedReader::new(Cursor::new(data.clone()), 30);
        let glyphs = probe_prefix(&mut reader, &tables, 1).unwrap();
        assert_eq!(glyphs.locations, vec![26, 27]);
        assert!(glyphs.global_subrs.is_empty());
        assert!(glyphs.single_subrs.is_empty());
        assert_eq!(reader.bytes_read(), 25);
        let mut bad = data;
        bad[7] = 2;
        let mut reader = BoundedReader::new(Cursor::new(bad), 20);
        assert_eq!(
            probe_prefix(&mut reader, &tables, 1).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
