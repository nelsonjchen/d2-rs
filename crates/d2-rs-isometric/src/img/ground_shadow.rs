// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg/ground_shadow.go: receiver-height bands and world-space softness.
use super::*;

pub(crate) const LAYERS: [(f64, f64, f64); 3] = [
    (f64::INFINITY, 0.085, 0.079),
    (0.65, 0.062, 0.117),
    (0.20, 0.034, 0.170),
];
#[derive(Default)]
pub(super) struct GroundShadow {
    camera: Camera,
    alpha: std::vec::Vec<u16>,
    size: usize,
    scale: f64,
}
struct Mask {
    alpha: Pixels,
    size: usize,
    scale: f64,
    weight: f64,
}
pub(crate) fn grid(light: Camera, sigma: f64) -> (f64, usize) {
    let radius = ((sigma * light.scale * 6.0f64.sqrt()).floor() as i64 - 1).clamp(1, 4) as usize;
    (
        go_min(light.scale, (radius + 1) as f64 / (6.0f64.sqrt() * sigma)),
        radius,
    )
}
pub(super) fn build(
    work: &mut Work,
    ts: &[Triangle],
    ground: f64,
    light: Camera,
) -> Result<GroundShadow, String> {
    let mut shadow = GroundShadow {
        camera: light,
        ..Default::default()
    };
    let receiver = ground + 0.025;
    let min_y = go_fmadd(
        go_max(0.012, 2.0 / light.scale),
        light.direction.y,
        receiver,
    );
    let mut layers = vec![];
    let mut span = 0.0;
    for (height, sigma, opacity) in LAYERS {
        let (scale, radius) = grid(light, sigma);
        let size =
            (light.width as f64 * scale / light.scale).ceil() as usize + 2 * (2 * radius + 2);
        let camera = Camera {
            scale,
            width: size as i64,
            height: size as i64,
            ..light
        };
        span = go_max(span, size as f64 / scale);
        shadow.scale = go_max(shadow.scale, scale);
        let mut atlas = rasterize_shadow(ts, work, camera, min_y, receiver + height)?;
        if atlas.opacity.is_empty() {
            continue;
        }
        blur(work, &mut atlas.opacity, size, radius)?;
        layers.push(Mask {
            alpha: atlas.opacity,
            size,
            scale,
            weight: opacity / 255.0,
        });
    }
    if layers.is_empty() {
        return Ok(shadow);
    }
    shadow.size = (light.width as usize).min((span * shadow.scale).ceil() as usize);
    shadow.scale = shadow.size as f64 / span;
    shadow.alpha = vec![0; shadow.size * shadow.size];
    work.charge(shadow.alpha.len() as i64 * layers.len() as i64 * 4)?;
    for y in 0..shadow.size {
        if y % 32 == 0 {
            work.check()?;
        }
        let v = (y as f64 - (shadow.size - 1) as f64 / 2.0) / shadow.scale;
        for x in 0..shadow.size {
            let u = (x as f64 - (shadow.size - 1) as f64 / 2.0) / shadow.scale;
            let mut remaining = 1.0;
            for m in &layers {
                remaining *= go_fmsub(
                    m.weight,
                    bilinear(&m.alpha, m.size, u * m.scale, v * m.scale),
                    1.0,
                );
            }
            shadow.alpha[y * shadow.size + x] = ((1.0 - remaining) * 65535.0).round() as u16;
        }
    }
    Ok(shadow)
}
fn blur(work: &mut Work, alpha: &mut Pixels, size: usize, radius: usize) -> Result<(), String> {
    work.charge(size as i64 * size as i64 * (2 * (2 * radius + 1)) as i64)?;
    let mut scratch = vec![0; alpha.len()];
    let divisor = (radius + 1) * (radius + 1);
    for step in [1, size] {
        for y in radius..size - radius {
            if y % 32 == 0 {
                work.check()?;
            }
            for x in radius..size - radius {
                let at = y * size + x;
                let mut sum = (radius + 1) * alpha[at] as usize;
                for k in 1..=radius {
                    sum += (radius + 1 - k)
                        * (alpha[at - k * step] as usize + alpha[at + k * step] as usize);
                }
                scratch[at] = ((sum + divisor / 2) / divisor) as u8;
            }
        }
        std::mem::swap(alpha, &mut scratch);
    }
    Ok(())
}
impl GroundShadow {
    pub(super) fn coverage(&self, mut p: Vec) -> f64 {
        p.y += 0.025;
        let x = (ndot(p, self.camera.right) - self.camera.center_x) * self.scale;
        let y = (self.camera.center_y - ndot(p, self.camera.up)) * self.scale;
        bilinear(&self.alpha, self.size, x, y) / 65535.0
    }
}
fn bilinear<T: Copy + Into<f64>>(alpha: &[T], size: usize, x: f64, y: f64) -> f64 {
    if size < 2 {
        return 0.0;
    }
    let (u, v) = (x + (size - 1) as f64 / 2.0, y + (size - 1) as f64 / 2.0);
    if u < 0.0 || v < 0.0 || u >= (size - 1) as f64 || v >= (size - 1) as f64 {
        return 0.0;
    }
    let (i, j) = (u as usize, v as usize);
    let (fx, fy) = (u - i as f64, v - j as f64);
    let at = j * size + i;
    let top = go_fmadd(fx, alpha[at + 1].into(), (1.0 - fx) * alpha[at].into());
    let bottom = go_fmadd(
        fx,
        alpha[at + size + 1].into(),
        (1.0 - fx) * alpha[at + size].into(),
    );
    go_fmadd(fy, bottom, (1.0 - fy) * top)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shadow_falls_off_from_caster_foot() {
        // Upstream b1f83e43c TestGroundShadowFallsOffFromCasterFoot.
        let build_wall = |height| {
            let mut b = super::super::super::geometry_core::MeshGeometry::default();
            let mat = material(Material {
                color: Nrgba::new(255, 255, 255, 255),
                ..Default::default()
            });
            b.flat(
                nv(-1.0, 0.0, 0.0),
                nv(1.0, 0.0, 0.0),
                nv(1.0, height, 0.0),
                &mat,
                true,
            );
            b.flat(
                nv(-1.0, 0.0, 0.0),
                nv(1.0, height, 0.0),
                nv(-1.0, height, 0.0),
                &mat,
                true,
            );
            let light = raster_fit(&b.triangles, raster_shadow_direction(), 2048, 2048, 1.08);
            build(&mut Work::new(), &b.triangles, 0.0, light).unwrap()
        };
        let tall = build_wall(1.3);
        let near = tall.coverage(nv(0.0, 0.0, -0.06));
        let middle = tall.coverage(nv(0.0, 0.0, -0.22));
        let end = tall.coverage(nv(0.0, 0.0, -0.52));
        assert!(
            near >= 0.21
                && middle >= 0.12
                && end >= 0.06
                && near - middle >= 0.05
                && middle - end >= 0.04,
            "{near} {middle} {end}"
        );
        let tail = tall.coverage(nv(0.0, 0.0, -0.78));
        assert!(tail >= 0.005 && tail < end / 2.0);
        let low = build_wall(0.2);
        assert!(low.coverage(nv(0.0, 0.0, -0.05)) >= 0.15);
        assert!(low.coverage(nv(0.0, 0.0, -0.30)) <= 0.01);
    }
    #[test]
    fn texture_alpha_is_retained_across_height_bands() {
        let texture = Rc::new(Texture::with_storage(
            3,
            1,
            12,
            TextureFormat::Nrgba,
            vec![0, 0, 0, 255, 0, 0, 0, 64, 0, 0, 0, 0],
        ));
        let mat = material(Material {
            color: Nrgba::new(0, 0, 0, 255),
            texture: Some(texture),
            ..Default::default()
        });
        let light = raster_shadow_direction();
        let vertex = |x, z, u, v| Vertex {
            position: nadd(nv(x, 0.025, z), nmul(light, 0.16 / light.y)),
            normal: nv(0.0, 1.0, 0.0),
            u,
            v,
        };
        let (a, b, c, d) = (
            vertex(-3.0, -1.0, 0.0, 0.0),
            vertex(3.0, -1.0, 1.0, 0.0),
            vertex(3.0, 1.0, 1.0, 1.0),
            vertex(-3.0, 1.0, 0.0, 1.0),
        );
        let ts = [
            Triangle {
                v: [a, b, c],
                material: Some(mat.clone()),
                cast_shadow: true,
                ..Default::default()
            },
            Triangle {
                v: [a, c, d],
                material: Some(mat),
                cast_shadow: true,
                ..Default::default()
            },
        ];
        let shadow = build(
            &mut Work::new(),
            &ts,
            0.0,
            raster_fit(&ts, light, 2048, 2048, 1.08),
        )
        .unwrap();
        for (i, alpha) in [1.0, 64.0 / 255.0, 0.0].into_iter().enumerate() {
            let transmission = LAYERS.iter().fold(1.0, |t, l| t * (1.0 - l.2 * alpha));
            let got = shadow.coverage(nv(-2.0 + i as f64 * 2.0, 0.0, 0.0));
            assert!((got - (1.0 - transmission)).abs() < 0.01, "band {i}: {got}");
        }
    }
    #[test]
    fn level_caps_on_band_ceiling_keep_full_coverage() {
        for (height, _, _) in &LAYERS[1..] {
            let height = 0.025 + height;
            let ts = [Triangle {
                v: [
                    Vertex {
                        position: nv(-2.1, height, 3.4),
                        ..Default::default()
                    },
                    Vertex {
                        position: nv(-1.4, height, -0.7),
                        ..Default::default()
                    },
                    Vertex {
                        position: nv(2.8, height, -0.3),
                        ..Default::default()
                    },
                ],
                cast_shadow: true,
                ..Default::default()
            }];
            let light = raster_fit(&ts, raster_shadow_direction(), 128, 128, 1.08);
            let full = rasterize_shadow(&ts, &mut Work::new(), light, 0.0, f64::INFINITY).unwrap();
            let capped = rasterize_shadow(&ts, &mut Work::new(), light, 0.0, height).unwrap();
            assert!(full.opacity.contains(&255));
            assert_eq!(full.opacity, capped.opacity);
        }
    }
}
