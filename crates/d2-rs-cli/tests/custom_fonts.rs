// SPDX-License-Identifier: MPL-2.0
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn sketch_custom_family_uses_source_sans_for_unsupplied_styles() {
    // Pinned d2cli.loadFonts registers a primary family for any proportional
    // override. d2fonts.AddFontFamily supplies SourceSansPro for missing
    // styles, and d2lib.applyDefaults must then leave that family in place.
    // These canvas dimensions are from the pinned Go CLI with its bundled
    // SourceCodePro-Regular.ttf supplied as the override (d2graph.SetDimensions).
    let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../d2-rs-text/assets/SourceCodePro-Regular.ttf");
    for engine in ["dagre", "elk", "tala"] {
        for (role, width) in [
            ("regular", "436"),
            ("italic", "436"),
            ("bold", "467"),
            ("semibold", "436"),
            // A mono override leaves the proportional family unspecified,
            // so Go still selects HandDrawn for the shape label.
            ("mono", "464"),
        ] {
            for configured_sketch in [false, true] {
                let mut command = Command::new(env!("CARGO_BIN_EXE_d2-rs"));
                command.args([
                    &format!("--layout={engine}"),
                    &format!("--font-{role}={}", font.display()),
                ]);
                if !configured_sketch {
                    command.arg("--sketch");
                }
                let mut child = command
                    .args(["-", "-"])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                let source = if configured_sketch {
                    "vars: {d2-config: {sketch: true}}\na: Custom font measurement\n"
                } else {
                    "a: Custom font measurement\n"
                };
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(source.as_bytes())
                    .unwrap();
                let output = child.wait_with_output().unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let svg = String::from_utf8(output.stdout).unwrap();
                let document = roxmltree::Document::parse(&svg).unwrap();
                assert_eq!(
                    document.root_element().attribute("viewBox"),
                    Some(format!("0 0 {width} 268").as_str()),
                    "{engine}/{role}/configured-sketch={configured_sketch}"
                );
            }
        }
    }
}
