// SPDX-License-Identifier: MPL-2.0
//! Original d2svg/d2sketch SVG connection transport, separate from native scene.
use super::*;
use d2_rs_sketch::{SketchOptions, SvgSketchCommand as C, rough_svg_path};

fn parse(data: &str) -> Option<Vec<C>> {
    use svgtypes::PathSegment::*;
    let mut out = Vec::new();
    let mut current = Point::default();
    let mut first = current;
    let mut control: Option<Point> = None;
    for segment in svgtypes::PathParser::from(data) {
        let command = match segment.ok()? {
            MoveTo { abs: true, x, y } => {
                first = Point { x, y };
                C::Move(first)
            }
            LineTo { abs: true, x, y } => C::Line(Point { x, y }),
            HorizontalLineTo { abs: true, x } => C::Line(Point { x, y: current.y }),
            VerticalLineTo { abs: true, y } => C::Line(Point { x: current.x, y }),
            CurveTo {
                abs: true,
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => C::Cubic(
                Point { x: x1, y: y1 },
                Point { x: x2, y: y2 },
                Point { x, y },
            ),
            SmoothCurveTo {
                abs: true,
                x2,
                y2,
                x,
                y,
            } => C::Cubic(
                control
                    .map(|c| Point {
                        x: 2.0 * current.x - c.x,
                        y: 2.0 * current.y - c.y,
                    })
                    .unwrap_or(current),
                Point { x: x2, y: y2 },
                Point { x, y },
            ),
            ClosePath { .. } => C::Close,
            _ => return None,
        };
        match command {
            C::Move(p) | C::Line(p) => {
                current = p;
                control = None;
            }
            C::Cubic(_, c, p) => {
                current = p;
                control = Some(c);
            }
            C::Close => {
                current = first;
                control = None;
            }
        }
        out.push(command);
    }
    Some(out)
}

fn svg_number(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    // NumberString uses scientific notation outside JavaScript's fixed range;
    // truncatePathData truncates the mantissa, not the numeric value.
    let mut s = if value.abs() < 1e-6 || value.abs() >= 1e21 {
        format!("{value:e}")
    } else {
        value.to_string()
    };
    if let Some(dot) = s.find('.') {
        let end = s.find('e').unwrap_or(s.len());
        if end > dot + 7 {
            s.replace_range(dot + 7..end, "");
        }
    }
    s
}

pub(super) fn path_data(data: &str) -> String {
    use svgtypes::PathSegment::*;
    let mut out = String::new();
    for segment in svgtypes::PathParser::from(data) {
        match segment.expect("rough primitives emit absolute M/L/C") {
            MoveTo { abs: true, x, y } => {
                let _ = write!(out, "M{} {} ", svg_number(x), svg_number(y));
            }
            LineTo { abs: true, x, y } => {
                let _ = write!(out, "L{} {} ", svg_number(x), svg_number(y));
            }
            CurveTo {
                abs: true,
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                let _ = write!(
                    out,
                    "C{} {} {} {} {} {} ",
                    svg_number(x1),
                    svg_number(y1),
                    svg_number(x2),
                    svg_number(y2),
                    svg_number(x),
                    svg_number(y)
                );
            }
            _ => unreachable!(),
        }
    }
    out.trim_end().to_owned()
}

pub(super) fn route(data: &str) -> String {
    let source = parse(data).expect("D2 route formatter emits absolute M/L/C");
    path_data(&rough_svg_path(
        &source,
        SketchOptions {
            roughness: 0.5,
            seed: 1,
            ..Default::default()
        },
    ))
}

/// lib/svg.SplitPath uses endpoint chord lengths. At the selected cubic it
/// splits at 0.5 regardless of the chord's local percentage; S is kept whole.
pub(super) fn split(data: &str) -> Option<(String, String)> {
    let tokens = data.split_whitespace().collect::<Vec<_>>();
    let mut pieces = Vec::new();
    let mut index = 0;
    let mut previous = Point::default();
    let mut total = 0.0;
    while index < tokens.len() {
        let count = match tokens[index] {
            "M" | "L" => 3,
            "C" => 7,
            "S" => 5,
            _ => return None,
        };
        let args = tokens.get(index..index + count)?;
        let end = Point {
            x: args[count - 2].parse().ok()?,
            y: args[count - 1].parse().ok()?,
        };
        let length = if args[0] == "M" {
            0.0
        } else {
            ((end.x - previous.x).powi(2) + (end.y - previous.y).powi(2)).sqrt()
        };
        pieces.push((args, previous, end, length));
        total += length;
        previous = end;
        index += count;
    }
    if total == 0.0 {
        return None;
    }
    let mut first = String::new();
    let mut second = String::new();
    let mut sum = 0.0;
    let mut past = false;
    for (args, previous, end, length) in pieces {
        sum += length;
        let ordinary = format!("{} ", args.join(" "));
        if past {
            second.push_str(&ordinary);
            continue;
        }
        if sum < total * 0.5 {
            first.push_str(&ordinary);
            continue;
        }
        match args[0] {
            "L" => {
                let t = (total * 0.5 - sum + length) / length;
                let middle = Point {
                    x: (end.x - previous.x).mul_add(t, previous.x),
                    y: (end.y - previous.y).mul_add(t, previous.y),
                };
                let _ = write!(first, "L {:.6} {:.6} ", middle.x, middle.y);
                let _ = write!(
                    second,
                    "M {:.6} {:.6} L {:.6} {:.6} ",
                    middle.x, middle.y, end.x, end.y
                );
            }
            "C" => {
                let a = Point {
                    x: args[1].parse().ok()?,
                    y: args[2].parse().ok()?,
                };
                let b = Point {
                    x: args[3].parse().ok()?,
                    y: args[4].parse().ok()?,
                };
                let half = |a: Point, b: Point| Point {
                    x: (a.x + b.x) / 2.0,
                    y: (a.y + b.y) / 2.0,
                };
                let (ab, bc, cd) = (half(previous, a), half(a, b), half(b, end));
                let (abc, bcd) = (half(ab, bc), half(bc, cd));
                let middle = half(abc, bcd);
                let _ = write!(
                    first,
                    "C {:.6} {:.6} {:.6} {:.6} {:.6} {:.6} ",
                    ab.x, ab.y, abc.x, abc.y, middle.x, middle.y
                );
                let _ = write!(
                    second,
                    "M {:.6} {:.6} C {:.6} {:.6} {:.6} {:.6} {:.6} {:.6} ",
                    middle.x, middle.y, bcd.x, bcd.y, cd.x, cd.y, end.x, end.y
                );
            }
            "S" => {
                first.push_str(&ordinary);
                let _ = write!(second, "M {} {} ", args[3], args[4]);
            }
            _ => return None,
        }
        past = true;
    }
    Some((first, second))
}

pub(super) fn arrows(
    svg: &mut String,
    points: &[Point],
    source: Option<&str>,
    target: Option<&str>,
    source_adjustment: Point,
    target_adjustment: Point,
    width: f64,
    stroke: &str,
    background: &str,
) {
    if points.len() < 2 {
        return;
    }
    // Original Arrowheads returns an empty result for the whole connection
    // when either endpoint uses a kind unsupported by its SVG switch.
    if [source, target]
        .into_iter()
        .flatten()
        .any(|n| matches!(n, "filled-circle" | "line"))
    {
        return;
    }
    for (name, is_target, adjustment) in [
        (source, false, source_adjustment),
        (target, true, target_adjustment),
    ] {
        let Some(name) = name else {
            continue;
        };
        let Some(parts) = sketch_connection::arrow_parts(name, width, stroke, background, false)
        else {
            continue;
        };
        let (a, b) = if is_target {
            (points[points.len() - 2], points[points.len() - 1])
        } else {
            (points[0], points[1])
        };
        let sign = if is_target { 1.0 } else { -1.0 };
        let radians = (sign * (b.y - a.y)).atan2(sign * (b.x - a.x)) as f32;
        let angle = f64::from(radians) * 180.0 / std::f64::consts::PI;
        let endpoint = if is_target { b } else { a };
        for (data, fill, stroke, width) in parts {
            let stroke = stroke.unwrap_or_else(|| "none".to_owned());
            let _ = writeln!(
                svg,
                "<path class=\"connection\" d=\"{}\" fill=\"{}\" stroke=\"{}\" style=\"stroke-width:{};\" transform=\"translate({:.6} {:.6}) rotate({})\" />",
                path_data(&data),
                escape(&fill),
                escape(&stroke),
                width,
                endpoint.x + adjustment.x,
                endpoint.y + adjustment.y,
                angle
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    use serde_json::Value;
    #[derive(Debug, PartialEq)]
    struct Paint {
        geometry: Vec<C>,
        fill: String,
        stroke: String,
        width: f64,
        transform: Vec<f64>,
    }
    fn paints(svg: &str) -> Vec<Paint> {
        let wrapped = format!("<svg>{svg}</svg>");
        let doc = roxmltree::Document::parse(&wrapped).unwrap();
        doc.descendants()
            .filter(|n| n.has_tag_name("path"))
            .map(|node| {
                let style = node.attribute("style").unwrap_or("");
                let weight = node
                    .attribute("stroke-width")
                    .or_else(|| {
                        style
                            .split(';')
                            .find_map(|s| s.strip_prefix("stroke-width:"))
                    })
                    .unwrap_or("0")
                    .parse()
                    .unwrap();
                let fill = node.attribute("fill").unwrap_or_else(|| {
                    if node
                        .attribute("class")
                        .unwrap_or("")
                        .split_whitespace()
                        .any(|s| s == "fill-N7")
                    {
                        "#FFFFFF"
                    } else {
                        "none"
                    }
                });
                let transform = node
                    .attribute("transform")
                    .unwrap_or("")
                    .replace("translate(", "")
                    .replace("rotate(", " ")
                    .replace(')', "")
                    .split_whitespace()
                    .map(|s| s.parse().unwrap())
                    .collect();
                Paint {
                    geometry: parse(node.attribute("d").unwrap_or("")).unwrap(),
                    fill: fill.to_owned(),
                    stroke: node.attribute("stroke").unwrap_or("none").to_owned(),
                    width: weight,
                    transform,
                }
            })
            .collect()
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_go_nonanimated_svg_connection_paths() {
        let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/sketch-svg-source-v090.json"
        ))
        .unwrap();
        let mut count = 0;
        for row in fixture.as_array().unwrap() {
            if row["input"]["animated"] == true {
                continue;
            }
            let source = paints(row["connection"].as_str().unwrap());
            assert_eq!(source.len(), 1);
            assert_eq!(
                parse(&route(row["input"]["path"].as_str().unwrap())).unwrap(),
                source[0].geometry,
                "{}",
                row["input"]["name"]
            );
            count += 1;
        }
        assert_eq!(count, 59);
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_go_svg_arrow_operations_paint_and_transform() {
        let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/sketch-svg-source-v090.json"
        ))
        .unwrap();
        for row in fixture.as_array().unwrap() {
            let input = &row["input"];
            let endpoint = |key: &str| input[key].as_str().filter(|s| !s.is_empty());
            let mut svg = String::new();
            arrows(
                &mut svg,
                &[Point { x: 0.0, y: 0.0 }, Point { x: 20.0, y: 5.0 }],
                endpoint("source"),
                endpoint("target"),
                Point { x: 1.0, y: 2.0 },
                Point { x: 1.0, y: 2.0 },
                input["width"].as_f64().unwrap(),
                "#123456",
                "#FFFFFF",
            );
            assert_eq!(
                paints(&svg),
                paints(row["arrows"].as_str().unwrap()),
                "{}",
                input["name"]
            );
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_go_animated_svg_path_split() {
        let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/sketch-svg-source-v090.json"
        ))
        .unwrap();
        let mut count = 0;
        for row in fixture.as_array().unwrap() {
            let i = &row["input"];
            if i["animated"] != true
                || i["source"].as_str().unwrap().is_empty()
                    != i["target"].as_str().unwrap().is_empty()
            {
                continue;
            }
            let (a, b) = split(i["path"].as_str().unwrap()).unwrap();
            let expected = paints(row["connection"].as_str().unwrap());
            assert_eq!(expected.len(), 2);
            assert_eq!(
                parse(&a).unwrap(),
                expected[0].geometry,
                "{} first",
                i["name"]
            );
            assert_eq!(
                parse(&b).unwrap(),
                expected[1].geometry,
                "{} second",
                i["name"]
            );
            count += 1;
        }
        assert_eq!(count, 8);
    }
}
