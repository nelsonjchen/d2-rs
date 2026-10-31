// SPDX-License-Identifier: MPL-2.0
//! d2renderers/d2isometric: scene.go (BuildScene), hierarchy.go, layout.go
//! and sequence.go. Floating-point contraction follows the pinned ARM64
//! build (`.reference/isometric-port/fma-sites.txt`); products by 0.5 are
//! exact and written plainly.

use std::collections::{BTreeMap, HashMap};

use d2_rs_render::go_float::{go_fmadd, go_fmsub, gomath};
use serde::Serialize;

use crate::target::{
    Connection, Diagram, Legend, Shape, Text, ThemeOverrides, SHAPE_CYLINDER, SHAPE_PERSON,
    SHAPE_QUEUE, SHAPE_SEQUENCE_DIAGRAM,
};
use crate::theme::{is_theme_color, Theme};

pub const MAX_NODES: usize = 2000;
pub const MAX_EDGES: usize = 5000;
pub const MAX_ID_BYTES: usize = 8192;
pub const MAX_ID_DEPTH: usize = 128;
const MAX_ENTRIES: usize = 100_000;
const MAX_BYTES: usize = 8 << 20;
const COORDINATE_LIMIT: f64 = 1e9;

/// layout.go SceneScale: compiled layout pixels to world-space units.
pub const SCENE_SCALE: f64 = 0.01;
pub const SURFACE_HEIGHT: f64 = 0.08;

const LIFELINE_Z_INDEX: i64 = 1;
const SPAN_Z_INDEX: i64 = 2;
const GROUP_Z_INDEX: i64 = 3;
const MESSAGE_Z_INDEX: i64 = 4;
const NOTE_Z_INDEX: i64 = 5;

