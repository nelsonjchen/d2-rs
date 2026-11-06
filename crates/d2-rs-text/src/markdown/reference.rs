// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Goldmark v1.8.5 parser/parser.go reference storage and lookup.
use super::{bytes::Bytes, link::normalize_reference_label};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

/// Shared reference fields; retain nil separately from an empty byte slice.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Reference {
    pub label: Option<Bytes>,
    pub destination: Option<Bytes>,
    pub title: Option<Bytes>,
}

pub type ReferenceHandle = Rc<RefCell<Reference>>;

#[derive(Debug, Default)]
pub struct References {
    entries: HashMap<Vec<u8>, ReferenceHandle>,
}

impl References {
    /// Normalize only at insertion. The first definition for a key wins,
    /// retaining the original reference object rather than copying its fields.
    pub fn add(&mut self, reference: ReferenceHandle) {
        let key = {
            let value = reference.borrow();
            value
                .label
                .as_ref()
                .map_or_else(Vec::new, |v| normalize_reference_label(&v.bytes()))
        };
        self.entries.entry(key).or_insert(reference);
    }

    /// The caller supplies a normalized key, matching Context.Reference.
    pub fn get(&self, key: &[u8]) -> Option<ReferenceHandle> {
        self.entries.get(key).cloned()
    }
}
