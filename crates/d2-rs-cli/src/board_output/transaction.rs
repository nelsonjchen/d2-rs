// SPDX-License-Identifier: MPL-2.0
//! d2cli/board_output_publish.go: prepare all backups, apply, then roll back
//! the applied journal in reverse on any failure. The manifest is installed last.
use super::*;
use d2_rs_core::stable_sort::StableSort;
use root::Info;
use root::Root;

struct Change {
    rel: PathBuf,
    source: Option<PathBuf>,
    expected_digest: Option<String>,
    original: Option<Info>,
    backup: Option<PathBuf>,
    skip: bool,
    installed: Option<Info>,
}
impl Change {
    fn new(rel: PathBuf, source: Option<PathBuf>, expected_digest: Option<String>) -> Self {
        Self {
            rel,
            source,
            expected_digest,
            original: None,
            backup: None,
            skip: false,
            installed: None,
        }
    }
}

pub(super) struct Transaction<'a> {
    workspace: &'a mut BoardOutputWorkspace,
    root: Root,
    changes: Vec<Change>,
    applied: Vec<usize>,
    created_dirs: Vec<PathBuf>,
    removed_dirs: Vec<(PathBuf, u32)>,
}

fn digest(bytes: &[u8]) -> String {
    hex(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref())
}
pub(super) fn same_file(first: &Info, second: &Info) -> bool {
    first.same_file(second)
}

