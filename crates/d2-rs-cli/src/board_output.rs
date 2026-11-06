// SPDX-License-Identifier: MPL-2.0
//! d2cli/board_output.go and board_output_publish.go: portable board output
//! names and transactional publication of a multiboard output tree.
//!
//! Boards render into a private staging directory beside the output root.
//! Publishing renames it into place when the root is absent; otherwise it
//! installs the generated files, removes only unchanged stale files recorded
//! by the previous manifest, removes emptied stale directories, and writes a
//! new `.d2-board-output-manifest-v1.json`. Unrelated files are preserved.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

#[cfg(test)]
#[path = "board_output/link_api_tests.rs"]
mod link_api_tests;
#[cfg(test)]
#[path = "board_output/manifest_api_tests.rs"]
mod manifest_api_tests;
#[cfg(test)]
#[path = "board_output/path_api_tests.rs"]
mod path_api_tests;
mod root;
mod transaction;
#[cfg(test)]
#[path = "board_output/transaction_api_tests.rs"]
mod transaction_api_tests;
mod unicode_fold;

const ESCAPED_BOARD_OUTPUT_PREFIX: &str = "_d2_";
const MAX_PORTABLE_BOARD_OUTPUT_COMPONENT_BYTES: usize = 200;
pub(crate) const BOARD_OUTPUT_MANIFEST_NAME: &str = ".d2-board-output-manifest-v1.json";
const BOARD_OUTPUT_MANIFEST_FORMAT: &str = "d2-board-output";
const MAX_BOARD_OUTPUT_MANIFEST_BYTES: u64 = 16 << 20;

/// boardOutputComponent: ordinary board names keep their spelling; names with
/// filesystem meaning (or the reserved prefix) become `_d2_<sha256 hex>`.
/// Go strings can contain invalid UTF-8; hash those original bytes too.
pub(crate) fn board_output_component(name: impl AsRef<[u8]>) -> String {
    let name = name.as_ref();
    if let Ok(text) = std::str::from_utf8(name)
        && is_portable_component(text)
        && !is_reserved_component(text)
    {
        return text.to_owned();
    }
    let digest = ring::digest::digest(&ring::digest::SHA256, name);
    format!("{ESCAPED_BOARD_OUTPUT_PREFIX}{}", hex(digest.as_ref()))
}

fn is_reserved_component(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.starts_with(ESCAPED_BOARD_OUTPUT_PREFIX)
        || matches!(lower.as_str(), "index" | "layers" | "scenarios" | "steps")
        || lower == BOARD_OUTPUT_MANIFEST_NAME
}

fn is_portable_component(name: &str) -> bool {
    if name.is_empty()
        || name.len() > MAX_PORTABLE_BOARD_OUTPUT_COMPONENT_BYTES
        || name == "."
        || name == ".."
        || name.ends_with('.')
        || name.ends_with(' ')
        || name.starts_with('/')
        || name.starts_with('\\')
    {
        return false;
    }
    // filepath.IsLocal on Unix: no absolute path and no `..` element.
    if name.split('/').any(|element| element == "..") {
        return false;
    }
    if name
        .chars()
        .any(|c| (c as u32) < 0x20 || c == '\u{7f}' || "<>:\"/\\|?*".contains(c))
    {
        return false;
    }
    // Windows reserves these names even when they have an extension.
    let base = name.find(['.', ':']).map_or(name, |index| &name[..index]);
    !matches!(
        base.trim_end_matches(' ').to_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "CLOCK$"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
            | "COM¹"
            | "COM²"
            | "COM³"
            | "LPT¹"
            | "LPT²"
            | "LPT³"
            | "CONIN$"
            | "CONOUT$"
    )
}

/// d2cli/board_output.go boardOutputCollisionKey: full Unicode case folding
/// followed by NFC, independent of the host filesystem's spelling rules.
fn collision_key(path: &Path) -> String {
    fold_normalized_path(&clean_path(path).to_string_lossy())
}

