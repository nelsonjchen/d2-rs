// SPDX-License-Identifier: BSD-3-Clause
// Copyright 2017 The Go Authors. All rights reserved.
// Rust translation of golang.org/x/image v0.45.0 font/sfnt/truetype.go.

use super::*;

const FLAG_ON_CURVE: u8 = 1 << 0;
const FLAG_X_SHORT_VECTOR: u8 = 1 << 1;
const FLAG_Y_SHORT_VECTOR: u8 = 1 << 2;
const FLAG_REPEAT: u8 = 1 << 3;
const FLAG_POSITIVE_X_SHORT_VECTOR: u8 = 1 << 4;
const FLAG_THIS_X_IS_SAME: u8 = 1 << 4;
const FLAG_POSITIVE_Y_SHORT_VECTOR: u8 = 1 << 5;
const FLAG_THIS_Y_IS_SAME: u8 = 1 << 5;

const FLAG_ARG1_AND2_ARE_WORDS: u16 = 1 << 0;
const FLAG_ARGS_ARE_XY_VALUES: u16 = 1 << 1;
const FLAG_WE_HAVE_A_SCALE: u16 = 1 << 3;
const FLAG_MORE_COMPONENTS: u16 = 1 << 5;
const FLAG_WE_HAVE_AN_X_AND_Y_SCALE: u16 = 1 << 6;
const FLAG_WE_HAVE_A_TWO_BY_TWO: u16 = 1 << 7;

const GLYF_HEADER_LEN: usize = 10;

fn mid_point(p: Point26_6, q: Point26_6) -> Point26_6 {
    // Go's integer division truncates toward zero.
    Point26_6 {
        x: p.x.wrapping_add(q.x) / 2,
        y: p.y.wrapping_add(q.y) / 2,
    }
}

pub(super) fn parse_loca(
    src: &Source,
    loca: Table,
    glyf_offset: u32,
    index_to_loc_format: bool,
    num_glyphs: i32,
) -> Result<Vec<u32>, Error> {
    if index_to_loc_format {
        if loca.length != 4 * (num_glyphs + 1) as u32 {
            return Err(ERR_INVALID_LOCA_TABLE);
        }
    } else if loca.length != 2 * (num_glyphs + 1) as u32 {
        return Err(ERR_INVALID_LOCA_TABLE);
    }
    let buf = src.view(i64::from(loca.offset), i64::from(loca.length))?;
    let n = (num_glyphs + 1) as usize;
    Ok((0..n)
        .map(|i| {
            if index_to_loc_format {
                u32_at(buf, 4 * i).wrapping_add(glyf_offset)
            } else {
                (2 * u32::from(u16_at(buf, 2 * i))).wrapping_add(glyf_offset)
            }
        })
        .collect())
}

#[derive(Clone, Copy, Default)]
pub(super) struct CompoundElem {
    glyph_index: GlyphIndex,
    dx: i16,
    dy: i16,
    has_transform: bool,
    transform_xx: i16,
    transform_xy: i16,
    transform_yx: i16,
    transform_yy: i16,
}

pub(super) fn load_glyf(
    f: &Font,
    segments: &mut Vec<Segment>,
    stack: &mut [CompoundElem; MAX_COMPOUND_STACK_SIZE as usize],
    x: GlyphIndex,
    stack_bottom: u32,
    recursion_depth: u32,
) -> Result<(), Error> {
    let data = f.view_glyph_data(x)?;
    if data.is_empty() {
        return Ok(());
    }
    if data.len() < GLYF_HEADER_LEN {
        return Err(ERR_INVALID_GLYPH_DATA);
    }
    let mut index = GLYF_HEADER_LEN;
    let num_contours = u16_at(data, 0) as i16;
    let mut num_points = 0usize;
    match num_contours {
        -1 => {}
        0 => return Ok(()),
        n if n > 0 => {
            index += 2 * n as usize;
            if index > data.len() {
                return Err(ERR_INVALID_GLYPH_DATA);
            }
            num_points = 1 + usize::from(u16_at(data, index - 2));
        }
        _ => return Err(ERR_INVALID_GLYPH_DATA),
    }
    if num_contours < 0 {
        return load_compound_glyf(
            f,
            segments,
            stack,
            &data[GLYF_HEADER_LEN..],
            stack_bottom,
            recursion_depth,
        );
    }
    load_simple_glyf(data, index, num_points, num_contours, segments)
}

