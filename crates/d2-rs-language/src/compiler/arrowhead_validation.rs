// SPDX-License-Identifier: MPL-2.0
//! Graph-stage arrowhead, style and position validation from pinned d2compiler/compile.go.
use super::{
    Compiler,
    edge_creation::error,
    graph::{ClassApplication, class_applications_at_root, class_maps_at_root},
};
use d2_rs_core::{
    ast::{
        self,
        keywords::{SIMPLE_RESERVED_KEYWORDS, STYLE_KEYWORDS, is_reserved_keyword},
    },
    ir::{Node, NodeId, equal_fold, go_lowercase},
};
use std::collections::{HashMap, HashSet};

pub(super) fn validate_arrowhead_attributes(
    compiler: &Compiler,
    root: NodeId,
    retain_hierarchy: bool,
) -> (
    Vec<ast::Error>,
    Option<super::late_validation::HierarchyOrder>,
) {
    let (errors, hierarchy, _) = validate_with_copies(compiler, root, retain_hierarchy);
    (errors, hierarchy)
}

/// Reserve the complete graph compilation once, before CLI planning and
/// rendering create immutable views of its boards.
pub(super) fn validate_graph_materialization(
    compiler: &Compiler,
    root: NodeId,
) -> Result<(), super::GraphCompileError> {
    let result = validate_with_copies(compiler, root, true);
    compiler.graph_copy_check()?;
    compiler.graph_check()?;
    if !result.0.is_empty() {
        return Err(super::GraphCompileError::InvalidAttributes(result.0));
    }
    compiler.share_graph_validation();
    compiler
        .arrowhead_validation_memo
        .borrow_mut()
        .as_mut()
        .unwrap()
        .insert(root, result);
    Ok(())
}

type ValidationResult = (
    Vec<ast::Error>,
    Option<super::late_validation::HierarchyOrder>,
    Vec<NodeId>,
);

fn validate_with_copies(
    compiler: &Compiler,
    root: NodeId,
    retain_hierarchy: bool,
) -> ValidationResult {
    // Pinned Compile returns IR errors before entering its graph compiler.
    // The caller retains those diagnostics in compiler.errors.
    if !compiler.errors.is_empty() {
        return (Vec::new(), None, Vec::new());
    }
    let mut validation = ArrowheadValidation {
        compiler,
        errors: Vec::new(),
        states: Vec::new(),
        object: 0,
        style_errors: HashMap::new(),
        active_class_maps: HashSet::new(),
        charge_copies: !compiler.graph_copies_accounted.get(),
        copies: Vec::new(),
    };
    let hierarchy = validation.board(root, retain_hierarchy);
    compiler.graph_copies_accounted.set(true);
    let mut errors = Vec::new();
    for error in validation.errors {
        // Go compiler.errorf deduplicates diagnostics by complete AST
        // range and message, including reused class/import definitions.
        if !errors.contains(&error) {
            errors.push(error);
        }
    }
    (errors, hierarchy, validation.copies)
}

/// Validation results of one compilation's board roots, keyed by root. None
/// until the compilation's owner shares validation (`share_graph_validation`).
pub(super) type ValidationMemo = std::cell::RefCell<Option<HashMap<NodeId, ValidationResult>>>;

/// `validate_arrowhead_attributes` for a board root of an unprepared graph
/// view. Each view of one compilation forks the same IR, errors and class
/// root, and validation only reads them, so every lowering of that root sees
/// the same result. The memo always retains the hierarchy (which does not
/// change the diagnostics) so that planning and rendering share one pass.
pub(super) fn validate_arrowhead_attributes_memoized(
    compiler: &Compiler,
    root: NodeId,
    retain_hierarchy: bool,
    memo: &ValidationMemo,
) -> (
    Vec<ast::Error>,
    Option<super::late_validation::HierarchyOrder>,
) {
    let retained = |result: &ValidationResult| {
        (
            result.0.clone(),
            result.1.as_ref().filter(|_| retain_hierarchy).cloned(),
        )
    };
    let cached = memo
        .borrow()
        .as_ref()
        .map(|results| results.get(&root).cloned());
    match cached {
        None => validate_arrowhead_attributes(compiler, root, retain_hierarchy),
        Some(Some(result)) => {
            // A complete preflight already reserved every board's copies.
            // Otherwise replay the ordered work in this fresh lowering session.
            if !compiler.graph_copies_accounted.replace(true) {
                for copy in &result.2 {
                    if let Err(error) = compiler.reserve_graph_copy(*copy) {
                        compiler.fail_graph_copy(error);
                        break;
                    }
                }
            }
            retained(&result)
        }
        Some(None) => {
            let result = validate_with_copies(compiler, root, true);
            let returned = retained(&result);
            if compiler.graph_copy_check().is_ok()
                && compiler.graph_check().is_ok()
                && let Some(results) = memo.borrow_mut().as_mut()
            {
                results.insert(root, result);
            }
            returned
        }
    }
}

/// compileLegend calls compileMap, not compileBoard: construction diagnostics
/// apply, but validateKeys/labels/near/edges/positions and child boards do not.
pub(super) fn validate_legend_attributes(
    compiler: &Compiler,
    legend: NodeId,
    board_root: NodeId,
) -> Vec<ast::Error> {
    if !compiler.errors.is_empty() {
        return Vec::new();
    }
    let mut validation = ArrowheadValidation {
        compiler,
        errors: Vec::new(),
        states: vec![super::late_validation::Board::new(compiler, board_root)],
        object: 0,
        style_errors: HashMap::new(),
        active_class_maps: HashSet::new(),
        charge_copies: !compiler.graph_copies_accounted.get(),
        copies: Vec::new(),
    };
    validation.scope(legend, board_root);
    let mut errors = Vec::new();
    for error in validation.errors {
        if !errors.contains(&error) {
            errors.push(error);
        }
    }
    errors
}

// Go's strings.ToLower comparison against an ASCII keyword. Keep its Unicode
// mapping for non-ASCII input; the common ASCII path needs no allocation.
fn lower_equals_keyword(name: &str, keyword: &str) -> bool {
    if name.is_ascii() {
        name.eq_ignore_ascii_case(keyword)
    } else {
        go_lowercase(name.as_bytes()) == keyword.as_bytes()
    }
}

#[cfg(test)]
mod lower_keyword_tests {
    use super::lower_equals_keyword;

    #[test]
    fn matches_go_lowercase_without_using_unicode_case_folding() {
        assert!(lower_equals_keyword("STROKE", "stroke"));
        assert!(lower_equals_keyword("K", "k"));
        assert!(!lower_equals_keyword("ſ", "s"));
        assert!(!lower_equals_keyword("STROKES", "stroke"));
    }
}

struct ArrowheadValidation<'a> {
    compiler: &'a Compiler,
    errors: Vec<ast::Error>,
    states: Vec<super::late_validation::Board<'a>>,
    object: usize,
    charge_copies: bool,
    copies: Vec<NodeId>,
    // A style map's admission and scalar checks depend only on its immutable
    // IR fields, not the object or board applying it. Replay diagnostics at
    // each visit so their order and final range/message deduplication stay exact.
    style_errors: HashMap<NodeId, Vec<ast::Error>>,
    // Go's beginClass keeps a class active until its complete compileMap
    // traversal returns, including invalid child maps inside that class.
    active_class_maps: HashSet<NodeId>,
}

// Suspended compileMap visits retain their field cursor and the post-child
// reference update. The Go compiler uses its growable stack for these frames.
struct ScopeFrame {
    map: NodeId,
    fields: std::vec::IntoIter<NodeId>,
    shape: Option<String>,
    parent_shape: Option<String>,
    after_child: Option<(usize, NodeId)>,
    compile_edges: bool,
}

struct KeyFrame {
    map: NodeId,
    object: usize,
    fields: std::vec::IntoIter<NodeId>,
    shape: String,
    in_sequence: bool,
    width: Option<String>,
    height: Option<String>,
}