fn clean_path(path: &Path) -> PathBuf {
    // filepath.Clean is lexical; do not resolve symlinks or touch the disk.
    let mut clean = PathBuf::new();
    for component in path.components() {
        use std::path::Component;
        match component {
            Component::CurDir => {}
            Component::ParentDir => match clean.components().next_back() {
                Some(Component::Normal(_)) => {
                    clean.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => clean.push(component.as_os_str()),
            },
            _ => clean.push(component.as_os_str()),
        }
    }
    if clean.as_os_str().is_empty() {
        clean.push(".");
    }
    clean
}

/// d2cli/board_output.go ensurePathWithin: lexical containment, without
/// resolving symlinks. Publication separately uses retained directory handles.
fn ensure_path_within(root: &Path, path: &Path) -> Result<(), String> {
    // filepath.Abs("") uses the current directory; Rust absolute rejects it.
    let absolute = |value: &Path| {
        std::path::absolute(if value.as_os_str().is_empty() {
            Path::new(".")
        } else {
            value
        })
        .map(|value| clean_path(&value))
        .map_err(|error| error.to_string())
    };
    let root_abs = absolute(root)?;
    let path_abs = absolute(path)?;
    if !path_abs.starts_with(&root_abs) {
        return Err(format!(
            "path {} escapes root {}",
            d2_rs_core::quote_go_bytes(path.as_os_str().as_encoded_bytes()),
            d2_rs_core::quote_go_bytes(root.as_os_str().as_encoded_bytes())
        ));
    }
    Ok(())
}

/// Shared by the CLI's board planner and original append-path observations.
pub(crate) fn contained_board_output_component(
    root: &Path,
    component: &str,
) -> Result<PathBuf, String> {
    let joined = root.join(component);
    ensure_path_within(root, &joined).map_err(|error| {
        format!(
            "invalid board output component {}: {error}",
            d2_rs_core::quote_go_bytes(component.as_bytes())
        )
    })?;
    Ok(joined)
}

fn fold_normalized_path(path: &str) -> String {
    let mut folded = String::new();
    for ch in path.chars() {
        match unicode_fold::FOLD.binary_search_by_key(&ch, |&(key, _)| key) {
            Ok(i) => folded.push_str(unicode_fold::FOLD[i].1),
            Err(_) => folded.push(ch),
        }
    }
    // x/text norm counts backward-combining starters (including Hangul)
    // as non-starters too. Preserve that policy before Rust's NFC pass.
    let mut safe = String::with_capacity(folded.len());
    let mut nonstarters = 0u8;
    for ch in folded.chars() {
        let (lead, trail) = if ('\u{ac00}'..'\u{d7a4}').contains(&ch) {
            (
                0,
                if (ch as u32 - 0xac00).is_multiple_of(28) {
                    1
                } else {
                    2
                },
            )
        } else {
            unicode_fold::STREAM_SAFE
                .binary_search_by_key(&ch, |&(key, _, _)| key)
                .map(|i| {
                    let (_, lead, trail) = unicode_fold::STREAM_SAFE[i];
                    (lead, trail)
                })
                .unwrap_or((0, 0))
        };
        if nonstarters + lead > 30 {
            safe.push('\u{34f}');
            nonstarters = 0;
        }
        nonstarters = if lead == 0 { trail } else { nonstarters + lead };
        safe.push(ch);
    }
    safe.nfc().collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn file_digest(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    Ok(hex(
        ring::digest::digest(&ring::digest::SHA256, &bytes).as_ref()
    ))
}

/// sameBoardOutputPathRoot: equal portable keys need not denote the same
/// file on a spelling-sensitive filesystem. Retire only the actual old file.
#[cfg(test)]
fn same_output_path(root: &Path, previous: &str, current: &str) -> bool {
    if previous == current {
        return true;
    }
    let (Ok(old), Ok(new)) = (
        std::fs::metadata(root.join(previous)),
        std::fs::metadata(root.join(current)),
    ) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        old.dev() == new.dev() && old.ino() == new.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (old, new);
        matches!((std::fs::canonicalize(root.join(previous)), std::fs::canonicalize(root.join(current))), (Ok(old), Ok(new)) if old == new)
    }
}

/// validateBoardOutputPaths: no two boards may resolve to the same file, and
/// no board file may occupy a directory another board needs.
pub(crate) fn validate_board_output_paths(outputs: &[(String, PathBuf)]) -> Result<(), String> {
    let mut files: BTreeMap<String, (String, PathBuf)> = BTreeMap::new();
    let mut dirs: BTreeMap<String, String> = BTreeMap::new();
    for (board, path) in outputs {
        let absolute = std::path::absolute(path).map_err(|error| error.to_string())?;
        let key = collision_key(&absolute);
        if let Some((previous, _)) = files.get(&key) {
            return Err(format!(
                "boards {} and {} resolve to the same output path {}",
                d2_rs_core::quote_go_bytes(previous.as_bytes()),
                d2_rs_core::quote_go_bytes(board.as_bytes()),
                d2_rs_core::quote_go_bytes(path.to_string_lossy().as_bytes())
            ));
        }
        if let Some(previous) = dirs.get(&key) {
            return Err(format!(
                "board {} output file {} collides with a directory required by board {}",
                d2_rs_core::quote_go_bytes(board.as_bytes()),
                d2_rs_core::quote_go_bytes(path.to_string_lossy().as_bytes()),
                d2_rs_core::quote_go_bytes(previous.as_bytes())
            ));
        }
        let mut dir = absolute.parent();
        while let Some(current) = dir {
            let dir_key = collision_key(current);
            if let Some((previous, previous_path)) = files.get(&dir_key) {
                return Err(format!(
                    "board {} output file {} requires a directory occupied by board {} output {}",
                    d2_rs_core::quote_go_bytes(board.as_bytes()),
                    d2_rs_core::quote_go_bytes(path.to_string_lossy().as_bytes()),
                    d2_rs_core::quote_go_bytes(previous.as_bytes()),
                    d2_rs_core::quote_go_bytes(previous_path.to_string_lossy().as_bytes())
                ));
            }
            dirs.entry(dir_key).or_insert_with(|| board.clone());
            dir = current.parent();
        }
        files.insert(key, (board.clone(), path.clone()));
    }
    Ok(())
}

#[derive(Default)]
struct Manifest {
    files: Vec<(String, String)>,
    directories: Vec<String>,
}

impl Manifest {
    /// json.MarshalIndent(manifest, "", "  ") plus a trailing newline. Go's
    /// encoder escapes HTML-significant characters inside strings.
    fn encode(&self) -> Vec<u8> {
        fn string(value: &str) -> String {
            serde_json::to_string(value)
                .expect("string")
                .replace('<', "\\u003c")
                .replace('>', "\\u003e")
                .replace('&', "\\u0026")
                .replace('\u{2028}', "\\u2028")
                .replace('\u{2029}', "\\u2029")
        }
        let mut out = format!(
            "{{\n  \"format\": {},\n  \"version\": 1,\n  \"files\": ",
            string(BOARD_OUTPUT_MANIFEST_FORMAT)
        );
        if self.files.is_empty() {
            out.push_str("[]");
        } else {
            out.push_str("[\n");
            for (index, (path, sha256)) in self.files.iter().enumerate() {
                out.push_str(&format!(
                    "    {{\n      \"path\": {},\n      \"sha256\": {}\n    }}",
                    string(path),
                    string(sha256)
                ));
                out.push_str(if index + 1 == self.files.len() {
                    "\n"
                } else {
                    ",\n"
                });
            }
            out.push_str("  ]");
        }
        out.push_str(",\n  \"directories\": ");
        if self.directories.is_empty() {
            out.push_str("[]");
        } else {
            out.push_str("[\n");
            for (index, directory) in self.directories.iter().enumerate() {
                out.push_str("    ");
                out.push_str(&string(directory));
                out.push_str(if index + 1 == self.directories.len() {
                    "\n"
                } else {
                    ",\n"
                });
            }
            out.push_str("  ]");
        }
        out.push_str("\n}\n");
        out.into_bytes()
    }

    /// readBoardOutputManifestRoot. An invalid manifest is an error: D2 will
    /// not guess which files it previously owned.
    fn read(root: &Path) -> Result<Option<Self>, String> {
        let path = root.join(BOARD_OUTPUT_MANIFEST_NAME);
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("inspect board output manifest: {error}")),
        };
        if !metadata.file_type().is_file() {
            return Err(format!(
                "board output manifest {:?} is not a regular file",
                path.display().to_string()
            ));
        }
        if metadata.len() > MAX_BOARD_OUTPUT_MANIFEST_BYTES {
            return Err(format!(
                "board output manifest exceeds {MAX_BOARD_OUTPUT_MANIFEST_BYTES} bytes"
            ));
        }
        let data =
            std::fs::read(&path).map_err(|error| format!("read board output manifest: {error}"))?;
        let value: serde_json::Value = serde_json::from_slice(&data)
            .map_err(|error| format!("decode board output manifest: {error}"))?;
        let invalid = || "unsupported board output manifest format or version".to_owned();
        if value["format"] != BOARD_OUTPUT_MANIFEST_FORMAT || value["version"] != 1 {
            return Err(invalid());
        }
        // Go's nil slices decode from null (and omitted fields) as empty.
        let list = |name: &str| -> Result<&[serde_json::Value], String> {
            if value[name].is_null() {
                Ok(&[])
            } else {
                value[name]
                    .as_array()
                    .map(Vec::as_slice)
                    .ok_or_else(invalid)
            }
        };
        let files = list("files")?;
        let directories = list("directories")?;
        if files.len() + directories.len() > 100_000 {
            return Err("board output manifest exceeds 100000 paths".to_owned());
        }
        let mut manifest = Self::default();
        let mut seen = BTreeMap::new();
        for file in files {
            let (Some(path), Some(sha256)) = (file["path"].as_str(), file["sha256"].as_str())
            else {
                return Err(invalid());
            };
            validate_manifest_path(path, true, &mut seen)?;
            if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(format!(
                    "board output manifest contains invalid digest for {}",
                    d2_rs_core::quote_go_bytes(path.as_bytes())
                ));
            }
            // staleFileChanges compares against strings.ToLower(file.SHA256).
            manifest
                .files
                .push((path.to_owned(), sha256.to_ascii_lowercase()));
        }
        for directory in directories {
            let Some(directory) = directory.as_str() else {
                return Err(invalid());
            };
            validate_manifest_path(directory, false, &mut seen)?;
            manifest.directories.push(directory.to_owned());
        }
        // validateBoardOutputManifest checks the entire inventory before
        // walking ancestors, so a later file can conflict with an earlier path.
        for (path, _) in seen.values() {
            let mut parent = Path::new(path).parent();
            while let Some(ancestor) = parent.filter(|p| !p.as_os_str().is_empty()) {
                if seen
                    .get(&collision_key(ancestor))
                    .is_some_and(|(_, file)| *file)
                {
                    return Err(format!(
                        "board output manifest path {} is beneath a file path",
                        d2_rs_core::quote_go_bytes(path.as_bytes())
                    ));
                }
                parent = ancestor.parent();
            }
        }
        Ok(Some(manifest))
    }
}

