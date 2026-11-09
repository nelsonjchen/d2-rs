// SPDX-License-Identifier: MPL-2.0
#![cfg(unix)]

use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::process::Command;

struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        // macOS clocks tick in microseconds, so parallel tests can read the
        // same time; a per-process counter keeps the directories distinct.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "d2-output-modes-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn compile(input: &Path, output: &Path, mask: &str, flags: &[&str]) {
    // Set the umask in the child only, avoiding process-global test races.
    let result = Command::new("sh")
        .args([
            "-c",
            "umask \"$1\"; shift; exec \"$@\"",
            "output-modes",
            mask,
        ])
        .arg(env!("CARGO_BIN_EXE_d2-rs"))
        .arg("--layout=dagre")
        .args(flags)
        .arg(input)
        .arg(output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

fn mode(path: &Path) -> u32 {
    let metadata = std::fs::metadata(path).unwrap();
    assert!(metadata.len() > 0, "{}", path.display());
    metadata.permissions().mode() & 0o777
}

#[test]
fn output_modes_follow_each_formats_go_publication_contract() {
    // Pinned d2cli/main.go writeWithStatus, raster_png_output.go
    // writePNGWithStatus: CreateTemp + Rename gives 0600 on publication.
    // lib/pdf/pdf.go and lib/pptx/pptx.go use os.Create (0666 & umask),
    // which preserves existing files and follows symlinks.
    let directory = Directory::new();
    let input = directory.0.join("input.d2");
    std::fs::write(&input, "a -> b\n").unwrap();
    for (mask, mask_bits) in [("000", 0), ("027", 0o027), ("077", 0o077)] {
        for format in ["svg", "png", "gif", "txt", "pdf", "pptx"] {
            let paged = matches!(format, "pdf" | "pptx");
            for state in ["new", "existing", "symlink"] {
                let output = directory.0.join(format!("{mask}-{state}.{format}"));
                let target = directory.0.join(format!("{mask}-{state}-{format}-target"));
                if state != "new" {
                    std::fs::write(&target, b"sentinel").unwrap();
                    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640))
                        .unwrap();
                    if state == "symlink" {
                        symlink(&target, &output).unwrap();
                    } else {
                        std::fs::write(&output, b"sentinel").unwrap();
                        std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o640))
                            .unwrap();
                    }
                }
                compile(&input, &output, mask, &[]);
                let expected = if paged {
                    if state == "new" {
                        0o666 & !mask_bits
                    } else {
                        0o640
                    }
                } else {
                    0o600
                };
                assert_eq!(mode(&output), expected, "{mask}/{state}/{format}");
                assert_eq!(output.is_symlink(), paged && state == "symlink");
                if state == "symlink" && !paged {
                    assert_eq!(std::fs::read(&target).unwrap(), b"sentinel");
                }
            }
        }
    }
}

#[test]
fn recursive_and_animated_outputs_publish_private_files() {
    let directory = Directory::new();
    let input = directory.0.join("input.d2");
    std::fs::write(&input, "a -> b\nlayers: {child: {c -> d}}\n").unwrap();
    for format in ["svg", "png", "txt"] {
        let output = directory.0.join(format!("tree.{format}"));
        compile(&input, &output, "000", &[]);
        assert_eq!(
            mode(&directory.0.join("tree/.d2-board-output-manifest-v1.json")),
            0o644
        );
        for board in ["index", "child"] {
            assert_eq!(
                mode(&directory.0.join(format!("tree/{board}.{format}"))),
                0o600
            );
        }
    }
    for format in ["svg", "gif"] {
        let output = directory.0.join(format!("animated.{format}"));
        compile(&input, &output, "000", &["--animate-interval=1000"]);
        assert_eq!(mode(&output), 0o600);
    }
}
