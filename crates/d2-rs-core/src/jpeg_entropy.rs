// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
// Copyright 2009 The Go Authors. All rights reserved.
// Go BSD-3-Clause conditions and disclaimer: THIRD_PARTY_NOTICES.md.
//! Source Go1.27 image/jpeg/huffman.go and reader.go buffered entropy reading.
#[derive(Clone)]
pub struct Huffman {
    pub count: i32,
    pub lut: [u16; 256],
    pub values: [u8; 256],
    pub min: [i32; 16],
    pub max: [i32; 16],
    pub indices: [i32; 16],
}
impl Default for Huffman {
    fn default() -> Self {
        Self {
            count: 0,
            lut: [0; 256],
            values: [0; 256],
            min: [0; 16],
            max: [0; 16],
            indices: [0; 16],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Format(&'static str),
    UnexpectedEof,
    Eof,
    Cancelled,
    Deadline,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Format(s) => write!(f, "invalid JPEG format: {s}"),
            Self::UnexpectedEof => f.write_str("unexpected EOF"),
            Self::Eof => f.write_str("EOF"),
            Self::Cancelled => f.write_str("context canceled"),
            Self::Deadline => f.write_str("context deadline exceeded"),
        }
    }
}
impl std::error::Error for Error {}
const SHORT: Error = Error::Format("short Huffman data");
const MISSING: Error = Error::Format("missing 0xff00 sequence");
/// Go reader result preserves a nonzero byte count together with an error.
pub trait Input {
    fn read(&mut self, destination: &mut [u8]) -> (usize, Option<Error>);
}
pub struct Reader<'a> {
    transport: Option<&'a mut dyn Input>,
    input: &'a [u8],
    pub input_offset: usize,
    buffer: [u8; 4096],
    pub start: usize,
    pub end: usize,
    pub unread: usize,
    pub accumulator: u32,
    pub mask: u32,
    pub bits: i32,
    pub tables: [[Huffman; 4]; 2],
}
impl<'a> Reader<'a> {
    pub fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            transport: None,
            input_offset: 0,
            buffer: [0; 4096],
            start: 0,
            end: 0,
            unread: 0,
            accumulator: 0,
            mask: 0,
            bits: 0,
            tables: std::array::from_fn(|_| std::array::from_fn(|_| Huffman::default())),
        }
    }
    fn fill(&mut self) -> Result<(), Error> {
        assert_eq!(
            self.start, self.end,
            "jpeg: fill called when unread bytes exist"
        );
        if self.end > 2 {
            self.buffer[0] = self.buffer[self.end - 2];
            self.buffer[1] = self.buffer[self.end - 1];
            self.start = 2;
            self.end = 2;
        }
        let (n, error) = if let Some(reader) = self.transport.as_mut() {
            reader.read(&mut self.buffer[self.end..])
        } else {
            let n = (self.input.len() - self.input_offset).min(4096 - self.end);
            self.buffer[self.end..self.end + n]
                .copy_from_slice(&self.input[self.input_offset..self.input_offset + n]);
            (n, if n == 0 { Some(Error::Eof) } else { None })
        };
        self.end += n;
        self.input_offset += n;
        if n > 0 {
            Ok(())
        } else {
            match error {
                Some(Error::Eof) => Err(Error::UnexpectedEof),
                Some(e) => Err(e),
                None => Ok(()),
            }
        }
    }
    pub fn from_input(input: &'a mut dyn Input) -> Self {
        let mut reader = Self::new(&[]);
        reader.transport = Some(input);
        reader
    }

    pub fn byte(&mut self) -> Result<u8, Error> {
        while self.start == self.end {
            self.fill()?;
        }
        let b = self.buffer[self.start];
        self.start += 1;
        self.unread = 0;
        Ok(b)
    }
    pub fn stuffed(&mut self) -> Result<u8, Error> {
        if self.start + 2 <= self.end {
            let b = self.buffer[self.start];
            self.start += 1;
            self.unread = 1;
            if b != 255 {
                return Ok(b);
            }
            if self.buffer[self.start] != 0 {
                return Err(MISSING);
            }
            self.start += 1;
            self.unread = 2;
            return Ok(255);
        }
        self.unread = 0;
        let b = self.byte()?;
        self.unread = 1;
        if b != 255 {
            return Ok(b);
        }
        let b = self.byte()?;
        self.unread = 2;
        if b != 0 { Err(MISSING) } else { Ok(255) }
    }
    pub fn undo_stuffed(&mut self) {
        self.start -= self.unread;
        self.unread = 0;
        if self.bits >= 8 {
            self.accumulator >>= 8;
            self.bits -= 8;
            self.mask >>= 8;
        }
    }
    pub fn full(&mut self, output: &mut [u8]) -> Result<(), Error> {
        if self.unread != 0 {
            if self.bits >= 8 {
                self.undo_stuffed();
            }
            self.unread = 0;
        }
        let mut offset = 0;
        loop {
            let n = (self.end - self.start).min(output.len() - offset);
            output[offset..offset + n].copy_from_slice(&self.buffer[self.start..self.start + n]);
            self.start += n;
            offset += n;
            if offset == output.len() {
                return Ok(());
            }
            self.fill()?;
        }
    }
    fn ensure(&mut self, n: i32) -> Result<(), Error> {
        loop {
            let b = self
                .stuffed()
                .map_err(|e| if e == Error::UnexpectedEof { SHORT } else { e })?;
            self.accumulator = self.accumulator.wrapping_shl(8) | u32::from(b);
            self.bits += 8;
            self.mask = if self.mask == 0 {
                128
            } else {
                self.mask.wrapping_shl(8)
            };
            if self.bits >= n {
                return Ok(());
            }
        }
    }
    pub fn receive(&mut self, t: u8) -> Result<i32, Error> {
        assert!(t <= 16);
        if self.bits < i32::from(t) {
            self.ensure(i32::from(t))?;
        }
        self.bits -= i32::from(t);
        self.mask = self.mask.checked_shr(u32::from(t)).unwrap_or(0);
        let s = 1i32 << t;
        let mut x = (self.accumulator.checked_shr(self.bits as u32).unwrap_or(0) as i32) & (s - 1);
        let sign = x.checked_shr(u32::from(t.wrapping_sub(1))).unwrap_or(0) - 1;
        x += sign & ((-1i32 << t) + 1);
        Ok(x)
    }
    pub fn bit(&mut self) -> Result<bool, Error> {
        if self.bits == 0 {
            self.ensure(1)?;
        }
        let b = self.accumulator & self.mask != 0;
        self.bits -= 1;
        self.mask >>= 1;
        Ok(b)
    }
    pub fn read_bits(&mut self, n: i32) -> Result<u32, Error> {
        assert!((0..=16).contains(&n));
        if self.bits < n {
            self.ensure(n)?;
        }
        let b = self
            .accumulator
            .checked_shr((self.bits - n) as u32)
            .unwrap_or(0)
            & ((1u32 << n) - 1);
        self.bits -= n;
        self.mask = self.mask.checked_shr(n as u32).unwrap_or(0);
        Ok(b)
    }
    pub fn dht(&mut self, mut n: i32, baseline: bool) -> Result<(), Error> {
        while n > 0 {
            if n < 17 {
                return Err(Error::Format("DHT has wrong length"));
            }
            let mut header = [0; 17];
            self.full(&mut header)?;
            let tc = usize::from(header[0] >> 4);
            if tc > 1 {
                return Err(Error::Format("bad Tc value"));
            }
            let th = usize::from(header[0] & 15);
            if th > 3 || (baseline && th > 1) {
                return Err(Error::Format("bad Th value"));
            }
            self.tables[tc][th].count = header[1..].iter().map(|v| i32::from(*v)).sum();
            let count = self.tables[tc][th].count;
            if count == 0 {
                return Err(Error::Format("Huffman table has zero length"));
            }
            if count > 256 {
                return Err(Error::Format("Huffman table has excessive length"));
            }
            n -= count + 17;
            if n < 0 {
                return Err(Error::Format("DHT has wrong length"));
            }
            let mut values = self.tables[tc][th].values[..count as usize].to_vec();
            let read_result = self.full(&mut values);
            self.tables[tc][th].values[..values.len()].copy_from_slice(&values);
            read_result?;
            let h = &mut self.tables[tc][th];
            h.lut.fill(0);
            let (mut x, mut code) = (0usize, 0u32);
            for i in 0..8 {
                code <<= 1;
                for _ in 0..header[i + 1] {
                    let base = (code << (7 - i)) as u8;
                    let value = u16::from(h.values[x]) << 8 | (2 + i) as u16;
                    for k in 0..(1u16 << (7 - i)) {
                        h.lut[usize::from(base | k as u8)] = value;
                    }
                    code += 1;
                    x += 1;
                }
            }
            let (mut code, mut index) = (0i32, 0i32);
            for i in 0..16 {
                let count = i32::from(header[i + 1]);
                if count == 0 {
                    h.min[i] = -1;
                    h.max[i] = -1;
                    h.indices[i] = -1;
                } else {
                    h.min[i] = code;
                    h.max[i] = code + count - 1;
                    h.indices[i] = index;
                    code += count;
                    index += count;
                }
                code <<= 1;
            }
        }
        Ok(())
    }
    pub fn symbol(&mut self, tc: usize, th: usize) -> Result<u8, Error> {
        if self.tables[tc][th].count == 0 {
            return Err(Error::Format("uninitialized Huffman table"));
        }
        let mut slow = false;
        if self.bits < 8 {
            if let Err(e) = self.ensure(8) {
                if e != MISSING && e != SHORT {
                    return Err(e);
                }
                if self.unread != 0 {
                    self.undo_stuffed();
                }
                slow = true;
            }
        }
        if !slow {
            let v = self.tables[tc][th].lut
                [((self.accumulator >> ((self.bits - 8) as u32)) & 255) as usize];
            if v != 0 {
                let n = u32::from((v & 255) - 1);
                self.bits -= n as i32;
                self.mask >>= n;
                return Ok((v >> 8) as u8);
            }
        }
        let mut code = 0i32;
        for i in 0..16 {
            if self.bits == 0 {
                self.ensure(1)?;
            }
            if self.accumulator & self.mask != 0 {
                code |= 1;
            }
            self.bits -= 1;
            self.mask >>= 1;
            let h = &self.tables[tc][th];
            if code <= h.max[i] {
                return Ok(h.values[(h.indices[i] + code - h.min[i]) as usize]);
            }
            code <<= 1;
        }
        Err(Error::Format("bad Huffman code"))
    }
}