fn valid_manifest_path(path: &str) -> bool {
    !path.is_empty()
        && path != "."
        && !path.contains('\\')
        && !path.starts_with('/')
        && path
            .split('/')
            .all(|element| !element.is_empty() && element != "." && element != "..")
}

/// d2cli/board_output_publish.go validateBoardOutputManifest.validatePath.
fn validate_manifest_path(
    path: &str,
    file: bool,
    seen: &mut BTreeMap<String, (String, bool)>,
) -> Result<(), String> {
    let quote = |value: &str| d2_rs_core::quote_go_bytes(value.as_bytes());
    if !valid_manifest_path(path) {
        return Err(format!(
            "board output manifest contains invalid path {}",
            quote(path)
        ));
    }
    let key = collision_key(Path::new(path));
    if let Some((previous, _)) = seen.get(&key) {
        return Err(format!(
            "board output manifest paths {} and {} collide",
            quote(previous),
            quote(path)
        ));
    }
    if key == collision_key(Path::new(BOARD_OUTPUT_MANIFEST_NAME)) {
        return Err("board output manifest cannot own itself".to_owned());
    }
    seen.insert(key, (path.to_owned(), file));
    Ok(())
}

/// boardOutputWorkspace: a private staging directory created beside the
/// final output root. Dropping it unpublished removes the staged tree.
pub(crate) struct BoardOutputWorkspace {
    final_root: PathBuf,
    stage_root: Option<PathBuf>,
    visible_mutation: bool,
    #[cfg(test)]
    before_publish: Option<Box<dyn FnMut(&Path) -> Result<(), String>>>,
}

