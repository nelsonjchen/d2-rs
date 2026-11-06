// SPDX-License-Identifier: MPL-2.0
//! Dev-only access to regenerated oracle fixtures.
//!
//! Go-oracle output is not committed. `scripts/regen-fixtures.py` writes each
//! artifact to `<workspace>/oracle-cache/fixtures/<logical path>` together with
//! a `<logical path>.stamp.json` sidecar. The logical path is the artifact's
//! old tracked location, e.g. `crates/d2-rs-text/tests/fixtures/link.json`.
//! See `docs/FIXTURES.md` for the recipe contract.
//!
//! Tests replace `include_str!("fixtures/link.json")` with
//! `d2_rs_testdata::oracle_fixture!("tests/fixtures/link.json")`: the path is
//! relative to the calling crate's manifest directory, which the macro takes
//! from `env!("CARGO_MANIFEST_DIR")` expanded at the call site. Set
//! `D2RS_FIXTURES_DIR` to read another cache root (relative values resolve
//! against the workspace root).
//!
//! A missing artifact, a missing or malformed stamp, or an artifact whose
//! size/SHA-256 differs from its stamp panics with the exact regeneration
//! command. Nothing is ever skipped silently.

use sha2::{Digest, Sha256};
use std::fmt;
use std::path::{Component, Path, PathBuf};

/// Environment variable that overrides the fixture cache root.
pub const FIXTURES_DIR_ENV: &str = "D2RS_FIXTURES_DIR";

/// This crate lives at `<workspace>/crates/d2-rs-testdata`.
const WORKSPACE_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

/// `&'static str` contents of a regenerated fixture, loaded once per call site.
#[macro_export]
macro_rules! oracle_fixture {
    ($path:expr $(,)?) => {{
        static CELL: ::std::sync::OnceLock<&'static str> = ::std::sync::OnceLock::new();
        *CELL.get_or_init(|| $crate::load_str(env!("CARGO_MANIFEST_DIR"), $path))
    }};
}

/// `&'static [u8]` contents of a regenerated fixture, loaded once per call site.
#[macro_export]
macro_rules! oracle_fixture_bytes {
    ($path:expr $(,)?) => {{
        static CELL: ::std::sync::OnceLock<&'static [u8]> = ::std::sync::OnceLock::new();
        *CELL.get_or_init(|| $crate::load_bytes(env!("CARGO_MANIFEST_DIR"), $path))
    }};
}

/// Verified filesystem path of a regenerated fixture, for readers that open
/// files themselves.
#[macro_export]
macro_rules! oracle_fixture_path {
    ($path:expr $(,)?) => {
        $crate::fixture_path(env!("CARGO_MANIFEST_DIR"), $path)
    };
}

/// A fixture located in the cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Located {
    /// Workspace-relative logical path (the recipe `outputs` entry).
    pub logical: String,
    /// Artifact path inside the cache.
    pub artifact: PathBuf,
    /// Sidecar stamp path.
    pub stamp: PathBuf,
}

/// Why a fixture cannot be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// The requested path is absolute, escapes its crate, or the crate is
    /// outside the workspace.
    BadPath(String),
    MissingArtifact,
    MissingStamp,
    BadStamp(String),
    Mismatch(String),
}

/// Formatted failure for one fixture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixtureError {
    pub located: Option<Located>,
    pub problem: Problem,
}

impl fmt::Display for FixtureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(located) = &self.located else {
            return write!(f, "invalid oracle fixture path: {:?}", self.problem);
        };
        let (what, force) = match &self.problem {
            Problem::BadPath(why) => (format!("has an invalid path ({why})"), false),
            Problem::MissingArtifact => ("is missing from the regenerated cache".into(), false),
            Problem::MissingStamp => (
                format!(
                    "has no stamp {} (incomplete or hand-placed)",
                    located.stamp.display()
                ),
                true,
            ),
            Problem::BadStamp(why) => (format!("has an unreadable stamp ({why})"), true),
            Problem::Mismatch(why) => (format!("does not match its stamp ({why})"), true),
        };
        write!(
            f,
            "oracle fixture `{}` {what}\n  expected at: {}\n  regenerate it with:\n    \
             uv run --no-project python scripts/regen-fixtures.py --only {}{}\n  \
             (see docs/FIXTURES.md; {FIXTURES_DIR_ENV} overrides the cache root)",
            located.logical,
            located.artifact.display(),
            located.logical,
            if force { " --force" } else { "" },
        )
    }
}

impl std::error::Error for FixtureError {}

/// Canonical workspace root of this crate's own checkout.
pub fn workspace_root() -> PathBuf {
    Path::new(WORKSPACE_ROOT)
        .canonicalize()
        .unwrap_or_else(|e| panic!("cannot resolve workspace root {WORKSPACE_ROOT}: {e}"))
}

/// Workspace root of the checkout that contains `manifest_dir`: the nearest
/// ancestor whose `Cargo.toml` declares `[workspace]`. Checkouts may share one
/// Cargo target (AGENTS.md), and cargo then reuses a single build of this
/// crate for all of them, so the compile-time [`workspace_root`] can name
/// another checkout; the caller's manifest directory cannot.
pub fn workspace_root_of(manifest_dir: &Path) -> PathBuf {
    manifest_dir
        .ancestors()
        .find(|dir| {
            std::fs::read_to_string(dir.join("Cargo.toml"))
                .is_ok_and(|toml| toml.lines().any(|line| line.trim() == "[workspace]"))
        })
        .and_then(|dir| dir.canonicalize().ok())
        .unwrap_or_else(workspace_root)
}