impl ArrowheadValidation<'_> {
    fn copy(&mut self, map: NodeId) -> bool {
        if !self.charge_copies {
            return true;
        }
        self.copies.push(map);
        match self.compiler.reserve_graph_copy(map) {
            Ok(()) => true,
            Err(error) => {
                self.compiler.fail_graph_copy(error);
                false
            }
        }
    }
    fn fields(&self, map: NodeId) -> Vec<NodeId> {
        match self.compiler.arena.node(map) {
            Node::Map(map) => map.fields.clone().unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    fn unquoted_name(&self, field: NodeId) -> Option<String> {
        let Node::Field(field) = self.compiler.arena.node(field) else {
            return None;
        };
        field.name.unquoted_string.as_ref()?;
        Some(field.name.scalar_string())
    }

    fn object_reserved(&self, field: NodeId) -> bool {
        self.unquoted_name(field).is_some_and(|name| {
            is_reserved_keyword(&name)
                || SIMPLE_RESERVED_KEYWORDS
                    .iter()
                    .chain(STYLE_KEYWORDS)
                    .any(|key| lower_equals_keyword(&name, key))
        })
    }

    fn board(
        &mut self,
        map: NodeId,
        retain_hierarchy: bool,
    ) -> Option<super::late_validation::HierarchyOrder> {
        if self.compiler.graph_copy_failed.get() {
            return None;
        }
        // d2compiler.compileBoard copies before class lookup and once more
        // after child boards for its CopyBase folder comparison.
        if (!self.compiler.arena.root(map) || self.compiler.has_graph_board_fields(map))
            && !self.copy(map)
        {
            return None;
        }
        self.states
            .push(super::late_validation::Board::new(self.compiler, map));
        let outer_object = std::mem::replace(&mut self.object, 0);
        self.scope(map, map);
        if self.compiler.graph_copy_failed.get() {
            self.states.pop();
            self.object = outer_object;
            return None;
        }
        self.errors
            .extend(self.states.last().unwrap().grid_dimensions());
        // Source compileBoard skips validateKeys if any preceding graph
        // compilation failed, including failures in an earlier board.
        if self.errors.is_empty() {
            self.validate_keys(map, map);
        }
        // Go validates and exports the same compileMap hierarchy. Only the
        // requested render root needs its order; planning and recursive
        // validation of child boards do not consume that projection.
        let (errors, hierarchy) =
            super::late_validation::validate(self.compiler, map, retain_hierarchy);
        self.errors.extend(errors);
        if let Some(legend) = self.field_map(map, &["vars", "d2-legend"]) {
            self.states
                .push(super::late_validation::Board::new(self.compiler, map));
            self.object = 0;
            self.scope(legend, map);
            self.states.pop();
            self.object = 0;
        }
        // compileBoard delays boards until after this board's graph and
        // legend, in wrapper order rather than declaration-position order.
        let mut has_children = false;
        let mut board_names = HashSet::new();
        for wrapper in ["layers", "scenarios", "steps"] {
            let Some(boards) = self.field_map(map, &[wrapper]) else {
                continue;
            };
            for field in self.fields(boards) {
                if let Some(error) =
                    super::boards::validate_board_name(self.compiler, field, &mut board_names)
                {
                    self.errors.push(error);
                    continue;
                }
                if let Some(board) = self.compiler.arena.map(field) {
                    has_children = true;
                    self.board(board, false);
                } else {
                    // Scalar boards compile as empty graphs too.
                    has_children = true;
                }
                self.markdown_label(field, false);
            }
        }
        if has_children {
            self.copy(map);
        }
        self.states.pop();
        self.object = outer_object;
        hierarchy
    }

    fn field_map(&self, map: NodeId, path: &[&str]) -> Option<NodeId> {
        self.compiler
            .graph_field(map, path)
            .and_then(|field| self.compiler.arena.map(field))
    }

    fn effective_field(&self, map: NodeId, board_root: NodeId, path: &[&str]) -> Option<NodeId> {
        // compileMap applies classes in order, then direct fields. These reads
        // have no diagnostics: stop at the last assignment with a primary.
        let primary_field = |scope| {
            self.compiler
                .graph_field(scope, path)
                .filter(|field| self.compiler.arena.primary(*field).is_some())
        };
        primary_field(map).or_else(|| {
            class_maps_at_root(self.compiler, map, board_root)
                .iter()
                .copied()
                .rev()
                .find_map(primary_field)
        })
    }

    fn field_text(&self, field: NodeId) -> Option<String> {
        self.compiler.arena.primary(field).and_then(|primary| {
            match self.compiler.arena.node(primary) {
                Node::Scalar(scalar) => Some(scalar.value.scalar_string()),
                _ => None,
            }
        })
    }

    fn key_error(&mut self, field: NodeId, message: impl Into<String>) {
        if let Some(key) = self.compiler.arena.last_primary_key(field) {
            self.errors.push(error(&ast::Child::Key(key), message));
        }
    }

    fn validate_keys(&mut self, map: NodeId, board_root: NodeId) {
        let mut frames = vec![self.key_frame(map, 0, false)];
        while let Some(frame) = frames.last_mut() {
            let Some(field) = frame.fields.next() else {
                frames.pop();
                continue;
            };
            let KeyFrame {
                map,
                object,
                shape,
                in_sequence,
                width,
                height,
                ..
            } = frame;
            let in_sequence = *in_sequence;
            let name = self.unquoted_name(field);
            if matches!(name.as_deref(), Some("layers" | "scenarios" | "steps")) {
                continue;
            }
            if name.as_deref().is_some_and(|name| {
                is_reserved_keyword(&String::from_utf8_lossy(&go_lowercase(name.as_bytes())))
            }) {
                self.validate_reserved_key(
                    field,
                    *map,
                    board_root,
                    shape,
                    (width, height),
                    name.as_deref().unwrap(),
                );
                continue;
            }
            if shape == "image" && !in_sequence {
                if let Some(node) = self.compiler.arena.last_ref(field).and_then(|r| r.ast()) {
                    self.errors
                        .push(error(&node, "image shapes cannot have children."));
                }
            } else if !matches!(shape.as_str(), "sql_table" | "class")
                && let Some(child) = self.compiler.arena.map(field)
                && let Some(child_object) =
                    self.states.last().unwrap().validation_child(*object, field)
            {
                frames.push(self.key_frame(child, child_object, in_sequence));
            }
        }
    }

    fn key_frame(&self, map: NodeId, object: usize, in_sequence: bool) -> KeyFrame {
        // validateKey reads the compiled object's attributes, including class
        // precedence and setDefaultShapes, after resolving HasChild.
        let (shape, width, height) = self.states.last().unwrap().validation_attributes(object);
        let in_sequence = in_sequence || shape == "sequence_diagram";
        let (width, height) = if matches!(shape.as_str(), "circle" | "square") {
            (
                width.and_then(|field| self.field_text(field)),
                height.and_then(|field| self.field_text(field)),
            )
        } else {
            (None, None)
        };
        KeyFrame {
            map,
            object,
            fields: self.fields(map).into_iter(),
            shape,
            in_sequence,
            width,
            height,
        }
    }

    fn validate_reserved_key(
        &mut self,
        field: NodeId,
        map: NodeId,
        board_root: NodeId,
        shape: &str,
        dimensions: (&Option<String>, &Option<String>),
        keyword: &str,
    ) {
        let (width, height) = dimensions;
        if matches!(shape, "circle" | "square")
            && matches!(
                String::from_utf8_lossy(&go_lowercase(keyword.as_bytes())).as_ref(),
                "width" | "height"
            )
            && width.is_some()
            && height.is_some()
            && width != height
        {
            self.key_error(
                field,
                format!("width and height must be equal for {shape} shapes"),
            );
        }
        match keyword {
            "shape" => {
                if shape == "image" && self.effective_field(map, board_root, &["icon"]).is_none() {
                    self.key_error(field, "image shape must include an \"icon\" field");
                }
                if matches!(
                    shape,
                    "none"
                        | "arrow"
                        | "triangle"
                        | "box"
                        | "cross"
                        | "cf-one"
                        | "cf-many"
                        | "cf-one-required"
                        | "cf-many-required"
                ) {
                    self.key_error(
                        field,
                        format!("invalid shape, can only set \"{shape}\" for arrowheads"),
                    );
                }
            }
            "constraint" if shape != "sql_table" => self.key_error(
                field,
                "\"constraint\" keyword can only be used in \"sql_table\" shapes",
            ),
            "style" => {
                if !matches!(shape, "square" | "rectangle" | "hexagon")
                    && let Some(style) = self.effective_field(map, board_root, &["style", "3d"])
                {
                    self.key_error(
                        style,
                        "key \"3d\" can only be applied to squares, rectangles, and hexagons",
                    );
                }
                if !matches!(shape, "" | "square" | "rectangle" | "circle" | "oval")
                    && let Some(style) =
                        self.effective_field(map, board_root, &["style", "double-border"])
                {
                    self.key_error(style, "key \"double-border\" can only be applied to squares, rectangles, circles, ovals");
                }
            }
            _ => {}
        }
    }

    fn class_applications(
        &mut self,
        node: NodeId,
        root: NodeId,
        edge: bool,
    ) -> Vec<ClassApplication> {
        let compiler = self.compiler;
        let events =
            class_applications_at_root(compiler, node, root, edge, &mut |map| self.copy(map));
        // compileMap/compileEdgeMap resolve the same ordered class maps that
        // later inherited-property reads need. Keep diagnostics in the event
        // stream; cache only its map projection in this immutable session.
        self.compiler.remember_class_maps(node, root, || {
            events
                .iter()
                .filter_map(|event| match event {
                    ClassApplication::Map { map, .. } => Some(*map),
                    ClassApplication::Error(_) => None,
                })
                .collect()
        });
        events
    }

    fn scope(&mut self, map: NodeId, board_root: NodeId) {
        self.scope_with_parent_mode(map, board_root, None, true, true);
    }

    fn scope_with_parent_mode(
        &mut self,
        map: NodeId,
        board_root: NodeId,
        parent_shape: Option<&str>,
        compile_edges: bool,
        apply_classes: bool,
    ) {
        let Some(frame) = self.scope_frame(
            map,
            board_root,
            parent_shape.map(str::to_owned),
            compile_edges,
            apply_classes,
        ) else {
            return;
        };
        let mut frames = vec![frame];
        while let Some(frame) = frames.last_mut() {
            if let Some((parent, field)) = frame.after_child.take() {
                self.object = parent;
                self.states.last_mut().unwrap().reference_scopes(field);
            }
            let Some(field) = frame.fields.next() else {
                let map = frame.map;
                let compile_edges = frame.compile_edges;
                frames.pop();
                if compile_edges {
                    self.scope_edges(map, board_root);
                }
                continue;
            };
            if self.compiler.graph_copy_failed.get() {
                frames.pop();
                continue;
            }
            if self.unquoted_name(field).as_deref() == Some("shape") {
                continue;
            }
            let reserved = self.object_reserved(field);
            // compileField handles reserved attributes before rejecting child
            // objects inside class fields or SQL columns.
            if !reserved && let Some(kind @ ("class" | "sql_table")) = frame.parent_shape.as_deref()
            {
                if let Some(node) = self.compiler.arena.last_ref(field).and_then(|r| r.ast()) {
                    let message = if kind == "class" {
                        "class fields cannot have children"
                    } else {
                        "sql_table columns cannot have children"
                    };
                    self.errors.push(error(&node, message));
                }
                continue;
            }
            self.object_style_field(field);
            if reserved {
                continue;
            }
            let parent = self.object;
            let child = self.states.last_mut().unwrap().child(parent, field);
            self.states.last_mut().unwrap().set_label(child, field);
            self.object = child;
            frame.after_child = Some((parent, field));
            if let Some(map) = self.compiler.arena.map(field) {
                let parent_shape = frame.shape.clone();
                if let Some(child) = self.scope_frame(map, board_root, parent_shape, true, true) {
                    frames.push(child);
                }
            }
        }
    }

    fn scope_frame(
        &mut self,
        map: NodeId,
        board_root: NodeId,
        parent_shape: Option<String>,
        compile_edges: bool,
        apply_classes: bool,
    ) -> Option<ScopeFrame> {
        if self.compiler.graph_copy_failed.get() {
            return None;
        }
        // Source compileMap applies classes before direct fields, even if
        // a later direct style overrides an invalid class value. Unused
        // definition maps are not live objects and are never validated here.
        if apply_classes {
            for event in self.class_applications(map, board_root, false) {
                match event {
                    ClassApplication::Map {
                        map: class,
                        field,
                        name,
                    } => {
                        self.object_class(class, field, &name, board_root, parent_shape.as_deref())
                    }
                    ClassApplication::Error(error) => self.errors.push(error),
                }
            }
        }
        self.object_shape(map);
        let shape = self.object_shape_value(map, board_root);
        Some(ScopeFrame {
            map,
            fields: self.fields(map).into_iter(),
            shape,
            parent_shape,
            after_child: None,
            compile_edges,
        })
    }

    fn scope_edges(&mut self, map: NodeId, board_root: NodeId) {
        // compileMap visits object fields recursively before the scope's
        // edge slice. Definition/configuration maps are not live objects.
        self.states.last_mut().unwrap().finish_fields(self.object);
        let edges = match self.compiler.arena.node(map) {
            Node::Map(map) => map.edges.clone().unwrap_or_default(),
            _ => Vec::new(),
        };
        for edge in edges {
            // d2graph.Object.Connect rejects reserved endpoint components
            // even when compileField has already reported another error.
            let invalid_endpoint = match self.compiler.arena.node(edge) {
                Node::Edge(node) => node.id.as_ref().is_some_and(|id| {
                    let id = id.read();
                    id.src_path
                        .iter()
                        .chain(id.dst_path.iter())
                        .flatten()
                        .any(|part| {
                            if let Some(ast::Child::Unquoted(value)) = part {
                                let name = ast::StringBox {
                                    unquoted_string: Some(value.clone()),
                                    ..Default::default()
                                };
                                is_reserved_keyword(&name.scalar_string())
                            } else {
                                false
                            }
                        })
                }),
                _ => false,
            };
            if invalid_endpoint {
                if let Node::Edge(node) = self.compiler.arena.node(edge)
                    && let Some(reference) = node.references.as_ref().and_then(|refs| refs.first())
                    && let Some(at) = reference.read().ast()
                {
                    self.errors
                        .push(error(&at, "cannot connect to reserved keyword"));
                }
                continue;
            }
            let parent = self.object;
            let _ = self.states.last_mut().unwrap().connect_nodes(parent, edge);
            self.object = self.states.last_mut().unwrap().edge_attributes(edge);
            self.markdown_label(edge, false);
            // Go compileBoard copies each board into an independent root
            // before its classes are looked up. Keep that lookup boundary
            // while validating the shared IR without mutating it.
            for event in self.class_applications(edge, board_root, true) {
                match event {
                    ClassApplication::Map { map: class, .. } => {
                        self.edge_map(class);
                    }
                    ClassApplication::Error(error) => self.errors.push(error),
                }
            }
            if let Some(map) = self.compiler.arena.map(edge) {
                self.edge_map(map);
            }
            self.states.last_mut().unwrap().reference_scopes(edge);
            self.object = parent;
        }
    }

    fn edge_map(&mut self, map: NodeId) {
        for field in self.fields(map) {
            // compileEdgeMap checks the exact unquoted reserved union before
            // compileEdgeField. Its diagnostic belongs to the first reference.
            if !self
                .unquoted_name(field)
                .is_some_and(|name| is_reserved_keyword(&name))
            {
                if let Node::Field(node) = self.compiler.arena.node(field)
                    && let Some(reference) = node.references.as_ref().and_then(|refs| refs.first())
                    && let Some(node) = reference.read().ast()
                {
                    self.errors
                        .push(error(&node, "edge map keys must be reserved keywords"));
                }
                continue;
            }
            if let Some(name) = self.unquoted_name(field)
                && STYLE_KEYWORDS.contains(&name.to_lowercase().as_str())
            {
                if let Some(node) = self.compiler.arena.last_ref(field).and_then(|r| r.ast()) {
                    self.errors
                        .push(error(&node, format!("{name} must be style.{name}")));
                }
                continue;
            }
            if self
                .unquoted_name(field)
                .is_some_and(|name| SIMPLE_RESERVED_KEYWORDS.contains(&name.as_str()))
                && self.reserved_value_boundary(field)
            {
                continue;
            }
            if self
                .unquoted_name(field)
                .is_some_and(|name| SIMPLE_RESERVED_KEYWORDS.contains(&name.as_str()))
            {
                self.compile_reserved(field);
                continue;
            }
            // compileEdgeField accepts a style holder by its scalar name.
            if let Node::Field(node) = self.compiler.arena.node(field)
                && node.name.scalar_string() == "style"
                && let Some(style) = self.compiler.arena.map(field)
            {
                self.style(style);
            }
            if self.unquoted_name(field).is_some_and(|name| {
                matches!(name.as_str(), "source-arrowhead" | "target-arrowhead")
            }) {
                let edge = self.object;
                let source = self.unquoted_name(field).as_deref() == Some("source-arrowhead");
                self.object = self
                    .states
                    .last_mut()
                    .unwrap()
                    .arrowhead_attributes(edge, field, source);
                self.markdown_label(field, false);
                if let Some(map) = self.compiler.arena.map(field) {
                    self.arrowhead_map(map);
                }
                self.object = edge;
            }
        }
    }

    fn arrowhead_map(&mut self, map: NodeId) {
        for field in self.fields(map) {
            // compileArrowheads case-folds simple-key admission, but style
            // admission is exact and every accepted key must be unquoted.
            let name = self.unquoted_name(field);
            let simple = name.as_ref().is_some_and(|name| {
                SIMPLE_RESERVED_KEYWORDS
                    .iter()
                    .any(|key| lower_equals_keyword(name, key))
            });
            if !simple && name.as_deref() != Some("style") {
                if let Some(node) = self.compiler.arena.last_ref(field).and_then(|r| r.ast()) {
                    self.errors.push(error(
                        &node,
                        "source-arrowhead/target-arrowhead map keys must be reserved keywords",
                    ));
                }
                continue;
            }
            if simple && self.reserved_value_boundary(field) {
                continue;
            }
            if simple {
                self.compile_reserved(field);
                continue;
            }
            if let Some("style") = name.as_deref() {
                let Some(style) = self.compiler.arena.map(field) else {
                    continue;
                };
                self.style(style);
            }
        }
    }

    fn compile_reserved(&mut self, field: NodeId) {
        if !self.reserved_scalar(field) {
            if self.compiler.arena.primary(field).is_none()
                && self
                    .unquoted_name(field)
                    .is_some_and(|name| matches!(name.as_str(), "label" | "icon" | "tooltip"))
            {
                self.position(field);
            }
            return;
        }
        let name = match self.compiler.arena.node(field) {
            Node::Field(node) => node.name.scalar_string(),
            _ => return,
        };
        if matches!(name.as_str(), "label" | "icon" | "tooltip") {
            self.position(field);
        }
        if name == "icon"
            && let Some(map) = self.compiler.arena.map(field)
        {
            for child in self.fields(map) {
                if self.unquoted_name(child).as_deref() == Some("style") {
                    if let Some(style) = self
                        .compiler
                        .arena
                        .map(child)
                        .filter(|map| !self.fields(*map).is_empty())
                    {
                        self.style(style);
                    } else {
                        if let Some(node) =
                            self.compiler.arena.last_ref(field).and_then(|r| r.ast())
                        {
                            self.errors.push(error(&node,"\"style\" expected to be set to a map of key-values, or contain an additional keyword like \"style.opacity: 0.4\""));
                        }
                        return;
                    }
                }
            }
        }
        self.errors
            .extend(self.states.last_mut().unwrap().security(self.object, field));
    }

    fn reserved_scalar(&mut self, field: NodeId) -> bool {
        let Node::Field(holder) = self.compiler.arena.node(field) else {
            return false;
        };
        let name = holder.name.scalar_string();
        if name == "label" || name == "tooltip" {
            self.markdown_label(field, name == "tooltip");
        }
        if name == "shape" {
            self.shape(field);
            return self.compiler.arena.primary(field).is_some_and(|primary|matches!(self.compiler.arena.node(primary),Node::Scalar(scalar) if valid_shape(&go_lowercase(&scalar.value.scalar_bytes()))));
        }
        let Some(primary) = self.compiler.arena.primary(field) else {
            return false;
        };
        let Node::Scalar(scalar) = self.compiler.arena.node(primary) else {
            return false;
        };
        let value = scalar.value.scalar_string();
        if name == "constraint"
            && (scalar.value.null.is_some()
                || scalar.value.suspension.is_some()
                || scalar.value.boolean.is_some()
                || scalar.value.number.is_some())
        {
            self.key_error(field, "constraint value must be a string");
            return false;
        }
        if name == "near" {
            if let Err(cause) = crate::parse_key(&scalar.value.scalar_bytes())
                && let Some(node) = scalar.value.unbox()
            {
                self.errors.push(error(
                    &node,
                    format!(
                        "bad near key {}: {cause}",
                        crate::parser::quote_input(&scalar.value.scalar_bytes())
                    ),
                ));
            }
            return crate::parse_key(&scalar.value.scalar_bytes()).is_ok();
        }
        let quoted = crate::parser::quote_input(&scalar.value.scalar_bytes());
        let message = if name == "icon" {
            super::import_urls::parse(&scalar.value.scalar_bytes(), false)
                .err()
                .map(|cause| format!("bad icon url {quoted}: {cause}"))
        } else if name == "link"
            && d2_rs_text::markdown::link_safety::is_dangerous_link(&scalar.value.scalar_bytes())
        {
            Some("link uses an unsafe URL scheme".to_owned())
        } else if matches!(
            name.as_str(),
            "width"
                | "height"
                | "top"
                | "left"
                | "grid-rows"
                | "grid-columns"
                | "grid-gap"
                | "vertical-gap"
                | "horizontal-gap"
        ) {
            match value.parse::<i64>() {
                Err(err) => {
                    let cause = match err.kind() {
                        std::num::IntErrorKind::PosOverflow
                        | std::num::IntErrorKind::NegOverflow => "value out of range",
                        _ => "invalid syntax",
                    };
                    Some(format!(
                        "non-integer {name} {quoted}: strconv.Atoi: parsing {quoted}: {cause}"
                    ))
                }
                Ok(number)
                    if matches!(name.as_str(), "grid-rows" | "grid-columns") && number <= 0 =>
                {
                    Some(format!("{name} must be a positive integer: {quoted}"))
                }
                Ok(number)
                    if matches!(name.as_str(), "grid-rows" | "grid-columns")
                        && number > super::late_validation::MAX_GRID_DIMENSION =>
                {
                    Some(format!(
                        "{name} {number} exceeds the maximum of {}",
                        super::late_validation::MAX_GRID_DIMENSION
                    ))
                }
                Ok(number) if !matches!(name.as_str(), "width" | "height") && number < 0 => {
                    Some(format!("{name} must be a non-negative integer: {quoted}"))
                }
                _ => None,
            }
        } else if name == "direction"
            && ![b"up".as_slice(), b"down", b"right", b"left"]
                .contains(&go_lowercase(value.as_bytes()).as_slice())
        {
            Some(format!(
                "direction must be one of up, down, right, left, got {quoted}"
            ))
        } else {
            None
        };
        let succeeds = message.is_none();
        if let Some(message) = message
            && let Some(node) = scalar.value.unbox()
        {
            self.errors.push(error(&node, message));
        }
        succeeds
    }

    // compileReserved's no-primary boundary precedes its scalar switch.
    // Class/constraint composites are accepted; position holders have their
    // own map validation. Object vars bypass compileReserved altogether.
    fn reserved_value_boundary(&mut self, field: NodeId) -> bool {
        let Node::Field(holder) = self.compiler.arena.node(field) else {
            return false;
        };
        if holder.primary.is_some() {
            return false;
        }
        let name = holder.name.scalar_string();
        if matches!(name.as_str(), "label" | "icon" | "tooltip") {
            return false;
        }
        if holder.composite.is_some() {
            if !matches!(name.as_str(), "class" | "constraint")
                && let Some(key) = self.compiler.arena.last_primary_key(field)
            {
                self.errors.push(error(
                    &ast::Child::Key(key),
                    format!("reserved field {name} does not accept composite"),
                ));
            }
        } else if let Some(node) = self.compiler.arena.last_ref(field).and_then(|r| r.ast()) {
            self.errors.push(error(
                &node,
                format!("reserved field \"{name}\" must have a value"),
            ));
        }
        true
    }

    fn class_definitions(&mut self, field: NodeId) {
        let Some(map) = self.compiler.arena.map(field) else {
            return;
        };
        if let Node::Map(node) = self.compiler.arena.node(map)
            && let Some(edge) = node.edges.as_ref().and_then(|edges| edges.first())
            && let Some(node) = self.compiler.arena.last_ref(*edge).and_then(|r| r.ast())
        {
            self.errors
                .push(error(&node, "classes cannot contain an edge"));
        }
        for class in self.fields(map) {
            if let Some(map) = self.compiler.arena.map(class) {
                for field in self.fields(map) {
                    let Node::Field(holder) = self.compiler.arena.node(field) else {
                        continue;
                    };
                    let name = holder.name.scalar_string();
                    // Source tests the outer classes holder's quotedness in
                    // its reserved-union guard, not the individual class key.
                    if !is_reserved_keyword(&name)
                        && let Some(node) =
                            self.compiler.arena.last_ref(field).and_then(|r| r.ast())
                    {
                        self.errors.push(error(
                            &node,
                            format!("{name} is an invalid class field, must be reserved keyword"),
                        ));
                    }
                    if self.unquoted_name(field).as_deref() == Some("class")
                        && let Some(node) =
                            self.compiler.arena.last_ref(field).and_then(|r| r.ast())
                    {
                        self.errors
                            .push(error(&node, "\"class\" cannot appear within \"classes\""));
                    }
                }
            }
        }
    }

    fn shape(&mut self, field: NodeId) {
        // compileReserved checks the scalar first. A shape scalar with a
        // composite still uses the scalar, unlike an ordinary node shape.
        if let Some(primary) = self.compiler.arena.primary(field)
            && let Node::Scalar(scalar) = self.compiler.arena.node(primary)
        {
            let value = go_lowercase(&scalar.value.scalar_bytes());
            if !valid_shape(&value)
                && let Some(node) = scalar.value.unbox()
            {
                let quoted = crate::parser::quote_input(&scalar.value.scalar_bytes());
                self.errors
                    .push(error(&node, format!("unknown shape {quoted}")));
            }
            return;
        }
        let Node::Field(field_node) = self.compiler.arena.node(field) else {
            return;
        };
        let (node, message) = if field_node.composite.is_some() {
            (
                self.compiler
                    .arena
                    .last_primary_key(field)
                    .map(ast::Child::Key),
                "reserved field shape does not accept composite",
            )
        } else {
            (
                self.compiler
                    .arena
                    .last_ref(field)
                    .and_then(|reference| reference.ast()),
                "reserved field \"shape\" must have a value",
            )
        };
        if let Some(node) = node {
            self.errors.push(error(&node, message));
        }
    }

    fn object_class(
        &mut self,
        map: NodeId,
        field: NodeId,
        name: &[u8],
        board_root: NodeId,
        parent_shape: Option<&str>,
    ) {
        if !self.active_class_maps.insert(map) {
            if let Some(node) = self.compiler.arena.last_ref(field).and_then(|r| r.ast()) {
                self.errors.push(error(
                    &node,
                    format!(
                        "class {} forms a reference cycle",
                        crate::parser::quote_input(name)
                    ),
                ));
            }
            return;
        }
        // A class map itself contributes no edges, but compileMap still
        // descends through invalid object children and compiles their maps.
        // `class_applications` already flattened the class map's own class
        // chain (and emitted its cycle diagnostic). Compile this root without
        // resolving that chain a second time; child object maps remain live
        // scopes and resolve their own classes while this class stays active.
        self.scope_with_parent_mode(map, board_root, parent_shape, false, false);
        self.active_class_maps.remove(&map);
    }

    fn object_shape_value(&self, map: NodeId, board_root: NodeId) -> Option<String> {
        class_maps_at_root(self.compiler, map, board_root)
            .iter()
            .copied()
            .chain([map])
            .filter_map(|scope| {
                self.fields(scope)
                    .into_iter()
                    .find(|field| self.unquoted_name(*field).as_deref() == Some("shape"))
                    .and_then(|field| self.compiler.arena.primary(field))
                    .and_then(|primary| match self.compiler.arena.node(primary) {
                        Node::Scalar(scalar) => Some(
                            String::from_utf8_lossy(&go_lowercase(&scalar.value.scalar_bytes()))
                                .into_owned(),
                        ),
                        _ => None,
                    })
            })
            .next_back()
    }

    fn object_shape(&mut self, map: NodeId) {
        // compileMap looks up the first unquoted shape with GetField's
        // EqualFold comparison. Its later field loop skips only literal
        // "shape", so capitalized spellings can produce a second diagnostic.
        if let Some(field) = self.compiler.graph_field(map, &["shape"]) {
            if let Node::Field(holder) = self.compiler.arena.node(field)
                && holder.composite.is_some()
            {
                if let Some(key) = self.compiler.arena.last_primary_key(field) {
                    self.errors.push(error(
                        &ast::Child::Key(key),
                        "reserved field shape does not accept composite",
                    ));
                }
            } else {
                self.object_style_field(field);
            }
        }
    }

    fn object_style_field(&mut self, field: NodeId) {
        let name = self.unquoted_name(field);
        if let Some(name) = &name
            && STYLE_KEYWORDS
                .iter()
                .any(|keyword| lower_equals_keyword(name, keyword))
        {
            if let Some(node) = self.compiler.arena.last_ref(field).and_then(|r| r.ast()) {
                self.errors
                    .push(error(&node, format!("{name} must be style.{name}")));
            }
            return;
        }
        if !self.object_reserved(field) {
            self.markdown_label(field, false);
        }
        if name.as_deref().is_some_and(|name| {
            name != "vars"
                && SIMPLE_RESERVED_KEYWORDS
                    .iter()
                    .any(|key| lower_equals_keyword(name, key))
        }) {
            if self.reserved_value_boundary(field) {
                return;
            }
            self.compile_reserved(field);
            return;
        }
        match self.unquoted_name(field).as_deref() {
            Some("classes") => self.class_definitions(field),
            Some("source-arrowhead" | "target-arrowhead") => {
                if let Some(node) = self.compiler.arena.last_ref(field).and_then(|r| r.ast()) {
                    let name = self.unquoted_name(field).unwrap();
                    let quoted = crate::parser::quote_input(name.as_bytes());
                    self.errors.push(error(
                        &node,
                        format!("{quoted} can only be used on connections"),
                    ));
                }
            }
            Some("style") => {
                if let Some(style) = self
                    .compiler
                    .arena
                    .map(field)
                    .filter(|map| !self.fields(*map).is_empty())
                {
                    self.style(style);
                } else if let Some(node) = self.compiler.arena.last_ref(field).and_then(|r| r.ast())
                {
                    self.errors.push(error(&node, "\"style\" expected to be set to a map of key-values, or contain an additional keyword like \"style.opacity: 0.4\""));
                }
            }
            _ => {}
        }
    }

    // compileLabel validates the raw block before caps-lock/text-transform;
    // compileReserved's tooltip always interprets its scalar as Markdown.
    fn markdown_label(&mut self, field: NodeId, tooltip: bool) {
        let Some(primary) = self.compiler.arena.primary(field) else {
            return;
        };
        let Node::Scalar(scalar) = self.compiler.arena.node(primary) else {
            return;
        };
        let raw = scalar.value.scalar_bytes();
        let block = scalar.value.block_string.as_ref();
        if !tooltip {
            let Some(block) = block else {
                return;
            };
            if String::from_utf8_lossy(&raw).trim().is_empty() {
                if let Some(key) = self.compiler.arena.last_primary_key(field) {
                    self.errors
                        .push(error(&ast::Child::Key(key), "block string cannot be empty"));
                }
            }
            if !matches!(block.read().tag.as_str(), "md" | "markdown") {
                return;
            }
        }
        let html = d2_rs_text::markdown::html::render_markdown_html_bytes(&raw);
        if let Err(message) = d2_rs_text::xml::validate_markdown_html_bytes(&html)
            && let Some(key) = self.compiler.arena.last_primary_key(field)
        {
            self.errors.push(error(
                &ast::Child::Key(key),
                format!("malformed Markdown: {message}"),
            ));
        }
    }

    // Pinned compileReserved/compilePosition: only an unquoted near child is
    // positional; implicit holder fields with no primary key do not invent
    // errors. Keep source map order and the final primary-key provenance.
    fn position(&mut self, field: NodeId) {
        let Node::Field(holder) = self.compiler.arena.node(field) else {
            return;
        };
        let name = holder.name.scalar_string();
        if holder.primary.is_none() && holder.composite.is_none() {
            if let Some(node) = self.compiler.arena.last_ref(field).and_then(|r| r.ast()) {
                self.errors.push(error(
                    &node,
                    format!("reserved field \"{name}\" must have a value"),
                ));
            }
            return;
        }
        let Some(map) = self.compiler.arena.map(field) else {
            return;
        };
        for child in self.fields(map) {
            if self.unquoted_name(child).as_deref() == Some("near") {
                let valid = self.compiler.arena.primary(child).is_some_and(|primary| {
                    let Node::Scalar(scalar) = self.compiler.arena.node(primary) else {
                        return false;
                    };
                    if matches!(scalar.value.unbox(), Some(ast::Child::Null(_))) {
                        return true;
                    }
                    let value = scalar.value.scalar_string();
                    if name == "tooltip" {
                        matches!(
                            value.as_str(),
                            "top-left"
                                | "top-center"
                                | "top-right"
                                | "center-left"
                                | "center-right"
                                | "bottom-left"
                                | "bottom-center"
                                | "bottom-right"
                        )
                    } else {
                        ast::keywords::label_position(&value).is_some()
                    }
                });
                if !valid {
                    if let Some(node) = self
                        .compiler
                        .arena
                        .last_primary_key(child)
                        .map(ast::Child::Key)
                    {
                        self.errors.push(error(&node, "invalid \"near\" field"));
                    }
                }
            } else if let Some(node) = self
                .compiler
                .arena
                .last_primary_key(child)
                .map(ast::Child::Key)
            {
                let Node::Field(child) = self.compiler.arena.node(child) else {
                    continue;
                };
                self.errors.push(error(
                    &node,
                    format!("unexpected field {}", child.name.scalar_string()),
                ));
            }
        }
        if matches!(self.compiler.arena.node(map),Node::Map(map) if !map.edges.as_ref().is_none_or(Vec::is_empty))
        {
            if let Some(node) = self
                .compiler
                .arena
                .last_primary_key(field)
                .map(ast::Child::Key)
            {
                self.errors.push(error(&node, "unexpected edges in map"));
            }
        }
    }

    fn style(&mut self, map: NodeId) {
        if let Some(errors) = self.style_errors.get(&map) {
            self.errors.extend_from_slice(errors);
            return;
        }
        let start = self.errors.len();
        self.style_uncached(map);
        self.style_errors.insert(map, self.errors[start..].to_vec());
    }

    fn style_uncached(&mut self, map: NodeId) {
        for field in self.fields(map) {
            let Node::Field(node) = self.compiler.arena.node(field) else {
                continue;
            };
            let name = node.name.scalar_string();
            let lower = String::from_utf8_lossy(&go_lowercase(name.as_bytes())).into_owned();
            // Source compileStyleField checks the keyword before primary
            // presence. Quoted names remain invalid even when recognized.
            if node.name.unquoted_string.is_none() || !STYLE_KEYWORDS.contains(&lower.as_str()) {
                if let Some(ast) = self.compiler.arena.last_ref(field).and_then(|r| r.ast()) {
                    self.errors.push(error(
                        &ast,
                        // Go compileStyleField uses a raw %s between literal quotes.
                        format!("invalid style keyword: \"{name}\""),
                    ));
                }
                continue;
            }
            let Some(primary) = self.compiler.arena.primary(field) else {
                continue;
            };
            let Node::Scalar(scalar) = self.compiler.arena.node(primary) else {
                continue;
            };
            let value = scalar.value.scalar_string();
            if !STYLE_KEYWORDS.contains(&name.as_str()) {
                if let Some(ast) = scalar.value.unbox() {
                    self.errors
                        .push(error(&ast, format!("unknown style key: {name}")));
                }
                continue;
            }
            // compileStyleFieldInit/Style.Apply switches on the exact key,
            // after the case-insensitive inventory check above.
            let message = match name.as_str() {
                "opacity" => d2_rs_core::parse_d2_float(&value)
                    .is_none_or(|number| !number.is_finite() || !(0.0..=1.0).contains(&number))
                    .then_some("expected \"opacity\" to be a number between 0.0 and 1.0"),
                "stroke-width" => invalid_integer(&value, 0, 15)
                    .then_some("expected \"stroke-width\" to be a number between 0 and 15"),
                "stroke-dash" => invalid_integer(&value, 0, 10)
                    .then_some("expected \"stroke-dash\" to be a number between 0 and 10"),
                "border-radius" => invalid_integer(&value, 0, i64::MAX)
                    .then_some("expected \"border-radius\" to be a number greater or equal to 0"),
                "font-size" => invalid_integer(&value, 8, 100)
                    .then_some("expected \"font-size\" to be a number between 8 and 100"),
                "fill-pattern" => {
                    let lower = go_lowercase(value.as_bytes());
                    (!["none", "dots", "lines", "grain", "paper"]
                        .iter()
                        .any(|allowed| lower == allowed.as_bytes()))
                    .then_some(
                        "expected \"fill-pattern\" to be one of: none, dots, lines, grain, paper",
                    )
                }
                "text-transform" => {
                    let lower = go_lowercase(value.as_bytes());
                    (!["none", "uppercase", "lowercase", "capitalize"]
                        .iter()
                        .any(|allowed| lower == allowed.as_bytes()))
                    .then_some("expected \"text-transform\" to be one of (none, uppercase, lowercase, capitalize)")
                }
                _ => None,
            };
            if matches!(name.as_str(), "stroke" | "fill" | "font-color")
                && !super::style_colors::valid_color(&value)
            {
                if let Some(ast) = scalar.value.unbox() {
                    self.errors.push(error(&ast, format!(
                        "expected \"{name}\" to be a valid named color (\"orange\"), a hex code (\"#f0ff3a\"), or a gradient (\"linear-gradient(red, blue)\")"
                    )));
                }
            } else if name == "font" {
                let lower = go_lowercase(value.as_bytes());
                if lower != b"default"
                    && lower != b"mono"
                    && let Some(ast) = scalar.value.unbox()
                {
                    self.errors.push(error(
                        &ast,
                        format!("\"{value}\" is not a valid font in our system"),
                    ));
                }
            } else if let Some(message) = message {
                if let Some(ast) = scalar.value.unbox() {
                    self.errors.push(error(&ast, message));
                }
            } else if matches!(
                name.as_str(),
                "shadow"
                    | "3d"
                    | "multiple"
                    | "animated"
                    | "bold"
                    | "italic"
                    | "underline"
                    | "filled"
                    | "double-border"
            ) && d2_rs_core::parse_d2_bool(&value).is_none()
                && let Some(ast) = scalar.value.unbox()
            {
                self.errors.push(error(
                    &ast,
                    format!("expected \"{name}\" to be true or false"),
                ));
            }
        }
    }
}

fn invalid_integer(value: &str, minimum: i64, maximum: i64) -> bool {
    // Go strconv.Atoi on the pinned 64-bit host uses signed decimal,
    // including an optional sign, with no whitespace/base prefixes/underscores.
    value
        .parse::<i64>()
        .map_or(true, |number| number < minimum || number > maximum)
}

pub(super) fn valid_shape(shape: &[u8]) -> bool {
    // compileReserved accepts the union of d2target.Shapes and Arrowheads.
    // In particular, empty/default and ordinary object shapes are valid
    // here; TALA's narrower adapter validation is a separate boundary.
    const SHAPES: &[&str] = &[
        "rectangle",
        "square",
        "page",
        "parallelogram",
        "document",
        "cylinder",
        "queue",
        "package",
        "step",
        "callout",
        "stored_data",
        "person",
        "c4-person",
        "diamond",
        "oval",
        "circle",
        "hexagon",
        "cloud",
        "text",
        "code",
        "class",
        "sql_table",
        "image",
        "sequence_diagram",
        "hierarchy",
    ];
    const ARROWHEADS: &[&str] = &[
        "none",
        "arrow",
        "triangle",
        "diamond",
        "circle",
        "box",
        "cross",
        "cf-one",
        "cf-many",
        "cf-one-required",
        "cf-many-required",
    ];
    // IsShape uses EqualFold, but the Arrowheads map lookup is exact after
    // strings.ToLower. In particular, long-s is valid in square, not cross.
    shape.is_empty()
        || SHAPES
            .iter()
            .any(|known| equal_fold(shape, known.as_bytes()))
        || ARROWHEADS.iter().any(|known| shape == known.as_bytes())
}

#[cfg(test)]
mod tests {
    use crate::{
        compiler::{GraphCompileError, compile_ir, lower_graph},
        parse,
    };
    use d2_rs_core::ast::{Error, Shared};

    fn diagnostics(source: &str) -> Vec<Error> {
        let parsed = parse("validation.d2", source.as_bytes(), None);
        assert!(parsed.error.is_none(), "{:?}", parsed.error);
        let (compiler, root) = compile_ir(Shared::new(parsed.map)).expect("translated IR");
        assert!(
            compiler.errors.is_empty(),
            "graph attributes must remain valid IR: {:?}",
            compiler.errors
        );
        match lower_graph(&compiler, root) {
            Ok(_) => Vec::new(),
            Err(GraphCompileError::InvalidAttributes(errors)) => errors,
            Err(error) => panic!("unexpected graph error: {error}"),
        }
    }

    #[test]
    fn dimension_validation_preserves_class_precedence_and_error_order() {
        // Pinned compile.go:1516 only compares circle/square dimensions.
        // Complete diagnostics verified against the pinned Go CLI.
        let cases: &[(&str, &[&str])] = &[
            ("a: {shape: rectangle; width: 80; height: 90}\n", &[]),
            (
                "a: {shape: circle; width: 80; height: 90}\n",
                &[
                    "validation.d2:1:20: width and height must be equal for circle shapes",
                    "validation.d2:1:31: width and height must be equal for circle shapes",
                ],
            ),
            (
                "classes: {size: {width: 80; height: 90}}\na: {class: size; shape: square; width: 90}\n",
                &[],
            ),
            (
                "classes: {first: {width: 80; height: 90}; last: {width: 90}}\na: {class: [first; last]; shape: square; height: 90}\n",
                &[],
            ),
            (
                "classes: {size: {width: 80; height: 90}}\na: {class: size; shape: circle; width}\n",
                &["validation.d2:2:33: reserved field \"width\" must have a value"],
            ),
            (
                "a: {shape: rectangle; width: x; height: 90}\nb -> c: {target-arrowhead.shape: bad}\n",
                &[
                    "validation.d2:1:30: non-integer width \"x\": strconv.Atoi: parsing \"x\": invalid syntax",
                    "validation.d2:2:34: unknown shape \"bad\"",
                ],
            ),
            ("a: {shape: circle; \"width\": 80; height: 90}\n", &[]),
            // compile.go:1559 passes ScalarString to HasChild; d2graph's
            // EnsureChild stores FormatKeyPath IDs instead. These names
            // therefore skip recursive validateKeys, as the pinned CLI does.
            (r"a\nb: {shape: circle; width: 80; height: 90}", &[]),
            (r#""a.b": {shape: circle; width: 80; height: 90}"#, &[]),
            (r"a: {b\nc: {shape: circle; width: 80; height: 90}}", &[]),
            (r#""width": {shape: circle; width: 80; height: 90}"#, &[]),
            ("a\\nb.width: 80\na\\nb.height: 90\n*.shape: circle\n", &[]),
            (
                "classes: {size: {width: 80; height: 90}}\nlayers: {child: {classes: {size: {width: 90; height: 90}}; a: {class: size; shape: circle; width: 90}}}\n",
                &[],
            ),
        ];
        for (source, expected) in cases {
            assert_eq!(
                diagnostics(source)
                    .iter()
                    .map(|error| error.message.as_str())
                    .collect::<Vec<_>>(),
                *expected,
                "{source}"
            );
        }
    }

    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn applied_color_scope_classes_icons_legends_and_boards_match_original_go() {
        let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/style-color-scope-go-v090.json"
        ))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 36);
        for case in cases {
            let actual = diagnostics(case["source"].as_str().unwrap());
            let expected: Vec<_> = case["errors"]
                .as_array()
                .unwrap()
                .iter()
                .map(|error| error.as_str().unwrap())
                .collect();
            assert_eq!(
                actual
                    .iter()
                    .map(|error| error.message.as_str())
                    .collect::<Vec<_>>(),
                expected,
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn applied_color_values_and_diagnostic_order_match_original_go() {
        let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/style-color-diagnostics-go-master.json"
        ))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 88);
        for case in cases {
            let actual = diagnostics(case["source"].as_str().unwrap());
            let expected: Vec<_> = case["errors"]
                .as_array()
                .unwrap()
                .iter()
                .map(|error| error.as_str().unwrap())
                .collect();
            assert_eq!(
                actual
                    .iter()
                    .map(|error| error.message.as_str())
                    .collect::<Vec<_>>(),
                expected,
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn applied_opacity_syntax_ranges_and_locations_match_original_go() {
        let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/style-opacity-go-v090.json"
        ))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 27);
        for case in cases {
            let actual = diagnostics(case["source"].as_str().unwrap());
            let expected: Vec<_> = case["errors"]
                .as_array()
                .unwrap()
                .iter()
                .map(|error| error.as_str().unwrap())
                .collect();
            assert_eq!(
                actual
                    .iter()
                    .map(|error| error.message.as_str())
                    .collect::<Vec<_>>(),
                expected,
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn applied_style_enumerations_match_original_go_diagnostics() {
        let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/style-enumerations-go-v090.json"
        ))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 30);
        for case in cases {
            let actual = diagnostics(case["source"].as_str().unwrap());
            let expected: Vec<_> = case["errors"]
                .as_array()
                .unwrap()
                .iter()
                .map(|error| error.as_str().unwrap())
                .collect();
            assert_eq!(
                actual
                    .iter()
                    .map(|error| error.message.as_str())
                    .collect::<Vec<_>>(),
                expected,
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn invalid_arrowhead_scalars_report_original_values_and_token_ranges() {
        let source = "a <-> b: {\n  source-arrowhead.shape: LiNe\n  target-arrowhead: {\n    shape: diamond\n    style.filled: \"tRuE\"\n  }\n}\n";
        let errors = diagnostics(source);
        assert_eq!(errors.len(), 2);
        assert_eq!(
            errors[0].message,
            "validation.d2:2:27: unknown shape \"LiNe\""
        );
        assert_eq!(
            errors[1].message,
            "validation.d2:5:19: expected \"filled\" to be true or false"
        );
        assert_eq!(
            (
                errors[0].range.start.line,
                errors[0].range.start.column,
                errors[0].range.end.line,
                errors[0].range.end.column
            ),
            (1, 26, 1, 30)
        );
        assert_eq!(
            (
                errors[1].range.start.line,
                errors[1].range.start.column,
                errors[1].range.end.line,
                errors[1].range.end.column
            ),
            (4, 18, 4, 24)
        );
        for value in ["2", "yes", "\"\""] {
            let source = format!("a -> b: {{target-arrowhead.style.filled: {value}}}\n");
            let errors = diagnostics(&source);
            assert_eq!(errors.len(), 1);
            assert_eq!(
                errors[0].message,
                "validation.d2:1:41: expected \"filled\" to be true or false"
            );
        }
    }

    #[test]
    fn arrowhead_shape_requires_a_value_and_uses_the_go_error_location() {
        let missing = diagnostics("a -> b: {target-arrowhead.shape}\n");
        assert_eq!(missing.len(), 1);
        assert_eq!(
            missing[0].message,
            "validation.d2:1:27: reserved field \"shape\" must have a value"
        );
        assert_eq!(
            (missing[0].range.start.column, missing[0].range.end.column),
            (26, 31)
        );
        for composite in ["{}", "[]"] {
            let source = format!("a -> b: {{target-arrowhead.shape: {composite}}}\n");
            let errors = diagnostics(&source);
            assert_eq!(errors.len(), 1);
            assert_eq!(
                errors[0].message,
                "validation.d2:1:10: reserved field shape does not accept composite"
            );
            assert_eq!(errors[0].range.start.column, 9);
        }
    }

    #[test]
    fn arrowhead_errors_keep_graph_field_class_and_board_traversal_order() {
        // These orders come from frozen official v0.9.0 stderr captures,
        // not source-position sorting or hardcoded source/target order.
        let cases = [
            (
                "a -> b: {target-arrowhead.shape: line}\nbox: {c -> d: {target-arrowhead.style.filled: yes}}\n",
                vec![
                    "validation.d2:2:47: expected \"filled\" to be true or false",
                    "validation.d2:1:34: unknown shape \"line\"",
                ],
            ),
            (
                "a -> b: {target-arrowhead.style.filled: yes; source-arrowhead.shape: LiNe}\n",
                vec![
                    "validation.d2:1:41: expected \"filled\" to be true or false",
                    "validation.d2:1:70: unknown shape \"LiNe\"",
                ],
            ),
            (
                "layers: {first: {a -> b: {target-arrowhead.shape: line}}}\na -> b: {target-arrowhead.style.filled: yes}\n",
                vec![
                    "validation.d2:2:41: expected \"filled\" to be true or false",
                    "validation.d2:1:51: unknown shape \"line\"",
                ],
            ),
            (
                "classes: {bad: {target-arrowhead.shape: line}}\na -> b: {class: bad; target-arrowhead.shape: triangle}\nc -> d: {class: bad}\n",
                vec!["validation.d2:1:41: unknown shape \"line\""],
            ),
            (
                "classes: {bad: {target-arrowhead.shape: triangle}}\nlayers: {child: {classes: {bad: {target-arrowhead.shape: line}}; a -> b: {class: bad}}}\n",
                vec!["validation.d2:2:58: unknown shape \"line\""],
            ),
        ];
        for (source, expected) in cases {
            assert_eq!(
                diagnostics(source)
                    .iter()
                    .map(|error| error.message.as_str())
                    .collect::<Vec<_>>(),
                expected,
                "{source}"
            );
        }
    }

    #[test]
    fn applied_classes_compile_invalid_child_maps_and_keep_cycles_bounded() {
        // Pinned d2compiler.compileMap descends into non-reserved class
        // fields when the class is applied, even though the class catalog
        // separately rejects those fields. beginClass remains active across
        // that complete child traversal.
        let cases = [
            (
                "a.class: empty\nclasses: {empty: {st'layersyle: {fill: t}}}\n",
                vec![
                    "validation.d2:2:34: fill must be style.fill",
                    "validation.d2:2:19: st'layersyle is an invalid class field, must be reserved keyword",
                ],
            ),
            (
                "classes: {x: {foo: {fill: red; bar: {stroke: blue}}}}\na.class: x\n",
                vec![
                    "validation.d2:1:15: foo is an invalid class field, must be reserved keyword",
                    "validation.d2:1:21: fill must be style.fill",
                    "validation.d2:1:38: stroke must be style.stroke",
                ],
            ),
            (
                "classes: {x: {foo: {class: x}}}\na.class: x\n",
                vec![
                    "validation.d2:1:15: foo is an invalid class field, must be reserved keyword",
                    "validation.d2:1:21: class \"x\" forms a reference cycle",
                ],
            ),
        ];
        for (source, expected) in cases {
            assert_eq!(
                diagnostics(source)
                    .iter()
                    .map(|error| error.message.as_str())
                    .collect::<Vec<_>>(),
                expected,
                "{source}"
            );
        }
    }

    #[test]
    fn accepted_empty_shapes_boolean_scalars_and_unused_classes_remain_valid() {
        for source in [
            "a <-> b: {source-arrowhead.shape: \"\"; target-arrowhead.shape: rectangle}\n",
            "a -> b: {target-arrowhead.shape: diamond; target-arrowhead.style.filled: tRuE}\n",
            "a -> b: {target-arrowhead.shape: triangle; target-arrowhead.style.filled: fAlSe}\n",
            "a -> b: {target-arrowhead.shape: diamond; target-arrowhead.style.filled}\n",
            "a -> b: {target-arrowhead.shape: diamond; target-arrowhead.style.filled: {}}\n",
            "a -> b: {target-arrowhead.shape: triangle {unused: value}}\n",
            "classes: {unused: {target-arrowhead.shape: line}}\na -> b\n",
            "a -> b: {target-arrowhead.shape: line}\n(a -> b)[0].target-arrowhead.shape: triangle\n",
            "a -> b: {target-arrowhead.shape: line}\n(a -> b)[0]: null\n",
        ] {
            assert!(
                diagnostics(source).is_empty(),
                "{source}: {:?}",
                diagnostics(source)
            );
        }
    }

    #[test]
    fn unicode_shapes_use_go_lowercase_and_shape_specific_case_folding() {
        for source in [
            "a -> b: {target-arrowhead.shape: cİrcle}\n",
            "a -> b: {target-arrowhead.shape: trİangle}\n",
            "a -> b: {target-arrowhead.shape: ſquare}\n",
        ] {
            assert!(diagnostics(source).is_empty(), "{source}");
        }
        let errors = diagnostics("a -> b: {target-arrowhead.shape: croſſ}\n");
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].message,
            "validation.d2:1:34: unknown shape \"croſſ\""
        );
    }

    #[test]
    fn duplicate_board_names_report_before_visiting_the_duplicate_attributes() {
        let source = "layers: {same: {a -> b}}\nscenarios: {same: {a -> b: {target-arrowhead.shape: line}}}\n";
        let errors = diagnostics(source);
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].message,
            "validation.d2:2:13: board name same already used by another board"
        );
        assert_eq!(
            (
                errors[0].range.start.line,
                errors[0].range.start.column,
                errors[0].range.end.line,
                errors[0].range.end.column
            ),
            (1, 12, 1, 16)
        );

        let source = "a -> b: {target-arrowhead.shape: line}\nsteps: {same: {c -> d: {target-arrowhead.shape: line}}}\nlayers: {same: {e -> f: {target-arrowhead.style.filled: yes}}}\nscenarios: {same: {g -> h: {target-arrowhead.shape: mystery}}}\n";
        assert_eq!(
            diagnostics(source)
                .iter()
                .map(|error| error.message.as_str())
                .collect::<Vec<_>>(),
            [
                "validation.d2:1:34: unknown shape \"line\"",
                "validation.d2:3:57: expected \"filled\" to be true or false",
                "validation.d2:4:13: board name same already used by another board",
                "validation.d2:2:9: board name same already used by another board",
            ]
        );
    }

    #[test]
    fn triple_glob_spread_into_duplicate_boards_reports_master_board_name_error() {
        // v0.9.0 d2ir panicked on this input; master's triple-glob nil-name
        // guard compiles it, then reports the duplicate board (master CLI:
        // "1:74: board name one already used by another board").
        let errors = diagnostics(
            "***.style.fill: red; vars: {x: {a; b}}; layers.one: {...${x}}; scenarios.one: {...${x}}",
        );
        assert_eq!(
            errors
                .iter()
                .map(|error| error.message.as_str())
                .collect::<Vec<_>>(),
            ["validation.d2:1:74: board name one already used by another board"]
        );
    }

    #[test]
    fn board_names_are_case_sensitive_and_scoped_to_the_parent_board() {
        let source = "layers: {same: {layers: {same: {a -> b}}}}\nscenarios: {Same: {c -> d}}\n";
        assert!(diagnostics(source).is_empty());
    }

    #[test]
    fn ir_failures_precede_graph_arrowhead_validation() {
        let source = "a.shape.foo: bar\na -> b: {target-arrowhead.shape: line}\n";
        let parsed = parse("validation.d2", source.as_bytes(), None);
        let (compiler, root) = compile_ir(Shared::new(parsed.map)).expect("translated IR");
        let errors = &compiler.errors;
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].message,
            "validation.d2:1:3: \"shape\" must be the last part of the key"
        );
        assert!(
            super::validate_arrowhead_attributes(&compiler, root, false)
                .0
                .is_empty()
        );
    }

    #[test]
    fn graph_diagnostics_display_the_complete_ordered_error_list() {
        let source = "a -> b: {target-arrowhead.style.filled: yes; source-arrowhead.shape: LiNe}\n";
        let parsed = parse("validation.d2", source.as_bytes(), None);
        let (compiler, root) = compile_ir(Shared::new(parsed.map)).expect("translated IR");
        assert!(compiler.errors.is_empty());
        let error = lower_graph(&compiler, root).expect_err("invalid graph attributes");
        assert_eq!(
            error.to_string(),
            "validation.d2:1:41: expected \"filled\" to be true or false\nvalidation.d2:1:70: unknown shape \"LiNe\""
        );
    }

    #[test]
    fn integer_style_diagnostics_match_source_scalar_ranges() {
        let cases: &[(&str, &[&str])] = &[
            (
                "a: {style.border-radius: 8.25}\n",
                &[
                    "validation.d2:1:26: expected \"border-radius\" to be a number greater or equal to 0",
                ],
            ),
            (
                "a: {style.border-radius: -1}\n",
                &[
                    "validation.d2:1:26: expected \"border-radius\" to be a number greater or equal to 0",
                ],
            ),
            (
                "a: {style.border-radius: 9223372036854775808}\n",
                &[
                    "validation.d2:1:26: expected \"border-radius\" to be a number greater or equal to 0",
                ],
            ),
            (
                "a: {style.border-radius: 1e1}\n",
                &[
                    "validation.d2:1:26: expected \"border-radius\" to be a number greater or equal to 0",
                ],
            ),
            (
                "a: {style.border-radius: \"0x8\"}\n",
                &[
                    "validation.d2:1:26: expected \"border-radius\" to be a number greater or equal to 0",
                ],
            ),
            (
                "a: {style.border-radius: \"8_0\"}\n",
                &[
                    "validation.d2:1:26: expected \"border-radius\" to be a number greater or equal to 0",
                ],
            ),
            (
                "a: {style.border-radius: \" 8\"}\n",
                &[
                    "validation.d2:1:26: expected \"border-radius\" to be a number greater or equal to 0",
                ],
            ),
            ("a: {style.border-radius: \"+8\"}\n", &[]),
            ("a: {style.border-radius: \"08\"}\n", &[]),
            ("a: {style.border-radius: 0}\n", &[]),
            ("a: {style.border-radius: 8}\n", &[]),
            (
                "a: {style.stroke-width: 1.5}\n",
                &["validation.d2:1:25: expected \"stroke-width\" to be a number between 0 and 15"],
            ),
            (
                "a: {style.stroke-width: -1}\n",
                &["validation.d2:1:25: expected \"stroke-width\" to be a number between 0 and 15"],
            ),
            (
                "a: {style.stroke-width: 16}\n",
                &["validation.d2:1:25: expected \"stroke-width\" to be a number between 0 and 15"],
            ),
            ("a: {style.stroke-width: 15}\n", &[]),
            ("a: {style.stroke-width: 0}\n", &[]),
            ("a: {style.stroke-width: \"+2\"}\n", &[]),
            (
                "a: {style.stroke-width: \"2_0\"}\n",
                &["validation.d2:1:25: expected \"stroke-width\" to be a number between 0 and 15"],
            ),
            (
                "a: {style.stroke-dash: 2.5}\n",
                &["validation.d2:1:24: expected \"stroke-dash\" to be a number between 0 and 10"],
            ),
            (
                "a: {style.stroke-dash: -1}\n",
                &["validation.d2:1:24: expected \"stroke-dash\" to be a number between 0 and 10"],
            ),
            (
                "a: {style.stroke-dash: 11}\n",
                &["validation.d2:1:24: expected \"stroke-dash\" to be a number between 0 and 10"],
            ),
            ("a: {style.stroke-dash: 10}\n", &[]),
            ("a: {style.stroke-dash: 0}\n", &[]),
            ("a: {style.stroke-dash: \"+2\"}\n", &[]),
            (
                "a: {style.font-size: 8.5}\n",
                &["validation.d2:1:22: expected \"font-size\" to be a number between 8 and 100"],
            ),
            (
                "a: {style.font-size: 7}\n",
                &["validation.d2:1:22: expected \"font-size\" to be a number between 8 and 100"],
            ),
            (
                "a: {style.font-size: 101}\n",
                &["validation.d2:1:22: expected \"font-size\" to be a number between 8 and 100"],
            ),
            ("a: {style.font-size: 8}\n", &[]),
            ("a: {style.font-size: 100}\n", &[]),
        ];
        for (source, expected) in cases {
            let actual = diagnostics(source);
            assert_eq!(
                actual
                    .iter()
                    .map(|error| error.message.as_str())
                    .collect::<Vec<_>>(),
                *expected,
                "{source}"
            );
        }
    }

    #[test]
    fn invalid_style_keyword_preserves_raw_scalar_text() {
        // d2compiler/compile.go compileStyleField formats %s, not %q.
        for (source, name) in [
            ("a: {style: {bad\u{202e}: true}}\n", "bad\u{202e}"),
            ("a: {style: {\"bad\\\"name\": true}}\n", "bad\"name"),
            ("a: {style: {\"bad\\nname\": true}}\n", "bad\nname"),
        ] {
            let actual = diagnostics(source);
            assert_eq!(actual.len(), 1, "{source}");
            assert_eq!(
                actual[0].message,
                format!("validation.d2:1:13: invalid style keyword: \"{name}\"")
            );
        }
    }

    #[test]
    fn applied_style_scope_and_error_order_match_source() {
        let cases: &[(&str, &[&str])] = &[
            (
                "a -> b: {style.border-radius: 8.25}\n",
                &[
                    "validation.d2:1:31: expected \"border-radius\" to be a number greater or equal to 0",
                ],
            ),
            (
                "a -> b: {target-arrowhead.style.stroke-width: 1.5}\n",
                &["validation.d2:1:47: expected \"stroke-width\" to be a number between 0 and 15"],
            ),
            (
                "style.font-size: 7\na\n",
                &["validation.d2:1:18: expected \"font-size\" to be a number between 8 and 100"],
            ),
            (
                "box: {a: {style.stroke-dash: 11}}\n",
                &["validation.d2:1:30: expected \"stroke-dash\" to be a number between 0 and 10"],
            ),
            (
                "classes: {bad: {style.border-radius: 8.25}}\na: {class: bad; style.border-radius: 8}\n",
                &[
                    "validation.d2:1:38: expected \"border-radius\" to be a number greater or equal to 0",
                ],
            ),
            (
                "classes: {bad: {style.stroke-dash: 11}}\na -> b: {class: bad; style.stroke-dash: 2}\n",
                &["validation.d2:1:36: expected \"stroke-dash\" to be a number between 0 and 10"],
            ),
            (
                "classes: {bad: {style.font-size: 7}}\na: {class: bad}\nb: {class: bad}\n",
                &["validation.d2:1:34: expected \"font-size\" to be a number between 8 and 100"],
            ),
            (
                "layers: {child: {a: {style.border-radius: 8.25}}}\na\n",
                &[
                    "validation.d2:1:43: expected \"border-radius\" to be a number greater or equal to 0",
                ],
            ),
            (
                "style.stroke-width: 16\na -> b: {target-arrowhead.shape: line; style.stroke-dash: 11}\nbox: {a: {style.font-size: 7}}\n",
                &[
                    "validation.d2:1:21: expected \"stroke-width\" to be a number between 0 and 15",
                    "validation.d2:3:28: expected \"font-size\" to be a number between 8 and 100",
                    "validation.d2:2:34: unknown shape \"line\"",
                    "validation.d2:2:59: expected \"stroke-dash\" to be a number between 0 and 10",
                ],
            ),
            (
                "a: {style: {\"border-radius\": 8.25}}\n",
                &["validation.d2:1:13: invalid style keyword: \"border-radius\""],
            ),
            (
                "a: {icon: https://example.com/icon.svg {style.border-radius: 8.25}}\n",
                &[
                    "validation.d2:1:62: expected \"border-radius\" to be a number greater or equal to 0",
                ],
            ),
            (
                "vars: {d2-legend: {a: {style.font-size: 7}}}\na\n",
                &["validation.d2:1:41: expected \"font-size\" to be a number between 8 and 100"],
            ),
            (
                "a: {style: {bogus}}\n",
                &["validation.d2:1:13: invalid style keyword: \"bogus\""],
            ),
            (
                "classes: {good: {style.font-size: 8}; bad: {style.font-size: 7}}\na: {class: [good; bad]; style.font-size: 8}\n",
                &["validation.d2:1:62: expected \"font-size\" to be a number between 8 and 100"],
            ),
        ];
        for (source, expected) in cases {
            let actual = diagnostics(source);
            assert_eq!(
                actual
                    .iter()
                    .map(|error| error.message.as_str())
                    .collect::<Vec<_>>(),
                *expected,
                "{source}"
            );
        }
    }

    #[test]
    fn inactive_overwritten_and_case_sensitive_styles_match_source() {
        let cases: &[(&str, &[&str])] = &[
            ("classes: {bad: {style.border-radius: 8.25}}\na\n", &[]),
            (
                "classes: {bad: {style.font-size: 7}}\nlayers: {child: {classes: {bad: {style.font-size: 8}}; a: {class: bad}}}\n",
                &[],
            ),
            ("a: {style.border-radius}\n", &[]),
            ("a: {style.border-radius: {}}\n", &[]),
            (
                "a: {style.border-radius: 8.25}\na.style.border-radius: 8\n",
                &[],
            ),
            ("a: {style.border-radius: 8.25}\na: null\n", &[]),
            ("vars: {unused: {style.border-radius: 8.25}}\na\n", &[]),
            (
                "a -> b: {style.border-radius: 8.25}\n(a -> b)[0]: null\n",
                &[],
            ),
            (
                "a -> b: {style.border-radius: 8.25}\n(a -> b)[0].style.border-radius: 8\n",
                &[],
            ),
            (
                "a: {style.Border-Radius: 8.25}\n",
                &["validation.d2:1:26: unknown style key: Border-Radius"],
            ),
        ];
        for (source, expected) in cases {
            let actual = diagnostics(source);
            assert_eq!(
                actual
                    .iter()
                    .map(|error| error.message.as_str())
                    .collect::<Vec<_>>(),
                *expected,
                "{source}"
            );
        }
    }

    #[test]
    fn style_booleans_retain_go_scalar_spelling() {
        let cases: &[(&str, &[&str])] = &[
            (
                "a: {style.shadow: yes}\n",
                &["validation.d2:1:19: expected \"shadow\" to be true or false"],
            ),
            (
                "a: {style.shadow: \"tRuE\"}\n",
                &["validation.d2:1:19: expected \"shadow\" to be true or false"],
            ),
            ("a: {style.shadow: true}\n", &[]),
            ("a: {style.shadow: \"TRUE\"}\n", &[]),
            (
                "a: {style.3d: yes}\n",
                &["validation.d2:1:15: expected \"3d\" to be true or false"],
            ),
            (
                "a: {style.3d: \"tRuE\"}\n",
                &["validation.d2:1:15: expected \"3d\" to be true or false"],
            ),
            ("a: {style.3d: true}\n", &[]),
            ("a: {style.3d: \"TRUE\"}\n", &[]),
            (
                "a: {style.multiple: yes}\n",
                &["validation.d2:1:21: expected \"multiple\" to be true or false"],
            ),
            (
                "a: {style.multiple: \"tRuE\"}\n",
                &["validation.d2:1:21: expected \"multiple\" to be true or false"],
            ),
            ("a: {style.multiple: true}\n", &[]),
            ("a: {style.multiple: \"TRUE\"}\n", &[]),
            (
                "a: {style.animated: yes}\n",
                &["validation.d2:1:21: expected \"animated\" to be true or false"],
            ),
            (
                "a: {style.animated: \"tRuE\"}\n",
                &["validation.d2:1:21: expected \"animated\" to be true or false"],
            ),
            ("a: {style.animated: true}\n", &[]),
            ("a: {style.animated: \"TRUE\"}\n", &[]),
            (
                "a: {style.bold: yes}\n",
                &["validation.d2:1:17: expected \"bold\" to be true or false"],
            ),
            (
                "a: {style.bold: \"tRuE\"}\n",
                &["validation.d2:1:17: expected \"bold\" to be true or false"],
            ),
            ("a: {style.bold: true}\n", &[]),
            ("a: {style.bold: \"TRUE\"}\n", &[]),
            (
                "a: {style.italic: yes}\n",
                &["validation.d2:1:19: expected \"italic\" to be true or false"],
            ),
            (
                "a: {style.italic: \"tRuE\"}\n",
                &["validation.d2:1:19: expected \"italic\" to be true or false"],
            ),
            ("a: {style.italic: true}\n", &[]),
            ("a: {style.italic: \"TRUE\"}\n", &[]),
            (
                "a: {style.underline: yes}\n",
                &["validation.d2:1:22: expected \"underline\" to be true or false"],
            ),
            (
                "a: {style.underline: \"tRuE\"}\n",
                &["validation.d2:1:22: expected \"underline\" to be true or false"],
            ),
            ("a: {style.underline: true}\n", &[]),
            ("a: {style.underline: \"TRUE\"}\n", &[]),
            (
                "a: {style.filled: yes}\n",
                &["validation.d2:1:19: expected \"filled\" to be true or false"],
            ),
            (
                "a: {style.filled: \"tRuE\"}\n",
                &["validation.d2:1:19: expected \"filled\" to be true or false"],
            ),
            ("a: {style.filled: true}\n", &[]),
            ("a: {style.filled: \"TRUE\"}\n", &[]),
            (
                "a: {style.double-border: yes}\n",
                &["validation.d2:1:26: expected \"double-border\" to be true or false"],
            ),
            (
                "a: {style.double-border: \"tRuE\"}\n",
                &["validation.d2:1:26: expected \"double-border\" to be true or false"],
            ),
            ("a: {style.double-border: true}\n", &[]),
            ("a: {style.double-border: \"TRUE\"}\n", &[]),
        ];
        for (source, expected) in cases {
            let actual = diagnostics(source);
            assert_eq!(
                actual
                    .iter()
                    .map(|error| error.message.as_str())
                    .collect::<Vec<_>>(),
                *expected,
                "{source}"
            );
        }
    }
}