impl BoardOutputWorkspace {
    pub(crate) fn new(final_root: &Path) -> Result<Self, String> {
        validate_root(final_root)?;
        let parent = final_root
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create board output parent: {error}"))?;
        // os.MkdirTemp(parent, ".d2-board-output-"): a fresh 0700 directory.
        let mut attempt = 0u32;
        let stage_root = loop {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| duration.subsec_nanos());
            let candidate = parent.join(format!(
                ".d2-board-output-{}{nanos:09}{attempt}",
                std::process::id()
            ));
            match create_private_dir(&candidate) {
                Ok(()) => break candidate,
                Err(error)
                    if error.kind() == std::io::ErrorKind::AlreadyExists && attempt < 10_000 =>
                {
                    attempt += 1;
                }
                Err(error) => {
                    return Err(format!("create board output staging directory: {error}"));
                }
            }
        };
        Ok(Self {
            final_root: final_root.to_path_buf(),
            stage_root: Some(stage_root),
            visible_mutation: false,
            #[cfg(test)]
            before_publish: None,
        })
    }

    /// The directory boards are written into; display paths stay final.
    pub(crate) fn stage_root(&self) -> &Path {
        self.stage_root.as_deref().expect("unpublished workspace")
    }

    pub(crate) fn publish(mut self) -> Result<bool, String> {
        let result = self.publish_inner();
        self.discard();
        result
    }

    fn before_mutation(&mut self, rel: &Path) -> Result<(), String> {
        #[cfg(test)]
        if let Some(callback) = &mut self.before_publish {
            let path = self.final_root.join(rel);
            callback(&path).map_err(|error| {
                format!(
                    "publish board output {}: {error}",
                    d2_rs_core::quote_go_bytes(path.to_string_lossy().as_bytes())
                )
            })?;
        }
        #[cfg(not(test))]
        let _ = rel;
        Ok(())
    }

    fn publish_inner(&mut self) -> Result<bool, String> {
        let stage = self.stage_root().to_path_buf();
        validate_root(&self.final_root)?;
        let mut directories = Vec::new();
        let mut files = Vec::new();
        collect_tree(
            &stage,
            Path::new(""),
            &self.final_root,
            &mut directories,
            &mut files,
        )?;
        let mut manifest = Manifest::default();
        for rel in &files {
            let digest = file_digest(&stage.join(rel)).map_err(|error| {
                format!(
                    "hash staged board output {:?}: {error}",
                    rel.display().to_string()
                )
            })?;
            manifest.files.push((slash(rel), digest));
        }
        manifest.directories = directories.iter().map(|rel| slash(rel)).collect();
        manifest.files.sort();
        manifest.directories.sort();
        let manifest_source = stage.join(BOARD_OUTPUT_MANIFEST_NAME);
        std::fs::write(&manifest_source, manifest.encode())
            .map_err(|error| format!("write board output manifest: {error}"))?;

        if std::fs::symlink_metadata(&self.final_root).is_err() {
            set_mode(&stage, 0o755).map_err(|error| {
                format!("set published board output directory permissions: {error}")
            })?;
            std::fs::rename(&stage, &self.final_root)
                .map_err(|error| format!("publish board output tree: {error}"))?;
            self.stage_root = None;
            return Ok(true);
        }

        let root = root::Root::open(&self.final_root)?;
        let previous = Manifest::read(&self.final_root)?.unwrap_or_default();
        transaction::Transaction::new(self, root).publish(
            &directories,
            &files,
            &previous,
            &manifest,
        )
    }

    fn discard(&mut self) {
        if let Some(stage) = self.stage_root.take() {
            let _ = std::fs::remove_dir_all(stage);
        }
    }
}

