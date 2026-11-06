// SPDX-License-Identifier: BSD-3-Clause
// Copyright 2016 The Go Authors. All rights reserved.
//! x/image/vector v0.45.0: the plain-label rasterizer, not d2raster's
//! high-precision scene scan converter. Float products round before addition.
use d2_rs_scene::{Nrgba, Path, PathCommandKind};

struct Raster {
    w: usize,
    h: usize,
    fixed: Vec<u32>,
    float: Vec<f32>,
    pen: (f32, f32),
    first: (f32, f32),
}
fn dev(a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> f32 {
    let x = a.0 - 2.0 * b.0 + c.0;
    let y = a.1 - 2.0 * b.1 + c.1;
    // ARM64 vector.go:260 rounds X squared and fuses Y squared.
    #[cfg(not(target_arch = "aarch64"))]
    {
        y * y + x * x
    }
    #[cfg(target_arch = "aarch64")]
    {
        y.mul_add(y, x * x)
    }
}
fn lerp(t: f32, p: (f32, f32), q: (f32, f32)) -> (f32, f32) {
    // ARM64 vector.go:52 fuses every de Casteljau interpolation.
    #[cfg(not(target_arch = "aarch64"))]
    {
        (t * (q.0 - p.0) + p.0, t * (q.1 - p.1) + p.1)
    }
    #[cfg(target_arch = "aarch64")]
    {
        (t.mul_add(q.0 - p.0, p.0), t.mul_add(q.1 - p.1, p.1))
    }
}
impl Raster {
    fn add_fixed(&mut self, y: i32, x: i32, value: i32) {
        let i = y as usize * self.w + x.clamp(0, self.w as i32) as usize;
        if let Some(v) = self.fixed.get_mut(i) {
            *v = v.wrapping_add(value as u32);
        }
    }
    fn add_float(&mut self, y: i32, x: i32, value: f32) {
        let i = y as usize * self.w + x.clamp(0, self.w as i32) as usize;
        if let Some(v) = self.float.get_mut(i) {
            *v += value;
        }
    }
    fn line(&mut self, next: (f32, f32)) {
        let ((mut ax, mut ay), (mut bx, mut by)) = (self.pen, next);
        self.pen = next;
        let mut direction = 1;
        if ay > by {
            direction = -1;
            std::mem::swap(&mut ax, &mut bx);
            std::mem::swap(&mut ay, &mut by);
        }
        if by - ay <= 0.000001 {
            return;
        }
        let slope = (bx - ax) / (by - ay);
        if !self.fixed.is_empty() {
            let (ay, by) = ((ay * 512.0) as i32, (by * 512.0) as i32);
            let mut x = (ax * 512.0) as i32;
            for y in (ay >> 9)..((by + 511) >> 9).min(self.h as i32) {
                let dy = ((y + 1) << 9).min(by) - (y << 9).max(ay);
                let next = x + (dy as f32 * slope) as i32;
                if y < 0 {
                    x = next;
                    continue;
                }
                let d = dy * direction;
                let (lo, hi) = (x.min(next), x.max(next));
                let (i, j) = (lo >> 9, (hi + 511) >> 9);
                let (floor, ceil) = (i << 9, j << 9);
                if j <= i + 1 {
                    let middle = ((x + next) >> 1) - floor;
                    self.add_fixed(y, i, d * (512 - middle));
                    self.add_fixed(y, i + 1, d * middle);
                } else {
                    let span = hi - lo;
                    let twice = 2 * span;
                    let f0 = lo - floor;
                    let square0 = (512 - f0) * (512 - f0);
                    let f1 = hi - ceil + 512;
                    let square1 = f1 * f1;
                    self.add_fixed(y, i, square0.wrapping_mul(d) / twice);
                    if j == i + 2 {
                        self.add_fixed(
                            y,
                            i + 1,
                            ((twice << 9) - square0 - square1).wrapping_mul(d) / twice,
                        );
                    } else {
                        self.add_fixed(
                            y,
                            i + 1,
                            (((768 - f0) << 10) - square0).wrapping_mul(d) / twice,
                        );
                        let step = (d << 18) / span;
                        for xi in i + 2..j - 1 {
                            self.add_fixed(y, xi, step);
                        }
                        self.add_fixed(
                            y,
                            j - 1,
                            ((((f1 << 1) + 512) << 9) - square1).wrapping_mul(d) / twice,
                        );
                    }
                    self.add_fixed(y, j, square1.wrapping_mul(d) / twice);
                }
                x = next;
            }
        } else {
            let mut x = ax;
            for y in (ay.floor() as i32)..(by.ceil() as i32).min(self.h as i32) {
                let dy = ((y + 1) as f32).min(by) - (y as f32).max(ay);
                let next = x + dy * slope;
                if y < 0 {
                    x = next;
                    continue;
                }
                let d = dy * direction as f32;
                let (lo, hi) = (x.min(next), x.max(next));
                let (i, j) = (lo.floor() as i32, hi.ceil() as i32);
                if j <= i + 1 {
                    let middle = 0.5 * (x + next) - i as f32;
                    self.add_float(y, i, d - d * middle);
                    self.add_float(y, i + 1, d * middle);
                } else {
                    let s = 1.0 / (hi - lo);
                    let f0 = lo - i as f32;
                    let a0 = 0.5 * s * (1.0 - f0) * (1.0 - f0);
                    let f1 = hi - j as f32 + 1.0;
                    let am = 0.5 * s * f1 * f1;
                    self.add_float(y, i, d * a0);
                    if j == i + 2 {
                        self.add_float(y, i + 1, d * (1.0 - a0 - am));
                    } else {
                        let a1 = s * (1.5 - f0);
                        self.add_float(y, i + 1, d * (a1 - a0));
                        for xi in i + 2..j - 1 {
                            self.add_float(y, xi, d * s);
                        }
                        let a2 = a1 + s * (j - i - 3) as f32;
                        self.add_float(y, j - 1, d * (1.0 - a2 - am));
                    }
                    self.add_float(y, j, d * am);
                }
                x = next;
            }
        }
    }
    fn quad(&mut self, b: (f32, f32), c: (f32, f32)) {
        let a = self.pen;
        let deviation = dev(a, b, c);
        if deviation >= 0.333 {
            let n = 1 + (3.0 * deviation as f64).sqrt().sqrt() as usize;
            let (mut t, step) = (0.0, 1.0 / n as f32);
            for _ in 0..n - 1 {
                t += step;
                self.line(lerp(t, lerp(t, a, b), lerp(t, b, c)));
            }
        }
        self.line(c);
    }
    fn cube(&mut self, b: (f32, f32), c: (f32, f32), d: (f32, f32)) {
        let a = self.pen;
        let deviation = dev(a, b, d).max(dev(a, c, d));
        if deviation >= 0.333 {
            let n = 1 + (3.0 * deviation as f64).sqrt().sqrt() as usize;
            let (mut t, step) = (0.0, 1.0 / n as f32);
            for _ in 0..n - 1 {
                t += step;
                self.line(lerp(
                    t,
                    lerp(t, lerp(t, a, b), lerp(t, b, c)),
                    lerp(t, lerp(t, b, c), lerp(t, c, d)),
                ));
            }
        }
        self.line(d);
    }
}
/// Draw one aggregate glyph outline with image.Uniform and draw.Over.
pub fn draw(
    path: &Path,
    w: usize,
    h: usize,
    foreground: Nrgba,
    background: Option<Nrgba>,
) -> Vec<u8> {
    let mut raster = Raster {
        w,
        h,
        fixed: if w <= 512 && h <= 512 {
            vec![0; w * h]
        } else {
            vec![]
        },
        float: if w > 512 || h > 512 {
            vec![0.0; w * h]
        } else {
            vec![]
        },
        pen: (0.0, 0.0),
        first: (0.0, 0.0),
    };
    for cmd in &path.commands {
        let p = |p: d2_rs_scene::Point| (p.x as f32, p.y as f32);
        match cmd.kind {
            PathCommandKind::MOVE => {
                raster.pen = p(cmd.p1);
                raster.first = raster.pen;
            }
            PathCommandKind::LINE => raster.line(p(cmd.p1)),
            PathCommandKind::QUADRATIC => raster.quad(p(cmd.p1), p(cmd.p2)),
            PathCommandKind::CUBIC => raster.cube(p(cmd.p1), p(cmd.p2), p(cmd.p3)),
            PathCommandKind::CLOSE => raster.line(raster.first),
            _ => unreachable!("glyph outline contains an arc"),
        }
    }
    let premul = |c: Nrgba| {
        let a = c.a as u32 * 257;
        [
            c.r as u32 * 257 * a / 65535,
            c.g as u32 * 257 * a / 65535,
            c.b as u32 * 257 * a / 65535,
            a,
        ]
    };
    let bg = premul(background.unwrap_or_default());
    let fg = premul(foreground);
    let mut pixels = vec![0; w * h * 4];
    #[cfg(target_arch = "x86_64")]
    let sse41 = std::is_x86_feature_detected!("sse4.1");
    // x/image/vector's amd64 SSE4.1 mask uses a four-wide prefix sum.
    // Its grouping changes float32 rounding compared with the scalar path.
    #[cfg(target_arch = "x86_64")]
    if !raster.float.is_empty() && sse41 {
        let mut offset = 0.0f32;
        let mut chunks = raster.float.chunks_exact_mut(4);
        for chunk in &mut chunks {
            let [a, b, c, d] = [chunk[0], chunk[1], chunk[2], chunk[3]];
            let ab = b + a;
            let bc = c + b;
            let cd = d + c;
            chunk[0] = a + offset;
            chunk[1] = ab + offset;
            chunk[2] = (bc + a) + offset;
            chunk[3] = (cd + ab) + offset;
            offset = chunk[3];
        }
        for value in chunks.into_remainder() {
            offset += *value;
            *value = offset;
        }
    }
    let (mut fixed, mut float) = (0i32, 0.0f32);
    for (i, pixel) in pixels.chunks_exact_mut(4).enumerate() {
        let mask = if !raster.fixed.is_empty() {
            fixed = fixed.wrapping_add(raster.fixed[i] as i32);
            (fixed.wrapping_abs() >> 2).min(65535) as u32
        } else {
            #[cfg(target_arch = "x86_64")]
            if sse41 {
                float = raster.float[i];
            } else {
                float += raster.float[i];
            }
            #[cfg(not(target_arch = "x86_64"))]
            {
                float += raster.float[i];
            }
            (f32::from_bits(0x477fffff) * float.abs().min(1.0)) as u32
        };
        let inverse = 65535 - fg[3] * mask / 65535;
        for j in 0..4 {
            pixel[j] = ((((bg[j] >> 8) * 257 * inverse + fg[j] * mask) / 65535) >> 8) as u8;
        }
    }
    pixels
}
