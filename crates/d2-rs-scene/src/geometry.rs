// SPDX-License-Identifier: MPL-2.0
//! d2scene/geometry.go: ARM64 contraction follows geometry.go:214–272.

use d2_rs_render::go_float::{go_fmadd, go_fmsub, gomath};
use serde::{Deserialize, Serialize};

/// A point or vector in logical D2 coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Origin and size; width and height must not be negative in a valid scene.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Box {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Min/max bounds; `valid` distinguishes the empty set from a zero-area bound.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Bounds {
    pub min: Point,
    pub max: Point,
    pub valid: bool,
}

pub(crate) fn finite(v: f64) -> bool {
    v.is_finite()
}

pub(crate) fn finite_point(p: Point) -> bool {
    finite(p.x) && finite(p.y)
}

impl Bounds {
    pub fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self {
            min: Point::new(go_min(x0, x1), go_min(y0, y1)),
            max: Point::new(go_max(x0, x1), go_max(y0, y1)),
            valid: true,
        }
    }

    pub fn from_points(points: &[Point]) -> Self {
        points.iter().fold(Self::default(), |b, p| b.include(*p))
    }

    pub fn width(&self) -> f64 {
        if self.valid {
            self.max.x - self.min.x
        } else {
            0.0
        }
    }

    pub fn height(&self) -> f64 {
        if self.valid {
            self.max.y - self.min.y
        } else {
            0.0
        }
    }

    pub fn to_box(&self) -> Box {
        if !self.valid {
            return Box::default();
        }
        Box {
            x: self.min.x,
            y: self.min.y,
            width: self.width(),
            height: self.height(),
        }
    }

    pub fn include(mut self, p: Point) -> Self {
        if !self.valid {
            return Self {
                min: p,
                max: p,
                valid: true,
            };
        }
        self.min.x = go_min(self.min.x, p.x);
        self.min.y = go_min(self.min.y, p.y);
        self.max.x = go_max(self.max.x, p.x);
        self.max.y = go_max(self.max.y, p.y);
        self
    }

    pub fn union(self, other: Self) -> Self {
        if !self.valid {
            return other;
        }
        if !other.valid {
            return self;
        }
        Self::new(
            go_min(self.min.x, other.min.x),
            go_min(self.min.y, other.min.y),
            go_max(self.max.x, other.max.x),
            go_max(self.max.y, other.max.y),
        )
    }

    pub fn intersect(self, other: Self) -> Self {
        if !self.valid || !other.valid {
            return Self::default();
        }
        let min_x = go_max(self.min.x, other.min.x);
        let min_y = go_max(self.min.y, other.min.y);
        let max_x = go_min(self.max.x, other.max.x);
        let max_y = go_min(self.max.y, other.max.y);
        if min_x > max_x || min_y > max_y {
            return Self::default();
        }
        Self::new(min_x, min_y, max_x, max_y)
    }

    pub fn expand(self, x: f64, y: f64) -> Self {
        if !self.valid {
            return self;
        }
        let (min_x, min_y) = (self.min.x - x, self.min.y - y);
        let (max_x, max_y) = (self.max.x + x, self.max.y + y);
        if min_x > max_x || min_y > max_y {
            return Self::default();
        }
        Self::new(min_x, min_y, max_x, max_y)
    }

    pub fn translate(self, v: Point) -> Self {
        if !self.valid {
            return self;
        }
        Self {
            min: Point::new(self.min.x + v.x, self.min.y + v.y),
            max: Point::new(self.max.x + v.x, self.max.y + v.y),
            valid: true,
        }
    }

    pub fn is_finite(&self) -> bool {
        !self.valid || (finite_point(self.min) && finite_point(self.max))
    }

    /// Axis-aligned bounds of the transformed corners.
    pub fn transform(self, m: Matrix) -> Self {
        if !self.valid {
            return self;
        }
        Self::from_points(&[
            m.point(self.min),
            m.point(Point::new(self.max.x, self.min.y)),
            m.point(self.max),
            m.point(Point::new(self.min.x, self.max.y)),
        ])
    }
}

impl Box {
    pub fn bounds(&self) -> Bounds {
        Bounds::new(self.x, self.y, self.x + self.width, self.y + self.height)
    }
}

/// Go's math.Min: NaN propagates, and -0 is less than +0.
pub fn go_min(x: f64, y: f64) -> f64 {
    if x.is_infinite() && x < 0.0 || y.is_infinite() && y < 0.0 {
        return f64::NEG_INFINITY;
    }
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    if x == 0.0 && x == y {
        return if x.is_sign_negative() { x } else { y };
    }
    if x < y { x } else { y }
}

/// Go's math.Max: NaN propagates, and +0 is greater than -0.
pub fn go_max(x: f64, y: f64) -> f64 {
    if x.is_infinite() && x > 0.0 || y.is_infinite() && y > 0.0 {
        return f64::INFINITY;
    }
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    if x == 0.0 && x == y {
        return if x.is_sign_negative() { y } else { x };
    }
    if x > y { x } else { y }
}

