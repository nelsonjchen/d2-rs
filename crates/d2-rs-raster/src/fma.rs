// SPDX-License-Identifier: MPL-2.0
//! Go ARM64 floating-point contraction helpers and the d2scene matrix
//! operations as the pinned compiler actually emits them.
//!
//! Fusion is decided on SSA values. Sites were read from the pinned Go 1.27
//! darwin/arm64 assembly (`go build -gcflags=-S`), not inferred from source
//! shape: for `p*q + r*s` the compiler fuses whichever product the rewrite
//! rule matches, which differs between functions. `d2scene.Matrix.Point`,
//! `Mul` and `Vector` fuse their *first* product and round the second (both
//! standalone and when inlined into d2raster), so this crate uses the local
//! versions below instead of `d2_rs_scene::Matrix::{point, mul, vector}`.

pub use d2_rs_render::go_float::{go_fmadd, go_fmsub, go_fnmsub, gomath};
use d2_rs_scene::{Matrix, Point};

/// `a + x*y` as ARM64 `FMADDS` (single precision); separate ops elsewhere.
#[inline]
pub fn go_fmadd32(x: f32, y: f32, a: f32) -> f32 {
    #[cfg(target_arch = "aarch64")]
    {
        x.mul_add(y, a)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        a + x * y
    }
}

/// d2scene.Matrix.Point: `FMADDD` of `A*x` onto the rounded `C*y`, then `+E`.
#[inline]
pub fn mpoint(m: &Matrix, p: Point) -> Point {
    Point::new(go_fmadd(m.a, p.x, m.c * p.y) + m.e, go_fmadd(m.b, p.x, m.d * p.y) + m.f)
}

/// d2scene.Matrix.Vector.
#[inline]
pub fn mvector(m: &Matrix, v: Point) -> Point {
    Point::new(go_fmadd(m.a, v.x, m.c * v.y), go_fmadd(m.b, v.x, m.d * v.y))
}

/// d2scene.Matrix.Mul (geometry.go:214–219): the left product is fused.
#[inline]
pub fn mmul(m: &Matrix, r: &Matrix) -> Matrix {
    Matrix {
        a: go_fmadd(m.a, r.a, m.c * r.b),
        b: go_fmadd(m.b, r.a, m.d * r.b),
        c: go_fmadd(m.a, r.c, m.c * r.d),
        d: go_fmadd(m.b, r.c, m.d * r.d),
        e: go_fmadd(m.a, r.e, m.c * r.f) + m.e,
        f: go_fmadd(m.b, r.e, m.d * r.f) + m.f,
    }
}

pub const IDENTITY: Matrix = Matrix::identity();

pub fn translate(x: f64, y: f64) -> Matrix {
    Matrix::translate(x, y)
}

pub fn scale(x: f64, y: f64) -> Matrix {
    Matrix::scale(x, y)
}

pub fn finite(v: f64) -> bool {
    v.is_finite()
}

pub fn finite_point(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite()
}