impl Drop for BoardOutputWorkspace {
    fn drop(&mut self) {
        self.discard();
    }
}

fn validate_root(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("inspect board output root: {error}")),
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(format!(
            "board output root {:?} must be a real directory",
            path.display().to_string()
        )),
    }
}

/// filepath.WalkDir order: lexical within each directory.
fn collect_tree(
    root: &Path,
    rel: &Path,
    final_root: &Path,
    directories: &mut Vec<PathBuf>,
    files: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let mut entries = std::fs::read_dir(root.join(rel))
        .and_then(|entries| entries.collect::<Result<Vec<_>, _>>())
        .map_err(|error| format!("preflight board output tree: {error}"))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let child = rel.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|error| format!("preflight board output tree: {error}"))?;
        // d2cli/board_output.go preflightMerge: inspect the complete tree in
        // WalkDir order before creating any destination directory or file.
        // A directory is visited before its children, so an existing symlink
        // ancestor is rejected before inspecting any path beneath it.
        let destination = final_root.join(&child);
        if file_type.is_dir() || file_type.is_file() {
            match std::fs::symlink_metadata(&destination) {
                Ok(info) if info.file_type().is_symlink() => {
                    return Err(format!(
                        "preflight board output tree: refusing to publish board output through symlink {}",
                        d2_rs_core::quote_go_bytes(destination.to_string_lossy().as_bytes())
                    ));
                }
                Ok(info)
                    if file_type.is_dir() != info.is_dir()
                        || (!file_type.is_dir() && !info.is_file()) =>
                {
                    return Err(format!(
                        "preflight board output tree: board output path {} has an incompatible existing type",
                        d2_rs_core::quote_go_bytes(destination.to_string_lossy().as_bytes())
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("preflight board output tree: {error}")),
            }
        }
        if file_type.is_dir() {
            directories.push(child.clone());
            collect_tree(root, &child, final_root, directories, files)?;
        } else if file_type.is_file() {
            files.push(child);
        } else {
            return Err(format!(
                "preflight board output tree: staged board output {:?} is not a regular file or directory",
                root.join(&child).display().to_string()
            ));
        }
    }
    Ok(())
}

