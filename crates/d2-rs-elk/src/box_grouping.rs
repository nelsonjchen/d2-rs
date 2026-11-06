// Copyright (c) 2009, 2020 Kiel University and others.
// Copyright (c) 2015, 2020 Kiel University and others.
// SPDX-License-Identifier: EPL-2.0
// elk-go v0.2.0/internal/box/provider.go group, mergeAndPlace*, placeInnerBoxes;
// internal/common/elkutil/elkutil.go ResizeNodeForConstraints and Translate.
use super::{java_compare, maximum, number};
use crate::layered::gwt_priority_queue as queue;
use crate::utility::{flag, minimum, option, padding, point};
use crate::{ElkEdge, ElkGraph, ElkNode, LayeringError};
use serde_json::Value;

#[derive(Default)]
struct Group {
    node: Option<usize>,
    children: Vec<usize>,
    size: (f64, f64),
    bottom: Vec<usize>,
    right: Vec<usize>,
}
struct Groups<'a> {
    graph: &'a mut ElkGraph,
    groups: Vec<Group>,
    direction: String,
}
impl Groups<'_> {
    fn new_group(&mut self, children: Vec<usize>) -> usize {
        let id = self.groups.len();
        self.groups.push(Group {
            children,
            ..Group::default()
        });
        id
    }
    fn width(&self, id: usize) -> f64 {
        self.groups[id]
            .node
            .map(|n| self.graph.children[n].width)
            .unwrap_or(self.groups[id].size.0)
    }
    fn height(&self, id: usize) -> f64 {
        self.groups[id]
            .node
            .map(|n| self.graph.children[n].height)
            .unwrap_or(self.groups[id].size.1)
    }
    fn area(&self, id: usize) -> f64 {
        self.width(id) * self.height(id)
    }
    fn set_width(&mut self, id: usize, width: f64) {
        if let Some(n) = self.groups[id].node {
            self.graph.children[n].width = width;
        } else {
            let delta = width - self.width(id);
            for child in self.groups[id].right.clone() {
                self.set_width(child, self.width(child) + delta);
            }
        }
        // Source group.size is deliberately unchanged by group.setWidth.
    }
    fn set_height(&mut self, id: usize, height: f64) {
        if let Some(n) = self.groups[id].node {
            self.graph.children[n].height = height;
        } else {
            let delta = height - self.height(id);
            for child in self.groups[id].bottom.clone() {
                self.set_height(child, self.height(child) + delta);
            }
        }
    }
    fn translate(&mut self, id: usize, x: f64, y: f64, inner: bool) {
        if let Some(n) = self.groups[id].node {
            if inner {
                translate_contents(self.graph, n, x, y);
            } else {
                self.graph.children[n].x += x;
                self.graph.children[n].y += y;
            }
        } else {
            for child in self.groups[id].children.clone() {
                self.translate(child, x, y, inner);
            }
        }
    }
    fn place(
        &mut self,
        id: usize,
        spacing: f64,
        pad: [f64; 4],
        min: (f64, f64),
        expand: bool,
        aspect: f64,
    ) -> (f64, f64) {
        let children = self.groups[id].children.clone();
        let [top, left, bottom, right] = pad;
        let mut max_width = 0.0;
        let mut area = 0.0;
        for &child in &children {
            if let Some(n) = self.groups[child].node {
                resize_for_constraints(&mut self.graph.children[n], &self.direction);
            }
            max_width = maximum(max_width, self.width(child));
            area += self.area(child);
        }
        let mean = area / children.len() as f64;
        let mut variance = 0.0;
        for &child in &children {
            let delta = self.area(child) - mean;
            variance += delta * delta;
        }
        area += children.len() as f64 * (variance / (children.len() as f64 - 1.0)).sqrt();
        max_width = maximum(max_width, (area * aspect).sqrt()) + left;
        let (mut x, mut y) = (left, top);
        let (mut highest, mut broadest) = (0.0, left + right);
        let mut rows = vec![0];
        let mut heights = Vec::new();
        let mut last = None;
        let mut bottoms = Vec::new();
        for (index, &child) in children.iter().enumerate() {
            let (width, height) = (self.width(child), self.height(child));
            if x + width > max_width {
                if expand {
                    heights.push(highest);
                    rows.push(index);
                    self.groups[id]
                        .right
                        .push(last.expect("source nonempty previous row"));
                    bottoms.clear();
                }
                x = left;
                y += highest + spacing;
                highest = 0.0;
                broadest = maximum(broadest, left + right + width);
            }
            bottoms.push(child);
            self.translate(child, x, y, false);
            broadest = maximum(broadest, x + width + right);
            highest = maximum(highest, height);
            x += width + spacing;
            last = Some(child);
        }
        self.groups[id].bottom.extend(bottoms.iter().copied());
        self.groups[id]
            .right
            .push(*bottoms.last().expect("source nonempty group"));
        broadest = maximum(broadest, min.0);
        let mut total_height = y + highest + bottom;
        if total_height < min.1 {
            highest += min.1 - total_height;
            total_height = min.1;
        }
        if expand {
            x = left;
            rows.push(children.len());
            heights.push(highest);
            let mut row = 0;
            let mut height = 0.0;
            for (index, &child) in children.iter().enumerate() {
                if index == rows[row] {
                    x = left;
                    height = heights[row];
                    row += 1;
                }
                self.set_height(child, height);
                if index + 1 == rows[row] {
                    let width = broadest - x - right;
                    let old_width = self.width(child);
                    self.set_width(child, width);
                    self.translate(child, (width - old_width) / 2.0, 0.0, true);
                }
                x += self.width(child) + spacing;
            }
        }
        (broadest, total_height)
    }
    fn merge(
        &mut self,
        mut groups: Vec<usize>,
        mode: &str,
        spacing: f64,
        min: (f64, f64),
        expand: bool,
    ) -> Vec<usize> {
        let mut placed = Vec::new();
        if mode == "GROUP_INC" {
            groups.sort_by(|&a, &b| java_compare(self.area(a), self.area(b)));
            let mut common = 0.0;
            for value in groups {
                if !placed.is_empty() && self.area(value) > common * 2.0 {
                    let merged = self.new_group(std::mem::take(&mut placed));
                    let aspect = self.width(value) / self.height(value);
                    self.groups[merged].size =
                        self.place(merged, spacing, [0.0; 4], min, expand, aspect);
                    placed.extend([merged, value]);
                    common = self.area(merged) + self.area(value);
                } else {
                    placed.push(value);
                    common += self.area(value);
                }
            }
        } else if mode == "GROUP_DEC" {
            groups.sort_by(|&a, &b| java_compare(self.area(b), self.area(a)));
            let count = groups.len();
            let mut maybe = Vec::new();
            let mut beat = None;
            let mut collected = 0.0;
            for (index, value) in groups.into_iter().enumerate() {
                if beat.is_none_or(|b| self.area(b) / 2.0 < self.area(value)) {
                    beat = Some(value);
                    placed.push(value);
                } else {
                    collected += self.area(value);
                    maybe.push(value);
                    if maybe.len() > 1
                        && (collected > self.area(beat.unwrap()) / 2.0 || index + 1 == count)
                    {
                        let inner = self.new_group(std::mem::take(&mut maybe));
                        let b = beat.unwrap();
                        let aspect = self.width(b) / self.height(b);
                        self.groups[inner].size =
                            self.place(inner, spacing, [0.0; 4], min, expand, aspect);
                        beat = Some(inner);
                        placed.push(inner);
                        collected = 0.0;
                    }
                }
            }
            placed.extend(maybe);
        } else {
            let mut cumulative = vec![0.0; groups.len()];
            let mut heap = Vec::new();
            queue::add_all(&mut heap, &groups, |a, b| {
                java_compare(self.area(a), self.area(b))
            });
            let mut index = 0;
            while let Some(&value) = heap.first() {
                if index > 1 && self.area(value) / 2.0 > cumulative[0] {
                    let mut selection = 0;
                    while selection < placed.len() - 1
                        && self.area(value) / 2.0 > cumulative[selection]
                    {
                        selection += 1;
                    }
                    let remaining = placed.split_off(selection + 1);
                    let inner = self.new_group(std::mem::take(&mut placed));
                    let aspect = self.width(value) / self.height(value);
                    self.groups[inner].size =
                        self.place(inner, spacing, [0.0; 4], min, expand, aspect);
                    queue::offer(&mut heap, inner, |a, b| {
                        java_compare(self.area(a), self.area(b))
                    });
                    queue::add_all(&mut heap, &remaining, |a, b| {
                        java_compare(self.area(a), self.area(b))
                    });
                    index = 0;
                    cumulative.fill(0.0);
                } else {
                    let value =
                        queue::poll(&mut heap, |a, b| java_compare(self.area(a), self.area(b)))
                            .unwrap();
                    if index > 0 {
                        cumulative[index] = cumulative[index - 1];
                    }
                    cumulative[index] += self.area(value);
                    index += 1;
                    placed.push(value);
                }
            }
        }
        placed
    }
}

