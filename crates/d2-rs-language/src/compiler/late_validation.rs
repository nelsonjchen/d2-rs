// SPDX-License-Identifier: MPL-2.0
//! Unmeasured object state for compileBoard's late validation passes.
//! Mirrors compileMap/EnsureChild creation order, not measured layout order.
use super::{Compiler, edge_creation::error, graph};
use d2_rs_core::{
    ast::{self, Child, Node as _, Shared, StringBox, keywords::*},
    ir::{Node, NodeId, equal_fold, go_lowercase},
};
use std::collections::{HashMap, HashSet};
// d2graph/grid_diagram.go: each dimension and the product bound empty cells too.
pub(super) use d2_rs_core::grid::MAX_GRID_DIMENSION;
#[derive(Default)]
struct Object {
    parent: Option<usize>,
    id: Vec<u8>,
    children: Vec<usize>,
    children_by_id: HashMap<Vec<u8>, usize>,
    alive: bool,
    label: Vec<u8>,
    label_key: Option<Shared<ast::Key>>,
    language: String,
    shape: Vec<u8>,
    link: bool,
    tooltip: Option<Vec<u8>>,
    source_arrowhead: Option<usize>,
    target_arrowhead: Option<usize>,
    near: Option<(Child, ast::KeyPath)>,
    grid_rows: Option<i64>,
    grid_columns: Option<i64>,
    grid_rows_key: Option<Shared<ast::Key>>,
    grid_columns_key: Option<Shared<ast::Key>>,
    top: Option<Shared<ast::Key>>,
    left: Option<Shared<ast::Key>>,
}
struct Connection {
    source: usize,
    target: usize,
    origin: NodeId,
}
pub(super) struct Board<'a> {
    compiler: &'a Compiler,
    classes: Option<NodeId>,
    objects: Vec<Object>,
    edges: Vec<Connection>,
    active: HashSet<NodeId>,
}

pub(super) type HierarchyOrder = Vec<(Vec<u8>, Vec<Vec<u8>>)>;

pub(super) fn validate(
    compiler: &Compiler,
    root: NodeId,
    retain_hierarchy: bool,
) -> (Vec<ast::Error>, Option<HierarchyOrder>) {
    let mut board = Board::new(compiler, root);
    board.compile_map(0, root);
    for id in 1..board.objects.len() {
        if board.objects[id].alive && board.objects[id].shape.is_empty() {
            board.objects[id].shape = if board.sequence(id).is_some() {
                b"rectangle".to_vec()
            } else {
                match board.objects[id].language.as_str() {
                    "markdown" | "latex" => b"text".to_vec(),
                    "" => b"rectangle".to_vec(),
                    _ => b"code".to_vec(),
                }
            };
        }
    }
    let mut errors = Vec::new();
    board.labels(&mut errors);
    board.near(&mut errors);
    board.edge_boundaries(&mut errors);
    board.positions(&mut errors);
    let hierarchy = retain_hierarchy.then(|| board.hierarchy_order());
    (errors, hierarchy)
}
/// Child arrays retain EnsureChild allocation order. Public Compile sorts the
/// graph's object slice by AST afterward, but never sorts these child arrays.
pub(super) fn hierarchy_order(compiler: &Compiler, root: NodeId) -> HierarchyOrder {
    let mut board = Board::new(compiler, root);
    board.compile_map(0, root);
    board.hierarchy_order()
}

impl<'a> Board<'a> {
    fn hierarchy_order(&self) -> HierarchyOrder {
        self.objects
            .iter()
            .enumerate()
            .filter(|(id, obj)| *id == 0 || obj.alive)
            .map(|(id, obj)| {
                (
                    self.abs_id(id),
                    obj.children
                        .iter()
                        .map(|child| self.abs_id(*child))
                        .collect(),
                )
            })
            .collect()
    }

