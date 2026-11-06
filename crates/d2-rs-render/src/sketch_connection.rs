// SPDX-License-Identifier: MPL-2.0
//! d2sketch/scene_connection.go: native rough route and endpoint geometry.
use super::*;
use d2_rs_sketch::{
    SketchOptions, rough_linear_path, rough_solid_ellipse_paths, rough_solid_polygon_paths,
    rough_zigzag_polygon_paths,
};

fn point(x: f64, y: f64) -> Point {
    Point { x, y }
}

fn commands(path: &str) -> Vec<RasterPathCommand> {
    let n = |v: f64| (v * 1_000_000.0).trunc() / 1_000_000.0;
    svgtypes::PathParser::from(path)
        .map(|s| match s.expect("rough primitives emit valid paths") {
            svgtypes::PathSegment::MoveTo { abs: true, x, y } => {
                RasterPathCommand::MoveTo(point(n(x), n(y)))
            }
            svgtypes::PathSegment::LineTo { abs: true, x, y } => {
                RasterPathCommand::LineTo(point(n(x), n(y)))
            }
            svgtypes::PathSegment::CurveTo {
                abs: true,
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => RasterPathCommand::CubicTo(
                point(n(x1), n(y1)),
                point(n(x2), n(y2)),
                point(n(x), n(y)),
            ),
            _ => unreachable!("rough connection primitives emit absolute M/L/C"),
        })
        .collect()
}

pub(super) fn route(source: &[RasterPathCommand]) -> Option<Vec<RasterPathCommand>> {
    // Count before expansion, as ConnectionDrawable does. Route generation is
    // bounded to 256 segments per seeded rough-go operation set.
    let mut segments = 0usize;
    let mut have_move = false;
    for command in source {
        match command {
            RasterPathCommand::MoveTo(_) if !have_move => have_move = true,
            RasterPathCommand::LineTo(_) if have_move => segments = segments.checked_add(1)?,
            RasterPathCommand::CubicTo(..) if have_move => segments = segments.checked_add(8)?,
            _ => return None,
        }
    }
    if segments == 0 {
        return None;
    }
    let mut points = Vec::with_capacity(segments.checked_add(1)?);
    let mut current = Point::default();
    let lerp = |a: Point, b: Point, t: f64| point(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
    for command in source {
        match *command {
            RasterPathCommand::MoveTo(p) | RasterPathCommand::LineTo(p) => {
                current = p;
                points.push(p);
            }
            RasterPathCommand::CubicTo(a, b, end) => {
                for step in 1..=8 {
                    let t = f64::from(step) / 8.0;
                    let (ab, bc, cd) = (lerp(current, a, t), lerp(a, b, t), lerp(b, end, t));
                    points.push(lerp(lerp(ab, bc, t), lerp(bc, cd, t), t));
                }
                current = end;
            }
            _ => unreachable!(),
        }
    }
    let mut out = Vec::new();
    for (index, first) in (0..segments).step_by(256).enumerate() {
        out.extend(commands(&rough_linear_path(
            &points[first..=(first + 256).min(segments)],
            SketchOptions {
                seed: index as u64 + 1,
                roughness: 0.5,
                ..Default::default()
            },
        )));
    }
    Some(out)
}

pub(super) fn arrow_parts(
    name: &str,
    width: f64,
    stroke: &str,
    background: &str,
    native: bool,
) -> Option<Vec<(String, String, Option<String>, f64)>> {
    let mut parts: Vec<(String, String, Option<String>, f64)> = Vec::new();
    let line = |pairs: &[(f64, f64)], seed| {
        rough_linear_path(
            &pairs.iter().map(|&(x, y)| point(x, y)).collect::<Vec<_>>(),
            SketchOptions {
                seed,
                ..Default::default()
            },
        )
    };
    let mut outline = |data| parts.push((data, "none".to_owned(), Some(stroke.to_owned()), width));
    if !native && matches!(name, "line" | "filled-circle") {
        return None;
    }
    if !native && name.starts_with("cf-") {
        use d2_rs_sketch::{SvgSketchCommand as C, rough_solid_open_line_paths, rough_svg_path};
        let (data, seed) = match name {
            "cf-many-required" => (
                vec![
                    C::Move(point(-15.0, -10.0)),
                    C::Line(point(-15.0, 10.0)),
                    C::Move(point(0.0, 10.0)),
                    C::Line(point(-15.0, 0.0)),
                    C::Move(point(0.0, -10.0)),
                    C::Line(point(-15.0, 0.0)),
                ],
                2,
            ),
            "cf-many" => (
                vec![
                    C::Move(point(0.0, 10.0)),
                    C::Line(point(-15.0, 0.0)),
                    C::Move(point(0.0, -10.0)),
                    C::Line(point(-15.0, 0.0)),
                ],
                8,
            ),
            "cf-one-required" => (
                vec![
                    C::Move(point(-15.0, -10.0)),
                    C::Line(point(-15.0, 10.0)),
                    C::Move(point(-10.0, -10.0)),
                    C::Line(point(-10.0, 10.0)),
                ],
                2,
            ),
            "cf-one" => (
                vec![C::Move(point(-10.0, -10.0)), C::Line(point(-10.0, 10.0))],
                3,
            ),
            _ => return None,
        };
        let options = SketchOptions {
            seed,
            ..Default::default()
        };
        let paths = if name == "cf-one" {
            rough_solid_open_line_paths(point(-10.0, -10.0), point(-10.0, 10.0), options)
        } else {
            d2_rs_sketch::SolidSketchPaths {
                fill: String::new(),
                outline: rough_svg_path(&data, options),
            }
        };
        parts.push((paths.fill, stroke.to_owned(), None, 0.0));
        parts.push((
            paths.outline,
            "none".to_owned(),
            Some(stroke.to_owned()),
            width,
        ));
    } else {
        match name {
            "arrow" | "line" => outline(line(&[(-10.0, -4.0), (0.0, 0.0), (-10.0, 4.0)], 3)),
            "cross" => outline(line(
                &[
                    (-6.0, -6.0),
                    (6.0, 6.0),
                    (0.0, 0.0),
                    (-6.0, 6.0),
                    (0.0, 0.0),
                    (6.0, -6.0),
                ],
                3,
            )),
            "cf-many-required" => {
                outline(line(&[(-15.0, -10.0), (-15.0, 10.0)], 2));
                outline(line(&[(0.0, 10.0), (-15.0, 0.0), (0.0, -10.0)], 2));
            }
            "cf-one-required" => {
                outline(line(&[(-15.0, -10.0), (-15.0, 10.0)], 2));
                outline(line(&[(-10.0, -10.0), (-10.0, 10.0)], 2));
            }
            "cf-many" => outline(line(&[(0.0, 10.0), (-15.0, 0.0), (0.0, -10.0)], 8)),
            "cf-one" => outline(line(&[(-10.0, -10.0), (-10.0, 10.0)], 3)),
            "triangle" | "unfilled-triangle" | "diamond" | "box" | "filled-box" => {
                let (pairs, seed): (&[(f64, f64)], u64) = match name {
                    "triangle" | "unfilled-triangle" => {
                        (&[(-10.0, -4.0), (0.0, 0.0), (-10.0, 4.0)], 2)
                    }
                    "diamond" => (
                        &[
                            (-20.0, 0.0),
                            (-10.0, 5.0),
                            (0.0, 0.0),
                            (-10.0, -5.0),
                            (-20.0, 0.0),
                        ],
                        1,
                    ),
                    _ => (
                        &[(0.0, -10.0), (0.0, 10.0), (-20.0, 10.0), (-20.0, -10.0)],
                        1,
                    ),
                };
                let paths = rough_solid_polygon_paths(
                    &pairs.iter().map(|&(x, y)| point(x, y)).collect::<Vec<_>>(),
                    SketchOptions {
                        seed,
                        ..Default::default()
                    },
                );
                let fill = if matches!(name, "triangle" | "filled-box") {
                    stroke
                } else {
                    background
                };
                parts.push((paths.fill, fill.to_owned(), None, 0.0));
                parts.push((
                    paths.outline,
                    "none".to_owned(),
                    Some(stroke.to_owned()),
                    width,
                ));
            }
            "filled-diamond" => {
                let paths = rough_zigzag_polygon_paths(
                    &[
                        point(-20.0, 0.0),
                        point(-10.0, 5.0),
                        point(0.0, 0.0),
                        point(-10.0, -5.0),
                        point(-20.0, 0.0),
                    ],
                    width,
                    SketchOptions {
                        seed: 1,
                        ..Default::default()
                    },
                )
                .ok()?;
                // CompileScenePaths skips empty fill sets while retaining outlines.
                if !native || !paths.fill.is_empty() {
                    parts.push((paths.fill, "none".to_owned(), Some(stroke.to_owned()), 4.0));
                }
                parts.push((
                    paths.outline,
                    "none".to_owned(),
                    Some(stroke.to_owned()),
                    width,
                ));
            }
            "circle" | "filled-circle" => (),
            _ => return None,
        }
    }
    if matches!(name, "circle" | "filled-circle" | "cf-one" | "cf-many") {
        let seed = if name == "cf-many" { 4 } else { 5 };
        let center = if matches!(name, "cf-one" | "cf-many") {
            point(-20.0, 0.0)
        } else {
            point(-2.0, -1.0)
        };
        let paths = rough_solid_ellipse_paths(
            center,
            8.0,
            8.0,
            SketchOptions {
                seed,
                ..Default::default()
            },
        );
        parts.push((
            paths.fill,
            if name == "filled-circle" {
                stroke
            } else {
                background
            }
            .to_owned(),
            None,
            0.0,
        ));
        parts.push((
            paths.outline,
            "none".to_owned(),
            Some(stroke.to_owned()),
            width,
        ));
    }
    Some(parts)
}

pub(super) fn arrow(
    name: Option<&str>,
    id: String,
    points: &[Point],
    target: bool,
    adjustment: Point,
    width: f64,
    stroke: &str,
    background: &str,
) -> Option<RasterInlineArrow> {
    let name = canonical_marker_name(name)?;
    if points.len() < 2 {
        return None;
    }
    let parts = arrow_parts(name, width, stroke, background, true)?;
    let (a, b) = if target {
        (points[points.len() - 2], points[points.len() - 1])
    } else {
        (points[0], points[1])
    };
    let angle = (b.y - a.y).atan2(b.x - a.x) + if target { 0.0 } else { std::f64::consts::PI };
    let endpoint = if target { b } else { a };
    let (sin, cos) = go_sincos_route_angle(angle);
    let transform = [
        cos,
        -sin,
        sin,
        cos,
        endpoint.x + adjustment.x,
        endpoint.y + adjustment.y,
    ];
    let map = |p: Point| {
        point(
            cos * p.x - sin * p.y + transform[4],
            sin * p.x + cos * p.y + transform[5],
        )
    };
    let paths = parts
        .into_iter()
        .enumerate()
        .map(|(index, (data, fill, stroke, path_width))| {
            let local = commands(&data);
            let mapped = local
                .iter()
                .map(|c| match *c {
                    RasterPathCommand::MoveTo(p) => RasterPathCommand::MoveTo(map(p)),
                    RasterPathCommand::LineTo(p) => RasterPathCommand::LineTo(map(p)),
                    RasterPathCommand::CubicTo(a, b, c) => {
                        RasterPathCommand::CubicTo(map(a), map(b), map(c))
                    }
                    RasterPathCommand::Close => RasterPathCommand::Close,
                })
                .collect::<Vec<_>>();
            RasterInlineArrowPath {
                id: format!("{id}-rough-{index}"),
                path_data: raster_path_data(&mapped),
                commands: local,
                source_transform: Some(transform),
                fill,
                stroke,
                stroke_width: Some(path_width),
                stroke_style: Some("stroke-linecap:round;stroke-linejoin:round"),
            }
        })
        .collect();
    Some(RasterInlineArrow { paths })
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    use serde_json::Value;
    fn source_commands(rows: &Value) -> Vec<RasterPathCommand> {
        let p = |v: &Value| point(v["X"].as_f64().unwrap(), v["Y"].as_f64().unwrap());
        rows.as_array()
            .unwrap()
            .iter()
            .map(|row| match row["Kind"].as_u64().unwrap() {
                0 => RasterPathCommand::MoveTo(p(&row["P1"])),
                1 => RasterPathCommand::LineTo(p(&row["P1"])),
                3 => RasterPathCommand::CubicTo(p(&row["P1"]), p(&row["P2"]), p(&row["P3"])),
                _ => panic!("unexpected source rough command"),
            })
            .collect()
    }
    #[test]
    fn native_rough_connection_operations_match_original_go() {
        let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/sketch-connection-source-v090.json"
        ))
        .unwrap();
        for row in fixture["routes"].as_array().unwrap() {
            let input = source_commands(&row["input"]);
            let expected = row["paths"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|p| source_commands(&p["Path"]["Commands"]))
                .collect::<Vec<_>>();
            assert_eq!(route(&input).unwrap(), expected);
        }
    }
    #[test]
    fn native_rough_arrow_operations_and_paint_match_original_go() {
        let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/sketch-connection-source-v090.json"
        ))
        .unwrap();
        for (name, rows) in fixture["arrows"].as_object().unwrap() {
            for target in [false, true] {
                let got = arrow(
                    Some(name),
                    "test".to_owned(),
                    &[point(0.0, 0.0), point(20.0, 5.0)],
                    target,
                    point(1.0, 2.0),
                    2.0,
                    "#123456",
                    "#FFFFFF",
                )
                .unwrap();
                let expected_transform = fixture["matrices"][usize::from(target)]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|n| n.as_f64().unwrap())
                    .collect::<Vec<_>>();
                for path in &got.paths {
                    assert_eq!(
                        path.source_transform.unwrap().as_slice(),
                        expected_transform.as_slice(),
                        "{name} target={target}"
                    );
                }
                let expected = rows.as_array().unwrap();
                assert_eq!(got.paths.len(), expected.len(), "{name}");
                for (path, row) in got.paths.iter().zip(expected) {
                    assert_eq!(
                        path.commands,
                        source_commands(&row["Path"]["Commands"]),
                        "{name}"
                    );
                    let source_fill = row["Fill"].as_str().unwrap();
                    assert_eq!(
                        path.fill,
                        if source_fill == "N7" {
                            "#FFFFFF"
                        } else {
                            source_fill
                        },
                        "{name}"
                    );
                    assert_eq!(
                        path.stroke.as_deref().unwrap_or("none"),
                        row["Stroke"].as_str().unwrap(),
                        "{name}"
                    );
                    assert_eq!(
                        path.stroke_width.unwrap_or(0.0),
                        row["StrokeWidth"].as_f64().unwrap(),
                        "{name} stroke width"
                    );
                }
            }
        }
    }
    #[test]
    fn native_zigzag_scene_operations_match_original_go_polygons() {
        let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/sketch-zigzag-source-v090.json"
        ))
        .unwrap();
        for row in fixture.as_array().unwrap() {
            let input = &row["input"];
            let points = input["points"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| point(p[0].as_f64().unwrap(), p[1].as_f64().unwrap()))
                .collect::<Vec<_>>();
            let width = input["width"].as_f64().unwrap();
            let options = SketchOptions {
                seed: input["seed"].as_u64().unwrap(),
                roughness: input["roughness"].as_f64().unwrap(),
                ..Default::default()
            };
            let paths = rough_zigzag_polygon_paths(&points, width, options).unwrap();
            let got = [(paths.fill, 4.0), (paths.outline, width)]
                .into_iter()
                .filter(|(data, _)| !data.is_empty())
                .collect::<Vec<_>>();
            let expected = row["paths"].as_array().unwrap();
            assert_eq!(got.len(), expected.len(), "{} path count", input["name"]);
            for ((path, weight), source) in got.iter().zip(expected) {
                assert_eq!(
                    commands(path),
                    source_commands(&source["Path"]["Commands"]),
                    "{} geometry",
                    input["name"]
                );
                assert_eq!(
                    *weight,
                    source["StrokeWidth"].as_f64().unwrap(),
                    "{} weight",
                    input["name"]
                );
                assert_eq!(source["Fill"], "none");
                assert_eq!(source["Stroke"], "#123456");
            }
        }
    }
}
