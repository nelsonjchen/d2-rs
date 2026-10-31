// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! D2 DeserializeGraph pointer reconstruction and source topology validation.
//! Lookup AbsID keys are independent of local ID and computed output AbsID.

use super::*;
use d2_rs_core::{ir::go_lowercase, quote_go_bytes};

pub(super) struct Identity {
    pub names: Vec<String>,
    pub parents: Vec<Option<usize>>,
    pub children: Vec<Vec<usize>>,
    pub lookup: BTreeMap<String, usize>,
    pub canonical: Vec<String>,
    pub topology_error: Option<String>,
    root: usize,
}

impl Identity {
    pub fn read(document: &Value) -> Result<Self, D2Error> {
        let mut records = object_records(document)?;
        let root = records.len();
        records.push(
            document
                .get("root")
                .and_then(Value::as_object)
                .ok_or(D2Error::RootType)?,
        );
        let names: Vec<_> = records
            .iter()
            .map(|o| o.get("id").and_then(Value::as_str).unwrap_or("").to_owned())
            .collect();
        let mut lookup = BTreeMap::from([(String::new(), root)]);
        for (index, object) in records[..root].iter().enumerate() {
            let key = object
                .get("AbsID")
                .and_then(Value::as_str)
                .ok_or(D2Error::MissingObjectId { index })?;
            lookup.insert(key.to_owned(), index);
        }
        let mut parents = vec![None; records.len()];
        let mut children = vec![Vec::new(); records.len()];
        // Objects precede root, matching append(sg.Objects, sg.Root), and a
        // later map sharing a lookup key replaces that pointer's child list.
        for record in &records {
            let Some(list) = record.get("ChildrenArray").filter(|v| !v.is_null()) else {
                continue;
            };
            let list = list.as_array().ok_or(D2Error::ArrayType("ChildrenArray"))?;
            let key = record
                .get("AbsID")
                .and_then(Value::as_str)
                .ok_or(D2Error::MissingObjectId { index: root })?;
            let owner = lookup
                .get(key)
                .copied()
                .ok_or_else(|| D2Error::MissingChild {
                    object: key.into(),
                    child: "<parent>".into(),
                })?;
            let mut next = Vec::with_capacity(list.len());
            for child in list {
                let child = child.as_str().ok_or_else(|| D2Error::MissingChild {
                    object: key.into(),
                    child: "<non-string>".into(),
                })?;
                let index = lookup
                    .get(child)
                    .copied()
                    .ok_or_else(|| D2Error::MissingChild {
                        object: key.into(),
                        child: child.into(),
                    })?;
                parents[index] = Some(owner);
                next.push(index);
            }
            children[owner] = next;
        }
        let mut identity = Self {
            names,
            parents,
            children,
            lookup,
            canonical: vec![String::new(); records.len()],
            topology_error: None,
            root,
        };
        match identity.validate() {
            Err(message) => identity.topology_error = Some(message),
            Ok(visits) => {
                identity.canonical[root] = identity.names[root].clone();
                for index in visits {
                    let parent = identity.parents[index].expect("validated non-root parent");
                    identity.canonical[index] = if identity.names[parent].is_empty() {
                        identity.names[index].clone()
                    } else {
                        format!("{}.{}", identity.canonical[parent], identity.names[index])
                    };
                }
            }
        }
        Ok(identity)
    }