fn quote(path: &Path) -> String {
    d2_rs_core::quote_go_bytes(path.to_string_lossy().as_bytes())
}

fn mode(metadata: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o777
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        0o644
    }
}

fn set_file_mode(file: &std::fs::File, mode: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(mode))
    }
    #[cfg(not(unix))]
    {
        let _ = (file, mode);
        Ok(())
    }
}

fn slash(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(unix)]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir(path)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn preflight_rejects_late_conflicts_before_publishing_any_entry() {
        let scratch =
            std::env::temp_dir().join(format!("d2rs-board-preflight-{}", std::process::id()));
        std::fs::create_dir(&scratch).unwrap();
        let root = scratch.join("out");
        std::fs::create_dir(&root).unwrap();
        let victim = scratch.join("victim");
        std::fs::write(&victim, b"keep me").unwrap();
        let conflict = root.join("z-child.svg");
        for symlink in [true, false] {
            if symlink {
                std::os::unix::fs::symlink(&victim, &conflict).unwrap();
            } else {
                std::fs::create_dir(&conflict).unwrap();
            }
            let workspace = BoardOutputWorkspace::new(&root).unwrap();
            let stage = workspace.stage_root().to_owned();
            std::fs::create_dir(stage.join("a-created")).unwrap();
            std::fs::write(stage.join("index.svg"), b"root").unwrap();
            std::fs::write(stage.join("z-child.svg"), b"child").unwrap();
            let error = workspace.publish().unwrap_err();
            assert!(
                error.starts_with("preflight board output tree: "),
                "{error}"
            );
            assert!(
                error.contains(if symlink {
                    "symlink"
                } else {
                    "incompatible existing type"
                }),
                "{error}"
            );
            assert!(!root.join("index.svg").exists());
            assert!(!root.join("a-created").exists());
            assert!(!root.join(BOARD_OUTPUT_MANIFEST_NAME).exists());
            assert!(!stage.exists());
            assert_eq!(std::fs::read(&victim).unwrap(), b"keep me");
            if symlink {
                assert_eq!(std::fs::read_link(&conflict).unwrap(), victim);
                std::fs::remove_file(&conflict).unwrap();
            } else {
                std::fs::remove_dir(&conflict).unwrap();
            }
        }
        std::fs::remove_dir_all(scratch).unwrap();
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_go_unicode_collision_keys() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/board-output-unicode.json"
        ))
        .unwrap();
        assert_eq!(unicode_normalization::UNICODE_VERSION, (17, 0, 0));
        assert_eq!(rows.len(), 23);
        for row in rows {
            let input = row["input"].as_str().unwrap();
            assert_eq!(
                fold_normalized_path(row["clean"].as_str().unwrap()),
                row["output"].as_str().unwrap(),
                "{input:?}"
            );
            // These complete filepath.Clean records were captured from Go
            // on Unix. Windows uses its own path separators and root rules.
            #[cfg(unix)]
            assert_eq!(
                collision_key(Path::new(input)),
                row["output"].as_str().unwrap(),
                "{input:?}"
            );
        }
    }

    #[test]
    fn rejects_unicode_equivalent_board_files_and_directories() {
        for (first, second) in [("é", "e\u{301}"), ("Straße", "STRASSE"), ("Σ", "ς")] {
            let outputs = [
                (first.into(), PathBuf::from(format!("out/{first}.svg"))),
                (second.into(), PathBuf::from(format!("out/{second}.svg"))),
            ];
            let error = validate_board_output_paths(&outputs).unwrap_err();
            assert!(error.contains("same output path"), "{error}");
            // Keep Go's literal combining characters in %q diagnostics.
            assert!(
                error.contains(&d2_rs_core::quote_go_bytes(second.as_bytes())),
                "{error}"
            );
            let outputs = [
                (first.into(), PathBuf::from(format!("out/{first}.svg"))),
                (
                    second.into(),
                    PathBuf::from(format!("out/{second}.svg/child.svg")),
                ),
            ];
            assert!(
                validate_board_output_paths(&outputs)
                    .unwrap_err()
                    .contains("requires a directory occupied")
            );
        }
    }

    #[test]
    fn equivalent_spelling_retires_distinct_old_files() {
        let scratch = std::env::temp_dir().join(format!("d2rs-board-fold-{}", std::process::id()));
        let root = scratch.join("out");
        let publish = |parent: &str| {
            let workspace = BoardOutputWorkspace::new(&root).unwrap();
            let file = workspace.stage_root().join(parent).join("child.svg");
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"same bytes").unwrap();
            workspace.publish().unwrap();
        };
        publish("Straße");
        let alias = root.join("Straße");
        publish("STRASSE");
        assert_eq!(
            std::fs::read(root.join("STRASSE/child.svg")).unwrap(),
            b"same bytes"
        );
        if alias.exists() {
            assert!(same_output_path(&root, "Straße", "STRASSE"));
        }
        std::fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn ordinary_board_names_keep_their_spelling() {
        assert_eq!(board_output_component("review"), "review");
        assert_eq!(board_output_component("a b"), "a b");
    }

    #[test]
    fn reserved_and_unsafe_names_are_hashed() {
        for name in ["index", "Layers", "a/b", "..", "CON.txt", "x.", "_d2_x", ""] {
            let component = board_output_component(name);
            assert!(component.starts_with("_d2_"), "{name}: {component}");
            assert_eq!(component.len(), 4 + 64);
        }
    }

    #[test]
    fn manifest_matches_go_indented_json() {
        let manifest = Manifest {
            files: vec![("d.png".into(), "ab".into())],
            directories: vec![],
        };
        assert_eq!(
            String::from_utf8(manifest.encode()).unwrap(),
            "{\n  \"format\": \"d2-board-output\",\n  \"version\": 1,\n  \"files\": [\n    {\n      \"path\": \"d.png\",\n      \"sha256\": \"ab\"\n    }\n  ],\n  \"directories\": []\n}\n"
        );
    }

    #[test]
    fn republishing_removes_only_unchanged_stale_outputs() {
        let scratch =
            std::env::temp_dir().join(format!("d2rs-board-output-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let root = scratch.join("out");
        let publish = |names: &[&str]| {
            let workspace = BoardOutputWorkspace::new(&root).unwrap();
            for name in names {
                let path = workspace.stage_root().join(name);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, name.as_bytes()).unwrap();
            }
            workspace.publish().unwrap();
        };
        publish(&["index.svg", "old.svg", "edited.svg", "sub/x.svg"]);
        std::fs::write(root.join("edited.svg"), b"user").unwrap();
        std::fs::write(root.join("notes.txt"), b"mine").unwrap();
        publish(&["index.svg"]);
        assert!(root.join("index.svg").is_file());
        assert!(!root.join("old.svg").exists());
        assert!(!root.join("sub").exists());
        assert_eq!(std::fs::read(root.join("edited.svg")).unwrap(), b"user");
        assert!(root.join("notes.txt").is_file());
        let leftovers = std::fs::read_dir(&scratch)
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".d2-board-output-")
            })
            .count();
        assert_eq!(leftovers, 0);
        std::fs::remove_dir_all(&scratch).unwrap();
    }
}
