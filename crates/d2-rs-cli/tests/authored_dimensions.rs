// SPDX-License-Identifier: MPL-2.0
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn compile(source: &str, engine: &str, format: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args([
            &format!("--layout={engine}"),
            &format!("--stdout-format={format}"),
            "-",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn negative_authored_dimensions_reach_the_engines_validation_boundary() {
    // Pinned d2graph.Object.SizeToContent preserves the signed value. Dagre
    // and ELK reach d2target.validateRenderShapeNumbers; TALA rejects it at
    // d2graph.ValidateForLayout first. These are the pin's exact CLI errors.
    for engine in ["dagre", "elk", "tala"] {
        for field in ["width", "height"] {
            for format in ["svg", "png", "pdf", "pptx", "gif", "txt"] {
                let output = compile(&format!("a.{field}: -5\n"), engine, format);
                // d2ascii admits signed dimensions and paints its minimum cell.
                if format == "txt" && engine != "tala" {
                    assert!(output.status.success());
                    let expected = if field == "width" {
                        "┌──┐\n│  │\n└──┘\n a  \n"
                    } else {
                        "┌────┐\n└────┘\n  a   \n"
                    };
                    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
                    continue;
                }
                assert!(!output.status.success(), "{engine}/{format}/{field}");
                assert!(output.stdout.is_empty(), "{engine}/{format}/{field}");
                let expected = if engine == "tala" {
                    format!("D2 object \"a\": {field} is outside the supported range: -5")
                } else if format == "svg" {
                    format!(
                        "render target root.shapes[0].{field} has invalid value -5: must be non-negative"
                    )
                } else {
                    let prefix = match format {
                        "pdf" | "pptx" => "paged board 0 scene: ",
                        "gif" => "GIF board 0 scene: ",
                        _ => "",
                    };
                    format!(
                        "{prefix}scene: shape \"a\" field \"{field}\" has invalid value -5: must be non-negative"
                    )
                };
                assert_eq!(
                    String::from_utf8(output.stderr).unwrap(),
                    format!("err: failed to compile -: {expected}\n"),
                    "{engine}/{format}/{field}",
                );
            }
        }
    }
}

#[test]
fn image_dimensions_keep_the_minimum_in_partial_svg_output() {
    // Pinned d2graph.SetDimensions handles images separately: zero uses the
    // 128px default, other authored values have MIN_SHAPE_SIZE (5px) as a
    // floor. A missing local icon makes bundling fail after this SVG is made.
    assert!(!std::path::Path::new("missing-dimension-contract.svg").exists());
    for engine in ["dagre", "elk"] {
        for field in ["width", "height"] {
            for value in [-1, 0, 1, 5] {
                let source = format!(
                    "a: {{shape: image; icon: ./missing-dimension-contract.svg; {field}: {value}}}\n"
                );
                let output = compile(&source, engine, "svg");
                assert!(!output.status.success());
                assert!(
                    String::from_utf8(output.stderr)
                        .unwrap()
                        .contains("partial render written")
                );
                let svg = String::from_utf8(output.stdout).unwrap();
                let document = roxmltree::Document::parse(&svg).unwrap();
                let image = document
                    .descendants()
                    .find(|node| node.has_tag_name("image"))
                    .unwrap();
                let dimensions = if value == 0 {
                    ["128", "128"]
                } else if field == "width" {
                    ["5", "128"]
                } else {
                    ["128", "5"]
                };
                assert_eq!(
                    [
                        image.attribute("width").unwrap(),
                        image.attribute("height").unwrap()
                    ],
                    dimensions
                );
                let viewbox = match (engine, field, value == 0) {
                    ("dagre", _, true) => "-101 -101 330 355",
                    ("dagre", "width", false) => "-101 -101 208 355",
                    ("dagre", _, false) => "-101 -101 330 232",
                    (_, _, true) => "-89 -89 330 355",
                    (_, "width", false) => "-90 -89 208 355",
                    _ => "-89 -89 330 232",
                };
                assert!(
                    document
                        .descendants()
                        .any(|node| node.attribute("viewBox") == Some(viewbox)),
                    "{engine}/{field}/{value}"
                );
            }
        }
    }
}
