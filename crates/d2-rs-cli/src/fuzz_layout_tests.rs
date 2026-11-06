// SPDX-License-Identifier: MPL-2.0
//! Reduced X16 fuzz contracts, checked against the pinned Go CLI.
use super::*;
use base64::engine::general_purpose::STANDARD as BASE64;

fn shape_attributes(source: &str, engine: &str, id: &str, attributes: &[&str]) -> Vec<String> {
    let svg = String::from_utf8(
        render_source(source.as_bytes(), &[format!("--engine={engine}")]).unwrap(),
    )
    .unwrap();
    let document = roxmltree::Document::parse(&svg).unwrap();
    let class = BASE64.encode(id);
    let group = document
        .descendants()
        .find(|node| node.has_tag_name("g") && node.attribute("class") == Some(class.as_str()))
        .unwrap();
    let shape = group
        .descendants()
        .find(|node| node.has_tag_name("rect") || node.has_tag_name("ellipse"))
        .unwrap();
    attributes
        .iter()
        .map(|attribute| shape.attribute(*attribute).unwrap().to_owned())
        .collect()
}

#[test]
fn repeated_grid_assignments_use_the_last_primary_key_for_direction() {
    // d2compiler.compileField sets GridRows.MapKey = f.LastPrimaryKey();
    // d2grid.newGridDiagram compares that range to GridColumns.MapKey.
    for engine in ["dagre", "elk", "tala"] {
        assert_eq!(
            shape_attributes(
                "grid-rows:1\ngrid-columns:2\ngrid-rows:19{}\na->b",
                engine,
                "b",
                &["x", "y"],
            ),
            ["0", "106"],
            "{engine}",
        );
    }
}

#[test]
fn dagre_does_not_fit_descendants_below_non_square_containers() {
    // d2dagrelayout.fitPadding returns before visiting a non-square's children.
    for (source, id, expected) in [
        (
            "container{g.h.i}\ncontainer{d{}\n*.shape:circle}",
            "container.g.h",
            ["40", "100", "129", "177"],
        ),
        (
            "*.shape:circle\nw{h{}\n3.e}",
            "w.3",
            ["133", "50", "133", "166"],
        ),
    ] {
        assert_eq!(
            shape_attributes(source, "dagre", id, &["x", "y", "width", "height"]),
            expected
        );
    }
}

#[test]
fn root_near_excludes_base_shapes_from_constant_near_bounds() {
    // d2near.boundingBox calls OuterNearContainer through the synthetic root.
    for engine in ["dagre", "elk", "tala"] {
        for (source, id, expected) in [
            ("near:t\na{near:center-right}\n1", "a", ["20", "-33"]),
            ("x{}\nnear:r\ny{near:center-left}", "y", ["-74", "-33"]),
        ] {
            assert_eq!(
                shape_attributes(source, engine, id, &["x", "y"]),
                expected,
                "{engine}"
            );
        }
    }
}

#[test]
fn elk_parent_padding_uses_child_sizes_before_degree_expansion() {
    // d2elklayout.Layout walks parents first; adjustPadding sees the child's
    // original SetDimensions width, before its 2 ports expand it to 80 px.
    assert_eq!(
        shape_attributes("a->c.d\n*.shape:circle\na->c.d", "elk", "c", &["cy", "ry"]),
        ["256.5", "92.5"],
    );
    assert_eq!(
        shape_attributes(
            "a{bb{c{d{}}\nii->c}\n*.shape:circle\nbb.ii->l}",
            "elk",
            "a.bb.c",
            &["x", "y", "width", "height"],
        ),
        ["112", "271", "154", "166"],
    );
}

#[test]
fn elk_remeasures_label_overflow_after_resizing_a_compound() {
    // elk-go ResizeNode moves labels relative to the old box, then the
    // parent's InnermostNodeMarginCalculator measures their new overflow.
    // The 12 px label no longer exceeds N.d after it grows from 10 to 153.
    assert_eq!(
        shape_attributes(
            "**{label:⬤\na\nwidth:10}\nN{d}",
            "elk",
            "N",
            &["width", "height"]
        ),
        ["326", "225"],
    );
}

#[test]
fn markdown_non_void_self_closing_tags_keep_html_measurement_semantics() {
    // x/net/html ignores the slash on <b/>; the following text remains bold
    // until the paragraph ends, in both LayoutMarkdown sizing and painting.
    for engine in ["dagre", "elk", "tala"] {
        let svg = String::from_utf8(
            render_source(b"y:|md <b/>d|", &[format!("--engine={engine}")]).unwrap(),
        )
        .unwrap();
        let document = roxmltree::Document::parse(&svg).unwrap();
        let viewport = document
            .descendants()
            .find(|node| node.has_tag_name("svg") && node.attribute("overflow") == Some("hidden"))
            .unwrap();
        assert_eq!(viewport.attribute("width"), Some("10"), "{engine}");
        assert_eq!(viewport.attribute("height"), Some("24"), "{engine}");
        let text = viewport
            .descendants()
            .find(|node| node.has_tag_name("text"))
            .unwrap();
        assert!(text.attribute("class").unwrap().contains("text-bold"));
        assert_eq!(text.text(), Some("d"));
    }
}

#[test]
fn markdown_whitespace_before_the_body_preserves_a_leading_break_line() {
    // x/net/html's initial insertion modes discard spaces outside the body;
    // measureMarkdownNode then counts the leading br as an empty 24 px line.
    for engine in ["dagre", "elk", "tala"] {
        let svg = String::from_utf8(
            render_source(
                b"copy: |md\n  <br />\nx\n|",
                &[format!("--engine={engine}")],
            )
            .unwrap(),
        )
        .unwrap();
        let document = roxmltree::Document::parse(&svg).unwrap();
        let viewport = document
            .descendants()
            .find(|node| node.has_tag_name("svg") && node.attribute("overflow") == Some("hidden"))
            .unwrap();
        assert_eq!(viewport.attribute("height"), Some("48"), "{engine}");
    }
}
