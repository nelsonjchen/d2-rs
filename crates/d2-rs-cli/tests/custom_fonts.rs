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

#[test]
fn custom_font_roles_keep_their_face_bytes_in_png() {
    // Pinned d2cli.loadFonts -> d2fonts.AddFontFamily retains each supplied
    // TTF, filling absent styles with SourceSansPro. d2scenebuild builder.font
    // selects that exact asset even if --font-bold receives a regular face.
    // Complete PNG digests below are from the pinned Go CLI with this shipped
    // SourceCodePro face; the test needs neither Go nor regenerated fixtures.
    let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../d2-rs-text/assets/SourceCodePro-Regular.ttf");
    for (role, style, sketch, expected) in [
        (
            "regular",
            "plain",
            false,
            ["79f62a74ee9a9b3cea4b35580e943456ac66cef41076b9e36c07dc1a944a9553"; 3],
        ),
        (
            "regular",
            "plain",
            true,
            [
                "151ff312e23b82de085834d134c59e4406c3f153e2e02ec48a73fd86aef4b3a5",
                "05a5932ceb0a1bbe312b3ff790dac2ec3f81295e5a1b37df8ad79b3f4959b39c",
                "151ff312e23b82de085834d134c59e4406c3f153e2e02ec48a73fd86aef4b3a5",
            ],
        ),
        (
            "bold",
            "bold",
            false,
            ["67606642da2e22b3b2d585b228c91c307b96da17b32e0295685364a11268cf23"; 3],
        ),
        (
            "bold",
            "bold",
            true,
            [
                "789dc244a6108583cf6cb23019933855b796d3a8c532bd9ef0165f9e3359420e",
                "a2c5b3f4b73a37c9d804bfb1864e31d9a125695a5a608c4fe61097010ee55552",
                "789dc244a6108583cf6cb23019933855b796d3a8c532bd9ef0165f9e3359420e",
            ],
        ),
        (
            "italic",
            "italic",
            false,
            ["0353bcf442bce8270ca3e9aadd75f07d75960fd03b76ef4b8cd7fc920f26e64c"; 3],
        ),
        (
            "italic",
            "italic",
            true,
            [
                "417a0509d11a8f17394d5feb9bccc3bb04d4d9d188781eeeb093170080bf68df",
                "62c6c2222e178a0f70d9ce4e3e74a54953446581b92f50f1999fb4660c3c6660",
                "417a0509d11a8f17394d5feb9bccc3bb04d4d9d188781eeeb093170080bf68df",
            ],
        ),
        (
            "regular",
            "italic",
            false,
            ["0353bcf442bce8270ca3e9aadd75f07d75960fd03b76ef4b8cd7fc920f26e64c"; 3],
        ),
        (
            "regular",
            "italic",
            true,
            [
                "417a0509d11a8f17394d5feb9bccc3bb04d4d9d188781eeeb093170080bf68df",
                "62c6c2222e178a0f70d9ce4e3e74a54953446581b92f50f1999fb4660c3c6660",
                "417a0509d11a8f17394d5feb9bccc3bb04d4d9d188781eeeb093170080bf68df",
            ],
        ),
        (
            "semibold",
            "bold",
            false,
            ["79f62a74ee9a9b3cea4b35580e943456ac66cef41076b9e36c07dc1a944a9553"; 3],
        ),
        (
            "semibold",
            "bold",
            true,
            [
                "151ff312e23b82de085834d134c59e4406c3f153e2e02ec48a73fd86aef4b3a5",
                "05a5932ceb0a1bbe312b3ff790dac2ec3f81295e5a1b37df8ad79b3f4959b39c",
                "151ff312e23b82de085834d134c59e4406c3f153e2e02ec48a73fd86aef4b3a5",
            ],
        ),
    ] {
        for (engine, expected) in ["dagre", "elk", "tala"].into_iter().zip(expected) {
            let mut command = Command::new(env!("CARGO_BIN_EXE_d2-rs"));
            command.args([
                &format!("--layout={engine}"),
                &format!("--font-{role}={}", font.display()),
                "--stdout-format=png",
            ]);
            if sketch {
                command.arg("--sketch");
            }
            let mut child = command
                .args(["-", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let source = if style == "plain" {
                "a: Custom font measurement\n".to_owned()
            } else {
                format!("a: Custom font measurement {{style.{style}: true}}\n")
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
            let digest = ring::digest::digest(&ring::digest::SHA256, &output.stdout);
            let actual = digest
                .as_ref()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            assert_eq!(actual, expected, "{engine}/{role}/{style}/sketch={sketch}");
        }
    }
}

#[test]
fn custom_primary_fonts_also_shape_appendix_text_and_numbers() {
    // Pinned d2scenebuild/appendix.go buildNumberedAppendixIcon and
    // buildAppendix use builder.font with the default family role. These
    // complete PNG digests come from pinned Go using the shipped regular
    // SourceCodePro face, including when supplied as the bold/italic role.
    let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../d2-rs-text/assets/SourceCodePro-Regular.ttf");
    for (role, sketch, expected) in [
        (
            "regular",
            false,
            [
                "bf47446f39008a53076b2a75d45e9cbbfdf113f269bef5589556f66cb6e23006",
                "799a80c35d839279ece91fe31efcd1861d334505878072e16f57840070e2642b",
                "bf47446f39008a53076b2a75d45e9cbbfdf113f269bef5589556f66cb6e23006",
            ],
        ),
        (
            "regular",
            true,
            [
                "5331b10615d1563469aa1d91db852a09b969f0c7feaf20d4152e90a7256df853",
                "e8319c098e5d54b334364d678023b0bc159beadfeaa34af4c1ea96be878d4521",
                "5331b10615d1563469aa1d91db852a09b969f0c7feaf20d4152e90a7256df853",
            ],
        ),
        (
            "bold",
            false,
            [
                "3366d0f76fdc3a750b3c5b6368247f32ed3e9ae3293bba99b2a2cbb47d85f559",
                "f961856c0b818a7bc3a02bfc6a8637385512d91f49a315a528f5b2a5a3dd7a4f",
                "3366d0f76fdc3a750b3c5b6368247f32ed3e9ae3293bba99b2a2cbb47d85f559",
            ],
        ),
        (
            "bold",
            true,
            [
                "5a631257112418a7944c3210def40a5141c69d1771fdadac9dae147d6b5a1307",
                "a5006c1f8976807fe615dd911e815151768f5ce859ef6bc7e0d5f0ff5263e2d3",
                "5a631257112418a7944c3210def40a5141c69d1771fdadac9dae147d6b5a1307",
            ],
        ),
        (
            "italic",
            false,
            [
                "2119309207a4b566abd917d74e62cce2ab717242c041c129a70f67bf6c146d11",
                "c00f6bfeb7301acfc1eb0430d7161c198617d09b3d02e6f2d3bd7ee2ac4d2e58",
                "2119309207a4b566abd917d74e62cce2ab717242c041c129a70f67bf6c146d11",
            ],
        ),
        (
            "italic",
            true,
            [
                "9f792ca9a2be26acde8914e720b3a3b62e8966b32b9868e9972f50870295302c",
                "4f0dabf203e9ab85ff0e83e33e7d6471cfbbe2e172b3e0201b546f7538fbc9f9",
                "9f792ca9a2be26acde8914e720b3a3b62e8966b32b9868e9972f50870295302c",
            ],
        ),
        (
            "mono",
            false,
            [
                "2119309207a4b566abd917d74e62cce2ab717242c041c129a70f67bf6c146d11",
                "c00f6bfeb7301acfc1eb0430d7161c198617d09b3d02e6f2d3bd7ee2ac4d2e58",
                "2119309207a4b566abd917d74e62cce2ab717242c041c129a70f67bf6c146d11",
            ],
        ),
        (
            "mono",
            true,
            [
                "e6e9556ef1c866101b09a5dac6df4ec9e9690da4063ace86db0aa6a992001303",
                "a438f303d8c1a17ea96a81daf463e0a19404f1a0a7f6a9e8cabd460495b8ecb7",
                "e6e9556ef1c866101b09a5dac6df4ec9e9690da4063ace86db0aa6a992001303",
            ],
        ),
    ] {
        for (engine, expected) in ["dagre", "elk", "tala"].into_iter().zip(expected) {
            let mut command = Command::new(env!("CARGO_BIN_EXE_d2-rs"));
            command.args([
                &format!("--layout={engine}"),
                &format!("--font-{role}={}", font.display()),
                "--stdout-format=png",
            ]);
            if sketch {
                command.arg("--sketch");
            }
            let mut child = command
                .args(["-", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"a: A label {link: https://example.com}\n")
                .unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let digest = ring::digest::digest(&ring::digest::SHA256, &output.stdout);
            let actual = digest
                .as_ref()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            assert_eq!(actual, expected, "{engine}/{role}/sketch={sketch}");
        }
    }
}

#[test]
fn custom_markdown_faces_measure_native_primitive_advances() {
    // Pinned d2scenebuild/markdown.go markdownFontFamilies and LayoutMarkdown
    // use the configured faces for the ruler as well as the painted glyphs.
    // Complete PNG digests were captured independently from pinned Go with
    // the shipped SourceCodePro regular face in each CLI role.
    let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../d2-rs-text/assets/SourceCodePro-Regular.ttf");
    for (role, sketch, expected) in [
        (
            "regular",
            false,
            [
                "6bc7f2f1237dddc7dd4327d4e089ed792af9ad7d5e0d52a75b8abe06cfdcd710",
                "6bc7f2f1237dddc7dd4327d4e089ed792af9ad7d5e0d52a75b8abe06cfdcd710",
                "6bc7f2f1237dddc7dd4327d4e089ed792af9ad7d5e0d52a75b8abe06cfdcd710",
            ],
        ),
        (
            "regular",
            true,
            [
                "6bc7f2f1237dddc7dd4327d4e089ed792af9ad7d5e0d52a75b8abe06cfdcd710",
                "6bc7f2f1237dddc7dd4327d4e089ed792af9ad7d5e0d52a75b8abe06cfdcd710",
                "6bc7f2f1237dddc7dd4327d4e089ed792af9ad7d5e0d52a75b8abe06cfdcd710",
            ],
        ),
        (
            "bold",
            false,
            [
                "486359fdac291aaf13af8c76c92528cc019016413660a6dca001c5d445a01984",
                "486359fdac291aaf13af8c76c92528cc019016413660a6dca001c5d445a01984",
                "486359fdac291aaf13af8c76c92528cc019016413660a6dca001c5d445a01984",
            ],
        ),
        (
            "bold",
            true,
            [
                "486359fdac291aaf13af8c76c92528cc019016413660a6dca001c5d445a01984",
                "486359fdac291aaf13af8c76c92528cc019016413660a6dca001c5d445a01984",
                "486359fdac291aaf13af8c76c92528cc019016413660a6dca001c5d445a01984",
            ],
        ),
        (
            "italic",
            false,
            [
                "ac4cd7b44a160fde0046549d6f31318f36baa856380cca81701cd1655a696460",
                "ac4cd7b44a160fde0046549d6f31318f36baa856380cca81701cd1655a696460",
                "ac4cd7b44a160fde0046549d6f31318f36baa856380cca81701cd1655a696460",
            ],
        ),
        (
            "italic",
            true,
            [
                "ac4cd7b44a160fde0046549d6f31318f36baa856380cca81701cd1655a696460",
                "ac4cd7b44a160fde0046549d6f31318f36baa856380cca81701cd1655a696460",
                "ac4cd7b44a160fde0046549d6f31318f36baa856380cca81701cd1655a696460",
            ],
        ),
        (
            "semibold",
            false,
            [
                "c5175f77a31f1ab85e24fbd1418b158f3cf896d10d7215895b87524ffdf23ef7",
                "c5175f77a31f1ab85e24fbd1418b158f3cf896d10d7215895b87524ffdf23ef7",
                "c5175f77a31f1ab85e24fbd1418b158f3cf896d10d7215895b87524ffdf23ef7",
            ],
        ),
        (
            "semibold",
            true,
            [
                "c5175f77a31f1ab85e24fbd1418b158f3cf896d10d7215895b87524ffdf23ef7",
                "c5175f77a31f1ab85e24fbd1418b158f3cf896d10d7215895b87524ffdf23ef7",
                "c5175f77a31f1ab85e24fbd1418b158f3cf896d10d7215895b87524ffdf23ef7",
            ],
        ),
        (
            "mono",
            false,
            [
                "f9285bc5e5419b18eaf2e293b144e7effdf1ef07b1c19d69b22bfde9f92847aa",
                "f9285bc5e5419b18eaf2e293b144e7effdf1ef07b1c19d69b22bfde9f92847aa",
                "f9285bc5e5419b18eaf2e293b144e7effdf1ef07b1c19d69b22bfde9f92847aa",
            ],
        ),
        (
            "mono",
            true,
            [
                "dfdd8c04c83581260cbf288c850df69999eeb83fcb4c6333baf141a7b9b0d97a",
                "dfdd8c04c83581260cbf288c850df69999eeb83fcb4c6333baf141a7b9b0d97a",
                "dfdd8c04c83581260cbf288c850df69999eeb83fcb4c6333baf141a7b9b0d97a",
            ],
        ),
    ] {
        for (engine, expected) in ["dagre", "elk", "tala"].into_iter().zip(expected) {
            let mut command = Command::new(env!("CARGO_BIN_EXE_d2-rs"));
            command.args([
                &format!("--layout={engine}"),
                &format!("--font-{role}={}", font.display()),
                "--stdout-format=png",
            ]);
            if sketch {
                command.arg("--sketch");
            }
            let mut child = command
                .args(["-", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"a: |md\n  # Heading\n  plain **bold** *italic* `code`\n|\n")
                .unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let digest = ring::digest::digest(&ring::digest::SHA256, &output.stdout);
            let actual = digest
                .as_ref()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            assert_eq!(actual, expected, "{engine}/{role}/sketch={sketch}");
        }
    }
}