impl<'a> Transaction<'a> {
    pub(super) fn new(workspace: &'a mut BoardOutputWorkspace, root: Root) -> Self {
        Self {
            workspace,
            root,
            changes: Vec::new(),
            applied: Vec::new(),
            created_dirs: Vec::new(),
            removed_dirs: Vec::new(),
        }
    }
    pub(super) fn publish(
        mut self,
        directories: &[PathBuf],
        files: &[PathBuf],
        previous: &Manifest,
        current: &Manifest,
    ) -> Result<bool, String> {
        if let Err(error) = self.apply(directories, files, previous, current) {
            return match self.rollback() {
                Ok(()) => {
                    self.workspace.visible_mutation = false;
                    Err(error)
                }
                Err(rollback_error) => Err(format!("{error}\n{rollback_error}")),
            };
        }
        Ok(self.workspace.visible_mutation)
    }
    fn equivalent(&self, previous: &str, current: &str) -> bool {
        previous == current
            || matches!((self.root.metadata(Path::new(previous)), self.root.metadata(Path::new(current))), (Ok(first), Ok(second)) if same_file(&first, &second))
    }
    fn apply(
        &mut self,
        directories: &[PathBuf],
        files: &[PathBuf],
        previous: &Manifest,
        current: &Manifest,
    ) -> Result<(), String> {
        let stage = self.workspace.stage_root().to_path_buf();
        self.changes.extend(
            files
                .iter()
                .map(|rel| Change::new(rel.clone(), Some(stage.join(rel)), None)),
        );
        let current_files = current
            .files
            .iter()
            .map(|(path, _)| (collision_key(Path::new(path)), path.as_str()))
            .collect::<BTreeMap<_, _>>();
        for (path, expected) in &previous.files {
            if current_files
                .get(&collision_key(Path::new(path)))
                .is_some_and(|new| self.equivalent(path, new))
            {
                continue;
            }
            let rel = Path::new(path);
            match self.root.kind(rel) {
                Ok(0x8000) => {}
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.to_string()),
            }
            let info = self.root.metadata(rel).map_err(|error| error.to_string())?;
            let bytes = self
                .root
                .read(rel, &info)
                .map_err(|error| format!("hash prior board output {}: {error}", quote(rel)))?;
            if digest(&bytes) == expected.to_lowercase() {
                self.changes.push(Change::new(
                    rel.to_owned(),
                    None,
                    Some(expected.to_lowercase()),
                ));
            }
        }
        self.changes.push(Change::new(
            PathBuf::from(BOARD_OUTPUT_MANIFEST_NAME),
            Some(stage.join(BOARD_OUTPUT_MANIFEST_NAME)),
            None,
        ));
        self.prepare_backups()?;
        for rel in directories {
            match self.root.kind(rel) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    self.workspace.before_mutation(rel)?;
                    let permissions = mode(
                        &std::fs::metadata(stage.join(rel)).map_err(|error| error.to_string())?,
                    );
                    self.root.mkdir(rel, permissions).map_err(|error| {
                        format!(
                            "create board output directory {}: mkdirat {}: {}",
                            self.root.display(rel),
                            rel.display(),
                            go_io_error(&error)
                        )
                    })?;
                    self.created_dirs.push(rel.clone());
                    self.workspace.visible_mutation = true;
                }
                Err(error) => return Err(error.to_string()),
                Ok(0x4000) => {}
                Ok(_) => {
                    return Err(format!(
                        "board output directory {} is not a real directory",
                        self.root.display(rel)
                    ));
                }
            }
        }
        let manifest_index = self.changes.len() - 1;
        for index in 0..manifest_index {
            self.apply_change(index)?;
        }
        self.remove_stale_directories(previous, current)?;
        self.apply_change(manifest_index)
    }
    fn prepare_backups(&mut self) -> Result<(), String> {
        let mut random = [0u8; 16];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut random)
            .map_err(|_| "generate board output rollback directory name".to_owned())?;
        let backup_root = self
            .workspace
            .stage_root()
            .join(format!(".d2-board-rollback-{}", hex(&random)));
        let mut backup_created = false;
        for (index, change) in self.changes.iter_mut().enumerate() {
            match self.root.kind(&change.rel) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.to_string()),
                Ok(0x8000) => {}
                Ok(_) => {
                    return Err(format!(
                        "board output path {} is not a regular file",
                        self.root.display(&change.rel)
                    ));
                }
            }
            let info = self
                .root
                .metadata(&change.rel)
                .map_err(|error| error.to_string())?;
            if !backup_created {
                create_private_dir(&backup_root)
                    .map_err(|error| format!("create board output rollback directory: {error}"))?;
                backup_created = true;
            }
            let backup = backup_root.join(format!("{index:06}"));
            let bytes = self.root.read(&change.rel, &info).map_err(|error| {
                format!(
                    "back up board output {}: {error}",
                    self.root.display(&change.rel)
                )
            })?;
            // Exclusive backup with original permissions, never a hard link.
            let mut backup_file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&backup)
                .map_err(|error| error.to_string())?;
            use std::io::Write;
            backup_file
                .write_all(&bytes)
                .map_err(|error| error.to_string())?;
            set_file_mode(&backup_file, mode(&info)).map_err(|error| error.to_string())?;
            if change
                .expected_digest
                .as_ref()
                .is_some_and(|expected| digest(&bytes) != *expected)
            {
                change.skip = true;
            }
            change.backup = Some(backup);
            change.original = Some(info);
        }
        Ok(())
    }
    fn apply_change(&mut self, index: usize) -> Result<(), String> {
        let change = &mut self.changes[index];
        if change.skip || (change.source.is_none() && change.original.is_none()) {
            return Ok(());
        }
        self.workspace.before_mutation(&change.rel)?;
        let current = self.root.metadata(&change.rel);
        match (&change.original, &current) {
            (None, Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
            (None, Ok(_)) => {
                return Err(format!(
                    "board output path {} appeared during publication",
                    self.root.display(&change.rel)
                ));
            }
            (None, Err(error)) => return Err(error.to_string()),
            (Some(original), Ok(info)) if info.is_file() && same_file(original, info) => {}
            (Some(_), Err(error)) => {
                return Err(format!(
                    "inspect board output {} before publication: {error}",
                    self.root.display(&change.rel)
                ));
            }
            _ => {
                return Err(format!(
                    "board output path {} changed during publication",
                    self.root.display(&change.rel)
                ));
            }
        }
        if let Some(source) = &change.source {
            let (applied, installed, result) =
                self.root
                    .install(source, &change.rel, change.original.is_some());
            change.installed = installed;
            if applied {
                self.applied.push(index);
                self.workspace.visible_mutation = true;
            }
            return result;
        } else {
            // A stale file can be edited after the backup without changing its
            // identity. Re-read it immediately before retiring it.
            let bytes = self
                .root
                .read(&change.rel, change.original.as_ref().unwrap())
                .map_err(|error| {
                    format!(
                        "verify stale board output {}: {error}",
                        self.root.display(&change.rel)
                    )
                })?;
            if change.expected_digest.as_deref() != Some(digest(&bytes).as_str()) {
                change.skip = true;
                return Ok(());
            }
            self.root.remove(&change.rel, false).map_err(|error| {
                format!(
                    "remove stale board output {}: {error}",
                    self.root.display(&change.rel)
                )
            })?;
        }
        self.applied.push(index);
        self.workspace.visible_mutation = true;
        Ok(())
    }
    fn remove_stale_directories(
        &mut self,
        previous: &Manifest,
        current: &Manifest,
    ) -> Result<(), String> {
        let current_dirs = current
            .directories
            .iter()
            .map(|path| (collision_key(Path::new(path)), path.as_str()))
            .collect::<BTreeMap<_, _>>();
        let mut stale = previous
            .directories
            .iter()
            .filter(|path| {
                !current_dirs
                    .get(&collision_key(Path::new(path)))
                    .is_some_and(|new| self.equivalent(path, new))
            })
            .collect::<Vec<_>>();
        stale.stable_sort_by_key(|path| std::cmp::Reverse(path.matches('/').count()));
        for path in stale {
            let rel = Path::new(path);
            match self.root.kind(rel) {
                Ok(0x4000) => {}
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.to_string()),
            }
            let original = self.root.metadata(rel).map_err(|error| error.to_string())?;
            if !self
                .root
                .entries(rel)
                .map_err(|error| error.to_string())?
                .is_empty()
            {
                continue;
            }
            self.workspace.before_mutation(rel)?;
            if !self
                .root
                .metadata(rel)
                .is_ok_and(|now| same_file(&original, &now))
            {
                return Err(format!(
                    "board output directory {} changed during publication",
                    self.root.display(rel)
                ));
            }
            if let Err(error) = self.root.remove(rel, true) {
                if self
                    .root
                    .entries(rel)
                    .is_ok_and(|children| !children.is_empty())
                {
                    continue;
                }
                return Err(format!(
                    "remove stale board output directory {}: {error}",
                    self.root.display(rel)
                ));
            }
            self.removed_dirs.push((rel.to_owned(), mode(&original)));
            self.workspace.visible_mutation = true;
        }
        Ok(())
    }
    fn rollback(&mut self) -> Result<(), String> {
        let mut errors = Vec::new();
        for (rel, mode) in self.removed_dirs.iter().rev() {
            if let Err(error) = self.root.mkdir(rel, *mode) {
                if error.kind() != std::io::ErrorKind::AlreadyExists {
                    errors.push(format!(
                        "restore board output directory {}: {error}",
                        self.root.display(rel)
                    ));
                }
            }
        }
        for &index in self.applied.iter().rev() {
            let change = &self.changes[index];
            let current = self.root.metadata(&change.rel);
            if let Some(installed) = &change.installed {
                if !current
                    .as_ref()
                    .is_ok_and(|info| same_file(installed, info))
                {
                    errors.push(format!(
                        "refusing to roll back changed board output {}",
                        self.root.display(&change.rel)
                    ));
                    continue;
                }
            } else {
                match current {
                    Ok(_) => {
                        errors.push(format!(
                            "refusing to roll back recreated board output {}",
                            self.root.display(&change.rel)
                        ));
                        continue;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        errors.push(error.to_string());
                        continue;
                    }
                }
            }
            if let Some(backup) = &change.backup {
                if let Err(error) = self
                    .root
                    .install(backup, &change.rel, change.installed.is_some())
                    .2
                {
                    errors.push(format!(
                        "restore board output {}: {error}",
                        self.root.display(&change.rel)
                    ));
                }
            } else if change.installed.is_some() {
                if let Err(error) = self.root.remove(&change.rel, false) {
                    errors.push(format!(
                        "remove newly published board output {}: {error}",
                        self.root.display(&change.rel)
                    ));
                }
            }
        }
        for rel in self.created_dirs.iter().rev() {
            if let Err(error) = self.root.remove(rel, true) {
                if error.kind() == std::io::ErrorKind::NotFound {
                    continue;
                }
                if self
                    .root
                    .entries(rel)
                    .is_ok_and(|children| !children.is_empty())
                {
                    errors.push(format!(
                        "refusing to remove non-empty transaction directory {}",
                        self.root.display(rel)
                    ));
                } else {
                    errors.push(format!(
                        "remove transaction directory {}: {error}",
                        self.root.display(rel)
                    ));
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("\n"))
        }
    }
}

// Go's PathError prints strerror without Rust's "(os error N)" suffix.
fn go_io_error(error: &std::io::Error) -> String {
    let text = error.to_string();
    #[cfg(unix)]
    if let Some(code) = error.raw_os_error() {
        let suffix = format!(" (os error {code})");
        return text.strip_suffix(&suffix).unwrap_or(&text).to_lowercase();
    }
    text
}
