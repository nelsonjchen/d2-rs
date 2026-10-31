// SPDX-License-Identifier: MPL-2.0
//! Test-only transport: run the original Go callback at each mutation boundary.
use super::*;
use std::io::{BufRead, Write};

#[test]
#[ignore = "original Go transaction tests with synchronous callback transport"]
fn replay_transaction() {
    let request: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var_os("D2RS_TRANSACTION_REQUEST").unwrap()).unwrap(),
    )
    .unwrap();
    let mut workspace = BoardOutputWorkspace {
        final_root: PathBuf::from(request["finalRoot"].as_str().unwrap()),
        stage_root: Some(PathBuf::from(request["stageRoot"].as_str().unwrap())),
        visible_mutation: false,
        before_publish: Some(Box::new(|path| {
            println!("D2RS_CALLBACK {}", serde_json::to_string(path).unwrap());
            std::io::stdout().flush().unwrap();
            let mut line = String::new();
            assert!(std::io::stdin().lock().read_line(&mut line).unwrap() > 0);
            let error: String = serde_json::from_str(&line).unwrap();
            if error.is_empty() { Ok(()) } else { Err(error) }
        })),
    };
    let result = workspace.publish_inner();
    workspace.discard();
    let (touched, error) = match result {
        Ok(touched) => (touched, String::new()),
        Err(error) => (workspace.visible_mutation, error),
    };
    std::fs::write(
        std::env::var_os("D2RS_TRANSACTION_RESPONSE").unwrap(),
        serde_json::to_vec(&serde_json::json!({"touched": touched, "error": error})).unwrap(),
    )
    .unwrap();
}

#[cfg(unix)]
mod contracts {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "d2rs-transaction-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn prepared() -> (Scratch, BoardOutputWorkspace, Vec<u8>) {
        let scratch = Scratch::new();
        let root = scratch.0.join("out");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("stale")).unwrap();
        set_mode(&root.join("stale"), 0o750).unwrap();
        std::fs::write(root.join("b-replaced.svg"), b"old replacement").unwrap();
        set_mode(&root.join("b-replaced.svg"), 0o640).unwrap();
        std::fs::write(root.join("stale/old.svg"), b"generated").unwrap();
        let previous = Manifest {
            files: vec![
                (
                    "b-replaced.svg".into(),
                    file_digest(&root.join("b-replaced.svg")).unwrap(),
                ),
                (
                    "stale/old.svg".into(),
                    file_digest(&root.join("stale/old.svg")).unwrap(),
                ),
            ],
            directories: vec!["stale".into()],
        }
        .encode();
        std::fs::write(root.join(BOARD_OUTPUT_MANIFEST_NAME), &previous).unwrap();
        let workspace = BoardOutputWorkspace::new(&root).unwrap();
        std::fs::create_dir(workspace.stage_root().join("new-directory")).unwrap();
        std::fs::write(workspace.stage_root().join("a-created.svg"), b"created").unwrap();
        std::fs::write(
            workspace.stage_root().join("b-replaced.svg"),
            b"replacement",
        )
        .unwrap();
        (scratch, workspace, previous)
    }

    #[test]
    fn manifest_failure_restores_files_modes_directories_and_ownership() {
        let (_scratch, mut workspace, previous) = prepared();
        let root = workspace.final_root.clone();
        let stage = workspace.stage_root().to_owned();
        workspace.before_publish = Some(Box::new(|path| {
            if path.file_name().unwrap() == BOARD_OUTPUT_MANIFEST_NAME {
                Err("injected failure".into())
            } else {
                Ok(())
            }
        }));
        assert!(
            workspace
                .publish_inner()
                .unwrap_err()
                .ends_with(": injected failure")
        );
        assert!(!workspace.visible_mutation);
        workspace.discard();
        assert_eq!(
            std::fs::read(root.join("b-replaced.svg")).unwrap(),
            b"old replacement"
        );
        assert_eq!(
            mode(&std::fs::metadata(root.join("b-replaced.svg")).unwrap()),
            0o640
        );
        assert_eq!(mode(&std::fs::metadata(root.join("stale")).unwrap()), 0o750);
        assert_eq!(
            std::fs::read(root.join("stale/old.svg")).unwrap(),
            b"generated"
        );
        assert_eq!(
            std::fs::read(root.join(BOARD_OUTPUT_MANIFEST_NAME)).unwrap(),
            previous
        );
        assert!(!root.join("a-created.svg").exists());
        assert!(!root.join("new-directory").exists());
        assert!(!stage.exists());
    }

    #[test]
    fn actual_source_read_failure_rolls_back_prior_publication() {
        let (_scratch, mut workspace, previous) = prepared();
        let root = workspace.final_root.clone();
        let source = workspace.stage_root().join("b-replaced.svg");
        workspace.before_publish = Some(Box::new(move |path| {
            if path.file_name().unwrap() == "b-replaced.svg" {
                std::fs::remove_file(&source).unwrap();
            }
            Ok(())
        }));
        assert!(workspace.publish_inner().is_err());
        assert!(!workspace.visible_mutation);
        assert!(!root.join("a-created.svg").exists());
        assert!(!root.join("new-directory").exists());
        assert_eq!(
            std::fs::read(root.join("b-replaced.svg")).unwrap(),
            b"old replacement"
        );
        assert_eq!(
            std::fs::read(root.join(BOARD_OUTPUT_MANIFEST_NAME)).unwrap(),
            previous
        );
    }

    #[test]
    fn stale_file_edit_is_preserved_and_relinquished() {
        let (_scratch, mut workspace, _) = prepared();
        let root = workspace.final_root.clone();
        workspace.before_publish = Some(Box::new(|path| {
            if path.ends_with("stale/old.svg") {
                std::fs::write(path, b"user edit").unwrap();
            }
            Ok(())
        }));
        assert!(workspace.publish().unwrap());
        assert_eq!(
            std::fs::read(root.join("stale/old.svg")).unwrap(),
            b"user edit"
        );
        assert!(
            Manifest::read(&root)
                .unwrap()
                .unwrap()
                .files
                .iter()
                .all(|(path, _)| path != "stale/old.svg")
        );
    }

    #[test]
    fn ancestor_swap_cannot_escape_and_created_directories_roll_back() {
        let scratch = Scratch::new();
        let root = scratch.0.join("out");
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join("nested/sentinel"), b"keep").unwrap();
        let victim = scratch.0.join("victim");
        std::fs::create_dir(&victim).unwrap();
        let moved = scratch.0.join("moved");
        let mut workspace = BoardOutputWorkspace::new(&root).unwrap();
        std::fs::create_dir(workspace.stage_root().join("a-created")).unwrap();
        std::fs::create_dir_all(workspace.stage_root().join("nested/leaf")).unwrap();
        std::fs::write(workspace.stage_root().join("nested/leaf/board.svg"), b"new").unwrap();
        let old_nested = root.join("nested");
        let destination = moved.clone();
        let outside = victim.clone();
        workspace.before_publish = Some(Box::new(move |path| {
            if path.ends_with("nested/leaf") {
                std::fs::rename(&old_nested, &destination).unwrap();
                std::os::unix::fs::symlink(&outside, &old_nested).unwrap();
            }
            Ok(())
        }));
        assert!(
            workspace
                .publish_inner()
                .unwrap_err()
                .ends_with("path escapes from parent")
        );
        assert!(!workspace.visible_mutation);
        assert!(!root.join("a-created").exists());
        assert!(!victim.join("leaf").exists());
        assert_eq!(std::fs::read(moved.join("sentinel")).unwrap(), b"keep");
    }
}
