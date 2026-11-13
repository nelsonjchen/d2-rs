// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Key comparison and glob helpers from pinned d2ast/d2ast.go.
use super::*;

impl Key {
    fn path_query(path: &Option<Shared<KeyPath>>, query: fn(&KeyPath) -> bool) -> bool {
        path.as_ref().is_some_and(|p| query(&p.read()))
    }
    fn glob_query(&self, query: fn(&KeyPath) -> bool) -> bool {
        if Self::path_query(&self.key, query) {
            return true;
        }
        for slot in self.edges.iter().flat_map(SharedSlice::iter) {
            let slot = slot.read();
            let edge = slot.as_ref().expect("nil glob edge").read();
            if Self::path_query(&edge.src, query) || Self::path_query(&edge.dst, query) {
                return true;
            }
        }
        Self::path_query(&self.edge_key, query)
    }
    pub fn has_glob(&self) -> bool {
        // EdgeIndex is inspected before EdgeKey in upstream.
        if Self::path_query(&self.key, KeyPath::has_glob) {
            return true;
        }
        for slot in self.edges.iter().flat_map(SharedSlice::iter) {
            let slot = slot.read();
            let edge = slot.as_ref().expect("nil glob edge").read();
            if Self::path_query(&edge.src, KeyPath::has_glob)
                || Self::path_query(&edge.dst, KeyPath::has_glob)
            {
                return true;
            }
        }
        if self.edge_index.as_ref().is_some_and(|i| i.read().glob) {
            return true;
        }
        Self::path_query(&self.edge_key, KeyPath::has_glob)
    }
    pub fn has_triple_glob(&self) -> bool {
        self.glob_query(KeyPath::has_triple_glob)
    }
    pub fn has_multi_glob(&self) -> bool {
        self.glob_query(KeyPath::has_multi_glob)
    }
    pub fn supports_glob_filters(&self) -> bool {
        if Self::path_query(&self.key, KeyPath::has_glob)
            && self.edges.as_ref().is_none_or(SharedSlice::is_empty)
        {
            return true;
        }
        if self.edge_index.as_ref().is_some_and(|i| i.read().glob) && self.edge_key.is_none() {
            return true;
        }
        Self::path_query(&self.edge_key, KeyPath::has_glob)
    }
    pub fn equals(&self, other: &Self) -> bool {
        self.compare(other, false)
    }
    pub fn d2_oracle_equals(&self, other: &Self) -> bool {
        self.compare(other, true)
    }
    fn compare(&self, other: &Self, oracle: bool) -> bool {
        if self.ampersand != other.ampersand
            || self.not_ampersand != other.not_ampersand
            || self.key.is_none() != other.key.is_none()
            || self.edge_index.is_none() != other.edge_index.is_none()
        {
            return false;
        }
        if !oracle
            && let (Some(a), Some(b)) = (&self.edge_index, &other.edge_index)
            && !a.read().equals(&b.read())
        {
            return false;
        }
        if self.edge_key.is_none() != other.edge_key.is_none() {
            return false;
        }
        let edges = |key: &Key| {
            key.edges
                .iter()
                .flat_map(SharedSlice::iter)
                .map(|slot| slot.read().clone())
                .collect::<Vec<_>>()
        };
        let left = edges(self);
        let right = edges(other);
        if left.len() != right.len() {
            return false;
        }
        if !oracle {
            for (a, b) in left.iter().zip(&right) {
                if !a
                    .as_ref()
                    .expect("nil equality edge")
                    .read()
                    .equals(&b.as_ref().expect("nil equality edge").read())
                {
                    return false;
                }
            }
        }
        let map_len = |map: &Option<Shared<Map>>| {
            map.as_ref()
                .map_or(0, |m| m.read().nodes.as_ref().map_or(0, SharedSlice::len))
        };
        if self.value.map.is_none() != other.value.map.is_none() {
            if map_len(&self.value.map) > 0 || map_len(&other.value.map) > 0 {
                return false;
            }
        } else if self.value.unbox().is_none() != other.value.unbox().is_none() {
            return false;
        }
        for (a, b) in [(&self.key, &other.key), (&self.edge_key, &other.edge_key)] {
            if let (Some(a), Some(b)) = (a, b)
                && !a.read().equals(&b.read())
            {
                return false;
            }
        }
        if map_len(&self.value.map) > 0 {
            if map_len(&self.value.map) != map_len(&other.value.map) {
                return false;
            }
            let entries = |map: &Option<Shared<Map>>| {
                map.as_ref()
                    .unwrap()
                    .read()
                    .nodes
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(|slot| slot.read().map_key.clone())
                    .collect::<Vec<_>>()
            };
            for (a, b) in entries(&self.value.map)
                .iter()
                .zip(entries(&other.value.map))
            {
                if !keys_equal(
                    a.as_ref().map(Shared::read).as_deref(),
                    b.as_ref().map(Shared::read).as_deref(),
                ) {
                    return false;
                }
            }
        }
        if self.value.unbox().is_some() {
            let a = self.value.scalar_box();
            let b = other.value.scalar_box();
            if a.unbox().is_none() != b.unbox().is_none() {
                return false;
            }
            if a.unbox().is_some() && a.scalar_bytes() != b.scalar_bytes() {
                return false;
            }
        }
        // Upstream only checks Primary when the left operand has one.
        oracle
            || self.primary.unbox().is_none()
            || (other.primary.unbox().is_some()
                && self.primary.scalar_bytes() == other.primary.scalar_bytes())
    }
}
/// Nil-receiver semantics of upstream Key.Equals.
pub fn keys_equal(a: Option<&Key>, b: Option<&Key>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.equals(b),
        _ => false,
    }
}
