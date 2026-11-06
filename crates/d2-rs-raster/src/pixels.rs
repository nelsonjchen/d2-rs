// SPDX-License-Identifier: MPL-2.0
//! The subset of Go's `image` package that d2raster relies on: integer
//! rectangles (`image.Rectangle`, with Go's 64-bit `int` coordinates),
//! premultiplied `image.RGBA` buffers with `SubImage` views, and
//! `image.Alpha` masks. Offsets and clipping follow the Go definitions.

/// `image.Point`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Pt {
    pub x: i64,
    pub y: i64,
}

impl Pt {
    pub const fn new(x: i64, y: i64) -> Self {
        Self { x, y }
    }
}

/// `image.Rectangle`: half-open `[min, max)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub min: Pt,
    pub max: Pt,
}

impl Rect {
    /// `image.Rect`, which canonicalizes swapped corners.
    pub fn new(mut x0: i64, mut y0: i64, mut x1: i64, mut y1: i64) -> Self {
        if x0 > x1 {
            std::mem::swap(&mut x0, &mut x1);
        }
        if y0 > y1 {
            std::mem::swap(&mut y0, &mut y1);
        }
        Self {
            min: Pt::new(x0, y0),
            max: Pt::new(x1, y1),
        }
    }

    pub fn dx(&self) -> i64 {
        self.max.x.wrapping_sub(self.min.x)
    }

    pub fn dy(&self) -> i64 {
        self.max.y.wrapping_sub(self.min.y)
    }

    pub fn empty(&self) -> bool {
        self.min.x >= self.max.x || self.min.y >= self.max.y
    }

    pub fn intersect(&self, s: Rect) -> Rect {
        let mut r = *self;
        if r.min.x < s.min.x {
            r.min.x = s.min.x;
        }
        if r.min.y < s.min.y {
            r.min.y = s.min.y;
        }
        if r.max.x > s.max.x {
            r.max.x = s.max.x;
        }
        if r.max.y > s.max.y {
            r.max.y = s.max.y;
        }
        // Letting r0 and s0 be the values of r and s at the time that the
        // method is called, this next line is equivalent to
        // r0.Overlaps(s0) && r.Empty() in image.Rectangle.Intersect.
        if r.empty() {
            return Rect::default();
        }
        r
    }

    pub fn union(&self, s: Rect) -> Rect {
        if self.empty() {
            return s;
        }
        if s.empty() {
            return *self;
        }
        let mut r = *self;
        r.min.x = r.min.x.min(s.min.x);
        r.min.y = r.min.y.min(s.min.y);
        r.max.x = r.max.x.max(s.max.x);
        r.max.y = r.max.y.max(s.max.y);
        r
    }

    /// `r.In(s)`: every point of r is in s; an empty r is in every s.
    pub fn is_in(&self, s: Rect) -> bool {
        if self.empty() {
            return true;
        }
        s.min.x <= self.min.x
            && self.max.x <= s.max.x
            && s.min.y <= self.min.y
            && self.max.y <= s.max.y
    }

    pub fn contains(&self, p: Pt) -> bool {
        self.min.x <= p.x && p.x < self.max.x && self.min.y <= p.y && p.y < self.max.y
    }
}

/// d2raster's unionRect: the union that ignores empty rectangles.
pub fn union_rect(left: Rect, right: Rect) -> Rect {
    if left.empty() {
        return right;
    }
    if right.empty() {
        return left;
    }
    left.union(right)
}

/// An owned premultiplied `image.RGBA`.
#[derive(Clone, Debug, Default)]
pub struct RgbaBuf {
    pub pix: Vec<u8>,
    pub stride: usize,
    pub rect: Rect,
}

impl RgbaBuf {
    /// `image.NewRGBA`: zeroed pixels.
    pub fn new(rect: Rect) -> Self {
        let width = rect.dx().max(0) as usize;
        let height = rect.dy().max(0) as usize;
        Self {
            pix: vec![0; width * height * 4],
            stride: width * 4,
            rect,
        }
    }

