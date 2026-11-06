// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Declaration and intermediate-edge branches of d2oracle.move.
use super::*;
use std::collections::HashMap;

type Parts = Vec<Option<Shared<ast::StringBox>>>;

fn parts(path: &Shared<KeyPath>) -> Parts {
    path.read()
        .path
        .iter()
        .flat_map(SharedSlice::iter)
        .map(|s| s.read().clone())
        .collect()
}
fn name(part: &Option<Shared<ast::StringBox>>) -> String {
    part.as_ref()
        .expect("compiled path component")
        .read()
        .scalar_string()
}
fn path(parts: Parts) -> Shared<KeyPath> {
    Shared::new(KeyPath {
        path: (!parts.is_empty()).then(|| parts.into()),
        ..Default::default()
    })
}
fn set(path: &Shared<KeyPath>, parts: Parts) {
    path.write().path = (!parts.is_empty()).then(|| parts.into());
}
fn new_range() -> ast::Range {
    let mut range = ast::Range::default();
    range.start.line = 1;
    range.end.line = 1;
    range
}
fn new_map() -> Shared<Map> {
    Shared::new(Map {
        range: new_range(),
        nodes: None,
    })
}
fn has_edges(key: &Shared<Key>) -> bool {
    key.read()
        .edges
        .as_ref()
        .is_some_and(|edges| !edges.is_empty())
}
fn common(a: &[String], b: &[String]) -> usize {
    // Source getCommonPath records the last equal index (not an early break).
    a.iter()
        .zip(b)
        .enumerate()
        .filter(|(_, (a, b))| a == b)
        .map(|(i, _)| i + 1)
        .next_back()
        .unwrap_or(0)
}
fn relative(scope: &[String], target: &[String], target_parts: &Parts) -> Parts {
    let count = common(scope, target);
    (count..scope.len())
        .map(|_| Some(Shared::new(ast::raw_string_box("_", true))))
        .chain(target_parts[count..].iter().cloned())
        .collect()
}
fn append_unique(map: &Shared<Map>, key: Key) {
    if map_nodes(&map.read()).iter().any(|node| {
        node.map_key
            .as_ref()
            .is_some_and(|k| k.read().d2_oracle_equals(&key))
    }) {
        return;
    }
    let reserved = key
        .key
        .as_ref()
        .and_then(|p| p.read().string_ida())
        .and_then(|p| p.first().cloned())
        .is_some_and(|s| is_reserved_keyword(&s));
    let mut map = map.write();
    let mut nodes = map_nodes(&map);
    nodes.push(MapNodeBox {
        map_key: Some(Shared::new(key)),
        ..Default::default()
    });
    let single_reserved = nodes.len() == 1 && reserved;
    map.nodes = Some(nodes.into());
    if !single_reserved && !map.is_file_map() && map.range.one_line() {
        map.range.end.line += 1
    }
}
fn delete(map: &Shared<Map>, key: &Shared<Key>) {
    let mut map = map.write();
    let mut nodes = map_nodes(&map);
    if let Some(index) = nodes
        .iter()
        .position(|node| node.map_key.as_ref().is_some_and(|k| k.ptr_eq(key)))
    {
        nodes.remove(index);
    }
    map.nodes = (!nodes.is_empty()).then(|| nodes.into());
}
fn most_nested(refs: &[Reference]) -> Vec<&Reference> {
    let mut most = None;
    for r in refs.iter().filter(|r| !has_edges(&r.key)) {
        if most.is_none_or(|m: &Reference| r.scope_path.len() > m.scope_path.len()) {
            most = Some(r)
        }
    }
    refs.iter()
        .filter(|r| !has_edges(&r.key) && most.is_some_and(|m| r.scope_path == m.scope_path))
        .collect()
}

