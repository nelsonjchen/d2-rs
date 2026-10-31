// SPDX-License-Identifier: MPL-2.0
//! Source d2renderers/internal/fontface/bundled_faces.go: the fixed,
//! authenticated registry of D2's ordinary bundled fonts.

use crate::error::Error;
use crate::face::{
    BundledFaceSource, Digest32, MAX_PARSED_FONT_FACES, parse_face_collection_with_digest, sha256,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

/// `maxBundledFaceSources`.
const MAX_BUNDLED_FACE_SOURCES: usize = 16;

struct Loaded {
    data: Arc<[u8]>,
    sources: Vec<Arc<BundledFaceSource>>,
}

struct Entry {
    size: usize,
    digest: Digest32,
    state: RwLock<Option<Result<Loaded, Error>>>,
}

impl Entry {
    fn loaded_data(&self) -> Option<Arc<[u8]>> {
        match &*self.state.read().unwrap() {
            Some(Ok(loaded)) => Some(loaded.data.clone()),
            _ => None,
        }
    }

    fn load(&self, candidate: &[u8]) -> Result<(), Error> {
        if let Some(state) = &*self.state.read().unwrap() {
            return state.as_ref().map(|_| ()).map_err(Clone::clone);
        }
        // Authenticate a private snapshot first; a bad candidate cannot
        // poison the fixed registry.
        if candidate.len() != self.size || sha256(candidate) != self.digest {
            return Err(Error::msg("bundled font resource is not authenticated"));
        }
        let owned: Arc<[u8]> = Arc::from(candidate);
        let mut state = self.state.write().unwrap();
        if let Some(state) = &*state {
            return state.as_ref().map(|_| ()).map_err(Clone::clone);
        }
        let result = (|| {
            let collection = parse_face_collection_with_digest(
                owned.clone(),
                MAX_PARSED_FONT_FACES,
                self.digest,
            )?;
            let mut sources = Vec::with_capacity(collection.num_faces());
            for index in 0..collection.num_faces() {
                let mut face = collection.face(index)?;
                face.set_bundled_outline();
                sources.push(Arc::new(BundledFaceSource { face }));
            }
            Ok(Loaded {
                data: owned,
                sources,
            })
        })();
        let outcome = result.as_ref().map(|_| ()).map_err(Clone::clone);
        *state = Some(result);
        outcome
    }
}

#[derive(Default)]
struct Registry {
    entries: Vec<Arc<Entry>>,
    by_digest: HashMap<Digest32, Arc<Entry>>,
    by_size: HashMap<usize, Vec<Arc<Entry>>>,
}

static REGISTRY: Mutex<Option<Registry>> = Mutex::new(None);

fn with_registry<T>(f: impl FnOnce(&mut Registry) -> T) -> T {
    let mut guard = REGISTRY.lock().unwrap();
    f(guard.get_or_insert_with(Registry::default))
}

/// `RegisterBundledFace`: record the authenticated size and digest of a D2
/// font. Parsing and a private backing copy happen lazily on first use.
pub fn register_bundled_face(data: &[u8], digest: Digest32) -> Result<(), Error> {
    if data.is_empty() || digest == [0; 32] {
        return Err(Error::msg("invalid bundled font registration"));
    }
    with_registry(|registry| {
        if let Some(existing) = registry.by_digest.get(&digest) {
            if data.len() == existing.size {
                return Ok(());
            }
            return Err(Error::msg(format!(
                "duplicate bundled font digest has size {}, want {}",
                data.len(),
                existing.size
            )));
        }
        if registry.entries.len() >= MAX_BUNDLED_FACE_SOURCES {
            return Err(Error::msg(format!(
                "bundled font registry exceeds limit {MAX_BUNDLED_FACE_SOURCES}"
            )));
        }
        let entry = Arc::new(Entry {
            size: data.len(),
            digest,
            state: RwLock::new(None),
        });
        registry.entries.push(entry.clone());
        registry.by_digest.insert(digest, entry.clone());
        registry.by_size.entry(data.len()).or_default().push(entry);
        Ok(())
    })
}

fn same_slice(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len() && (left.is_empty() || std::ptr::eq(left.as_ptr(), right.as_ptr()))
}

/// `RegisteredBundledFace`: Ok(None) when `data` is not a registered D2
/// font (arbitrary scene fonts continue down the ordinary parser path).
pub fn registered_bundled_face(
    data: &[u8],
    face_index: u16,
) -> Result<Option<Arc<BundledFaceSource>>, Error> {
    let entry = registered_bundled_face_entry(data, None);
    registered_bundled_face_source(entry, data, face_index)
}

/// `RegisteredBundledFaceDigest`: for callers that already hashed `data`.
pub fn registered_bundled_face_digest(
    data: &[u8],
    face_index: u16,
    digest: Digest32,
) -> Result<Option<Arc<BundledFaceSource>>, Error> {
    let entry = registered_bundled_face_entry(data, Some(digest));
    registered_bundled_face_source(entry, data, face_index)
}

/// `RegisteredBundledFaceBackingDigest`: identity-only match of a
/// registered font's private backing.
pub fn registered_bundled_face_backing_digest(data: &[u8]) -> Option<Digest32> {
    if data.is_empty() {
        return None;
    }
    with_registry(|registry| {
        registry
            .by_size
            .get(&data.len())?
            .iter()
            .find_map(|candidate| {
                let loaded = candidate.loaded_data()?;
                same_slice(data, &loaded).then_some(candidate.digest)
            })
    })
}

fn registered_bundled_face_entry(
    data: &[u8],
    known_digest: Option<Digest32>,
) -> Option<Arc<Entry>> {
    if data.is_empty() {
        return None;
    }
    let digest = match known_digest {
        Some(digest) => digest,
        None => {
            let hit = with_registry(|registry| {
                registry.by_size.get(&data.len()).and_then(|candidates| {
                    candidates
                        .iter()
                        .find(|candidate| {
                            candidate.loaded_data().is_some_and(|loaded| {
                                same_slice(data, &loaded) || data == &loaded[..]
                            })
                        })
                        .cloned()
                })
            });
            if hit.is_some() {
                return hit;
            }
            sha256(data)
        }
    };
    let entry = with_registry(|registry| {
        registry
            .by_digest
            .get(&digest)
            .filter(|entry| entry.size == data.len())
            .cloned()
    })?;
    if known_digest.is_none()
        && let Some(loaded) = entry.loaded_data()
        && !same_slice(data, &loaded)
        && data != &loaded[..]
    {
        return None;
    }
    Some(entry)
}

fn registered_bundled_face_source(
    entry: Option<Arc<Entry>>,
    data: &[u8],
    face_index: u16,
) -> Result<Option<Arc<BundledFaceSource>>, Error> {
    let Some(entry) = entry else {
        return Ok(None);
    };
    entry.load(data)?;
    let state = entry.state.read().unwrap();
    let Some(Ok(loaded)) = &*state else {
        return Err(Error::msg("bundled font registry entry is not loaded"));
    };
    match loaded.sources.get(usize::from(face_index)) {
        Some(source) => Ok(Some(source.clone())),
        None => Err(Error::msg(format!(
            "load font face {face_index}: collection has {} faces",
            loaded.sources.len()
        ))),
    }
}
