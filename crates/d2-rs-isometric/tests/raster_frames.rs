// SPDX-License-Identifier: MPL-2.0
//! Original raster_test.go depth/frame contracts. These use the native
//! complete-render API; prepared-buffer caching is a separate API concern.
use d2_rs_core::CancellationToken;
use d2_rs_isometric::img::{context::RenderContext, mesh::*, raster};
use d2_rs_scene::Nrgba;

fn quad(depth: f64, color: Nrgba) -> std::vec::Vec<Triangle> {
    let view = raster_unit(native_view_direction());
    let right = raster_unit(ncross(nv(0.0, 1.0, 0.0), view));
    let up = raster_unit(ncross(view, right));
    let vertex = |x, y, u, v| Vertex {
        position: nadd(nadd(nmul(right, x), nmul(up, y)), nmul(view, depth)),
        normal: view,
        u,
        v,
    };
    let (a, b, c, d) = (
        vertex(-1.0, 1.0, 0.0, 0.0),
        vertex(1.0, 1.0, 1.0, 0.0),
        vertex(1.0, -1.0, 1.0, 1.0),
        vertex(-1.0, -1.0, 0.0, 1.0),
    );
    let paint = material(Material {
        color,
        unlit: true,
        ..Default::default()
    });
    vec![
        Triangle {
            v: [a, b, c],
            material: Some(paint.clone()),
            ..Default::default()
        },
        Triangle {
            v: [a, c, d],
            material: Some(paint),
            ..Default::default()
        },
    ]
}

#[test]
fn dynamic_frames_keep_static_camera_depth_and_owned_output() {
    let green = Nrgba::new(0, 255, 0, 255);
    let red = Nrgba::new(255, 0, 0, 255);
    let ts = quad(1.0, green);
    let frame = |dynamic: &[Triangle]| {
        raster::render_mesh_with_dynamic(
            96,
            64,
            &ts,
            dynamic,
            Nrgba::new(0, 0, 0, 255),
            &RenderContext::default(),
        )
        .unwrap()
    };
    let first = frame(&[]);
    let behind = frame(&quad(-2.0, red));
    assert_eq!(first.pix, behind.pix);
    let front = frame(&quad(2.0, red));
    let at = (32 * 96 + 48) * 4;
    assert_eq!(&front.pix[at..at + 4], &[255, 0, 0, 255]);
    assert_eq!(&first.pix[at..at + 4], &[0, 255, 0, 255]);
    let mut caller_output = first.clone();
    caller_output.pix[0] = 255;
    assert_eq!(frame(&[]).pix, first.pix);
}

#[test]
fn cancellation_precedes_dimensions_and_dynamic_validation() {
    let token = CancellationToken::new();
    token.cancel();
    let context = RenderContext::from_token(&token);
    let mut dynamic = quad(0.0, Nrgba::new(255, 0, 0, 255));
    dynamic[0].v[0].position.x = f64::NAN;
    assert_eq!(
        raster::render_mesh_with_dynamic(0, 0, &[], &dynamic, Nrgba::default(), &context)
            .unwrap_err(),
        "context canceled"
    );
    assert_eq!(
        raster::render_mesh_with_dynamic(
            64,
            64,
            &[],
            &dynamic,
            Nrgba::default(),
            &RenderContext::default()
        )
        .unwrap_err(),
        "triangle 0 has invalid vertex"
    );
}
