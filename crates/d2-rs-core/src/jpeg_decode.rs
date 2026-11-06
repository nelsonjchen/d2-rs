// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
// Copyright 2009, 2012, 2025 The Go Authors. All rights reserved.
// Go BSD-3-Clause conditions and disclaimer: THIRD_PARTY_NOTICES.md.
//! Source Go 1.27 image/jpeg reader.go/scan.go, native coefficient reconstruction.
use crate::{
    jpeg_entropy::{Error as EntropyError, Reader},
    jpeg_idct,
};
const UNZIG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];
fn format(s: &str) -> String {
    format!("invalid JPEG format: {s}")
}
fn unsupported(s: &str) -> String {
    format!("unsupported JPEG feature: {s}")
}
fn subsampling() -> String {
    unsupported("luma/chroma subsampling ratio")
}
#[derive(Default, Clone)]
struct Component {
    id: u8,
    h: usize,
    v: usize,
    tq: usize,
    plane: Vec<u8>,
    stride: usize,
    coeff: Vec<[i32; 64]>,
}
/// Exact sixteen-bit RGBA values from decoded source image color models.
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<[u16; 4]>,
}
struct Decoder<'a> {
    r: Reader<'a>,
    width: usize,
    height: usize,
    comp: Vec<Component>,
    quant: [[i32; 64]; 4],
    baseline: bool,
    progressive: bool,
    jfif: bool,
    adobe: Option<u8>,
    max_h: usize,
    max_v: usize,
    flex: bool,
    ri: usize,
    eob: u16,
    mxx: usize,
    myy: usize,
    allocated: bool,
}
impl From<EntropyError> for String {
    fn from(e: EntropyError) -> Self {
        e.to_string()
    }
}
impl<'a> Decoder<'a> {
    fn full(&mut self, n: usize) -> Result<Vec<u8>, String> {
        let mut bytes = vec![0; n];
        self.r.full(&mut bytes)?;
        Ok(bytes)
    }
    fn pair(&mut self) -> Result<[u8; 2], String> {
        let mut pair = [0; 2];
        self.r.full(&mut pair)?;
        Ok(pair)
    }
    fn sof(&mut self, n: usize) -> Result<(), String> {
        if !self.comp.is_empty() {
            return Err(format("multiple SOF markers"));
        }
        let count = match n {
            9 => 1,
            15 => 3,
            18 => 4,
            _ => return Err(unsupported("number of components")),
        };
        let bytes = self.full(n)?;
        if bytes[0] != 8 {
            return Err(unsupported("precision"));
        }
        self.height = usize::from(u16::from_be_bytes([bytes[1], bytes[2]]));
        self.width = usize::from(u16::from_be_bytes([bytes[3], bytes[4]]));
        if usize::from(bytes[5]) != count {
            return Err(format("SOF has wrong length"));
        }
        for i in 0..count {
            let id = bytes[6 + i * 3];
            if self.comp.iter().any(|c| c.id == id) {
                return Err(format("repeated component identifier"));
            }
            let tq = usize::from(bytes[8 + i * 3]);
            if tq > 3 {
                return Err(format("bad Tq value"));
            }
            let hv = bytes[7 + i * 3];
            let (mut h, mut v) = (usize::from(hv >> 4), usize::from(hv & 15));
            if !(1..=4).contains(&h) || !(1..=4).contains(&v) {
                return Err(format("luma/chroma subsampling ratio"));
            }
            if h == 3 || v == 3 {
                return Err(subsampling());
            }
            if count == 1 {
                h = 1;
                v = 1;
            }
            if count == 4 {
                match i {
                    0 if hv != 0x11 && hv != 0x22 => return Err(subsampling()),
                    1 | 2 if hv != 0x11 => return Err(subsampling()),
                    3 if self.comp[0].h != h || self.comp[0].v != v => return Err(subsampling()),
                    _ => {}
                }
            }
            self.max_h = self.max_h.max(h);
            self.max_v = self.max_v.max(v);
            self.comp.push(Component {
                id,
                h,
                v,
                tq,
                ..Component::default()
            });
        }
        if count == 3
            && self
                .comp
                .iter()
                .any(|c| !self.max_h.is_multiple_of(c.h) || !self.max_v.is_multiple_of(c.v))
        {
            return Err(subsampling());
        }
        Ok(())
    }
    fn dqt(&mut self, mut n: usize) -> Result<(), String> {
        while n > 0 {
            n -= 1;
            let x = self.r.byte()?;
            let tq = usize::from(x & 15);
            if tq > 3 {
                return Err(format("bad Tq value"));
            }
            let size = match x >> 4 {
                0 => 64,
                1 => 128,
                _ => return Err(format("bad Pq value")),
            };
            if n < size {
                break;
            }
            n -= size;
            let bytes = self.full(size)?;
            for i in 0..64 {
                self.quant[tq][i] = if size == 64 {
                    i32::from(bytes[i])
                } else {
                    i32::from(u16::from_be_bytes([bytes[2 * i], bytes[2 * i + 1]]))
                };
            }
        }
        if n != 0 {
            Err(format("DQT has wrong length"))
        } else {
            Ok(())
        }
    }
    fn allocate(&mut self) -> Result<(), String> {
        self.mxx = self.width.div_ceil(8 * self.max_h);
        self.myy = self.height.div_ceil(8 * self.max_v);
        self.flex = self.comp.len() > 1
            && (self.comp[1].h != self.comp[2].h
                || self.comp[1].v != self.comp[2].v
                || self.max_h != self.comp[0].h
                || self.max_v != self.comp[0].v
                || ![(1, 1), (1, 2), (2, 1), (2, 2), (4, 1), (4, 2)]
                    .contains(&(self.max_h / self.comp[1].h, self.max_v / self.comp[1].v)));
        for (i, c) in self.comp.iter_mut().enumerate() {
            let (h, v) = if self.flex && i < 3 {
                (self.max_h, self.max_v)
            } else {
                (c.h, c.v)
            };
            c.stride = 8 * h * self.mxx;
            let len = c
                .stride
                .checked_mul(8 * v * self.myy)
                .ok_or_else(|| unsupported("image size"))?;
            c.plane = vec![0; len];
        }
        self.allocated = true;
        Ok(())
    }
    fn reconstruct(&mut self, mut b: [i32; 64], bx: usize, by: usize, index: usize) {
        for zig in 0..64 {
            b[UNZIG[zig]] = b[UNZIG[zig]].wrapping_mul(self.quant[self.comp[index].tq][zig]);
        }
        jpeg_idct::inverse(&mut b);
        let c = &mut self.comp[index];
        let (h, v) = if self.flex {
            (self.max_h / c.h, self.max_v / c.v)
        } else {
            (1, 1)
        };
        for y in 0..8 {
            for x in 0..8 {
                let value = b[y * 8 + x].wrapping_add(128).clamp(0, 255) as u8;
                for yy in 0..v {
                    for xx in 0..h {
                        c.plane[(by * 8 * v + y * v + yy) * c.stride + bx * 8 * h + x * h + xx] =
                            value;
                    }
                }
            }
        }
    }
    fn refine_nonzero(
        &mut self,
        b: &mut [i32; 64],
        mut zig: usize,
        end: usize,
        mut nz: i32,
        delta: i32,
    ) -> Result<usize, String> {
        while zig <= end {
            let u = UNZIG[zig];
            if b[u] == 0 {
                if nz == 0 {
                    break;
                }
                nz -= 1;
                zig += 1;
                continue;
            }
            if self.r.bit()? {
                b[u] = if b[u] >= 0 {
                    b[u].wrapping_add(delta)
                } else {
                    b[u].wrapping_sub(delta)
                };
            }
            zig += 1;
        }
        Ok(zig)
    }
    fn refine(
        &mut self,
        b: &mut [i32; 64],
        td: usize,
        start: usize,
        end: usize,
        delta: i32,
    ) -> Result<(), String> {
        if start == 0 {
            if self.r.bit()? {
                b[0] |= delta;
            }
            return Ok(());
        }
        let mut zig = start;
        if self.eob == 0 {
            while zig <= end {
                let value = self.r.symbol(1, td)?;
                let (run, size) = (value >> 4, value & 15);
                let mut z = 0;
                match size {
                    0 => {
                        if run != 15 {
                            self.eob = 1u16 << run;
                            if run != 0 {
                                self.eob |= self.r.read_bits(i32::from(run))? as u16;
                            }
                            break;
                        }
                    }
                    1 => {
                        z = if self.r.bit()? { delta } else { -delta };
                    }
                    _ => return Err(format("unexpected Huffman code")),
                }
                zig = self.refine_nonzero(b, zig, end, i32::from(run), delta)?;
                if zig > end {
                    return Err(format("too many coefficients"));
                }
                if z != 0 {
                    b[UNZIG[zig]] = z;
                }
                zig += 1;
            }
        }
        if self.eob > 0 {
            self.eob -= 1;
            self.refine_nonzero(b, zig, end, -1, delta)?;
        }
        Ok(())
    }
    fn restart(&mut self, expected: u8) -> Result<(), String> {
        let mut pair = self.pair()?;
        loop {
            let mut i = 0;
            if pair[0] == 255 {
                if pair[1] == expected {
                    return Ok(());
                } else if pair[1] == 255 {
                    i = 1;
                } else if pair[1] != 0 {
                    return Err(format("bad RST marker"));
                }
            } else if pair[1] == 255 {
                pair[0] = 255;
                i = 1;
            }
            self.r.full(&mut pair[i..])?;
        }
    }
    fn sos(&mut self, n: usize) -> Result<(), String> {
        if self.comp.is_empty() {
            return Err(format("missing SOF marker"));
        }
        if n < 6 || 4 + 2 * self.comp.len() < n || !n.is_multiple_of(2) {
            return Err(format("SOS has wrong length"));
        }
        let bytes = self.full(n)?;
        let count = usize::from(bytes[0]);
        if n != 4 + 2 * count {
            return Err(format("SOS length inconsistent with number of components"));
        }
        let mut scan = Vec::new();
        let mut total = 0;
        for i in 0..count {
            let index = self
                .comp
                .iter()
                .position(|c| c.id == bytes[1 + 2 * i])
                .ok_or_else(|| format("unknown component selector"))?;
            if scan.iter().any(|&(j, _, _)| j == index) {
                return Err(format("repeated component selector"));
            }
            total += self.comp[index].h * self.comp[index].v;
            let td = usize::from(bytes[2 + 2 * i] >> 4);
            if td > 3 || (self.baseline && td > 1) {
                return Err(format("bad Td value"));
            }
            let ta = usize::from(bytes[2 + 2 * i] & 15);
            if ta > 3 || (self.baseline && ta > 1) {
                return Err(format("bad Ta value"));
            }
            scan.push((index, td, ta));
        }
        if self.comp.len() > 1 && total > 10 {
            return Err(format("total sampling factors too large"));
        }
        let (start, end, ah, al) = if self.progressive {
            (
                usize::from(bytes[1 + 2 * count]),
                usize::from(bytes[2 + 2 * count]),
                bytes[3 + 2 * count] >> 4,
                bytes[3 + 2 * count] & 15,
            )
        } else {
            (0, 63, 0, 0)
        };
        if self.progressive {
            if (start == 0 && end != 0) || start > end || end >= 64 {
                return Err(format("bad spectral selection bounds"));
            }
            if start != 0 && count != 1 {
                return Err(format(
                    "progressive AC coefficients for more than one component",
                ));
            }
            if ah != 0 && ah != al + 1 {
                return Err(format("bad successive approximation values"));
            }
        }
        if !self.allocated {
            self.allocate()?;
        }
        if self.progressive {
            for &(i, _, _) in &scan {
                let c = &mut self.comp[i];
                if c.coeff.is_empty() {
                    c.coeff = vec![[0; 64]; self.mxx * self.myy * c.h * c.v];
                }
            }
        }
        self.r.accumulator = 0;
        self.r.mask = 0;
        self.r.bits = 0;
        let mut dc = [0i32; 4];
        let mut block_count = 0;
        let mut mcu = 0;
        let mut expected = 0xd0;
        for my in 0..self.myy {
            for mx in 0..self.mxx {
                for &(index, td, ta) in &scan {
                    let (hi, vi) = (self.comp[index].h, self.comp[index].v);
                    for j in 0..hi * vi {
                        let (bx, by) = if count != 1 {
                            (hi * mx + j % hi, vi * my + j / hi)
                        } else {
                            let q = self.mxx * hi;
                            let point = (block_count % q, block_count / q);
                            block_count += 1;
                            point
                        };
                        if count == 1 && (bx * 8 >= self.width || by * 8 >= self.height) {
                            continue;
                        }
                        let slot = by * self.mxx * hi + bx;
                        let mut b = if self.progressive {
                            self.comp[index].coeff[slot]
                        } else {
                            [0; 64]
                        };
                        if ah != 0 {
                            self.refine(&mut b, ta, start, end, 1i32 << al)?;
                        } else {
                            let mut zig = start;
                            if zig == 0 {
                                zig = 1;
                                let value = self.r.symbol(0, td)?;
                                if value > 16 {
                                    return Err(unsupported("excessive DC component"));
                                }
                                let delta = self.r.receive(value)?;
                                dc[index] = dc[index].wrapping_add(delta);
                                b[0] = dc[index].wrapping_shl(u32::from(al));
                            }
                            if zig <= end && self.eob > 0 {
                                self.eob -= 1;
                            } else {
                                while zig <= end {
                                    let value = self.r.symbol(1, ta)?;
                                    let (run, size) = (value >> 4, value & 15);
                                    if size != 0 {
                                        zig += usize::from(run);
                                        if zig > end {
                                            break;
                                        }
                                        let ac = self.r.receive(size)?;
                                        b[UNZIG[zig]] = ac.wrapping_shl(u32::from(al));
                                    } else {
                                        if run != 15 {
                                            self.eob = 1u16 << run;
                                            if run != 0 {
                                                self.eob |=
                                                    self.r.read_bits(i32::from(run))? as u16;
                                            }
                                            self.eob -= 1;
                                            break;
                                        }
                                        zig += 15;
                                    }
                                    zig += 1;
                                }
                            }
                        }
                        if self.progressive {
                            self.comp[index].coeff[slot] = b;
                        } else {
                            self.reconstruct(b, bx, by, index);
                        }
                    }
                }
                mcu += 1;
                if self.ri > 0 && mcu % self.ri == 0 && mcu < self.mxx * self.myy {
                    self.restart(expected)?;
                    expected = if expected == 0xd7 { 0xd0 } else { expected + 1 };
                    self.r.accumulator = 0;
                    self.r.mask = 0;
                    self.r.bits = 0;
                    dc = [0; 4];
                    self.eob = 0;
                }
            }
        }
        Ok(())
    }
    fn finish(&mut self) -> Result<Image, String> {
        if self.progressive {
            for i in 0..self.comp.len() {
                if self.comp[i].coeff.is_empty() {
                    continue;
                }
                let (h, v) = (
                    8 * self.max_h / self.comp[i].h,
                    8 * self.max_v / self.comp[i].v,
                );
                let stride = self.mxx * self.comp[i].h;
                for by in 0..self.height.div_ceil(v) {
                    for bx in 0..self.width.div_ceil(h) {
                        let b = self.comp[i].coeff[by * stride + bx];
                        self.reconstruct(b, bx, by, i);
                    }
                }
            }
        }
        if !self.allocated {
            return Err(format("missing SOS marker"));
        }
        let rgb = self.comp.len() == 3
            && !self.jfif
            && (self.adobe == Some(0) || self.comp.iter().map(|c| c.id).eq(*b"RGB"));
        if rgb
            && (self.comp[1].h != self.comp[2].h
                || !self.comp[0].h.is_multiple_of(self.comp[1].h)
                || self.comp[1].v != self.comp[2].v
                || !self.comp[0].v.is_multiple_of(self.comp[1].v))
        {
            return Err(subsampling());
        }
        if self.comp.len() == 4 && self.adobe.is_none() {
            return Err(unsupported(
                "unknown color model: 4-component JPEG doesn't have Adobe APP14 metadata",
            ));
        }
        let mut rgba = Vec::with_capacity(self.width * self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                let sample = |i: usize| {
                    let c = &self.comp[i];
                    let (h, v) = if self.flex && i < 3 {
                        (1, 1)
                    } else {
                        (self.max_h / c.h, self.max_v / c.v)
                    };
                    c.plane[(y / v) * c.stride + x / h]
                };
                let pixel = if self.comp.len() == 1 {
                    let g = u16::from(sample(0)) * 257;
                    [g, g, g, 65535]
                } else if rgb {
                    let c = &self.comp[1];
                    let v = if self.flex { 1 } else { self.max_v / c.v };
                    let offset = (y / v) * c.stride + x / (self.comp[0].h / c.h);
                    [
                        u16::from(sample(0)) * 257,
                        u16::from(c.plane[offset]) * 257,
                        u16::from(self.comp[2].plane[offset]) * 257,
                        65535,
                    ]
                } else if self.comp.len() == 4 {
                    let w = u32::from(sample(3)) * 257;
                    let channels = if self.adobe == Some(0) {
                        [sample(0), sample(1), sample(2)]
                    } else {
                        ycbcr8(sample(0), sample(1), sample(2)).map(|v| 255 - v)
                    };
                    [
                        ((u32::from(channels[0]) * 257) * w / 65535) as u16,
                        ((u32::from(channels[1]) * 257) * w / 65535) as u16,
                        ((u32::from(channels[2]) * 257) * w / 65535) as u16,
                        65535,
                    ]
                } else {
                    let color = ycbcr16(sample(0), sample(1), sample(2));
                    [color[0], color[1], color[2], 65535]
                };
                rgba.push(pixel);
            }
        }
        Ok(Image {
            width: self.width,
            height: self.height,
            rgba,
        })
    }
}
/// Go source sixteen-bit conversion; chroma selection is kept outside conversion.
pub fn ycbcr16(y: u8, cb: u8, cr: u8) -> [u16; 3] {
    let yy = i32::from(y) * 0x10101;
    let cb = i32::from(cb) - 128;
    let cr = i32::from(cr) - 128;
    [
        ((yy + 91881 * cr) >> 8).clamp(0, 65535) as u16,
        ((yy - 22554 * cb - 46802 * cr) >> 8).clamp(0, 65535) as u16,
        ((yy + 116130 * cb) >> 8).clamp(0, 65535) as u16,
    ]
}
fn ycbcr8(y: u8, cb: u8, cr: u8) -> [u8; 3] {
    let yy = i32::from(y) * 0x10101;
    let cb = i32::from(cb) - 128;
    let cr = i32::from(cr) - 128;
    [
        ((yy + 91881 * cr) >> 16).clamp(0, 255) as u8,
        ((yy - 22554 * cb - 46802 * cr) >> 16).clamp(0, 255) as u8,
        ((yy + 116130 * cb) >> 16).clamp(0, 255) as u8,
    ]
}
pub fn decode(data: &[u8]) -> Result<Image, String> {
    decode_reader(Reader::new(data))
}
/// Functional Go-style input; entropy buffering retains n>0/error semantics.
pub fn decode_input(input: &mut dyn crate::jpeg_entropy::Input) -> Result<Image, String> {
    decode_reader(Reader::from_input(input))
}
fn decode_reader(reader: Reader<'_>) -> Result<Image, String> {
    let mut d = Decoder {
        r: reader,
        width: 0,
        height: 0,
        comp: Vec::new(),
        quant: [[0; 64]; 4],
        baseline: false,
        progressive: false,
        jfif: false,
        adobe: None,
        max_h: 0,
        max_v: 0,
        flex: false,
        ri: 0,
        eob: 0,
        mxx: 0,
        myy: 0,
        allocated: false,
    };
    if d.pair()? != [255, 216] {
        return Err(format("missing SOI marker"));
    }
    loop {
        let mut pair = d.pair()?;
        while pair[0] != 255 {
            pair = [pair[1], d.r.byte()?];
        }
        let mut marker = pair[1];
        if marker == 0 {
            continue;
        }
        while marker == 255 {
            marker = d.r.byte()?;
        }
        if marker == 217 {
            break;
        }
        if (208..=215).contains(&marker) {
            continue;
        }
        let length = d.pair()?;
        let n = i32::from(u16::from_be_bytes(length)) - 2;
        if n < 0 {
            return Err(format("short segment length"));
        }
        let n = n as usize;
        match marker {
            0xc0..=0xc2 => {
                d.baseline = marker == 0xc0;
                d.progressive = marker == 0xc2;
                d.sof(n)?;
            }
            0xc4 => d.r.dht(n as i32, d.baseline)?,
            0xdb => d.dqt(n)?,
            0xda => d.sos(n)?,
            0xdd => {
                if n != 2 {
                    return Err(format("DRI has wrong length"));
                }
                let value = d.pair()?;
                d.ri = usize::from(u16::from_be_bytes(value));
            }
            0xe0 => {
                let bytes = d.full(n)?;
                if n >= 5 {
                    d.jfif = &bytes[..5] == b"JFIF\0";
                }
            }
            0xee => {
                let bytes = d.full(n)?;
                if n >= 12 && &bytes[..5] == b"Adobe" {
                    d.adobe = Some(bytes[11]);
                }
            }
            0xe1..=0xef | 0xfe => {
                d.full(n)?;
            }
            _ => {
                return Err(if marker < 0xc0 {
                    format("unknown marker")
                } else {
                    unsupported("unknown marker")
                });
            }
        }
    }
    d.finish()
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    #[test]
    fn source_complete_colors_progressive_restart_models_and_errors() {
        let fixtures: Vec<serde_json::Value> = serde_json::from_str(
            d2_rs_testdata::oracle_fixture!("tests/jpeg-frame-go127.json"),
        )
        .unwrap();
        assert_eq!(fixtures.len(), 438);
        for fixture in fixtures {
            let input = &fixture["input"];
            let data: Vec<u8> = input["Data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u8)
                .collect();
            let actual = match super::decode(&data) {
                Ok(image) => {
                    assert_eq!(image.rgba.len(), image.width * image.height);
                    serde_json::json!({"Width":image.width,"Height":image.height,"Error":"","Pixels":image.rgba})
                }
                Err(error) => serde_json::json!({"Width":0,"Height":0,"Error":error,"Pixels":[]}),
            };
            assert_eq!(
                actual, fixture["expected"],
                "source image {}",
                input["Name"]
            );
        }
    }
}
