// SPDX-License-Identifier: MPL-2.0
//! Raster storage/budget and compositing contracts behind whole-board PNGs.
// Oracle tier: tests reading regenerated fixtures are gated (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use d2_rs_isometric::img::{mesh::*, raster::render_mesh};
use d2_rs_scene::Nrgba;
use std::{cell::RefCell, rc::Rc};

#[cfg(feature = "oracle-fixtures")]
#[test]
fn png_convenience_entry_point_preserves_named_board_deadlines() {
    // I10's admitted 1ns failures on the original golden boards also apply
    // through the public String-returning convenience entry point.
    let fixture: serde_json::Value =
        serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/png-api.json")).unwrap();
    let mut checked = 0;
    for call in fixture["calls"].as_array().unwrap() {
        let request = &call["request"];
        if request["api"] != "render" || request["options"]["Timeout"] != 1 {
            continue;
        }
        let diagram = serde_json::from_value(request["diagram"].clone()).unwrap();
        let o = &request["options"];
        let options = d2_rs_isometric::img::render::Options {
            width: o["Width"].as_i64().unwrap_or(0),
            height: o["Height"].as_i64().unwrap_or(0),
            fit_content: o["FitContent"].as_bool().unwrap_or(false),
            timeout_ns: 1,
            ..Default::default()
        };
        assert_eq!(
            d2_rs_isometric::img::raster::render_png(&diagram, &options).unwrap_err(),
            call["want"]["error"].as_str().unwrap()
        );
        checked += 1;
    }
    assert_eq!(checked, 3);
}

#[test]
fn background_is_opaque_and_caller_owned() {
    let background = Nrgba::new(24, 57, 103, 22);
    let mut first = render_mesh(64, 65, &[], background, None, None).unwrap();
    assert_eq!(first.pix.len(), 64 * 65 * 4);
    assert!(first.pix.chunks_exact(4).all(|p| p == [24, 57, 103, 255]));
    first.pix[0] = 0;
    assert_eq!(
        render_mesh(64, 65, &[], background, None, None)
            .unwrap()
            .pix[0],
        24
    );
}

#[test]
fn malformed_geometry_and_texture_fail_before_sampling() {
    let background = Nrgba::new(255, 255, 255, 255);
    let mut triangle = Triangle::default();
    triangle.v[0].position.x = f64::NAN;
    assert_eq!(
        render_mesh(64, 64, &[triangle], background, None, None).unwrap_err(),
        "triangle 0 has invalid vertex"
    );
    let triangle = Triangle {
        material: Some(material(Material {
            texture: Some(Rc::new(Texture::new(2, 2, vec![0; 15]))),
            ..Default::default()
        })),
        ..Default::default()
    };
    assert_eq!(
        render_mesh(64, 64, &[triangle], background, None, None).unwrap_err(),
        "short native isometric texture storage"
    );
    let triangle = Triangle {
        opacity_group: Some(Rc::new(RefCell::new(OpacityGroup { opacity: 1.1 }))),
        ..Default::default()
    };
    assert_eq!(
        render_mesh(64, 64, &[triangle], background, None, None).unwrap_err(),
        "triangle 0 has invalid object opacity"
    );
}

#[test]
fn dimensions_and_camera_are_bounded() {
    for (width, height) in [(0, 64), (4097, 64), (4096, 4096)] {
        assert_eq!(
            render_mesh(width, height, &[], Nrgba::default(), None, None).unwrap_err(),
            "native isometric raster dimensions exceed limits"
        );
    }
    assert_eq!(
        render_mesh(
            64,
            64,
            &[],
            Nrgba::default(),
            Some(&Camera::default()),
            None
        )
        .unwrap_err(),
        "native raster camera requires positive dimensions"
    );
}