/// SVG-style affine transform: x' = A*x + C*y + E, y' = B*x + D*y + F.
/// The zero value is the zero transform, not identity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Matrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Matrix {
    pub const fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    pub const fn translate(x: f64, y: f64) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: x,
            f: y,
        }
    }

    pub const fn scale(x: f64, y: f64) -> Self {
        Self {
            a: x,
            b: 0.0,
            c: 0.0,
            d: y,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Counter-clockwise rotation in Cartesian coordinates (clockwise in D2's
    /// screen coordinates, as in SVG).
    pub fn rotate(radians: f64) -> Self {
        let (s, c) = (gomath::sin(radians), gomath::cos(radians));
        Self {
            a: c,
            b: s,
            c: -s,
            d: c,
            e: 0.0,
            f: 0.0,
        }
    }

    pub fn rotate_around(radians: f64, x: f64, y: f64) -> Self {
        Self::translate(x, y)
            .mul(Self::rotate(radians))
            .mul(Self::translate(-x, -y))
    }

    pub fn skew_x(radians: f64) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: go_tan(radians),
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    pub fn skew_y(radians: f64) -> Self {
        Self {
            a: 1.0,
            b: go_tan(radians),
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Composes self and right; right applies to a point first.
    pub fn mul(self, r: Self) -> Self {
        let m = self;
        // ARM64 geometry.go:214–219: round the second product with FMULD,
        // then fuse the first with FMADDD. Translation is a separate FADDD.
        Self {
            a: go_fmadd(m.a, r.a, m.c * r.b),
            b: go_fmadd(m.b, r.a, m.d * r.b),
            c: go_fmadd(m.a, r.c, m.c * r.d),
            d: go_fmadd(m.b, r.c, m.d * r.d),
            e: go_fmadd(m.a, r.e, m.c * r.f) + m.e,
            f: go_fmadd(m.b, r.e, m.d * r.f) + m.f,
        }
    }

    pub fn point(&self, p: Point) -> Point {
        // ARM64 geometry.go:225–226.
        Point::new(
            go_fmadd(self.a, p.x, self.c * p.y) + self.e,
            go_fmadd(self.b, p.x, self.d * p.y) + self.f,
        )
    }

    /// Transforms a vector, excluding translation.
    pub fn vector(&self, v: Point) -> Point {
        // ARM64 geometry.go:233–234.
        Point::new(
            go_fmadd(self.a, v.x, self.c * v.y),
            go_fmadd(self.b, v.x, self.d * v.y),
        )
    }

    pub fn determinant(&self) -> f64 {
        // ARM64 geometry.go:239: FMSUBD.
        go_fmsub(self.b, self.c, self.a * self.d)
    }

    pub fn inverse(&self) -> Result<Self, String> {
        let det = self.determinant();
        if det == 0.0 || !finite(det) {
            return Err("d2scene: singular transform".into());
        }
        let m = self;
        // ARM64 geometry.go:252–253.
        Ok(Self {
            a: m.d / det,
            b: -m.b / det,
            c: -m.c / det,
            d: m.a / det,
            e: go_fmsub(m.d, m.e, m.c * m.f) / det,
            f: go_fmsub(m.a, m.f, m.b * m.e) / det,
        })
    }

    pub fn is_finite(&self) -> bool {
        [self.a, self.b, self.c, self.d, self.e, self.f]
            .iter()
            .all(|v| finite(*v))
    }

    /// Largest singular value of the linear part.
    pub fn max_scale(&self) -> f64 {
        let maximum = go_max(
            go_max(self.a.abs(), self.b.abs()),
            go_max(self.c.abs(), self.d.abs()),
        );
        if maximum == 0.0 || maximum.is_infinite() || maximum.is_nan() {
            return maximum;
        }
        let (a, b, c, d) = (
            self.a / maximum,
            self.b / maximum,
            self.c / maximum,
            self.d / maximum,
        );
        // ARM64 geometry.go:270–272.
        let trace = go_fmadd(d, d, go_fmadd(c, c, go_fmadd(b, b, a * a)));
        let det = go_fmsub(b, c, a * d);
        let disc = go_max(0.0, go_fmsub(4.0 * det, det, trace * trace));
        maximum * go_max(0.0, (trace + disc.sqrt()) / 2.0).sqrt()
    }
}

/// Go `math.Tan`.
fn go_tan(x: f64) -> f64 {
    gomath::tan(x)
}

// These exact bits come from the pinned darwin/arm64 oracle. Other targets
// deliberately use separate arithmetic in go_fmadd and need their own oracle.
#[cfg(all(test, target_arch = "aarch64"))]
mod tests {
    use super::{Matrix, Point};

    #[test]
    fn cross_arrowhead_matrix_rounding_matches_go() {
        // stable/cross_arrowhead rotates the center of a 17x17 marker.
        // Pinned Go's independent Mul/Point/Vector call sites produce these
        // exact bits (run45-matrix.go); fusing the other product differs.
        let matrix = Matrix::rotate(std::f64::consts::PI / 4.0);
        let origin = Point::new(8.5, 8.5);
        let point = matrix.point(origin);
        let vector = matrix.vector(origin);
        let product = matrix.mul(Matrix {
            a: 8.5,
            b: 8.5,
            c: 8.5,
            d: 8.5,
            e: 8.5,
            f: 8.5,
        });
        let actual = [
            point.x, point.y, vector.x, vector.y, product.a, product.b, product.c, product.d,
            product.e, product.f,
        ];
        for (index, value) in actual.into_iter().enumerate() {
            let expected = if index % 2 == 0 {
                4_380_313_587_571_228_672
            } else {
                4_622_956_735_456_817_033
            };
            assert_eq!(value.to_bits(), expected, "component {index}");
        }
        assert_eq!(-point.x + 8.5, 8.5);
    }
}