    fn validate(&self) -> Result<Vec<usize>, String> {
        let mut stack = vec![(self.root, None, 0usize)];
        let mut visited = BTreeMap::new();
        let mut visits = Vec::new();
        let mut scheduled = 0usize;
        while let Some((index, parent, depth)) = stack.pop() {
            let name = || quote_go_bytes(self.names[index].as_bytes());
            if visited.contains_key(&index) {
                let mut cursor = parent;
                while let Some(ancestor) = cursor {
                    if ancestor == index {
                        return Err(format!("D2 object {} forms a ChildrenArray cycle", name()));
                    }
                    cursor = visited.get(&ancestor).copied().flatten();
                }
                return Err(format!(
                    "D2 object {} is referenced more than once in ChildrenArray",
                    name()
                ));
            }
            if self.parents[index] != parent {
                return Err(format!(
                    "D2 object {} has a Parent that does not match its ChildrenArray parent",
                    name()
                ));
            }
            if depth > 256 {
                return Err("D2 object nesting depth exceeds limit 256".into());
            }
            visited.insert(index, parent);
            if parent.is_some() {
                if visits.len() == 10_000 {
                    return Err("D2 object count exceeds limit 10000".into());
                }
                visits.push(index);
            }
            if self.children[index].len() > 10_000 - scheduled {
                return Err("D2 object count exceeds limit 10000".into());
            }
            scheduled += self.children[index].len();
            stack.extend(
                self.children[index]
                    .iter()
                    .rev()
                    .map(|&child| (child, Some(index), depth + 1)),
            );
        }
        // Children maps are keyed by Go's lowercase local ID; distinct child
        // pointers with equal lowercase names collapse in DeserializeGraph.
        for index in std::iter::once(self.root).chain(visits.iter().copied()) {
            let keys: BTreeSet<_> = self.children[index]
                .iter()
                .map(|&child| go_lowercase(self.names[child].as_bytes()))
                .collect();
            if keys.len() != self.children[index].len() {
                return Err(format!(
                    "D2 object {} has inconsistent Children and ChildrenArray sets",
                    quote_go_bytes(self.names[index].as_bytes())
                ));
            }
        }
        if self.root != visits.len() {
            return Err("D2 graph Objects and object tree contain different sets".into());
        }
        // Each serialized objects slot allocated a distinct pointer. All
        // visited non-root pointers must therefore be exactly those slots.
        for index in 0..self.root {
            if !visited.contains_key(&index) {
                return Err(format!(
                    "D2 graph Objects contains object {} outside the object tree",
                    quote_go_bytes(self.names[index].as_bytes())
                ));
            }
        }
        Ok(visits)
    }

    pub fn hierarchy(&self) -> BTreeMap<Option<NodeId>, Vec<NodeId>> {
        (0..=self.root)
            .map(|parent| {
                let key = (parent != self.root).then_some(NodeId(parent as u32));
                (
                    key,
                    self.children[parent]
                        .iter()
                        .map(|&child| NodeId(child as u32))
                        .collect(),
                )
            })
            .collect()
    }
}

pub(super) fn serialize(document: &mut Value) -> Result<(), D2Error> {
    let identity = Identity::read(document)?;
    debug_assert!(identity.topology_error.is_none());
    let mut records = object_records_mut(document)?;
    for (index, object) in records.iter_mut().enumerate() {
        object.insert(
            "AbsID".into(),
            Value::String(identity.canonical[index].clone()),
        );
        object.insert(
            "ChildrenArray".into(),
            json!(
                identity.children[index]
                    .iter()
                    .map(|&child| &identity.canonical[child])
                    .collect::<Vec<_>>()
            ),
        );
    }
    let root = document["root"].as_object_mut().expect("typed root");
    root.insert(
        "AbsID".into(),
        Value::String(identity.canonical[identity.root].clone()),
    );
    root.insert(
        "ChildrenArray".into(),
        json!(
            identity.children[identity.root]
                .iter()
                .map(|&child| &identity.canonical[child])
                .collect::<Vec<_>>()
        ),
    );
    for edge in edge_records_mut(document)? {
        for endpoint in ["Src", "Dst"] {
            if let Some(index) = edge
                .get(endpoint)
                .and_then(Value::as_str)
                .and_then(|key| identity.lookup.get(key))
            {
                edge.insert(
                    endpoint.into(),
                    Value::String(identity.canonical[*index].clone()),
                );
            }
        }
    }
    Ok(())
}