pub(super) fn layout(graph: &mut ElkGraph) -> Result<(), LayeringError> {
    let mode = option(&graph.layout_options, "box.packingMode")
        .and_then(Value::as_str)
        .unwrap_or("SIMPLE")
        .to_owned();
    let spacing = number(&graph.layout_options, "spacing.nodeNode", 15.0) as f32 as f64;
    let pad = padding(&graph.layout_options);
    // Grouping uses the raw minimum property, independently of constraints.
    let raw_min = point(&graph.layout_options, "nodeSize.minimum").unwrap_or_default();
    let min = (
        maximum(raw_min.0 - pad[1] - pad[3], 0.0),
        maximum(raw_min.1 - pad[0] - pad[2], 0.0),
    );
    let aspect = number(&graph.layout_options, "aspectRatio", 1.3_f32 as f64);
    let aspect = if aspect <= 0.0 { 1.3 } else { aspect };
    let expand = flag(&graph.layout_options, "expandNodes");
    let direction = option(&graph.layout_options, "direction")
        .and_then(Value::as_str)
        .unwrap_or("UNDEFINED")
        .to_owned();
    let mut state = Groups {
        graph,
        groups: Vec::new(),
        direction,
    };
    for (i, node) in state.graph.children.iter_mut().enumerate() {
        node.x = 0.0;
        node.y = 0.0;
        state.groups.push(Group {
            node: Some(i),
            ..Group::default()
        });
    }
    let ids = (0..state.groups.len()).collect();
    let placed = state.merge(ids, &mode, spacing, min, expand);
    let outer = state.new_group(placed);
    let size = state.place(outer, spacing, pad, min, expand, aspect);
    let effective = minimum(&state.graph.layout_options);
    state.graph.width = Some(maximum(size.0, effective.0));
    state.graph.height = Some(maximum(size.1, effective.1));
    state.graph.x = Some(state.graph.x.unwrap_or(0.0));
    state.graph.y = Some(state.graph.y.unwrap_or(0.0));
    Ok(())
}