/// Source graph snapshots references and primary maps before AST mutation.
pub(super) struct Graph<'a> {
    arena: &'a ir::Arena,
    root: ir::NodeId,
    ast: Shared<Map>,
    refs: HashMap<ir::NodeId, Vec<Reference>>,
    maps: HashMap<ir::NodeId, Shared<Map>>,
}
impl<'a> Graph<'a> {
    pub(super) fn new(arena: &'a ir::Arena, root: ir::NodeId, ast: Shared<Map>) -> Self {
        let mut out = Self {
            arena,
            root,
            ast,
            refs: HashMap::new(),
            maps: HashMap::new(),
        };
        out.collect(root);
        out
    }
    fn collect(&mut self, map: ir::NodeId) {
        let ir::Node::Map(map) = self.arena.node(map) else {
            return;
        };
        for id in map.fields.iter().flatten().copied() {
            let refs = references(self.arena, id);
            if let Some(map) = refs
                .iter()
                .filter(|r| r.primary)
                .filter_map(|r| r.key.read().value.map.clone())
                .next_back()
            {
                self.maps.insert(id, map);
            }
            self.refs.insert(id, refs);
            if let Some(map) = self.arena.map(id) {
                self.collect(map)
            }
        }
    }
    fn refs(&self, object: ir::NodeId) -> &[Reference] {
        self.refs
            .get(&object)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
    fn landing(&self, parent: Option<ir::NodeId>) -> Shared<Map> {
        let Some(parent) = parent else {
            return self.ast.clone();
        };
        let mut refs = self.refs(parent).to_vec();
        if !refs.iter().any(|r| !has_edges(&r.key)) {
            let most = refs
                .iter()
                .rev()
                .max_by_key(|r| r.scope_path.len())
                .expect("implicit destination reference");
            let p = path(parts(&most.path)[..=most.index].to_vec());
            let key = Shared::new(Key {
                key: Some(p.clone()),
                range: new_range(),
                ..Default::default()
            });
            most.scope.write().insert_after(
                Some(&Child::Key(most.key.clone())),
                Some(Child::Key(key.clone())),
            );
            refs.push(Reference {
                path: p,
                index: most.index,
                key,
                edge: None,
                scope: most.scope.clone(),
                scope_path: most.scope_path.clone(),
                primary: true,
                scope_ast: most.scope_ast.clone(),
            });
        }
        let most = most_nested(&refs);
        for r in &most {
            if r.index + 1 == parts(&r.path).len()
                && let Some(map) = &r.key.read().value.map
            {
                return map.clone();
            }
        }
        let landing = new_map();
        let r = most.last().expect("destination declaration reference");
        let p = parts(&r.path);
        if r.index + 1 < p.len() {
            let detached = Key {
                key: Some(path(p[r.index + 1..].to_vec())),
                value: r.key.read().value.clone(),
                range: new_range(),
                ..Default::default()
            };
            set(&r.path, p[..=r.index].to_vec());
            append_unique(&landing, detached);
        } else {
            let primary = r.key.read().value.scalar_box();
            r.key.write().primary = primary;
        }
        r.key.write().value = ValueBox::from_child(Some(Child::Map(landing.clone())));
        landing
    }
    pub(super) fn hoist_children(&self, absolute: &[String], reference: &Reference) {
        let Some(children) = reference.key.read().value.map.clone() else {
            return;
        };
        bump_underscores(&children);
        let mut scope = self.ast.clone();
        let mut prefix = absolute.to_vec();
        for length in (1..absolute.len()).rev() {
            if let Some(parent) = field(self.arena, self.root, &absolute[..length])
                && let Some(map) = self.maps.get(&parent)
            {
                scope = map.clone();
                prefix = absolute[length..].to_vec();
                break;
            }
        }
        prefix.pop();
        for node in map_nodes(&children.read()) {
            let Some(key) = &node.map_key else { continue };
            let original = key.read().key.clone();
            if original
                .as_ref()
                .and_then(|p| p.read().string_ida())
                .and_then(|p| p.first().cloned())
                .is_some_and(|s| is_reserved_keyword(&s))
            {
                continue;
            }
            let mut p = parts(&Shared::new(KeyPath::from_strings(prefix.clone())));
            if let Some(original) = original {
                p.extend(parts(&original))
            }
            if !p.is_empty() {
                key.write().key = Some(path(p));
            }
            scope
                .write()
                .insert_before(Some(&Child::Key(reference.key.clone())), node.unbox());
        }
    }
}
pub(super) fn bump_underscores(map: &Shared<Map>) {
    for node in map_nodes(&map.read()) {
        let Some(key) = node.map_key else { continue };
        let key = key.read();
        for p in key.key.iter().cloned().chain(
            key.edges
                .iter()
                .flat_map(SharedSlice::iter)
                .filter_map(|e| e.read().clone())
                .flat_map(|e| {
                    let e = e.read();
                    e.src
                        .iter()
                        .chain(e.dst.iter())
                        .cloned()
                        .collect::<Vec<_>>()
                }),
        ) {
            let pparts = parts(&p);
            if pparts.first().is_some_and(|p| name(p) == "_") {
                set(&p, pparts[1..].to_vec());
            }
        }
        if let Some(map) = &key.value.map {
            bump_underscores(map);
        }
    }
}
fn filter_reserved(value: &ValueBox) -> (ValueBox, ValueBox) {
    let Some(map) = &value.map else {
        return (value.clone(), ValueBox::default());
    };
    let mut with = Vec::new();
    let mut without = Vec::new();
    let mut comments = Vec::new();
    for node in map_nodes(&map.read()) {
        let Some(key) = &node.map_key else {
            if node.comment.is_some() || node.block_comment.is_some() {
                comments.push(node)
            }
            continue;
        };
        let p = key.read().key.as_ref().map(parts).unwrap_or_default();
        let bucket = if p.len() == 1 && is_reserved_keyword(&name(&p[0])) {
            &mut with
        } else {
            &mut without
        };
        bucket.append(&mut comments);
        bucket.push(node);
    }
    let mut reserved = value.clone();
    reserved.map = (!with.is_empty()).then(|| {
        Shared::new(Map {
            range: map.read().range.clone(),
            nodes: Some(with.into()),
        })
    });
    let ordinary = ValueBox {
        map: (!without.is_empty()).then(|| {
            Shared::new(Map {
                range: map.read().range.clone(),
                nodes: Some(without.into()),
            })
        }),
        ..Default::default()
    };
    (reserved, ordinary)
}
fn relocate_underscores(
    map: &Shared<Map>,
    old: &[String],
    new: &[String],
) -> Result<(), GraphEditError> {
    for node in map_nodes(&map.read()) {
        let Some(key) = node.map_key else { continue };
        let key = key.read();
        for p in key.key.iter().cloned().chain(
            key.edges
                .iter()
                .flat_map(SharedSlice::iter)
                .filter_map(|e| e.read().clone())
                .flat_map(|e| {
                    let e = e.read();
                    e.src
                        .iter()
                        .chain(e.dst.iter())
                        .cloned()
                        .collect::<Vec<_>>()
                }),
        ) {
            let ids = p.read().string_ida().unwrap_or_default();
            if ids.first().is_some_and(|s| s == "_") {
                let absolute = resolve_scoped_path(old, &ids)?;
                let ap = parts(&Shared::new(KeyPath::from_strings(absolute.clone())));
                set(&p, relative(new, &absolute, &ap));
            }
        }
    }
    Ok(())
}

pub(super) fn execute(
    arena: &ir::Arena,
    root: ir::NodeId,
    ast: Shared<Map>,
    object: ir::NodeId,
    old: &[String],
    new: &[String],
    new_parts: Parts,
    descendants: bool,
) -> Result<(), GraphEditError> {
    let graph = Graph::new(arena, root, ast.clone());
    let refs = graph.refs(object);
    let needs_landing = refs
        .iter()
        .any(|r| r.edge.is_none() || r.index + 1 != parts(&r.path).len());
    let parent = if new.len() > 1 {
        field(arena, root, &new[..new.len() - 1])
    } else {
        None
    };
    let landing = if needs_landing {
        graph.landing(parent)
    } else {
        ast
    };
    let most = most_nested(refs);
    for r in refs {
        let before = parts(&r.path);
        let reserved_end = before
            .iter()
            .position(|p| is_reserved_keyword(&name(p)))
            .unwrap_or(before.len());
        let explicit = r.index + 1 == reserved_end && r.key.read().value.map.is_none();
        if old.last() != new.last() {
            let slot = r
                .path
                .read()
                .path
                .as_ref()
                .and_then(|p| p.get(r.index))
                .expect("object reference component");
            *slot.write() = Some(Shared::new(ast::raw_string_box(
                new.last().unwrap().as_str(),
                true,
            )));
        }
        if has_edges(&r.key) {
            continue;
        }
        let p = parts(&r.path);
        let first = p.iter().position(|p| name(p) != "_").unwrap_or(0);
        // ResolveUnderscoreKey strips leading parent markers and returns the
        // climbed scope object independently of the remaining path.
        let mut ids = p.iter().map(name).collect::<Vec<_>>();
        let resolved_scope = &r.scope_path[..r.scope_path.len().saturating_sub(first)];
        if resolved_scope != absolute_path(arena, object) {
            ids = ids[first..].to_vec();
        }
        let ends_reserved = ids.last().is_some_and(|s| is_reserved_keyword(s));
        ids.retain(|s| !is_reserved_keyword(s));
        if (!descendants && ids.len() == 1) || (descendants && r.index == first) {
            let absolute = r
                .scope_path
                .iter()
                .cloned()
                .chain(p.iter().map(name))
                .collect::<Vec<_>>();
            if !descendants {
                graph.hoist_children(&absolute, r);
            }
            delete(&r.scope, &r.key);
            let source = r.key.read();
            let mut detached = Key {
                primary: source.primary.clone(),
                key: Some(path(
                    parts(source.key.as_ref().expect("declaration path"))
                        .into_iter()
                        .filter(|p| name(p) != "_")
                        .collect(),
                )),
                value: source.value.clone(),
                ..Default::default()
            };
            if let Some(map) = &source.value.map {
                if !descendants {
                    let nodes = map_nodes(&map.read())
                        .into_iter()
                        .filter(|n| {
                            n.map_key
                                .as_ref()
                                .and_then(|k| k.read().key.clone())
                                .and_then(|p| p.read().string_ida())
                                .and_then(|p| p.first().cloned())
                                .is_some_and(|s| is_reserved_keyword(&s))
                        })
                        .collect::<Vec<_>>();
                    detached.value.map = (!nodes.is_empty()).then(|| {
                        Shared::new(Map {
                            range: map.read().range.clone(),
                            nodes: Some(nodes.into()),
                        })
                    });
                } else {
                    relocate_underscores(map, &absolute_path(arena, object), new)?;
                }
            }
            drop(source);
            append_unique(&landing, detached);
        } else if ids.len() > 1
            && (ends_reserved || !explicit || most.iter().any(|m| std::ptr::eq(*m, r)))
        {
            let mut detached = Key {
                key: Some(path(if descendants {
                    p[r.index..].to_vec()
                } else {
                    vec![p[r.index].clone()]
                })),
                ..Default::default()
            };
            if descendants {
                detached.value = r.key.read().value.clone();
                r.key.write().value = ValueBox::default();
            } else if r.index + 1
                == p.iter()
                    .take_while(|p| !is_reserved_keyword(&name(p).to_lowercase()))
                    .count()
            {
                let (with, without) = filter_reserved(&r.key.read().value);
                detached.value = with;
                r.key.write().value = without;
                detached.key = Some(path(p[r.index..].to_vec()));
                set(&r.path, p[..=r.index].to_vec());
            }
            let mut remaining = parts(&r.path);
            if descendants {
                remaining.truncate(r.index);
            } else {
                remaining.remove(r.index);
            }
            set(&r.path, remaining);
            append_unique(&landing, detached);
        } else if common(old, new) > 0 {
            let mut extended = p[..r.index].to_vec();
            extended.extend_from_slice(&new_parts[common(old, new)..]);
            extended.extend_from_slice(&p[r.index + 1..]);
            set(&r.path, extended);
        } else if common(&r.scope_path, new) != r.scope_path.len() {
            delete(&r.scope, &r.key);
        } else {
            set(&r.path, p[r.index..].to_vec());
            if map_nodes(&landing.read()).iter().any(|n| {
                n.map_key
                    .as_ref()
                    .is_some_and(|k| !k.ptr_eq(&r.key) && k.read().d2_oracle_equals(&r.key.read()))
            }) {
                delete(&r.scope, &r.key);
            }
        }
    }
    for (i, r) in refs.iter().enumerate() {
        if !has_edges(&r.key) || (i > 0 && r.path.ptr_eq(&refs[i - 1].path)) {
            continue;
        }
        let mut p = parts(&r.path);
        let first = p.iter().position(|p| name(p) != "_").unwrap_or(0);
        let new_path = relative(&r.scope_path, new, &new_parts);
        if r.index + 1 != p.len() {
            let old_parts = parts(&Shared::new(KeyPath::from_strings(old.iter().cloned())));
            let old_path = relative(&r.scope_path, old, &old_parts);
            if descendants {
                let mut diff = old_path.len() as isize - new_path.len() as isize;
                if diff == 0 {
                    diff = old_path
                        .iter()
                        .zip(&new_path)
                        .enumerate()
                        .filter(|(_, (a, b))| name(a) != name(b))
                        .map(|(i, _)| (i + 1) as isize)
                        .next_back()
                        .unwrap_or(0);
                }
                if diff > 0 && r.index != first {
                    append_unique(
                        &r.scope,
                        Key {
                            key: Some(path(p[r.index - diff as usize..r.index].to_vec())),
                            ..Default::default()
                        },
                    );
                }
                let mut next = p[..r.index + 1 - old_path.len()].to_vec();
                next.extend(new_path);
                next.extend_from_slice(&p[r.index + 1..]);
                set(&r.path, next);
            } else {
                append_unique(
                    &landing,
                    Key {
                        key: Some(path(vec![p[r.index].clone()])),
                        ..Default::default()
                    },
                );
                p.remove(r.index);
                set(&r.path, p);
            }
        } else {
            ensure_parent(arena, root, refs, r)?;
            if descendants {
                let tail = (r.index + new_path.len()).min(p.len());
                let mut next = new_path;
                next.extend_from_slice(&p[tail..]);
                set(&r.path, next);
            } else {
                set(&r.path, new_path);
            }
        }
    }
    Ok(())
}