    pub fn view(&mut self) -> Rgba<'_> {
        Rgba {
            pix: &mut self.pix,
            stride: self.stride,
            rect: self.rect,
        }
    }

    pub fn as_ref(&self) -> RgbaRef<'_> {
        RgbaRef {
            pix: &self.pix,
            stride: self.stride,
            rect: self.rect,
        }
    }
}

/// A mutable `*image.RGBA` (possibly a SubImage view into a larger buffer).
pub struct Rgba<'a> {
    pub pix: &'a mut [u8],
    pub stride: usize,
    pub rect: Rect,
}

/// A shared `*image.RGBA` view.
#[derive(Clone, Copy)]
pub struct RgbaRef<'a> {
    pub pix: &'a [u8],
    pub stride: usize,
    pub rect: Rect,
}

impl<'a> Rgba<'a> {
    pub fn bounds(&self) -> Rect {
        self.rect
    }

    pub fn pix_offset(&self, x: i64, y: i64) -> usize {
        ((y - self.rect.min.y) as usize) * self.stride + ((x - self.rect.min.x) as usize) * 4
    }

    /// `SubImage`: a view sharing the pixels.
    pub fn sub_image(&mut self, r: Rect) -> Rgba<'_> {
        let r = r.intersect(self.rect);
        if r.empty() {
            return Rgba {
                pix: &mut [],
                stride: self.stride,
                rect: Rect::default(),
            };
        }
        let i = self.pix_offset(r.min.x, r.min.y);
        Rgba {
            pix: &mut self.pix[i..],
            stride: self.stride,
            rect: r,
        }
    }

    pub fn reborrow(&mut self) -> Rgba<'_> {
        Rgba {
            pix: self.pix,
            stride: self.stride,
            rect: self.rect,
        }
    }

    pub fn as_ref(&self) -> RgbaRef<'_> {
        RgbaRef {
            pix: self.pix,
            stride: self.stride,
            rect: self.rect,
        }
    }
}

impl<'a> RgbaRef<'a> {
    pub fn bounds(&self) -> Rect {
        self.rect
    }

    pub fn pix_offset(&self, x: i64, y: i64) -> usize {
        ((y - self.rect.min.y) as usize) * self.stride + ((x - self.rect.min.x) as usize) * 4
    }

    pub fn sub_image(&self, r: Rect) -> RgbaRef<'a> {
        let r = r.intersect(self.rect);
        if r.empty() {
            return RgbaRef {
                pix: &[],
                stride: self.stride,
                rect: Rect::default(),
            };
        }
        let i = self.pix_offset(r.min.x, r.min.y);
        RgbaRef {
            pix: &self.pix[i..],
            stride: self.stride,
            rect: r,
        }
    }
}

/// An owned `image.Alpha`.
#[derive(Clone, Debug, Default)]
pub struct AlphaBuf {
    pub pix: Vec<u8>,
    pub stride: usize,
    pub rect: Rect,
}

impl AlphaBuf {
    pub fn new(rect: Rect) -> Self {
        let width = rect.dx().max(0) as usize;
        let height = rect.dy().max(0) as usize;
        Self {
            pix: vec![0; width * height],
            stride: width,
            rect,
        }
    }

    pub fn bounds(&self) -> Rect {
        self.rect
    }

    pub fn pix_offset(&self, x: i64, y: i64) -> usize {
        ((y - self.rect.min.y) as usize) * self.stride + (x - self.rect.min.x) as usize
    }
}

/// The returned frame: straight-alpha `image.NRGBA` pixels.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NrgbaImage {
    pub pix: Vec<u8>,
    pub stride: usize,
    pub rect: Rect,
}

impl NrgbaImage {
    pub fn width(&self) -> usize {
        self.rect.dx() as usize
    }

    pub fn height(&self) -> usize {
        self.rect.dy() as usize
    }
}