fn translate_edge(edge: &mut ElkEdge, x: f64, y: f64) {
    for section in &mut edge.sections {
        section.start.x += x;
        section.start.y += y;
        section.end.x += x;
        section.end.y += y;
        for bend in &mut section.bends {
            bend.x += x;
            bend.y += y;
        }
    }
    for label in &mut edge.labels {
        label.x += x;
        label.y += y;
    }
    for point in &mut edge.junction_points {
        point.x += x;
        point.y += y;
    }
}
pub(super) fn translate_contents(graph: &mut ElkGraph, index: usize, x: f64, y: f64) {
    let owner = graph.children[index].id.clone();
    let containers = crate::edge_container_ids(graph, &graph.edges);
    for (edge, container) in graph.edges.iter_mut().zip(containers) {
        if container == owner {
            translate_edge(edge, x, y);
        }
    }
    let node = &mut graph.children[index];
    for child in &mut node.children {
        child.x += x;
        child.y += y;
    }
    for edge in &mut node.edges {
        translate_edge(edge, x, y);
    }
}

fn side(port: &crate::ElkPort, width: f64, height: f64, direction: &str) -> String {
    let given = option(&port.layout_options, "port.side")
        .and_then(Value::as_str)
        .unwrap_or("UNDEFINED");
    if given != "UNDEFINED" || (width <= 0.0 && height <= 0.0) {
        return given.to_owned();
    }
    let x = (port.x + port.width / 2.0) / width;
    let y = (port.y + port.height / 2.0) / height;
    if matches!(direction, "LEFT" | "RIGHT") && port.x < 0.0 {
        "WEST"
    } else if matches!(direction, "LEFT" | "RIGHT") && port.x + port.width > width {
        "EAST"
    } else if matches!(direction, "UP" | "DOWN") && port.y < 0.0 {
        "NORTH"
    } else if matches!(direction, "UP" | "DOWN") && port.y + port.height > height {
        "SOUTH"
    } else if x + y <= 1.0 && x - y <= 0.0 {
        "WEST"
    } else if x + y >= 1.0 && x - y >= 0.0 {
        "EAST"
    } else if y < 0.5 {
        "NORTH"
    } else {
        "SOUTH"
    }
    .to_owned()
}
pub(super) fn resize_for_constraints(node: &mut ElkNode, direction: &str) {
    let opts = node.layout_options.as_ref().unwrap_or(&Value::Null);
    let constraints = option(opts, "nodeSize.constraints")
        .and_then(Value::as_str)
        .unwrap_or("");
    if constraints.trim_matches(['[', ']', ' ']).is_empty() {
        return;
    }
    let fixed = option(opts, "portConstraints").and_then(Value::as_str) == Some("FIXED_POS");
    let old = (node.width, node.height);
    let sides: Vec<_> = node
        .ports
        .iter()
        .map(|p| side(p, old.0, old.1, direction))
        .collect();
    let mut new = (0.0, 0.0);
    if constraints.contains("PORTS") {
        let mut minima = [2.0; 4];
        for (port, side) in node.ports.iter().zip(&sides) {
            let (idx, pos, len) = match side.as_str() {
                "NORTH" => (0, port.x, port.width),
                "EAST" => (1, port.y, port.height),
                "SOUTH" => (2, port.x, port.width),
                "WEST" => (3, port.y, port.height),
                _ => continue,
            };
            if fixed {
                minima[idx] = maximum(minima[idx], pos + len);
            } else {
                minima[idx] += len + 2.0;
            }
        }
        new = (maximum(minima[0], minima[2]), maximum(minima[1], minima[3]));
    }
    let min = minimum(opts);
    new = (maximum(new.0, min.0), maximum(new.1, min.1));
    for (port, side) in node.ports.iter_mut().zip(sides) {
        match side.as_str() {
            "NORTH" => {
                if !fixed {
                    port.x *= new.0 / old.0;
                }
            }
            "EAST" => {
                port.x += new.0 - old.0;
                if !fixed {
                    port.y *= new.1 / old.1;
                }
            }
            "SOUTH" => {
                if !fixed {
                    port.x *= new.0 / old.0;
                }
                port.y += new.1 - old.1;
            }
            "WEST" if !fixed => {
                port.y *= new.1 / old.1;
            }
            _ => {}
        }
        if let Some(map) = port.layout_options.as_object_mut() {
            map.insert("org.eclipse.elk.port.side".into(), Value::String(side));
        }
    }
    for label in &mut node.labels {
        let midx = label.x + label.width / 2.0;
        let midy = label.y + label.height / 2.0;
        let px = midx / old.0;
        let py = midy / old.1;
        if px + py >= 1.0 {
            if px - py > 0.0 && midy >= 0.0 {
                label.x += new.0 - old.0;
                label.y = crate::js_trig::product_sum(new.1 - old.1, py, label.y);
            } else if px - py < 0.0 && midx >= 0.0 {
                label.x = crate::js_trig::product_sum(new.0 - old.0, px, label.x);
                label.y += new.1 - old.1;
            }
        }
    }
    node.width = new.0;
    node.height = new.1;
    if let Some(map) = node.layout_options.as_mut().and_then(Value::as_object_mut) {
        map.retain(|key, _| !key.ends_with("nodeSize.constraints"));
        map.insert(
            "org.eclipse.elk.nodeSize.constraints".into(),
            Value::String("[]".into()),
        );
    }
}