// Shared by the normal font loader and the bounded ReaderAt probe. The probe
// needs the same validation even though it does not retain the outline.
pub(super) fn load_simple_glyf(
    data: &[u8],
    mut index: usize,
    num_points: usize,
    num_contours: i16,
    segments: &mut Vec<Segment>,
) -> Result<(), Error> {
    index += 2;
    if index > data.len() {
        return Err(ERR_INVALID_GLYPH_DATA);
    }
    let hints_length = usize::from(u16_at(data, index - 2));
    index += hints_length;
    if index > data.len() {
        return Err(ERR_INVALID_GLYPH_DATA);
    }
    let flag_index = index;
    let Some((x_index, y_index)) = find_xy_indexes(data, index, num_points) else {
        return Err(ERR_INVALID_GLYPH_DATA);
    };
    let mut g = GlyfIter {
        data,
        err: None,
        flag_index,
        x_index,
        y_index,
        end_index: GLYF_HEADER_LEN,
        prev_end: -1,
        final_end: num_points as i32 - 1,
        c: 0,
        num_contours: i32::from(num_contours),
        p: 0,
        n_points: 0,
        x: 0,
        y: 0,
        on: false,
        flag: 0,
        repeats: 0,
        closing: false,
        closed: false,
        first_on_curve_valid: false,
        first_off_curve_valid: false,
        last_off_curve_valid: false,
        first_on_curve: Point26_6::default(),
        first_off_curve: Point26_6::default(),
        last_off_curve: Point26_6::default(),
        seg: Segment {
            op: SegmentOp::MoveTo,
            args: [Point26_6::default(); 3],
        },
    };
    while g.next_contour() {
        while g.next_segment() {
            segments.push(g.seg);
        }
    }
    match g.err {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn find_xy_indexes(data: &[u8], mut index: usize, num_points: usize) -> Option<(usize, usize)> {
    let mut x_data_len = 0usize;
    let mut y_data_len = 0usize;
    let mut i = 0usize;
    loop {
        if i > num_points {
            return None;
        }
        if i == num_points {
            break;
        }
        let mut repeat_count = 1usize;
        if index >= data.len() {
            return None;
        }
        let flag = data[index];
        index += 1;
        if flag & FLAG_REPEAT != 0 {
            if index >= data.len() {
                return None;
            }
            repeat_count += usize::from(data[index]);
            index += 1;
        }
        let x_size = if flag & FLAG_X_SHORT_VECTOR != 0 {
            1
        } else if flag & FLAG_THIS_X_IS_SAME == 0 {
            2
        } else {
            0
        };
        x_data_len += x_size * repeat_count;
        let y_size = if flag & FLAG_Y_SHORT_VECTOR != 0 {
            1
        } else if flag & FLAG_THIS_Y_IS_SAME == 0 {
            2
        } else {
            0
        };
        y_data_len += y_size * repeat_count;
        i += repeat_count;
    }
    if index + x_data_len + y_data_len > data.len() {
        return None;
    }
    Some((index, index + x_data_len))
}

fn load_compound_glyf(
    f: &Font,
    segments: &mut Vec<Segment>,
    stack: &mut [CompoundElem; MAX_COMPOUND_STACK_SIZE as usize],
    mut data: &[u8],
    stack_bottom: u32,
    mut recursion_depth: u32,
) -> Result<(), Error> {
    recursion_depth += 1;
    if recursion_depth == MAX_COMPOUND_RECURSION_DEPTH {
        return Err(ERR_UNSUPPORTED_COMPOUND_GLYPH);
    }
    let mut stack_top = stack_bottom;
    loop {
        if stack_top >= MAX_COMPOUND_STACK_SIZE {
            return Err(ERR_UNSUPPORTED_COMPOUND_GLYPH);
        }
        let elem = &mut stack[stack_top as usize];
        stack_top += 1;
        if data.len() < 4 {
            return Err(ERR_INVALID_GLYPH_DATA);
        }
        let flags = u16_at(data, 0);
        elem.glyph_index = u16_at(data, 2);
        if flags & FLAG_ARG1_AND2_ARE_WORDS == 0 {
            if data.len() < 6 {
                return Err(ERR_INVALID_GLYPH_DATA);
            }
            elem.dx = i16::from(data[4] as i8);
            elem.dy = i16::from(data[5] as i8);
            data = &data[6..];
        } else {
            if data.len() < 8 {
                return Err(ERR_INVALID_GLYPH_DATA);
            }
            elem.dx = u16_at(data, 4) as i16;
            elem.dy = u16_at(data, 6) as i16;
            data = &data[8..];
        }
        if flags & FLAG_ARGS_ARE_XY_VALUES == 0 {
            return Err(ERR_UNSUPPORTED_COMPOUND_GLYPH);
        }
        elem.has_transform = flags
            & (FLAG_WE_HAVE_A_SCALE | FLAG_WE_HAVE_AN_X_AND_Y_SCALE | FLAG_WE_HAVE_A_TWO_BY_TWO)
            != 0;
        if elem.has_transform {
            if flags & FLAG_WE_HAVE_A_SCALE != 0 {
                if data.len() < 2 {
                    return Err(ERR_INVALID_GLYPH_DATA);
                }
                elem.transform_xx = u16_at(data, 0) as i16;
                elem.transform_xy = 0;
                elem.transform_yx = 0;
                elem.transform_yy = elem.transform_xx;
                data = &data[2..];
            } else if flags & FLAG_WE_HAVE_AN_X_AND_Y_SCALE != 0 {
                if data.len() < 4 {
                    return Err(ERR_INVALID_GLYPH_DATA);
                }
                elem.transform_xx = u16_at(data, 0) as i16;
                elem.transform_xy = 0;
                elem.transform_yx = 0;
                elem.transform_yy = u16_at(data, 2) as i16;
                data = &data[4..];
            } else if flags & FLAG_WE_HAVE_A_TWO_BY_TWO != 0 {
                if data.len() < 8 {
                    return Err(ERR_INVALID_GLYPH_DATA);
                }
                elem.transform_xx = u16_at(data, 0) as i16;
                elem.transform_xy = u16_at(data, 2) as i16;
                elem.transform_yx = u16_at(data, 4) as i16;
                elem.transform_yy = u16_at(data, 6) as i16;
                data = &data[8..];
            }
        }
        if flags & FLAG_MORE_COMPONENTS == 0 {
            break;
        }
    }
    for i in stack_bottom..stack_top {
        let elem = stack[i as usize];
        let base = segments.len();
        load_glyf(
            f,
            segments,
            stack,
            elem.glyph_index,
            stack_top,
            recursion_depth,
        )?;
        let (dx, dy) = (i32::from(elem.dx), i32::from(elem.dy));
        for segment in &mut segments[base..] {
            if elem.has_transform {
                for arg in &mut segment.args {
                    *arg = tform(
                        elem.transform_xx,
                        elem.transform_xy,
                        elem.transform_yx,
                        elem.transform_yy,
                        dx,
                        dy,
                        *arg,
                    );
                }
            } else {
                for arg in &mut segment.args {
                    arg.x = arg.x.wrapping_add(dx);
                    arg.y = arg.y.wrapping_add(dy);
                }
            }
        }
    }
    Ok(())
}

fn tform(txx: i16, txy: i16, tyx: i16, tyy: i16, dx: i32, dy: i32, p: Point26_6) -> Point26_6 {
    const HALF: i64 = 1 << 13;
    Point26_6 {
        x: dx
            .wrapping_add(((i64::from(p.x) * i64::from(txx) + HALF) >> 14) as i32)
            .wrapping_add(((i64::from(p.y) * i64::from(tyx) + HALF) >> 14) as i32),
        y: dy
            .wrapping_add(((i64::from(p.x) * i64::from(txy) + HALF) >> 14) as i32)
            .wrapping_add(((i64::from(p.y) * i64::from(tyy) + HALF) >> 14) as i32),
    }
}

struct GlyfIter<'a> {
    data: &'a [u8],
    err: Option<Error>,
    flag_index: usize,
    x_index: usize,
    y_index: usize,
    end_index: usize,
    prev_end: i32,
    final_end: i32,
    c: i32,
    num_contours: i32,
    p: i32,
    n_points: i32,
    x: i16,
    y: i16,
    on: bool,
    flag: u8,
    repeats: u8,
    closing: bool,
    closed: bool,
    first_on_curve_valid: bool,
    first_off_curve_valid: bool,
    last_off_curve_valid: bool,
    first_on_curve: Point26_6,
    first_off_curve: Point26_6,
    last_off_curve: Point26_6,
    seg: Segment,
}

fn seg(op: SegmentOp, a: Point26_6, b: Point26_6) -> Segment {
    Segment {
        op,
        args: [a, b, Point26_6::default()],
    }
}

impl GlyfIter<'_> {
    fn next_contour(&mut self) -> bool {
        if self.c == self.num_contours {
            if self.prev_end != self.final_end {
                self.err = Some(ERR_INVALID_GLYPH_DATA);
            }
            return false;
        }
        self.c += 1;
        let end = i32::from(u16_at(self.data, self.end_index));
        self.end_index += 2;
        if end <= self.prev_end || self.final_end < end {
            self.err = Some(ERR_INVALID_GLYPH_DATA);
            return false;
        }
        self.n_points = end - self.prev_end;
        self.p = 0;
        self.prev_end = end;
        self.closing = false;
        self.closed = false;
        self.first_on_curve_valid = false;
        self.first_off_curve_valid = false;
        self.last_off_curve_valid = false;
        true
    }

    fn close(&mut self) {
        let zero = Point26_6::default();
        match (self.first_off_curve_valid, self.last_off_curve_valid) {
            (false, false) => {
                self.closed = true;
                self.seg = seg(SegmentOp::LineTo, self.first_on_curve, zero);
            }
            (false, true) => {
                self.closed = true;
                self.seg = seg(SegmentOp::QuadTo, self.last_off_curve, self.first_on_curve);
            }
            (true, false) => {
                self.closed = true;
                self.seg = seg(SegmentOp::QuadTo, self.first_off_curve, self.first_on_curve);
            }
            (true, true) => {
                self.last_off_curve_valid = false;
                self.seg = seg(
                    SegmentOp::QuadTo,
                    self.last_off_curve,
                    mid_point(self.last_off_curve, self.first_off_curve),
                );
            }
        }
    }

    fn next_segment(&mut self) -> bool {
        let zero = Point26_6::default();
        while !self.closed {
            if self.closing || !self.next_point() {
                self.closing = true;
                self.close();
                return true;
            }
            let p = Point26_6 {
                x: i32::from(self.x),
                y: i32::from(self.y),
            };
            if !self.first_on_curve_valid {
                if self.on {
                    self.first_on_curve = p;
                    self.first_on_curve_valid = true;
                    self.seg = seg(SegmentOp::MoveTo, p, zero);
                    return true;
                } else if !self.first_off_curve_valid {
                    self.first_off_curve = p;
                    self.first_off_curve_valid = true;
                    continue;
                } else {
                    self.first_on_curve = mid_point(self.first_off_curve, p);
                    self.first_on_curve_valid = true;
                    self.last_off_curve = p;
                    self.last_off_curve_valid = true;
                    self.seg = seg(SegmentOp::MoveTo, self.first_on_curve, zero);
                    return true;
                }
            } else if !self.last_off_curve_valid {
                if !self.on {
                    self.last_off_curve = p;
                    self.last_off_curve_valid = true;
                    continue;
                } else {
                    self.seg = seg(SegmentOp::LineTo, p, zero);
                    return true;
                }
            } else if !self.on {
                self.seg = seg(
                    SegmentOp::QuadTo,
                    self.last_off_curve,
                    mid_point(self.last_off_curve, p),
                );
                self.last_off_curve = p;
                self.last_off_curve_valid = true;
                return true;
            } else {
                self.seg = seg(SegmentOp::QuadTo, self.last_off_curve, p);
                self.last_off_curve_valid = false;
                return true;
            }
        }
        false
    }

    fn next_point(&mut self) -> bool {
        if self.p == self.n_points {
            return false;
        }
        self.p += 1;
        if self.repeats > 0 {
            self.repeats -= 1;
        } else {
            self.flag = self.data[self.flag_index];
            self.flag_index += 1;
            if self.flag & FLAG_REPEAT != 0 {
                self.repeats = self.data[self.flag_index];
                self.flag_index += 1;
            }
        }
        if self.flag & FLAG_X_SHORT_VECTOR != 0 {
            let delta = i16::from(self.data[self.x_index]);
            if self.flag & FLAG_POSITIVE_X_SHORT_VECTOR != 0 {
                self.x = self.x.wrapping_add(delta);
            } else {
                self.x = self.x.wrapping_sub(delta);
            }
            self.x_index += 1;
        } else if self.flag & FLAG_THIS_X_IS_SAME == 0 {
            self.x = self.x.wrapping_add(u16_at(self.data, self.x_index) as i16);
            self.x_index += 2;
        }
        if self.flag & FLAG_Y_SHORT_VECTOR != 0 {
            let delta = i16::from(self.data[self.y_index]);
            if self.flag & FLAG_POSITIVE_Y_SHORT_VECTOR != 0 {
                self.y = self.y.wrapping_add(delta);
            } else {
                self.y = self.y.wrapping_sub(delta);
            }
            self.y_index += 1;
        } else if self.flag & FLAG_THIS_Y_IS_SAME == 0 {
            self.y = self.y.wrapping_add(u16_at(self.data, self.y_index) as i16);
            self.y_index += 2;
        }
        self.on = self.flag & FLAG_ON_CURVE != 0;
        true
    }
}