    // compileBoard validates capacity after compileMap and before validateKeys.
    // Rejected dimensions never become attributes, so they cannot produce a
    // second, less specific capacity error. Report at the later attribute key.
    pub(super) fn grid_dimensions(&self) -> Vec<ast::Error> {
        let mut errors = Vec::new();
        for (id, obj) in self.objects.iter().enumerate() {
            if id != 0 && !obj.alive {
                continue;
            }
            let (Some(rows), Some(columns)) = (obj.grid_rows, obj.grid_columns) else {
                continue;
            };
            let Err(message) = d2_rs_core::grid::grid_capacity(rows, columns) else {
                continue;
            };
            let (Some(rows_key), Some(columns_key)) = (&obj.grid_rows_key, &obj.grid_columns_key)
            else {
                continue;
            };
            let key = if columns_key.read().range.before(&rows_key.read().range) {
                rows_key
            } else {
                columns_key
            };
            errors.push(error(&Child::Key(key.clone()), message));
        }
        errors
    }

    pub(super) fn new(compiler: &'a Compiler, root: NodeId) -> Self {
        let classes = compiler
            .graph_field(root, &["classes"])
            .and_then(|id| compiler.arena.map(id));
        Self {
            compiler,
            classes,
            objects: vec![Object::default()],
            edges: Vec::new(),
            active: HashSet::new(),
        }
    }
    /// compileField/compileEdge materialize reference ScopeObj through
    /// Root.EnsureChild(BoardIDA), even before that scope's own field visit.
    pub(super) fn reference_scopes(&mut self, origin: NodeId) {
        let scopes = match self.compiler.arena.node(origin) {
            Node::Field(node) => node
                .references
                .iter()
                .flatten()
                .filter_map(|r| r.read().context.as_ref().and_then(|c| c.read().scope_map))
                .collect::<Vec<_>>(),
            Node::Edge(node) => node
                .references
                .iter()
                .flatten()
                .filter_map(|r| r.read().context.as_ref().and_then(|c| c.read().scope_map))
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        for scope in scopes {
            if self.compiler.arena.is_var(scope) {
                continue;
            }
            if let Some(parts) = crate::ir::board_ida(&self.compiler.arena, scope) {
                let mut obj = 0;
                for part in parts {
                    obj = self.ensure(obj, &ast::ValueBox::from_child(part).string_box());
                }
            }
        }
    }
    pub(super) fn child(&mut self, parent: usize, field: NodeId) -> usize {
        let Node::Field(field) = self.compiler.arena.node(field) else {
            unreachable!()
        };
        self.ensure(parent, &field.name.clone())
    }
    pub(super) fn set_label(&mut self, obj: usize, field: NodeId) {
        self.label(obj, field);
    }
    pub(super) fn finish_fields(&mut self, obj: usize) {
        if matches!(self.objects[obj].shape.as_slice(), b"class" | b"sql_table") {
            let children = std::mem::take(&mut self.objects[obj].children);
            self.objects[obj].children_by_id.clear();
            for child in children {
                self.objects[child].alive = false;
            }
        }
    }
    pub(super) fn edge_attributes(&mut self, field: NodeId) -> usize {
        let obj = self.objects.len();
        self.objects.push(Object::default());
        self.label(obj, field);
        obj
    }
    pub(super) fn arrowhead_attributes(
        &mut self,
        edge: usize,
        field: NodeId,
        source: bool,
    ) -> usize {
        let prior = if source {
            self.objects[edge].source_arrowhead
        } else {
            self.objects[edge].target_arrowhead
        };
        let obj = prior.unwrap_or_else(|| {
            let obj = self.objects.len();
            self.objects.push(Object::default());
            if source {
                self.objects[edge].source_arrowhead = Some(obj)
            } else {
                self.objects[edge].target_arrowhead = Some(obj)
            }
            obj
        });
        self.label(obj, field);
        obj
    }
    /// Called only after compileReserved's successful scalar path and nested
    /// position/icon-style work. The reported AST is the current scalar.
    pub(super) fn security(&mut self, obj: usize, field: NodeId) -> Vec<ast::Error> {
        let Node::Field(holder) = self.compiler.arena.node(field) else {
            return Vec::new();
        };
        let name = holder.name.scalar_string();
        let Some(primary) = self.compiler.arena.primary(field) else {
            return Vec::new();
        };
        let Node::Scalar(scalar) = self.compiler.arena.node(primary) else {
            return Vec::new();
        };
        let bytes = scalar.value.scalar_bytes();
        let node = scalar.value.unbox().expect("reserved scalar AST");
        self.reserved(obj, field, &name);
        match name.as_str() {
            "link" => self.objects[obj].link = true,
            "tooltip" => self.objects[obj].tooltip = Some(bytes),
            _ => {}
        }
        let attrs = &self.objects[obj];
        let mut errors = Vec::new();
        if attrs.link {
            if super::import_urls::parse(&attrs.label, true).is_ok_and(|url| url.has_host) {
                errors.push(error(
                    &node,
                    "Label cannot be set to URL when link is also set (for security)",
                ));
            }
            if attrs.tooltip.as_ref().is_some_and(|text| {
                super::import_urls::parse(text, true).is_ok_and(|url| url.has_host)
            }) {
                errors.push(error(
                    &node,
                    "Tooltip cannot be set to URL when link is also set (for security)",
                ));
            }
        }
        errors
    }

    fn fields(&self, map: NodeId) -> Vec<NodeId> {
        match self.compiler.arena.node(map) {
            Node::Map(m) => m.fields.clone().unwrap_or_default(),
            _ => Vec::new(),
        }
    }
    fn shape(&self, id: usize, name: &[u8]) -> bool {
        equal_fold(&self.objects[id].shape, name)
    }
    fn sequence(&self, mut id: usize) -> Option<usize> {
        while let Some(parent) = self.objects[id].parent {
            if self.objects[parent].shape == b"sequence_diagram" {
                return Some(parent);
            }
            id = parent;
        }
        None
    }
    fn descendant(&self, mut id: usize, ancestor: usize) -> bool {
        loop {
            if id == ancestor {
                return true;
            }
            let Some(parent) = self.objects[id].parent else {
                return false;
            };
            id = parent;
        }
    }
    fn grid(&self, id: usize) -> bool {
        self.objects[id].grid_rows.is_some() || self.objects[id].grid_columns.is_some()
    }
    fn in_grid(&self, mut id: usize) -> bool {
        loop {
            if self.grid(id) {
                return true;
            }
            let Some(parent) = self.objects[id].parent else {
                return false;
            };
            id = parent;
        }
    }
    fn ensure(&mut self, mut parent: usize, name: &StringBox) -> usize {
        let bytes = name.scalar_bytes();
        if let Some(seq) = self.sequence(parent) {
            for &child in &self.objects[seq].children {
                if self.objects[child].id == bytes {
                    if self.objects[parent].id != bytes {
                        parent = seq;
                    }
                    break;
                }
            }
        }
        if name.unquoted_string.is_some() {
            if bytes == b"_" {
                return self.objects[parent]
                    .parent
                    .expect("IR resolved root underscore");
            }
            if bytes != b"style" && is_reserved_keyword(&name.scalar_string()) {
                return parent;
            }
        }
        let id = crate::format::format_key_path(std::slice::from_ref(name));
        let lower = go_lowercase(&id);
        if let Some(child) = self.objects[parent].children_by_id.get(&lower) {
            return *child;
        }
        let child = self.objects.len();
        self.objects.push(Object {
            parent: Some(parent),
            id,
            label: bytes,
            alive: true,
            ..Default::default()
        });
        self.objects[parent].children.push(child);
        self.objects[parent].children_by_id.insert(lower, child);
        child
    }
    fn label(&mut self, obj: usize, field: NodeId) {
        let Some(primary) = self.compiler.arena.primary(field) else {
            return;
        };
        let Node::Scalar(scalar) = self.compiler.arena.node(primary) else {
            return;
        };
        self.objects[obj].label = scalar.value.scalar_bytes();
        self.objects[obj].label_key = self.compiler.arena.last_primary_key(field);
        if let Some(block) = &scalar.value.block_string {
            self.objects[obj].language = graph::normalize_language(&block.read().tag);
        }
    }
    fn reserved(&mut self, obj: usize, field: NodeId, name: &str) {
        let Some(primary) = self.compiler.arena.primary(field) else {
            return;
        };
        let Node::Scalar(scalar) = self.compiler.arena.node(primary) else {
            return;
        };
        let bytes = scalar.value.scalar_bytes();
        match name {
            "label" => self.label(obj, field),
            "shape" => {
                let lower = go_lowercase(&bytes);
                if super::arrowhead_validation::valid_shape(&lower) {
                    self.objects[obj].shape = lower;
                    if self.shape(obj, b"code") {
                        self.objects[obj].language = "text".into();
                    }
                }
            }
            "near" => {
                if let Ok(mut key) = crate::parse_key(&bytes) {
                    let source = scalar.value.unbox().expect("near scalar AST");
                    key.range = source.range();
                    self.objects[obj].near = Some((source, key));
                }
            }
            "grid-rows" | "grid-columns" => {
                if let Ok(value) = scalar.value.scalar_string().parse::<i64>()
                    && (1..=MAX_GRID_DIMENSION).contains(&value)
                {
                    if name == "grid-rows" {
                        self.objects[obj].grid_rows = Some(value);
                        self.objects[obj].grid_rows_key =
                            self.compiler.arena.last_primary_key(field);
                    } else {
                        self.objects[obj].grid_columns = Some(value);
                        self.objects[obj].grid_columns_key =
                            self.compiler.arena.last_primary_key(field);
                    }
                }
            }
            "top" | "left" => {
                if scalar.value.scalar_string().parse::<i64>().is_ok() {
                    if name == "top" {
                        self.objects[obj].top = self.compiler.arena.last_primary_key(field)
                    } else {
                        self.objects[obj].left = self.compiler.arena.last_primary_key(field)
                    }
                }
            }
            _ => {}
        }
    }
    fn compile_map(&mut self, obj: usize, map: NodeId) {
        if let Some((_, names, _)) = graph::class_names(self.compiler, map) {
            for name in names {
                let class = self
                    .classes
                    .and_then(|catalog| {
                        // The immutable lowering session has already resolved
                        // these class names. Reuse its ordered field lookup;
                        // retain the byte-aware path for non-UTF8 names.
                        match std::str::from_utf8(&name) {
                            Ok(name) => self.compiler.graph_field(catalog, &[name]),
                            Err(_) => self
                                .compiler
                                .arena
                                .get_field(catalog, &[ast::raw_string_box(name, true)]),
                        }
                    })
                    .and_then(|field| self.compiler.arena.map(field));
                if let Some(class) = class
                    && self.active.insert(class)
                {
                    self.compile_map(obj, class);
                    self.active.remove(&class);
                }
            }
        }
        if let Some(shape) = self.compiler.graph_field(map, &["shape"])
            && matches!(self.compiler.arena.node(shape),Node::Field(field) if field.composite.is_none() && field.name.scalar_bytes()==b"shape")
        {
            self.reserved(obj, shape, "shape");
        }
        for field in self.fields(map) {
            let Node::Field(node) = self.compiler.arena.node(field) else {
                continue;
            };
            let name = node.name.clone();
            let text = name.scalar_string();
            let unquoted = name.unquoted_string.is_some();
            if unquoted && (text == "shape" || BOARD_KEYWORDS.contains(&text.as_str())) {
                continue;
            }
            let lower = unquoted.then(|| go_lowercase(text.as_bytes()));
            if unquoted
                && STYLE_KEYWORDS
                    .iter()
                    .any(|keyword| Some(keyword.as_bytes()) == lower.as_deref())
            {
                continue;
            }
            if unquoted
                && matches!(
                    text.as_str(),
                    "classes" | "vars" | "style" | "source-arrowhead" | "target-arrowhead"
                )
            {
                continue;
            }
            if unquoted
                && SIMPLE_RESERVED_KEYWORDS
                    .iter()
                    .any(|keyword| Some(keyword.as_bytes()) == lower.as_deref())
            {
                self.reserved(obj, field, &text);
                continue;
            }
            if self.objects[obj].parent.is_some_and(|parent| {
                self.shape(parent, b"class") || self.shape(parent, b"sql_table")
            }) {
                continue;
            }
            let child = self.ensure(obj, &name);
            self.label(child, field);
            if let Some(map) = self.compiler.arena.map(field) {
                self.compile_map(child, map)
            }
            if self.objects[child].label_key.is_none() {
                self.objects[child].label_key = self.compiler.arena.last_primary_key(field);
            }
            self.reference_scopes(field);
        }
        let is_class = self.classes.is_some_and(|catalog| {
            self.fields(catalog)
                .into_iter()
                .any(|field| self.compiler.arena.map(field) == Some(map))
        });
        if is_class {
            return;
        }
        // compileClass/compileSQLTable remove immediate row objects,not their
        // allocation or their parent pointers. Later lookups see no children.
        self.finish_fields(obj);
        let edges = match self.compiler.arena.node(map) {
            Node::Map(map) => map.edges.clone().unwrap_or_default(),
            _ => Vec::new(),
        };
        for edge in edges {
            if let Some((source, target)) = self.connect_nodes(obj, edge) {
                self.edges.push(Connection {
                    source,
                    target,
                    origin: edge,
                });
                self.reference_scopes(edge);
            }
        }
    }
    pub(super) fn connect_nodes(&mut self, obj: usize, edge: NodeId) -> Option<(usize, usize)> {
        let Node::Edge(node) = self.compiler.arena.node(edge) else {
            return None;
        };
        let Some(id) = &node.id else { return None };
        let id = id.read();
        let (Some(src), Some(dst)) = (&id.src_path, &id.dst_path) else {
            return None;
        };
        let strings = |path: &[Option<Child>]| -> Option<Vec<StringBox>> {
            path.iter()
                .map(|node| match node.as_ref()? {
                    Child::Unquoted(n) => Some(StringBox {
                        unquoted_string: Some(n.clone()),
                        ..Default::default()
                    }),
                    Child::DoubleQuoted(n) => Some(StringBox {
                        double_quoted_string: Some(n.clone()),
                        ..Default::default()
                    }),
                    Child::SingleQuoted(n) => Some(StringBox {
                        single_quoted_string: Some(n.clone()),
                        ..Default::default()
                    }),
                    _ => None,
                })
                .collect()
        };
        let (Some(src), Some(dst)) = (strings(src), strings(dst)) else {
            return None;
        };
        if src.iter().chain(&dst).any(|name| {
            name.unquoted_string.is_some() && is_reserved_keyword(&name.scalar_string())
        }) {
            return None;
        }
        Some((self.endpoint(obj, &src), self.endpoint(obj, &dst)))
    }
    fn endpoint(&mut self, mut obj: usize, path: &[StringBox]) -> usize {
        for name in path {
            if matches!(self.objects[obj].shape.as_slice(), b"class" | b"sql_table") {
                return obj;
            }
            obj = self.ensure(obj, name);
        }
        obj
    }
    fn parts(key: &ast::KeyPath) -> Vec<Vec<u8>> {
        crate::format::key_path(key).expect("parsed near components")
    }
    fn lookup(&self, parts: &[Vec<u8>]) -> Option<usize> {
        let mut obj = 0;
        for (i, part) in parts.iter().enumerate() {
            if i + 1 == parts.len()
                && part != b"style"
                && std::str::from_utf8(part)
                    .ok()
                    .is_some_and(is_reserved_keyword)
            {
                return Some(obj);
            }
            let lower = go_lowercase(part);
            obj = *self.objects[obj].children_by_id.get(&lower)?;
        }
        Some(obj)
    }
    fn constant(parts: &[Vec<u8>]) -> bool {
        parts
            .first()
            .is_some_and(|part| NEAR_CONSTANTS.iter().any(|value| part == value.as_bytes()))
    }
    fn constant_near(&self, obj: usize) -> bool {
        self.objects[obj].near.as_ref().is_some_and(|(_, key)| {
            let parts = Self::parts(key);
            self.lookup(&parts).is_none() && Self::constant(&parts)
        })
    }
    fn abs_id(&self, mut obj: usize) -> Vec<u8> {
        let mut parts = Vec::new();
        while self.objects[obj].parent.is_some() {
            parts.push(self.objects[obj].id.clone());
            obj = self.objects[obj].parent.unwrap();
        }
        parts.reverse();
        parts.join(&b'.')
    }
    fn labels(&self, errors: &mut Vec<ast::Error>) {
        for object in self.objects.iter().skip(1).filter(|object| object.alive) {
            let message = if equal_fold(&object.shape, b"text")
                && object.language.is_empty()
                && String::from_utf8_lossy(&object.label).trim().is_empty()
            {
                Some("shape text must have a non-empty label")
            } else if equal_fold(&object.shape, b"sql_table") && object.label.contains(&b'\n') {
                Some("shape sql_table cannot have newlines in label")
            } else {
                None
            };
            if let Some(message) = message {
                let key = object
                    .label_key
                    .clone()
                    .expect("compiled object label map key");
                errors.push(error(&Child::Key(key), message));
            }
        }
    }
    fn near(&self, errors: &mut Vec<ast::Error>) {
        for (obj, object) in self
            .objects
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(_, object)| object.alive)
        {
            let Some((source, key)) = &object.near else {
                continue;
            };
            let parts = Self::parts(key);
            let message = if let Some(target) = self.lookup(&parts) {
                if self.descendant(obj, target) {
                    Some("near keys cannot be set to an ancestor")
                } else if self.descendant(target, obj) {
                    Some("near keys cannot be set to an descendant")
                } else if self.sequence(target).is_some() {
                    Some("near keys cannot be set to an object within sequence diagrams")
                } else if self.objects[target]
                    .near
                    .as_ref()
                    .is_some_and(|(_, key)| Self::constant(&Self::parts(key)))
                {
                    Some("near keys cannot be set to an object with a constant near key")
                } else if self.in_grid(target) {
                    Some(
                        "near keys cannot be set to descendants of special objects, like grid cells",
                    )
                } else {
                    None
                }
            } else if Self::constant(&parts) {
                (object.parent != Some(0))
                    .then_some("constant near keys can only be set on root level shapes")
            } else {
                let bytes = crate::format::format(&Child::KeyPath(Shared::new(key.clone())));
                errors.push(error(source,format!("near key {} must be the absolute path to a shape or one of the following constants: {}",crate::parser::quote_input(&bytes),NEAR_CONSTANTS.join(", "))));
                None
            };
            if let Some(message) = message {
                errors.push(error(source, message));
            }
        }
        self.edge_checks(errors, true);
    }
    fn edge_boundaries(&self, errors: &mut Vec<ast::Error>) {
        self.edge_checks(errors, false);
    }
    fn edge_checks(&self, errors: &mut Vec<ast::Error>, near: bool) {
        for edge in &self.edges {
            let mut violation = None;
            for kind in if near {
                &["constant near"][..]
            } else {
                &["grid diagram", "grid cell", "sequence diagram"][..]
            } {
                for (owner, other) in [(edge.source, edge.target), (edge.target, edge.source)] {
                    let special = match *kind {
                        "constant near" => self.constant_near(owner),
                        "grid diagram" => self.grid(owner),
                        "grid cell" => self.objects[owner]
                            .parent
                            .is_some_and(|parent| self.grid(parent)),
                        "sequence diagram" => self.objects[owner].shape == b"sequence_diagram",
                        _ => unreachable!(),
                    };
                    if special && self.descendant(other, owner) {
                        violation = Some((*kind, owner));
                        break;
                    }
                }
                if violation.is_some() {
                    break;
                }
            }
            if let Some((kind, owner)) = violation {
                let Node::Edge(origin) = self.compiler.arena.node(edge.origin) else {
                    unreachable!()
                };
                let source = origin
                    .references
                    .as_ref()
                    .and_then(|refs| refs.first())
                    .and_then(|r| r.read().context.clone())
                    .and_then(|c| c.read().edge.clone())
                    .expect("compiled edge AST");
                errors.push(error(
                    &Child::Edge(source),
                    format!(
                        "edge from {kind} {} cannot enter itself",
                        crate::parser::quote_input(&self.abs_id(owner))
                    ),
                ));
            }
        }
    }
    fn positions(&self, errors: &mut Vec<ast::Error>) {
        for (id, object) in self
            .objects
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(_, object)| object.alive)
        {
            let Some(parent) = object.parent else {
                continue;
            };
            for key in [&object.top, &object.left].into_iter().flatten() {
                if self.shape(parent, b"hierarchy") {
                    errors.push(error(
                        &Child::Key(key.clone()),
                        "position keywords cannot be used with shape \"hierarchy\"",
                    ));
                }
                if self.sequence(id).is_some() {
                    errors.push(error(
                        &Child::Key(key.clone()),
                        "position keywords cannot be used inside shape \"sequence_diagram\"",
                    ));
                }
                if self.grid(parent) {
                    errors.push(error(
                        &Child::Key(key.clone()),
                        "position keywords cannot be used with grids",
                    ));
                }
            }
        }
    }
}