#[cfg(test)]
#[cfg(feature = "oracle-fixtures")]
mod position_source_contract {
    use crate::{
        compiler::{compile_ir, lower_graph},
        parse,
    };
    use d2_rs_core::ast::Shared;
    #[test]
    fn complete_position_diagnostics_match_pristine_source() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/position-validation-source-v090.json"
        ))
        .unwrap();
        let mut differences = Vec::new();
        for row in rows.as_array().unwrap() {
            let source = row["source"].as_str().unwrap();
            let parsed = parse(
                "validation.d2",
                source.as_bytes(),
                Some(crate::ParseOptions {
                    utf16_positions: row["utf16"].as_bool().unwrap_or(false),
                    ..Default::default()
                }),
            );
            let actual = if let Some(error) = parsed.error {
                error.to_string()
            } else {
                let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
                if !compiler.errors.is_empty() {
                    compiler
                        .errors
                        .iter()
                        .map(|e| e.message.clone())
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    lower_graph(&compiler, root)
                        .err()
                        .map(|e| e.to_string())
                        .unwrap_or_default()
                }
            };
            let expected = row["error"].as_str().unwrap();
            if actual != expected {
                differences.push((
                    row["name"].as_str().unwrap().to_owned(),
                    actual,
                    expected.to_owned(),
                ));
            }
        }
        assert!(
            differences.is_empty(),
            "complete Source diagnostic differences: {differences:#?}"
        );
    }
}