#[derive(Clone, Debug, Default, Serialize)]
pub struct RenderOpts {
    pub theme_id: Option<i64>,
    pub theme_overrides: Option<ThemeOverrides>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Board {
    pub id: String,
    #[serde(rename = "sourceId", skip_serializing_if = "String::is_empty")]
    pub source_id: String,
    pub label: String,
    #[serde(rename = "parentId", skip_serializing_if = "String::is_empty")]
    pub parent_id: String,
    pub kind: String,
    pub level: i64,
    pub position: Vec3,
    pub size: Vec3,
    #[serde(rename = "nodeIds")]
    pub node_ids: Vec<String>,
    #[serde(rename = "headerDepth")]
    pub header_depth: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct NodeMetadata {
    pub original: Shape,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Node {
    pub id: String,
    pub label: String,
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(rename = "boardId")]
    pub board_id: String,
    #[serde(rename = "parentId", skip_serializing_if = "String::is_empty")]
    pub parent_id: String,
    pub container: bool,
    #[serde(rename = "sequenceRole", skip_serializing_if = "String::is_empty")]
    pub sequence_role: String,
    pub position: Vec3,
    pub size: Vec3,
    pub fill: String,
    #[serde(rename = "fillExplicit")]
    pub fill_explicit: bool,
    pub stroke: String,
    #[serde(rename = "strokeExplicit")]
    pub stroke_explicit: bool,
    #[serde(rename = "fontColor")]
    pub font_color: String,
    #[serde(rename = "fontExplicit")]
    pub font_explicit: bool,
    pub opacity: f64,
    #[serde(rename = "strokeWidth")]
    pub stroke_width: i64,
    #[serde(rename = "strokeDash")]
    pub stroke_dash: f64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub tooltip: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub link: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub icon: String,
    pub metadata: NodeMetadata,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct EdgeMetadata {
    pub original: Connection,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Edge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub label: String,
    #[serde(rename = "sourceLabel", skip_serializing_if = "Option::is_none")]
    pub source_label: Option<Text>,
    #[serde(rename = "targetLabel", skip_serializing_if = "Option::is_none")]
    pub target_label: Option<Text>,
    #[serde(rename = "sourceArrow")]
    pub source_arrow: String,
    #[serde(rename = "targetArrow")]
    pub target_arrow: String,
    pub stroke: String,
    #[serde(rename = "strokeExplicit")]
    pub stroke_explicit: bool,
    #[serde(rename = "fontColor")]
    pub font_color: String,
    #[serde(rename = "fontExplicit")]
    pub font_explicit: bool,
    #[serde(rename = "strokeWidth")]
    pub stroke_width: i64,
    #[serde(rename = "strokeDash")]
    pub stroke_dash: f64,
    pub opacity: f64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub link: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub tooltip: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub icon: String,
    pub points: Option<Vec<Vec3>>,
    #[serde(rename = "sequenceRole", skip_serializing_if = "String::is_empty")]
    pub sequence_role: String,
    pub metadata: EdgeMetadata,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Scene {
    #[serde(rename = "fontFamily", skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(rename = "monoFontFamily", skip_serializing_if = "Option::is_none")]
    pub mono_font_family: Option<String>,
    #[serde(rename = "themeId")]
    pub theme_id: i64,
    #[serde(rename = "themeOverrides", skip_serializing_if = "Option::is_none")]
    pub theme_overrides: Option<ThemeOverrides>,
    #[serde(rename = "hasSequence", skip_serializing_if = "std::ops::Not::not")]
    pub has_sequence: bool,
    #[serde(rename = "pixelScale")]
    pub pixel_scale: f64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub background: String,
    pub boards: Vec<Board>,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub root: Shape,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legend: Option<Legend>,
    /// The resolved theme; not part of Go's Scene JSON.
    #[serde(skip)]
    pub theme: Option<Theme>,
}

mod admission;
use admission::Budget;

/// d2parser.ParseKey + ScalarString of every path component.
fn key_path(id: &str) -> Result<Vec<String>, String> {
    let key = d2_rs_language::parse_key(id.as_bytes()).map_err(|error| error.to_string())?;
    Ok(key.string_ida().unwrap_or_default())
}

/// BuildScene maps a single D2 board. IDs, text, endpoint direction, and
/// original shape/edge metadata are retained.
pub fn build_scene(diagram: &Diagram, opts: Option<&RenderOpts>) -> Result<Scene, String> {
    let shapes = diagram.shapes();
    let connections = diagram.connections();
    if shapes.len() > MAX_NODES || connections.len() > MAX_EDGES {
        return Err(format!(
            "isometric: scene exceeds {MAX_NODES} nodes or {MAX_EDGES} edges"
        ));
    }
    let default_opts = RenderOpts::default();
    let opts = opts.unwrap_or(&default_opts);
    let mut admission = Budget::default();
    admission.check(&diagram.name)?;
    admission.check(&diagram.description)?;
    admission.shape(&diagram.root)?;
    admission.legend(&diagram.legend)?;
    admission.shapes(shapes)?;
    admission.connections(connections)?;
    admission.check(opts)?;
    for s in shapes {
        if s.width < 0
            || s.height < 0
            || s.text.label_width < 0
            || s.text.label_height < 0
            || s.stroke_width < 0
            || s.opacity < 0.0
            || s.opacity > 1.0
        {
            return Err(format!(
                "isometric: node {:?} has invalid dimensions or opacity",
                s.id
            ));
        }
    }
    for c in connections {
        if c.stroke_width < 0 || c.opacity < 0.0 || c.opacity > 1.0 {
            return Err(format!("isometric: edge {:?} has invalid stroke or opacity", c.id));
        }
        if c.route().iter().any(Option::is_none) {
            return Err(format!("isometric: edge {:?} has nil route point", c.id));
        }
    }
    let mut theme_id = 0;
    let mut overrides = None;
    if let Some(config) = &diagram.config {
        if let Some(id) = config.theme_id {
            theme_id = id;
        }
        overrides = config.theme_overrides.clone();
    }
    if let Some(id) = opts.theme_id {
        theme_id = id;
    }
    if opts.theme_overrides.is_some() {
        overrides = opts.theme_overrides.clone();
    }
    admission.check(&overrides)?;
    let mut theme =
        Theme::find(theme_id).ok_or_else(|| format!("isometric: unknown theme {theme_id}"))?;
    theme.apply_overrides(overrides.as_ref());
    let has_overrides = overrides.is_some();
    let mut scene = Scene {
        pixel_scale: SCENE_SCALE,
        description: diagram.description.clone(),
        background: theme.resolve(&diagram.root.fill),
        root: diagram.root.clone(),
        legend: diagram.legend.clone(),
        theme_id,
        theme_overrides: overrides,
        font_family: diagram.font_family.clone(),
        mono_font_family: diagram.mono_font_family.clone(),
        ..Default::default()
    };
    if scene.background.is_empty() {
        scene.background = "#f5f7fa".into();
    }
    let mut indices: HashMap<String, usize> = HashMap::new();
    let mut keys: BTreeMap<Vec<String>, usize> = BTreeMap::new();
    let mut paths = Vec::with_capacity(shapes.len());
    for (i, s) in shapes.iter().enumerate() {
        if s.id.len() > MAX_ID_BYTES {
            return Err(format!("isometric: node ID exceeds {MAX_ID_BYTES} bytes"));
        }
        if s.id.is_empty() {
            return Err(format!("isometric: node {i} has empty ID"));
        }
        if indices.contains_key(&s.id) {
            return Err(format!("isometric: duplicate node ID {:?}", s.id));
        }
        let path = key_path(&s.id).map_err(|error| format!("isometric: node ID: {error}"))?;
        if path.len() > MAX_ID_DEPTH {
            return Err(format!(
                "isometric: node ID nesting exceeds {MAX_ID_DEPTH} components"
            ));
        }
        if keys.contains_key(&path) {
            return Err(format!("isometric: equivalent node ID {:?}", s.id));
        }
        indices.insert(s.id.clone(), i);
        keys.insert(path.clone(), i);
        paths.push(path);
        let explicit = |value: &str| has_overrides || (!value.is_empty() && !is_theme_color(value));
        scene.nodes.push(Node {
            id: s.id.clone(),
            label: s.text.label.clone(),
            type_: s.type_.clone(),
            fill: theme.resolve(&s.fill),
            stroke: theme.resolve(&s.stroke),
            font_color: theme.resolve(&s.font_color()),
            opacity: s.opacity,
            stroke_width: s.stroke_width,
            stroke_dash: s.stroke_dash,
            tooltip: s.tooltip.clone(),
            link: s.link.clone(),
            fill_explicit: explicit(&s.fill),
            stroke_explicit: explicit(&s.stroke),
            font_explicit: explicit(&s.text.color),
            icon: s.icon.as_ref().map(|icon| icon.string()).unwrap_or_default(),
            metadata: NodeMetadata { original: s.clone() },
            ..Default::default()
        });
    }
    let mut parents = vec![None; scene.nodes.len()];
    for (i, path) in paths.iter().enumerate() {
        for depth in (1..path.len()).rev() {
            if let Some(&parent) = keys.get(&path[..depth]) {
                parents[i] = Some(parent);
                scene.nodes[i].parent_id = scene.nodes[parent].id.clone();
                scene.nodes[parent].container = true;
                break;
            }
        }
    }
    let mut edge_ids = std::collections::HashSet::new();
    for c in connections {
        if c.id.is_empty() || !edge_ids.insert(c.id.clone()) {
            return Err(format!("isometric: empty or duplicate edge ID {:?}", c.id));
        }
        let explicit = |value: &str| has_overrides || (!value.is_empty() && !is_theme_color(value));
        scene.edges.push(Edge {
            id: c.id.clone(),
            source: c.src.clone(),
            target: c.dst.clone(),
            label: c.text.label.clone(),
            source_label: c.src_label.clone(),
            target_label: c.dst_label.clone(),
            source_arrow: c.src_arrow.clone(),
            target_arrow: c.dst_arrow.clone(),
            stroke: theme.resolve(&c.stroke),
            font_color: theme.resolve(&c.text.color),
            stroke_width: c.stroke_width,
            stroke_dash: c.stroke_dash,
            opacity: c.opacity,
            link: c.link.clone(),
            tooltip: c.tooltip.clone(),
            stroke_explicit: explicit(&c.stroke),
            font_explicit: explicit(&c.text.color),
            icon: c.icon.as_ref().map(|icon| icon.string()).unwrap_or_default(),
            metadata: EdgeMetadata { original: c.clone() },
            ..Default::default()
        });
        if (!indices.contains_key(&c.src) || !indices.contains_key(&c.dst)) && c.route().len() < 2
        {
            return Err(format!(
                "isometric: edge {:?} has an unknown endpoint and no usable original route",
                c.id
            ));
        }
    }
    classify_sequence(&mut scene, &indices, &parents);
    compose_hierarchy(&mut scene, &indices, &parents)?;
    if diagram.has_children() {
        scene.warnings.push("This scene contains the selected D2 board. The export pipeline renders other layers, scenarios and steps separately.".into());
    }
    scene.theme = Some(theme);
    Ok(scene)
}

/// sourceOrder: stable sort by source position, then ID.
pub fn source_order(nodes: &[Node], ids: &mut [usize]) {
    ids.sort_by(|&i, &j| {
        let (a, b) = (&nodes[i].metadata.original, &nodes[j].metadata.original);
        a.pos
            .y
            .cmp(&b.pos.y)
            .then(a.pos.x.cmp(&b.pos.x))
            .then_with(|| a.id.cmp(&b.id))
    });
}

// ------------------------------------------------------------- layout.go

pub fn module_size(source: &Shape) -> Vec3 {
    let height = match source.type_.as_str() {
        SHAPE_CYLINDER => 1.15,
        SHAPE_QUEUE => 0.85,
        SHAPE_PERSON => 1.35,
        _ => 0.7,
    };
    Vec3::new(
        source.width.max(1) as f64 * SCENE_SCALE,
        height,
        source.height.max(1) as f64 * SCENE_SCALE,
    )
}

pub fn board_label_metrics(
    board: &Board,
    nodes: &[Node],
    index: &HashMap<String, usize>,
) -> (f64, f64) {
    let mut font_size = 16;
    let (mut label_width, mut label_height) = (0, 0);
    if !board.source_id.is_empty() {
        if let Some(&i) = index.get(&board.source_id) {
            let s = &nodes[i].metadata.original;
            label_width = s.text.label_width;
            label_height = s.text.label_height;
            if s.text.font_size > 0 {
                font_size = s.text.font_size;
            }
        }
    }
    let lines: Vec<&str> = board.label.split('\n').collect();
    if label_width <= 0 {
        for line in &lines {
            let runes = line.chars().count() as i64;
            label_width = label_width.max(((runes * font_size) as f64 * 0.65).ceil() as i64);
        }
    }
    if label_height <= 0 {
        label_height = (font_size as f64 * 1.3).ceil() as i64 * (lines.len() as i64).max(1);
    }
    (
        label_width as f64 * SCENE_SCALE,
        label_height as f64 * SCENE_SCALE,
    )
}

// ---------------------------------------------------------- hierarchy.go

fn compose_hierarchy(
    scene: &mut Scene,
    indices: &HashMap<String, usize>,
    parents: &[Option<usize>],
) -> Result<(), String> {
    let mut board_of: Vec<Option<usize>> = vec![None; scene.nodes.len()];
    for i in 0..scene.nodes.len() {
        let s = scene.nodes[i].metadata.original.clone();
        let size = module_size(&s);
        let mut position = hierarchy_center(&s);
        position.y = 0.07 + size.y / 2.0;
        scene.nodes[i].size = size;
        scene.nodes[i].position = position;
        if !scene.nodes[i].container {
            continue;
        }
        let mut depth = 0;
        let mut p = parents[i];
        while let Some(parent) = p {
            if scene.nodes[parent].container {
                depth += 1;
            }
            p = parents[parent];
        }
        let mut b = Board {
            id: format!("@container:{}", scene.nodes[i].id),
            source_id: scene.nodes[i].id.clone(),
            label: scene.nodes[i].label.clone(),
            kind: if depth > 0 { "group" } else { "platform" }.into(),
            level: depth,
            position: hierarchy_center(&s),
            size: Vec3::new(size.x, 0.14, size.z),
            ..Default::default()
        };
        if !b.label.is_empty() {
            let (_, height) = board_label_metrics(&b, &scene.nodes, indices);
            // Header metadata never enlarges the source footprint.
            b.header_depth = b.size.z.min(0.5f64.max(height + 0.3));
        }
        board_of[i] = Some(scene.boards.len());
        let node = &mut scene.nodes[i];
        node.position = b.position;
        node.size = b.size;
        node.board_id = b.id.clone();
        scene.boards.push(b);
    }
    let mut root = None;
    for i in 0..scene.nodes.len() {
        let mut parent = parents[i];
        // Sequence semantic parents keep ParentID; render membership attaches
        // to the nearest actual source container board.
        while let Some(p) = parent {
            if board_of[p].is_some() {
                break;
            }
            parent = parents[p];
        }
        if let Some(p) = parent {
            let board = board_of[p].expect("container board");
            scene.boards[board].node_ids.push(scene.nodes[i].id.clone());
            let board_id = scene.boards[board].id.clone();
            if scene.nodes[i].container {
                let own = board_of[i].expect("container board");
                scene.boards[own].parent_id = board_id;
            } else {
                scene.nodes[i].board_id = board_id;
            }
        } else if !scene.nodes[i].container {
            let board = *root.get_or_insert_with(|| {
                scene.boards.push(Board {
                    id: "@ungrouped".into(),
                    kind: "ungrouped".into(),
                    ..Default::default()
                });
                scene.boards.len() - 1
            });
            scene.boards[board].node_ids.push(scene.nodes[i].id.clone());
            scene.nodes[i].board_id = scene.boards[board].id.clone();
        }
    }
    if let Some(board) = root {
        // Membership metadata only; no visible plate.
        let (mut left, mut top) = (f64::INFINITY, f64::INFINITY);
        let (mut right, mut bottom) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for id in &scene.boards[board].node_ids {
            let n = &scene.nodes[indices[id]];
            left = left.min(n.position.x - n.size.x / 2.0);
            right = right.max(n.position.x + n.size.x / 2.0);
            top = top.min(n.position.z - n.size.z / 2.0);
            bottom = bottom.max(n.position.z + n.size.z / 2.0);
        }
        let b = &mut scene.boards[board];
        b.position = Vec3::new((left + right) / 2.0, 0.0, (top + bottom) / 2.0);
        b.size = Vec3::new(
            SCENE_SCALE.max(right - left),
            0.14,
            SCENE_SCALE.max(bottom - top),
        );
    }
    let mut points = 0;
    let mut fallback = false;
    for i in 0..scene.edges.len() {
        let original = scene.edges[i].metadata.original.clone();
        if original.route().len() >= 2 {
            scene.edges[i].points = Some(
                hierarchy_route(&original, &mut points).map_err(|error| {
                    format!("isometric: hierarchy route {:?}: {error}", scene.edges[i].id)
                })?,
            );
        } else {
            let a = &scene.nodes[indices[&scene.edges[i].source]];
            let b = &scene.nodes[indices[&scene.edges[i].target]];
            let route = hierarchy_fallback(a, b);
            points += route.len();
            scene.edges[i].points = Some(route);
            fallback = true;
        }
        if points > MAX_ENTRIES {
            return Err(format!(
                "isometric: hierarchy route points exceed {MAX_ENTRIES}"
            ));
        }
    }
    if fallback {
        scene.warnings.push("Connections without a usable compiled route use a simple perimeter fallback; obstacle avoidance requires a routed D2 layout.".into());
    }
    Ok(())
}

fn hierarchy_center(s: &Shape) -> Vec3 {
    Vec3::new(
        (s.pos.x as f64 + s.width.max(1) as f64 / 2.0) * SCENE_SCALE,
        0.0,
        (s.pos.y as f64 + s.height.max(1) as f64 / 2.0) * SCENE_SCALE,
    )
}

/// hierarchyRoute: polylines exactly; cubics flattened to a quarter source
/// pixel, at most 24 subdivisions deep, within the shared point budget.
fn hierarchy_route(c: &Connection, count: &mut usize) -> Result<Vec<Vec3>, String> {
    let route: Vec<_> = c.route().iter().map(|p| p.expect("validated")).collect();
    if c.is_curve && (route.len() - 1) % 3 != 0 {
        return Err(format!(
            "invalid cubic control point count {}",
            route.len()
        ));
    }
    let mut out = Vec::with_capacity(route.len());
    let mut append = |out: &mut Vec<Vec3>, p: Vec3| -> Result<(), String> {
        if *count >= MAX_ENTRIES {
            return Err(format!("flattened route points exceed {MAX_ENTRIES}"));
        }
        *count += 1;
        out.push(p);
        Ok(())
    };
    let point = |i: usize| {
        Vec3::new(
            route[i].x * SCENE_SCALE,
            SURFACE_HEIGHT,
            route[i].y * SCENE_SCALE,
        )
    };
    if !c.is_curve {
        for i in 0..route.len() {
            append(&mut out, point(i))?;
        }
        return Ok(out);
    }
    append(&mut out, point(0))?;
    fn flatten(
        a: Vec3,
        b: Vec3,
        c: Vec3,
        d: Vec3,
        depth: usize,
        out: &mut Vec<Vec3>,
        append: &mut dyn FnMut(&mut Vec<Vec3>, Vec3) -> Result<(), String>,
    ) -> Result<(), String> {
        if hierarchy_control_distance(b, a, d) <= SCENE_SCALE * 0.25
            && hierarchy_control_distance(c, a, d) <= SCENE_SCALE * 0.25
        {
            return append(out, d);
        }
        if depth >= 24 {
            return Err("curve subdivision exceeds precision budget".into());
        }
        let (ab, bc, cd) = (midpoint(a, b), midpoint(b, c), midpoint(c, d));
        let (abc, bcd) = (midpoint(ab, bc), midpoint(bc, cd));
        let mid = midpoint(abc, bcd);
        flatten(a, ab, abc, mid, depth + 1, out, append)?;
        flatten(mid, bcd, cd, d, depth + 1, out, append)
    }
    let mut i = 1;
    while i < route.len() {
        flatten(point(i - 1), point(i), point(i + 1), point(i + 2), 0, &mut out, &mut append)?;
        i += 3;
    }
    Ok(out)
}

fn midpoint(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new((a.x + b.x) / 2.0, SURFACE_HEIGHT, (a.z + b.z) / 2.0)
}

fn hierarchy_control_distance(p: Vec3, a: Vec3, b: Vec3) -> f64 {
    let (dx, dz) = (b.x - a.x, b.z - a.z);
    let mut t = 0.0;
    // ARM64 hierarchy.go:197–200: FMADDD n, FMADDD dot, FMSUBD offsets.
    let n = go_fmadd(dz, dz, dx * dx);
    if n > 0.0 {
        t = 0f64.max(1f64.min(go_fmadd(p.z - a.z, dz, (p.x - a.x) * dx) / n));
    }
    gomath::hypot(go_fmsub(dx, t, p.x - a.x), go_fmsub(dz, t, p.z - a.z))
}

#[cfg(test)]
mod cubic_budget_tests {
    use super::*;
    use crate::target::GeoPoint;

    /// hierarchy_test.go:258–262: flattening shares the scene point budget.
    #[test]
    fn cubic_flattening_stops_at_shared_point_budget() {
        let curve = Connection {
            is_curve: true,
            route: Some(
                [(0.0, 0.0), (0.0, 100.0), (100.0, 100.0), (100.0, 0.0)]
                    .map(|(x, y)| Some(GeoPoint { x, y }))
                    .to_vec(),
            ),
            ..Default::default()
        };
        let mut count = MAX_ENTRIES - 1;
        assert_eq!(
            hierarchy_route(&curve, &mut count).unwrap_err(),
            "flattened route points exceed 100000"
        );
        assert_eq!(count, MAX_ENTRIES);
    }
}

fn hierarchy_fallback(a: &Node, b: &Node) -> Vec<Vec3> {
    let h = SURFACE_HEIGHT;
    if a.id == b.id {
        let (x, z) = (a.size.x / 2.0, a.size.z / 2.0);
        let p = a.position;
        return vec![
            Vec3::new(p.x + x, h, p.z),
            Vec3::new(p.x + x + 0.35, h, p.z),
            Vec3::new(p.x + x + 0.35, h, p.z + z + 0.35),
            Vec3::new(p.x, h, p.z + z + 0.35),
            Vec3::new(p.x, h, p.z + z),
        ];
    }
    let port = |n: &Node, toward: Vec3, fallback: f64| {
        let (mut dx, dz) = (toward.x - n.position.x, toward.z - n.position.z);
        if dx == 0.0 && dz == 0.0 {
            dx = fallback;
        }
        let t = 1.0 / (dx.abs() / (n.size.x / 2.0)).max(dz.abs() / (n.size.z / 2.0));
        // ARM64 hierarchy.go:215: FMADDD.
        Vec3::new(go_fmadd(dx, t, n.position.x), h, go_fmadd(dz, t, n.position.z))
    };
    vec![port(a, b.position, 1.0), port(b, a.position, -1.0)]
}

// ----------------------------------------------------------- sequence.go

/// d2sequence.LifelineEndID: suffix plus the FNV-1a 32 hash.
fn lifeline_end_id(actor: &str) -> String {
    let base = format!("{actor}-lifeline-end");
    let hash = base.bytes().fold(2_166_136_261u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(16_777_619)
    });
    format!("{base}-{hash}")
}

/// The formatted last component of a D2 key (d2format of a one-element
/// KeyPath), which retains quoting.
fn formatted_local_key(id: &str) -> Option<String> {
    use d2_rs_core::ast;
    let key = d2_rs_language::parse_key(id.as_bytes()).ok()?;
    let path = key.path.as_ref().filter(|path| !path.is_empty())?;
    let last = path.get(path.len() - 1)?.read().clone();
    let node = ast::Child::KeyPath(ast::Shared::new(ast::KeyPath {
        range: Default::default(),
        path: Some(vec![last].into()),
    }));
    String::from_utf8(d2_rs_language::format::format(&node)).ok()
}

fn sequence_semantic_child(role: &str) -> bool {
    matches!(role, "span" | "note" | "group")
}

fn classify_sequence(scene: &mut Scene, indices: &HashMap<String, usize>, parents: &[Option<usize>]) {
    // Scope None is the source root.
    let mut confirmed: HashMap<Option<usize>, bool> = HashMap::new();
    confirmed.insert(None, scene.root.type_ == SHAPE_SEQUENCE_DIAGRAM);
    let mut scopes = vec![None; scene.nodes.len()];
    for i in 0..scene.nodes.len() {
        let mut p = parents[i];
        while let Some(parent) = p {
            if scene.nodes[parent].type_ == SHAPE_SEQUENCE_DIAGRAM {
                scopes[i] = Some(parent);
                break;
            }
            p = parents[parent];
        }
        if scene.nodes[i].type_ == SHAPE_SEQUENCE_DIAGRAM {
            scene.nodes[i].sequence_role = "container".into();
            confirmed.insert(Some(i), true);
        }
    }
    for i in 0..scene.edges.len() {
        let edge = &scene.edges[i];
        let Some(&actor) = indices.get(&edge.source) else { continue };
        if indices.contains_key(&edge.target)
            || edge.metadata.original.z_index != LIFELINE_Z_INDEX
            || edge.source_arrow != "none"
            || edge.target_arrow != "none"
            || edge.metadata.original.route().len() != 2
        {
            continue;
        }
        let Some(local) = formatted_local_key(&edge.source) else { continue };
        if edge.target != lifeline_end_id(&local) {
            continue;
        }
        scene.edges[i].sequence_role = "lifeline".into();
        scene.nodes[actor].sequence_role = "actor".into();
        confirmed.insert(scopes[actor], true);
    }
    scene.has_sequence = confirmed.values().any(|exists| *exists);
    if !scene.has_sequence {
        return;
    }
    let is_confirmed = |scope: &Option<usize>| confirmed.get(scope).copied().unwrap_or(false);
    for i in 0..scene.nodes.len() {
        let node = &mut scene.nodes[i];
        if !node.sequence_role.is_empty() || !is_confirmed(&scopes[i]) {
            continue;
        }
        match node.metadata.original.z_index {
            SPAN_Z_INDEX => node.sequence_role = "span".into(),
            GROUP_Z_INDEX if node.metadata.original.blend => node.sequence_role = "group".into(),
            NOTE_Z_INDEX => node.sequence_role = "note".into(),
            _ => {}
        }
    }
    for i in 0..scene.edges.len() {
        let edge = &scene.edges[i];
        let (Some(&source), Some(&target)) = (indices.get(&edge.source), indices.get(&edge.target))
        else {
            continue;
        };
        if edge.sequence_role.is_empty()
            && edge.metadata.original.z_index == MESSAGE_Z_INDEX
            && scopes[source] == scopes[target]
            && is_confirmed(&scopes[source])
        {
            scene.edges[i].sequence_role = "message".into();
        }
    }
    // ParentID remains the authored hierarchy; spans and notes do not make
    // their actor a physical container, real ordinary children still do.
    for node in &mut scene.nodes {
        node.container = node.sequence_role == "container";
    }
    for i in 0..scene.nodes.len() {
        if sequence_semantic_child(&scene.nodes[i].sequence_role) {
            continue;
        }
        let mut parent = parents[i];
        while let Some(p) = parent {
            if !sequence_semantic_child(&scene.nodes[p].sequence_role) {
                break;
            }
            parent = parents[p];
        }
        if let Some(p) = parent {
            scene.nodes[p].container = true;
        }
    }
}
