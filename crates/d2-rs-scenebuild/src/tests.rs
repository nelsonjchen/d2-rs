// SPDX-License-Identifier: MPL-2.0
//! Contract tests for the native scene builder. Whole-document parity with
//! Go is measured by scripts/scene-scoreboard.py --tier scenebuild; these
//! pin the individual contracts that parity depends on.

use d2_rs_scene::{Matrix, Point, Primitive};
use d2_rs_target::Diagram;

use crate::{LinkBudget, Options, build};

fn diagram(value: serde_json::Value) -> Diagram {
    serde_json::from_value(value).expect("diagram JSON")
}

fn probe_options() -> Options {
    Options {
        pad: Some(0),
        appendix: true,
        link_budget: LinkBudget {
            max_regions: 4096,
            max_string_bytes: 1 << 20,
        },
        ..Default::default()
    }
}

fn rectangle(id: &str, x: i64, label: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id, "type": "rectangle", "pos": {"x": x, "y": 0}, "width": 100, "height": 66,
        "opacity": 1, "strokeWidth": 2, "fill": "B6", "stroke": "B1",
        "label": label, "fontSize": 16, "fontFamily": "DEFAULT", "bold": true, "color": "N1",
        "labelWidth": 40, "labelHeight": 21, "labelPosition": "INSIDE_MIDDLE_CENTER",
        "zIndex": 0, "level": 1
    })
}

#[test]
fn inlined_matrix_point_fuses_the_first_product() {
    // stable/cross_arrowhead: crossTransform.E = -Rotate(pi/4).Point(origin).X
    // + width/2 is exactly 8.5 in Go; fusing the second product instead
    // yields 8.499999999999998.
    let origin = Point::new(8.5, 8.5);
    let rotated = Matrix::rotate(std::f64::consts::PI / 4.0).point(origin);
    assert_eq!(-rotated.x + 8.5, 8.5);
}

#[test]
fn go_folds_untyped_constants_exactly() {
    // -12*math.Pi/180 is rounded once from the exact value.
    assert_ne!(
        -12.0 * std::f64::consts::PI / 180.0,
        -0.209_439_510_239_319_56
    );
}

#[test]
fn build_rejects_nested_boards_and_unknown_themes() {
    let mut nested =
        diagram(serde_json::json!({"name": "", "root": {}, "layers": [{"name": "x", "root": {}}]}));
    assert_eq!(
        build(&nested, &probe_options()).unwrap_err().to_string(),
        "scene: diagram uses unsupported nested boards in a single scene"
    );
    nested.layers = None;
    let options = Options {
        theme_id: Some(12345),
        ..probe_options()
    };
    assert_eq!(
        build(&nested, &options).unwrap_err().to_string(),
        "scene: unknown theme 12345"
    );
}

#[test]
fn shape_label_materializes_glyphs_and_fonts() {
    let d = diagram(serde_json::json!({
        "name": "", "root": {"fill": "N7"}, "shapes": [rectangle("a", 0, "hello")], "connections": []
    }));
    let document = build(&d, &probe_options()).expect("build");
    let root = document.root.as_ref().expect("root");
    let ids: Vec<&str> = root.children().iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, ["root:background", "a"]);
    let label = &root.children()[1].children()[1];
    assert_eq!(label.id, "a:label:0");
    let Some(Primitive::TextRun(run)) = &label.primitive else {
        panic!("text run")
    };
    assert_eq!(run.glyphs.as_ref().map(Vec::len), Some(5));
    assert!(document.assets.contains_key("font:SourceSansPro:bold"));
    assert_eq!(document.view_box.width, document.logical_width);
}

#[test]
fn metadata_requires_a_link_budget() {
    let mut shape = rectangle("a", 0, "x");
    shape["link"] = "https://example.com".into();
    let d = diagram(serde_json::json!({"name": "", "root": {}, "shapes": [shape]}));
    let options = Options {
        link_budget: LinkBudget::default(),
        ..probe_options()
    };
    assert_eq!(
        build(&d, &options).unwrap_err().to_string(),
        "scene: options field \"linkBudget\" has invalid value {0 0}: must provide positive MaxRegions and MaxStringBytes for link/tooltip metadata"
    );
    let document = build(&d, &probe_options()).expect("build");
    let links = document.links.expect("links");
    assert_eq!(links[0].url, "https://example.com");
    // A bare link has no appendix text, so no badge layout exists.
    let ids: Vec<&str> = document
        .root
        .as_ref()
        .unwrap()
        .children()
        .iter()
        .map(|n| n.id.as_str())
        .collect();
    assert!(!ids.contains(&"appendix"));
    // A tooltip adds an appendix row, which numbers both badges.
    let mut shape = rectangle("a", 0, "x");
    shape["link"] = "https://example.com".into();
    shape["tooltip"] = "tip".into();
    let d = diagram(serde_json::json!({"name": "", "root": {}, "shapes": [shape]}));
    let document = build(&d, &probe_options()).expect("build");
    let ids: Vec<&str> = document
        .root
        .as_ref()
        .unwrap()
        .children()
        .iter()
        .map(|n| n.id.as_str())
        .collect();
    assert!(
        ids.contains(&"appendix:shape:0:tooltip")
            && ids.contains(&"appendix:shape:0:link")
            && ids.contains(&"appendix")
    );
}