#[cfg(test)]
#[cfg(feature = "oracle-fixtures")]
mod edge_key_source_contract {
    use crate::{
        compiler::{compile_ir, lower_graph},
        parse,
    };
    use d2_rs_core::ast::Shared;
    #[test]
    fn complete_edge_key_diagnostics_match_pristine_source() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/edge-key-validation-source-master.json"
        ))
        .unwrap();
        let mut differences = Vec::new();
        for row in rows.as_array().unwrap() {
            let source = row["source"].as_str().unwrap();
            let parsed = parse(
                "validation.d2",
                source.as_bytes(),
                Some(crate::ParseOptions {
                    utf16_positions: row["utf16"].as_bool().unwrap_or(false),
                    ..Default::default()
                }),
            );
            let actual = if let Some(error) = parsed.error {
                error.to_string()
            } else {
                let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
                if !compiler.errors.is_empty() {
                    compiler
                        .errors
                        .iter()
                        .map(|e| e.message.clone())
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    lower_graph(&compiler, root)
                        .err()
                        .map(|e| e.to_string())
                        .unwrap_or_default()
                }
            };
            let expected = row["error"].as_str().unwrap();
            if actual != expected {
                differences.push((
                    row["name"].as_str().unwrap().to_owned(),
                    actual,
                    expected.to_owned(),
                ));
            }
        }
        assert!(
            differences.is_empty(),
            "complete Source diagnostic differences: {differences:#?}"
        );
    }
}

#[cfg(test)]
#[cfg(feature = "oracle-fixtures")]
mod markdown_compiler_source_contract {
    use crate::{
        compiler::{GraphCompileError, compile_ir, lower_graph},
        parse,
    };
    use d2_rs_core::ast::Shared;
    #[test]
    fn original_go_markdown_compilation_diagnostics_and_order() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/markdown-compiler-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 38);
        for row in rows.as_array().unwrap() {
            let parsed = parse(
                "validation.d2",
                row["source"].as_str().unwrap().as_bytes(),
                None,
            );
            assert!(parsed.error.is_none(), "{} {:?}", row["name"], parsed.error);
            let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
            let errors = if !compiler.errors.is_empty() {
                compiler.errors.clone()
            } else {
                match lower_graph(&compiler, root) {
                    Ok(_) => Vec::new(),
                    Err(GraphCompileError::InvalidAttributes(errors)) => errors,
                    Err(error) => panic!("{} {error}", row["name"]),
                }
            };
            let got = errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            assert_eq!(got, row["error"].as_str().unwrap(), "{}", row["name"]);
        }
    }
}
