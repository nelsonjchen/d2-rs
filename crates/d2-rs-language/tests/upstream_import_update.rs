// SPDX-License-Identifier: MPL-2.0
//! d2ir/import_test.go contracts added by 816c893b6.
use d2_rs_core::{ast::Shared, ir::Node};
use d2_rs_language::{
    compiler::{self, imports::native_import_path},
    parse,
};
use std::{collections::HashMap, path::PathBuf};

fn assert_meow(c: &compiler::Compiler, root: d2_rs_core::ir::NodeId) {
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    let field = c.arena.get_field_unquoted(root, &["x"]).unwrap();
    let Node::Scalar(s) = c.arena.node(c.arena.primary(field).unwrap()) else {
        unreachable!()
    };
    assert_eq!(s.value.scalar_string(), "meow");
}
#[test]
fn imports_resolve_next_to_native_input_and_nested_importer() {
    // Keep native test files within the shared cargo target / worktree.
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
    let dir = target.join(format!("language-native-import-{}", std::process::id()));
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let _cleanup = Cleanup(dir.clone());
    std::fs::write(dir.join("sub/a.d2"), "...@b").unwrap();
    std::fs::write(dir.join("sub/b.d2"), "x: meow").unwrap();
    let parsed = parse(dir.join("index.d2").to_str().unwrap(), b"...@sub/a", None);
    assert!(parsed.error.is_none());
    let (c, root) = compiler::compile_ir_with_import_resolver(
        Shared::new(parsed.map),
        compiler::CompileOptions {
            import_opener: Some(Box::new(|path| {
                std::fs::File::open(path)
                    .map(|file| Box::new(file) as Box<dyn d2_rs_language::stream::ByteReader>)
                    .map_err(|e| e.to_string())
            })),
            ..Default::default()
        },
        Default::default(),
        None,
        Some(Box::new(native_import_path)),
    )
    .unwrap();
    assert_meow(&c, root);
}
#[test]
fn virtual_directory_backslashes_are_literal_on_every_host() {
    let parsed = parse(r"team\archive/index.d2", b"...@sub/a", None);
    let files: HashMap<_, _> = [
        (r"team\archive/sub/a.d2", "...@b"),
        (r"team\archive/sub/b.d2", "x: meow"),
    ]
    .into_iter()
    .collect();
    let (c, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions {
            import_opener: Some(Box::new(move |name| {
                files
                    .get(name)
                    .map(|text| {
                        Box::new(std::io::Cursor::new(text.as_bytes().to_vec()))
                            as Box<dyn d2_rs_language::stream::ByteReader>
                    })
                    .ok_or_else(|| format!("missing {name}"))
            })),
            ..Default::default()
        },
    )
    .unwrap();
    assert_meow(&c, root);
}
#[test]
fn native_resolver_cleans_relative_names_and_keeps_absolute_names() {
    let parent = PathBuf::from("gallery").join("sub").join("input.d2");
    assert_eq!(
        native_import_path(parent.to_str().unwrap(), "../sibling.d2"),
        PathBuf::from("gallery")
            .join("sibling.d2")
            .to_str()
            .unwrap()
    );
    let absolute = std::env::current_dir().unwrap().join("elsewhere.d2");
    assert_eq!(
        native_import_path(parent.to_str().unwrap(), absolute.to_str().unwrap()),
        absolute.to_str().unwrap()
    );
}
#[cfg(windows)]
#[test]
fn native_drive_and_share_roots_match_go_localfile() {
    for (parent, name, want) in [
        (
            r"C:\gallery\radio.d2",
            "sub/tideglass.d2",
            r"C:\gallery\sub\tideglass.d2",
        ),
        (r"gallery\radio.d2", "tideglass.d2", r"gallery\tideglass.d2"),
        (
            r"\\studio\diagrams\radio.d2",
            "sub/tideglass.d2",
            r"\\studio\diagrams\sub\tideglass.d2",
        ),
        (
            r"\\studio\diagrams\sub\radio.d2",
            "../tideglass.d2",
            r"\\studio\diagrams\tideglass.d2",
        ),
        (
            r"C:\gallery\radio.d2",
            r"D:\other\tideglass.d2",
            r"D:\other\tideglass.d2",
        ),
        (
            r"C:\gallery\radio.d2",
            r"\\studio\diagrams\tideglass.d2",
            r"\\studio\diagrams\tideglass.d2",
        ),
    ] {
        assert_eq!(native_import_path(parent, name), want);
    }
}