#[test]
fn board_link_targets_are_classified() {
    let mut a = rectangle("a", 0, "x");
    a["link"] = "root.layers.x".into();
    let d = diagram(serde_json::json!({"name": "", "root": {}, "shapes": [a]}));
    let links = build(&d, &probe_options()).unwrap().links.unwrap();
    assert_eq!(
        (links[0].url.as_str(), links[0].target.as_str()),
        ("", "root.layers.x")
    );
}

#[test]
fn dangerous_links_follow_master_normalization() {
    use crate::markdown::{is_dangerous_link, safe_markdown_link};
    assert!(is_dangerous_link("java\tscript:alert(1)"));
    assert!(is_dangerous_link("&#106;avascript:alert(1)"));
    assert!(is_dangerous_link("%6Aavascript:alert(1)"));
    assert!(!is_dangerous_link("https://d2lang.com"));
    assert_eq!(safe_markdown_link("  root.html "), "root.html");
}

#[test]
fn builtin_paper_pattern_matches_its_topology() {
    let source = crate::pattern_sources::shared_paper_pattern_source().expect("paper source");
    assert_eq!(
        source.paths.len(),
        crate::pattern_sources::PAPER_PATTERN_PATH_COUNT
    );
    let commands: usize = source.paths.iter().map(|p| p.commands.len()).sum();
    assert_eq!(
        commands,
        crate::pattern_sources::PAPER_PATTERN_COMMAND_COUNT
    );
    assert_eq!(
        crate::pattern_sources::shared_sketch_streak_commands()
            .expect("streak")
            .len(),
        687
    );
}

#[test]
fn svg_path_parser_resolves_relative_and_smooth_commands() {
    use crate::svgimport_path::{PathLimits, parse_path};
    use d2_rs_scene::PathCommandKind;
    let limits = || PathLimits {
        max_bytes: 1024,
        max_commands: 64,
    };
    let commands = parse_path("t", "m1 2 l3 4 h1 v-1 s1 1 2 2 z", limits()).expect("path");
    let kinds: Vec<PathCommandKind> = commands.iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        [
            PathCommandKind::MOVE,
            PathCommandKind::LINE,
            PathCommandKind::LINE,
            PathCommandKind::LINE,
            PathCommandKind::CUBIC,
            PathCommandKind::CLOSE
        ]
    );
    assert_eq!(commands[1].p1, Point::new(4.0, 6.0));
    // A smooth cubic after a line uses the current point as its control.
    assert_eq!(commands[4].p1, Point::new(5.0, 5.0));
    assert_eq!(
        parse_path("t", "M,1 2", limits()).unwrap_err(),
        "d2svgimport: t path byte 1: comma is not allowed before a command's first argument"
    );
    assert_eq!(
        parse_path("", "M0 0 L1", limits()).unwrap_err(),
        "d2svgimport: SVG asset path byte 7: expected a finite number"
    );
}

#[test]
fn gradients_become_typed_paints() {
    let mut a = rectangle("a", 0, "");
    a["fill"] = "linear-gradient(to right, red 0%, #0000ff 100%)".into();
    let d = diagram(serde_json::json!({"name": "", "root": {}, "shapes": [a]}));
    let document = build(&d, &probe_options()).unwrap();
    let outline = &document.root.as_ref().unwrap().children()[1].children()[0];
    let Some(Primitive::Rect(rect)) = &outline.primitive else {
        panic!("rect")
    };
    let Some(d2_rs_scene::Paint::LinearGradient(gradient)) = &rect.fill else {
        panic!("gradient")
    };
    assert_eq!(
        (gradient.start, gradient.end),
        (Point::new(0.0, 0.5), Point::new(1.0, 0.5))
    );
    assert_eq!(
        gradient.stops[1].color,
        d2_rs_scene::Nrgba::new(0, 0, 255, 255)
    );
}

#[test]
fn latex_admission_matches_go() {
    let mut a = rectangle("a", 0, "\\frac{1}{2}");
    a["language"] = "latex".into();
    let d = diagram(serde_json::json!({"name": "", "root": {}, "shapes": [a]}));
    assert_eq!(
        build(&d, &probe_options()).unwrap_err().to_string(),
        "scene: shape \"a\" uses unsupported label language latex without configured SVG import limits"
    );
    let deep = "{".repeat(129);
    assert_eq!(
        crate::latex::validate_latex_input("shape \"a\"", &deep)
            .unwrap_err()
            .to_string(),
        "scene: shape \"a\" latex group nesting depth 129 exceeds limit 128"
    );
}