/// Fixture cache root: `$D2RS_FIXTURES_DIR` or `<workspace>/oracle-cache/fixtures`.
pub fn fixtures_root() -> PathBuf {
    fixtures_root_in(&workspace_root())
}

fn fixtures_root_in(root: &Path) -> PathBuf {
    match std::env::var_os(FIXTURES_DIR_ENV) {
        Some(dir) if !dir.is_empty() => root.join(dir),
        _ => root.join("oracle-cache").join("fixtures"),
    }
}

/// Locate `path` (relative to `manifest_dir`) in `fixtures_root`.
pub fn locate_in(
    fixtures_root: &Path,
    workspace_root: &Path,
    manifest_dir: &Path,
    path: &str,
) -> Result<Located, FixtureError> {
    let bad = |why: String| FixtureError {
        located: None,
        problem: Problem::BadPath(why),
    };
    let relative = Path::new(path);
    if path.is_empty()
        || !relative
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(bad(format!(
            "`{path}` must be a relative path without `..`"
        )));
    }
    let manifest = manifest_dir
        .canonicalize()
        .unwrap_or_else(|_| manifest_dir.to_path_buf());
    let workspace = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let crate_dir = manifest.strip_prefix(&workspace).map_err(|_| {
        bad(format!(
            "crate {} is outside workspace {}",
            manifest.display(),
            workspace.display()
        ))
    })?;
    let parts: Vec<String> = crate_dir
        .components()
        .chain(relative.components())
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let logical = parts.join("/");
    let artifact = parts
        .iter()
        .fold(fixtures_root.to_path_buf(), |p, s| p.join(s));
    let mut stamp = artifact.clone().into_os_string();
    stamp.push(".stamp.json");
    Ok(Located {
        logical,
        artifact,
        stamp: PathBuf::from(stamp),
    })
}

/// Read and verify a located fixture against its stamp.
pub fn read_verified(located: &Located) -> Result<Vec<u8>, FixtureError> {
    let fail = |problem| FixtureError {
        located: Some(located.clone()),
        problem,
    };
    let bytes = std::fs::read(&located.artifact).map_err(|_| fail(Problem::MissingArtifact))?;
    let stamp = std::fs::read(&located.stamp).map_err(|_| fail(Problem::MissingStamp))?;
    let stamp: serde_json::Value =
        serde_json::from_slice(&stamp).map_err(|e| fail(Problem::BadStamp(e.to_string())))?;
    fn lookup<'a>(stamp: &'a serde_json::Value, name: &str) -> Option<&'a serde_json::Value> {
        stamp.get(name)
    }
    let field = |name: &'static str| {
        lookup(&stamp, name).ok_or_else(|| fail(Problem::BadStamp(format!("no `{name}` field"))))
    };
    let logical = field("logical_path")?.as_str().unwrap_or_default();
    if logical != located.logical {
        return Err(fail(Problem::Mismatch(format!(
            "stamp is for `{logical}`, not `{}`",
            located.logical
        ))));
    }
    let size = field("size")?.as_u64();
    if size != Some(bytes.len() as u64) {
        return Err(fail(Problem::Mismatch(format!(
            "size {} != stamped {size:?}",
            bytes.len()
        ))));
    }
    let expected = field("sha256")?
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let actual: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != expected {
        return Err(fail(Problem::Mismatch(format!(
            "sha256 {actual} != stamped {expected}"
        ))));
    }
    Ok(bytes)
}

fn located_or_panic(manifest_dir: &str, path: &str) -> Located {
    let root = workspace_root_of(Path::new(manifest_dir));
    locate_in(
        &fixtures_root_in(&root),
        &root,
        Path::new(manifest_dir),
        path,
    )
    .unwrap_or_else(|e| panic!("{e}"))
}

/// Verified bytes of a fixture; leaked so callers may hold `'static` slices.
pub fn load_bytes(manifest_dir: &str, path: &str) -> &'static [u8] {
    let located = located_or_panic(manifest_dir, path);
    let bytes = read_verified(&located).unwrap_or_else(|e| panic!("{e}"));
    Box::leak(bytes.into_boxed_slice())
}

/// Verified UTF-8 contents of a fixture.
pub fn load_str(manifest_dir: &str, path: &str) -> &'static str {
    let located = located_or_panic(manifest_dir, path);
    let bytes = read_verified(&located).unwrap_or_else(|e| panic!("{e}"));
    let text = String::from_utf8(bytes)
        .unwrap_or_else(|e| panic!("oracle fixture `{}` is not UTF-8: {e}", located.logical));
    Box::leak(text.into_boxed_str())
}

/// Verified path of a fixture (the artifact is checked against its stamp).
pub fn fixture_path(manifest_dir: &str, path: &str) -> PathBuf {
    let located = located_or_panic(manifest_dir, path);
    read_verified(&located).unwrap_or_else(|e| panic!("{e}"));
    located.artifact
}
