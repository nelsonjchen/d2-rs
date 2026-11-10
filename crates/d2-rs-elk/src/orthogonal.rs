//! Source-shaped port-segment routing for ELK Layered's orthogonal edge phase.
//!
//! The segment graph determines both bend lanes and the horizontal space
//! reserved between adjacent layers. Counting crossing edges directly loses
//! shared port trunks and produces too few lanes on dense graphs.

use crate::layered::{JavaRandom, LEdgeId, LGraphId, LNodeId, LPortId, LayeredArena, PortSide};
use d2_rs_core::Point;
use d2_rs_core::stable_sort::StableSort;
use std::collections::{BTreeSet, HashMap, VecDeque};

const TOLERANCE: f64 = 1e-3;
const CRITICAL_CONFLICTS: i32 = -1;
const CONFLICT_PENALTY: i32 = 1;
const CROSSING_PENALTY: i32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DependencyKind {
    Regular,
    Critical,
}

#[derive(Clone, Debug)]
struct Dependency {
    kind: DependencyKind,
    source: Option<usize>,
    target: Option<usize>,
    weight: i32,
}

#[derive(Clone, Debug)]
struct Segment {
    ports: Vec<LPortId>,
    mark: i64,
    routing_slot: usize,
    start: f64,
    end: f64,
    incoming_coordinates: Vec<f64>,
    outgoing_coordinates: Vec<f64>,
    outgoing_dependencies: Vec<usize>,
    incoming_dependencies: Vec<usize>,
    out_weight: i32,
    critical_out_weight: i32,
    in_weight: i32,
    critical_in_weight: i32,
    split_partner: Option<usize>,
    split_by: Option<usize>,
}

impl Segment {
    fn new() -> Self {
        Self {
            ports: Vec::new(),
            mark: 0,
            routing_slot: 0,
            start: f64::NAN,
            end: f64::NAN,
            incoming_coordinates: Vec::new(),
            outgoing_coordinates: Vec::new(),
            outgoing_dependencies: Vec::new(),
            incoming_dependencies: Vec::new(),
            out_weight: 0,
            critical_out_weight: 0,
            in_weight: 0,
            critical_in_weight: 0,
            split_partner: None,
            split_by: None,
        }
    }

    fn length(&self) -> f64 {
        self.end - self.start
    }

    fn represents_hyperedge(&self) -> bool {
        self.incoming_coordinates.len() + self.outgoing_coordinates.len() > 2
    }

    fn is_dummy(&self) -> bool {
        self.split_partner.is_some() && self.split_by.is_none()
    }

    fn recompute_extent(&mut self) {
        self.start = f64::NAN;
        self.end = f64::NAN;
        self.include_extent(&self.incoming_coordinates.clone());
        self.include_extent(&self.outgoing_coordinates.clone());
    }

    fn include_extent(&mut self, coordinates: &[f64]) {
        let Some(first) = coordinates.first().copied() else {
            return;
        };
        let last = *coordinates.last().unwrap();
        if self.start.is_nan() {
            self.start = first;
        } else {
            self.start = self.start.min(first);
        }
        if self.end.is_nan() {
            self.end = last;
        } else {
            self.end = self.end.max(last);
        }
    }
}

/// Routes one adjacent layer pair and returns the number of source-equivalent
/// routing slots. Empty slices represent the virtual graph boundaries.
pub(crate) fn route_adjacent_layers(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    source_nodes: &[LNodeId],
    target_nodes: &[LNodeId],
    start_position: f64,
    edge_spacing: f64,
    random: &mut JavaRandom,
) -> usize {
    let mut segments = Vec::<Segment>::new();
    let mut port_segments = HashMap::<LPortId, usize>::new();
    create_hyperedge_segments(
        arena,
        source_nodes,
        PortSide::East,
        &mut segments,
        &mut port_segments,
    );
    create_hyperedge_segments(
        arena,
        target_nodes,
        PortSide::West,
        &mut segments,
        &mut port_segments,
    );

    let critical_threshold = 0.2 * minimum_horizontal_segment_distance(&segments);
    let conflict_threshold = 0.5 * edge_spacing;
    let mut critical_dependencies = 0;
    let mut dependencies = Vec::<Dependency>::new();
    for first in 0..segments.len().saturating_sub(1) {
        for second in first + 1..segments.len() {
            critical_dependencies += create_dependency_if_necessary(
                first,
                second,
                &mut segments,
                &mut dependencies,
                conflict_threshold,
                critical_threshold,
            );
        }
    }

    #[cfg(feature = "diagnostic-traces")]
    emit_segment_trace("constructed", graph_id, arena, &segments, &dependencies);

    if critical_dependencies >= 2 {
        let cycles = detect_cycles(&mut segments, &mut dependencies, true, random);
        if !cycles.is_empty() {
            split_critical_cycles(
                &mut segments,
                &mut dependencies,
                &cycles,
                critical_threshold,
                conflict_threshold,
                random,
            );
        }
    }

    #[cfg(feature = "diagnostic-traces")]
    emit_segment_trace(
        "critical-cycle-break",
        graph_id,
        arena,
        &segments,
        &dependencies,
    );

    let cycles = detect_cycles(&mut segments, &mut dependencies, false, random);
    for dependency_id in cycles {
        if dependencies[dependency_id].weight == 0 {
            remove_dependency(dependency_id, &mut segments, &mut dependencies);
        } else {
            reverse_dependency(dependency_id, &mut segments, &mut dependencies);
        }
    }

    #[cfg(feature = "diagnostic-traces")]
    emit_segment_trace("all-cycle-break", graph_id, arena, &segments, &dependencies);

    topological_numbering(&mut segments, &dependencies);
    #[cfg(feature = "diagnostic-traces")]
    emit_segment_trace("numbered", graph_id, arena, &segments, &dependencies);

    let mut max_rank: Option<usize> = None;
    let mut created_junctions = BTreeSet::<(u64, u64)>::new();
    for segment_id in 0..segments.len() {
        let segment = &segments[segment_id];
        if (segment.start - segment.end).abs() < TOLERANCE {
            continue;
        }
        max_rank =
            Some(max_rank.map_or(segment.routing_slot, |rank| rank.max(segment.routing_slot)));
        if segment.is_dummy() {
            continue;
        }
        for port_id in segment.ports.clone() {
            let source_point = absolute_anchor(arena, port_id);
            for edge_id in arena.ports[port_id as usize].outgoing_edges.clone() {
                let edge = &arena.edges[edge_id as usize];
                if !edge.active || edge.source == edge.target {
                    continue;
                }
                let Some(target_port) = edge.target_port else {
                    continue;
                };
                let target_point = absolute_anchor(arena, target_port);
                if (source_point.y - target_point.y).abs() <= TOLERANCE {
                    continue;
                }
                let mut current_x = start_position + segment.routing_slot as f64 * edge_spacing;
                let mut current_segment = segment_id;
                let mut bend = Point {
                    x: current_x,
                    y: source_point.y,
                };
                add_bend_and_junction(
                    arena,
                    edge_id,
                    current_segment,
                    bend,
                    &segments,
                    &mut created_junctions,
                );
                if let Some(partner) = segments[segment_id].split_partner {
                    let Some(split_y) = segments[partner].incoming_coordinates.first().copied()
                    else {
                        continue;
                    };
                    bend = Point {
                        x: current_x,
                        y: split_y,
                    };
                    add_bend_and_junction(
                        arena,
                        edge_id,
                        current_segment,
                        bend,
                        &segments,
                        &mut created_junctions,
                    );
                    current_x =
                        start_position + segments[partner].routing_slot as f64 * edge_spacing;
                    current_segment = partner;
                    bend = Point {
                        x: current_x,
                        y: split_y,
                    };
                    add_bend_and_junction(
                        arena,
                        edge_id,
                        current_segment,
                        bend,
                        &segments,
                        &mut created_junctions,
                    );
                }
                bend = Point {
                    x: current_x,
                    y: target_point.y,
                };
                add_bend_and_junction(
                    arena,
                    edge_id,
                    current_segment,
                    bend,
                    &segments,
                    &mut created_junctions,
                );
            }
        }
    }
    let _ = graph_id;
    max_rank.map_or(0, |rank| rank + 1)
}

fn create_hyperedge_segments(
    arena: &LayeredArena,
    nodes: &[LNodeId],
    side: PortSide,
    segments: &mut Vec<Segment>,
    port_segments: &mut HashMap<LPortId, usize>,
) {
    for node_id in nodes {
        for port_id in &arena.nodes[*node_id as usize].ports {
            let port = &arena.ports[*port_id as usize];
            if port.side != side
                || !port
                    .outgoing_edges
                    .iter()
                    .any(|edge_id| participates_in_orthogonal_routing(arena, *edge_id))
                || port_segments.contains_key(port_id)
            {
                continue;
            }
            let segment_id = segments.len();
            segments.push(Segment::new());
            add_port_positions(arena, *port_id, segment_id, segments, port_segments);
        }
    }
}

fn add_port_positions(
    arena: &LayeredArena,
    port_id: LPortId,
    segment_id: usize,
    segments: &mut [Segment],
    port_segments: &mut HashMap<LPortId, usize>,
) {
    if port_segments.contains_key(&port_id) {
        return;
    }
    port_segments.insert(port_id, segment_id);
    segments[segment_id].ports.push(port_id);
    let position = absolute_anchor(arena, port_id).y;
    if arena.ports[port_id as usize].side == PortSide::East {
        insert_sorted_unique(&mut segments[segment_id].incoming_coordinates, position);
    } else {
        insert_sorted_unique(&mut segments[segment_id].outgoing_coordinates, position);
    }
    segments[segment_id].recompute_extent();
    let port = &arena.ports[port_id as usize];
    for edge_id in &port.incoming_edges {
        if !participates_in_orthogonal_routing(arena, *edge_id) {
            continue;
        }
        if let Some(other) = arena.edges[*edge_id as usize].source_port
            && !port_segments.contains_key(&other)
        {
            add_port_positions(arena, other, segment_id, segments, port_segments);
        }
    }
    for edge_id in &port.outgoing_edges {
        if !participates_in_orthogonal_routing(arena, *edge_id) {
            continue;
        }
        if let Some(other) = arena.edges[*edge_id as usize].target_port
            && !port_segments.contains_key(&other)
        {
            add_port_positions(arena, other, segment_id, segments, port_segments);
        }
    }
}

/// Self-loop preprocessing in elk-go temporarily detaches loop endpoints
/// before P5. The Rust arena retains the original endpoints for loop routing,
/// so exclude those edges wherever the ordinary orthogonal route generator
/// traverses port adjacency.
fn participates_in_orthogonal_routing(arena: &LayeredArena, edge_id: LEdgeId) -> bool {
    let edge = &arena.edges[edge_id as usize];
    edge.active && edge.source != edge.target
}

fn insert_sorted_unique(values: &mut Vec<f64>, value: f64) {
    for index in 0..values.len() {
        let narrowed = f64::from(values[index] as f32);
        if narrowed == value {
            return;
        }
        if narrowed > value {
            values.insert(index, value);
            return;
        }
    }
    values.push(value);
}

#[cfg(feature = "diagnostic-traces")]
fn emit_segment_trace(
    stage: &str,
    graph_id: LGraphId,
    arena: &LayeredArena,
    segments: &[Segment],
    dependencies: &[Dependency],
) {
    let coordinates = |values: &[f64]| {
        values
            .iter()
            .map(|value| format!("{:016x}", value.to_bits()))
            .collect::<Vec<_>>()
            .join(",")
    };
    for (segment_id, segment) in segments.iter().enumerate() {
        let ports = segment
            .ports
            .iter()
            .map(|port_id| {
                let port = &arena.ports[*port_id as usize];
                let node = &arena.nodes[port.node as usize];
                let incoming = port
                    .incoming_edges
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                let outgoing = port
                    .outgoing_edges
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                format!(
                    "{}:{}:{:?}:{}:{incoming}:{outgoing}",
                    node.source_id,
                    port.source_id.as_deref().unwrap_or("synthetic"),
                    port.side,
                    port_id,
                )
            })
            .collect::<Vec<_>>()
            .join("|");
        let outgoing_dependencies = segment
            .outgoing_dependencies
            .iter()
            .map(|dependency_id| {
                let dependency = &dependencies[*dependency_id];
                format!(
                    "{}:{:?}:{}",
                    dependency.target.unwrap_or(usize::MAX),
                    dependency.kind,
                    dependency.weight,
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        crate::trace::emit_value(
            "routing",
            &format!("p5_orthogonal_segments_{stage}"),
            &format!("graphs[{graph_id}].segments[{segment_id}]"),
            &format!(
                "slot={};start={:016x};end={:016x};incoming={};outgoing={};ports={ports};dependencies={outgoing_dependencies}",
                segment.routing_slot,
                segment.start.to_bits(),
                segment.end.to_bits(),
                coordinates(&segment.incoming_coordinates),
                coordinates(&segment.outgoing_coordinates),
            ),
        );
    }
}

fn minimum_horizontal_segment_distance(segments: &[Segment]) -> f64 {
    fn minimum_difference(values: &mut [f64]) -> f64 {
        if values.len() < 2 {
            return f64::MAX;
        }
        values.stable_sort_by(f64::total_cmp);
        let mut minimum = f64::MAX;
        let mut previous = values[0];
        for current in values.iter().copied().skip(1) {
            if current != previous {
                minimum = minimum.min(current - previous);
                previous = current;
            }
        }
        minimum
    }
    let mut incoming = Vec::new();
    let mut outgoing = Vec::new();
    for segment in segments {
        incoming.extend_from_slice(&segment.incoming_coordinates);
        outgoing.extend_from_slice(&segment.outgoing_coordinates);
    }
    minimum_difference(&mut incoming).min(minimum_difference(&mut outgoing))
}

fn create_dependency_if_necessary(
    first: usize,
    second: usize,
    segments: &mut [Segment],
    dependencies: &mut Vec<Dependency>,
    conflict_threshold: f64,
    critical_threshold: f64,
) -> i32 {
    if (segments[first].start - segments[first].end).abs() < TOLERANCE
        || (segments[second].start - segments[second].end).abs() < TOLERANCE
    {
        return 0;
    }
    let conflicts_first = count_conflicts(
        &segments[first].outgoing_coordinates,
        &segments[second].incoming_coordinates,
        conflict_threshold,
        critical_threshold,
    );
    let conflicts_second = count_conflicts(
        &segments[second].outgoing_coordinates,
        &segments[first].incoming_coordinates,
        conflict_threshold,
        critical_threshold,
    );
    if conflicts_first == CRITICAL_CONFLICTS || conflicts_second == CRITICAL_CONFLICTS {
        let mut count = 0;
        if conflicts_first == CRITICAL_CONFLICTS {
            add_dependency(
                DependencyKind::Critical,
                second,
                first,
                1,
                segments,
                dependencies,
            );
            count += 1;
        }
        if conflicts_second == CRITICAL_CONFLICTS {
            add_dependency(
                DependencyKind::Critical,
                first,
                second,
                1,
                segments,
                dependencies,
            );
            count += 1;
        }
        return count;
    }
    let crossings_first = count_crossings(
        &segments[first].outgoing_coordinates,
        segments[second].start,
        segments[second].end,
    ) + count_crossings(
        &segments[second].incoming_coordinates,
        segments[first].start,
        segments[first].end,
    );
    let crossings_second = count_crossings(
        &segments[second].outgoing_coordinates,
        segments[first].start,
        segments[first].end,
    ) + count_crossings(
        &segments[first].incoming_coordinates,
        segments[second].start,
        segments[second].end,
    );
    let value_first = CONFLICT_PENALTY * conflicts_first + CROSSING_PENALTY * crossings_first;
    let value_second = CONFLICT_PENALTY * conflicts_second + CROSSING_PENALTY * crossings_second;
    if value_first < value_second {
        add_dependency(
            DependencyKind::Regular,
            first,
            second,
            value_second - value_first,
            segments,
            dependencies,
        );
    } else if value_first > value_second {
        add_dependency(
            DependencyKind::Regular,
            second,
            first,
            value_first - value_second,
            segments,
            dependencies,
        );
    } else if value_first > 0 && value_second > 0 {
        add_dependency(
            DependencyKind::Regular,
            first,
            second,
            0,
            segments,
            dependencies,
        );
        add_dependency(
            DependencyKind::Regular,
            second,
            first,
            0,
            segments,
            dependencies,
        );
    }
    0
}

fn count_conflicts(first: &[f64], second: &[f64], conflict: f64, critical: f64) -> i32 {
    if first.is_empty() || second.is_empty() {
        return 0;
    }
    let (mut fi, mut si) = (0, 0);
    let (mut fp, mut sp) = (first[0], second[0]);
    let mut conflicts = 0;
    loop {
        if fp > sp - critical && fp < sp + critical {
            return CRITICAL_CONFLICTS;
        }
        if fp > sp - conflict && fp < sp + conflict {
            conflicts += 1;
        }
        if fp <= sp && fi + 1 < first.len() {
            fi += 1;
            fp = first[fi];
        } else if sp <= fp && si + 1 < second.len() {
            si += 1;
            sp = second[si];
        } else {
            break;
        }
    }
    conflicts
}

fn count_crossings(values: &[f64], start: f64, end: f64) -> i32 {
    let mut count = 0;
    for value in values {
        if *value > end {
            break;
        }
        if *value >= start {
            count += 1;
        }
    }
    count
}

fn add_dependency(
    kind: DependencyKind,
    source: usize,
    target: usize,
    weight: i32,
    segments: &mut [Segment],
    dependencies: &mut Vec<Dependency>,
) -> usize {
    let id = dependencies.len();
    dependencies.push(Dependency {
        kind,
        source: Some(source),
        target: Some(target),
        weight,
    });
    segments[source].outgoing_dependencies.push(id);
    segments[target].incoming_dependencies.push(id);
    id
}

fn remove_dependency(id: usize, segments: &mut [Segment], dependencies: &mut [Dependency]) {
    if let Some(source) = dependencies[id].source.take() {
        segments[source]
            .outgoing_dependencies
            .retain(|item| *item != id);
    }
    if let Some(target) = dependencies[id].target.take() {
        segments[target]
            .incoming_dependencies
            .retain(|item| *item != id);
    }
}

fn reverse_dependency(id: usize, segments: &mut [Segment], dependencies: &mut [Dependency]) {
    let source = dependencies[id].source.unwrap();
    let target = dependencies[id].target.unwrap();
    segments[source]
        .outgoing_dependencies
        .retain(|item| *item != id);
    segments[target]
        .incoming_dependencies
        .retain(|item| *item != id);
    dependencies[id].source = Some(target);
    dependencies[id].target = Some(source);
    segments[target].outgoing_dependencies.push(id);
    segments[source].incoming_dependencies.push(id);
}

fn detect_cycles(
    segments: &mut [Segment],
    dependencies: &mut [Dependency],
    critical_only: bool,
    random: &mut JavaRandom,
) -> Vec<usize> {
    let count = segments.len();
    let mut sources = VecDeque::new();
    let mut sinks = VecDeque::new();
    for (index, segment) in segments.iter_mut().enumerate() {
        segment.mark = -(index as i64) - 1;
        segment.critical_in_weight = 0;
        segment.critical_out_weight = 0;
        for dependency_id in &segment.incoming_dependencies {
            let dependency = &dependencies[*dependency_id];
            if dependency.kind == DependencyKind::Critical {
                segment.critical_in_weight += dependency.weight;
            }
        }
        for dependency_id in &segment.outgoing_dependencies {
            let dependency = &dependencies[*dependency_id];
            if dependency.kind == DependencyKind::Critical {
                segment.critical_out_weight += dependency.weight;
            }
        }
        if critical_only {
            segment.in_weight = segment.critical_in_weight;
            segment.out_weight = segment.critical_out_weight;
        } else {
            segment.in_weight = segment
                .incoming_dependencies
                .iter()
                .map(|id| dependencies[*id].weight)
                .sum();
            segment.out_weight = segment
                .outgoing_dependencies
                .iter()
                .map(|id| dependencies[*id].weight)
                .sum();
        }
        if segment.out_weight == 0 {
            sinks.push_back(index);
        } else if segment.in_weight == 0 {
            sources.push_back(index);
        }
    }
    let unprocessed_order = (0..count).rev().collect::<Vec<_>>();
    let mut unprocessed = vec![true; count];
    let mut next_sink_mark = count as i64 - 1;
    let mut next_source_mark = count as i64 + 1;
    while unprocessed.iter().any(|item| *item) {
        while let Some(sink) = sinks.pop_front() {
            if !unprocessed[sink] {
                continue;
            }
            unprocessed[sink] = false;
            segments[sink].mark = next_sink_mark;
            next_sink_mark -= 1;
            update_neighbors(
                sink,
                &mut sources,
                &mut sinks,
                critical_only,
                segments,
                dependencies,
            );
        }
        while let Some(source) = sources.pop_front() {
            if !unprocessed[source] {
                continue;
            }
            unprocessed[source] = false;
            segments[source].mark = next_source_mark;
            next_source_mark += 1;
            update_neighbors(
                source,
                &mut sources,
                &mut sinks,
                critical_only,
                segments,
                dependencies,
            );
        }
        let mut max_outflow = i32::MIN;
        let mut max_segments = Vec::new();
        for segment_id in unprocessed_order.iter().copied() {
            if !unprocessed[segment_id] {
                continue;
            }
            let segment = &segments[segment_id];
            if !critical_only && segment.critical_out_weight > 0 && segment.critical_in_weight <= 0
            {
                max_segments.clear();
                max_segments.push(segment_id);
                break;
            }
            let outflow = segment.out_weight - segment.in_weight;
            if outflow >= max_outflow {
                if outflow > max_outflow {
                    max_segments.clear();
                    max_outflow = outflow;
                }
                max_segments.push(segment_id);
            }
        }
        if !max_segments.is_empty() {
            let selected = max_segments[random.next_int_n(max_segments.len())];
            unprocessed[selected] = false;
            segments[selected].mark = next_source_mark;
            next_source_mark += 1;
            update_neighbors(
                selected,
                &mut sources,
                &mut sinks,
                critical_only,
                segments,
                dependencies,
            );
        }
    }
    let mark_base = count as i64;
    let shift = count as i64 + 1;
    for segment in segments.iter_mut() {
        if segment.mark < mark_base {
            segment.mark += shift;
        }
    }
    let mut cycles = Vec::new();
    for (source, segment) in segments.iter().enumerate() {
        for dependency_id in &segment.outgoing_dependencies {
            let dependency = &dependencies[*dependency_id];
            if (!critical_only || dependency.kind == DependencyKind::Critical)
                && segment.mark > segments[dependency.target.unwrap()].mark
            {
                cycles.push(*dependency_id);
            }
        }
        let _ = source;
    }
    cycles
}

fn update_neighbors(
    node: usize,
    sources: &mut VecDeque<usize>,
    sinks: &mut VecDeque<usize>,
    critical_only: bool,
    segments: &mut [Segment],
    dependencies: &[Dependency],
) {
    for dependency_id in segments[node].outgoing_dependencies.clone() {
        let dependency = &dependencies[dependency_id];
        if critical_only && dependency.kind != DependencyKind::Critical {
            continue;
        }
        let target = dependency.target.unwrap();
        if segments[target].mark < 0 && dependency.weight > 0 {
            segments[target].in_weight -= dependency.weight;
            if dependency.kind == DependencyKind::Critical {
                segments[target].critical_in_weight -= dependency.weight;
            }
            if segments[target].in_weight <= 0 && segments[target].out_weight > 0 {
                sources.push_back(target);
            }
        }
    }
    for dependency_id in segments[node].incoming_dependencies.clone() {
        let dependency = &dependencies[dependency_id];
        if critical_only && dependency.kind != DependencyKind::Critical {
            continue;
        }
        let source = dependency.source.unwrap();
        if segments[source].mark < 0 && dependency.weight > 0 {
            segments[source].out_weight -= dependency.weight;
            if dependency.kind == DependencyKind::Critical {
                segments[source].critical_out_weight -= dependency.weight;
            }
            if segments[source].out_weight <= 0 && segments[source].in_weight > 0 {
                sinks.push_back(source);
            }
        }
    }
}

fn topological_numbering(segments: &mut [Segment], dependencies: &[Dependency]) {
    let mut sources = VecDeque::new();
    let mut rightward_targets = VecDeque::new();
    for (id, segment) in segments.iter_mut().enumerate() {
        segment.in_weight = segment.incoming_dependencies.len() as i32;
        segment.out_weight = segment.outgoing_dependencies.len() as i32;
        if segment.in_weight == 0 {
            sources.push_back(id);
        }
        if segment.out_weight == 0 && segment.incoming_coordinates.is_empty() {
            rightward_targets.push_back(id);
        }
    }
    let mut max_rank: Option<usize> = None;
    while let Some(source) = sources.pop_front() {
        for dependency_id in segments[source].outgoing_dependencies.clone() {
            let target = dependencies[dependency_id].target.unwrap();
            segments[target].routing_slot = segments[target]
                .routing_slot
                .max(segments[source].routing_slot + 1);
            max_rank = Some(max_rank.map_or(segments[target].routing_slot, |rank| {
                rank.max(segments[target].routing_slot)
            }));
            segments[target].in_weight -= 1;
            if segments[target].in_weight == 0 {
                sources.push_back(target);
            }
        }
    }
    if max_rank.is_none() {
        return;
    }
    let max_rank = max_rank.unwrap();
    for target in rightward_targets.iter().copied() {
        segments[target].routing_slot = max_rank;
    }
    while let Some(target) = rightward_targets.pop_front() {
        for dependency_id in segments[target].incoming_dependencies.clone() {
            let source = dependencies[dependency_id].source.unwrap();
            if !segments[source].incoming_coordinates.is_empty() {
                continue;
            }
            segments[source].routing_slot = segments[source]
                .routing_slot
                .min(segments[target].routing_slot.saturating_sub(1));
            segments[source].out_weight -= 1;
            if segments[source].out_weight == 0 {
                rightward_targets.push_back(source);
            }
        }
    }
}

fn absolute_anchor(arena: &LayeredArena, port_id: LPortId) -> Point {
    let port = &arena.ports[port_id as usize];
    let node = &arena.nodes[port.node as usize];
    Point {
        x: node.position_x + port.x,
        y: node.position_y + port.y,
    }
}

fn add_bend_and_junction(
    arena: &mut LayeredArena,
    edge_id: LEdgeId,
    segment_id: usize,
    point: Point,
    segments: &[Segment],
    created: &mut BTreeSet<(u64, u64)>,
) {
    arena.edges[edge_id as usize].bend_points.push(point);
    let segment = &segments[segment_id];
    let inside = point.y > segment.start && point.y < segment.end;
    let boundary =
        if !segment.incoming_coordinates.is_empty() && !segment.outgoing_coordinates.is_empty() {
            let first = (point.y - segment.incoming_coordinates[0]).abs() < TOLERANCE
                && (point.y - segment.outgoing_coordinates[0]).abs() < TOLERANCE;
            let last = (point.y - *segment.incoming_coordinates.last().unwrap()).abs() < TOLERANCE
                && (point.y - *segment.outgoing_coordinates.last().unwrap()).abs() < TOLERANCE;
            first || last
        } else {
            false
        };
    let key = (point.x.to_bits(), point.y.to_bits());
    if (inside || boundary) && created.insert(key) {
        arena.edges[edge_id as usize].junction_points.push(point);
    }
}

fn split_critical_cycles(
    segments: &mut Vec<Segment>,
    dependencies: &mut Vec<Dependency>,
    cycles: &[usize],
    critical_threshold: f64,
    conflict_threshold: f64,
    _random: &mut JavaRandom,
) {
    let mut coordinates = Vec::new();
    for segment in segments.iter() {
        coordinates.extend_from_slice(&segment.incoming_coordinates);
        coordinates.extend_from_slice(&segment.outgoing_coordinates);
    }
    coordinates.stable_sort_by(f64::total_cmp);
    let mut areas = Vec::<FreeArea>::new();
    for pair in coordinates.windows(2) {
        if pair[1] - pair[0] >= 2.0 * critical_threshold {
            let start = pair[0] + critical_threshold;
            let end = pair[1] - critical_threshold;
            areas.push(FreeArea {
                start,
                end,
                size: end - start,
            });
        }
    }
    let mut selected = Vec::<usize>::new();
    let mut seen = BTreeSet::new();
    for dependency_id in cycles {
        let Some(source) = dependencies[*dependency_id].source else {
            continue;
        };
        let Some(target) = dependencies[*dependency_id].target else {
            continue;
        };
        if seen.contains(&source) || seen.contains(&target) {
            continue;
        }
        let (segment, cause) = if segments[source].represents_hyperedge()
            && !segments[target].represents_hyperedge()
        {
            (target, source)
        } else {
            (source, target)
        };
        selected.push(segment);
        seen.insert(segment);
        segments[segment].split_by = Some(cause);
    }
    selected.stable_sort_by(|left, right| {
        segments[*left]
            .length()
            .total_cmp(&segments[*right].length())
    });
    for segment_id in selected {
        let position = compute_split_position(
            segment_id,
            segments,
            dependencies,
            &mut areas,
            critical_threshold,
        );
        let partner = split_segment(segment_id, position, segments, dependencies);
        let cause = segments[segment_id].split_by.unwrap();
        add_dependency(
            DependencyKind::Critical,
            segment_id,
            cause,
            1,
            segments,
            dependencies,
        );
        add_dependency(
            DependencyKind::Critical,
            cause,
            partner,
            1,
            segments,
            dependencies,
        );
        for other in 0..segments.len() {
            if other == cause || other == segment_id || other == partner {
                continue;
            }
            create_dependency_if_necessary(
                other,
                segment_id,
                segments,
                dependencies,
                conflict_threshold,
                critical_threshold,
            );
            create_dependency_if_necessary(
                other,
                partner,
                segments,
                dependencies,
                conflict_threshold,
                critical_threshold,
            );
        }
    }
}

#[derive(Clone, Copy)]
struct FreeArea {
    start: f64,
    end: f64,
    size: f64,
}

#[derive(Clone, Copy, Default)]
struct AreaRating {
    dependencies: i32,
    crossings: i32,
}

fn compute_split_position(
    segment_id: usize,
    segments: &[Segment],
    dependencies: &[Dependency],
    areas: &mut Vec<FreeArea>,
    critical_threshold: f64,
) -> f64 {
    let segment = &segments[segment_id];
    let mut first = None;
    let mut last = None;
    for (index, area) in areas.iter().enumerate() {
        if area.start > segment.end {
            break;
        }
        if area.end >= segment.start {
            first.get_or_insert(index);
            last = Some(index);
        }
    }
    let Some(from) = first else {
        return (segment.start + segment.end) / 2.0;
    };
    let to = last.unwrap();
    let first_candidate = simulate_split(segment);
    let (mut best_area, mut best_rating) = {
        let area = areas[from];
        (
            area,
            rate_area(
                segment_id,
                &first_candidate.0,
                &first_candidate.1,
                area,
                segments,
                dependencies,
            ),
        )
    };
    for area in areas.iter().copied().take(to + 1).skip(from + 1) {
        let rating = rate_area(
            segment_id,
            &first_candidate.0,
            &first_candidate.1,
            area,
            segments,
            dependencies,
        );
        if is_better_area(area, rating, best_area, best_rating) {
            best_area = area;
            best_rating = rating;
            // ELK 0.8.1 intentionally leaves bestIndex unchanged here.
        }
    }
    let position = (areas[from].start + areas[from].end) / 2.0;
    use_area(areas, from, critical_threshold);
    let _ = best_area;
    let _ = best_rating;
    position
}

fn simulate_split(segment: &Segment) -> (Segment, Segment) {
    let mut first = Segment::new();
    let mut second = Segment::new();
    first
        .incoming_coordinates
        .clone_from(&segment.incoming_coordinates);
    first.split_by = segment.split_by;
    second
        .outgoing_coordinates
        .clone_from(&segment.outgoing_coordinates);
    first.recompute_extent();
    second.recompute_extent();
    (first, second)
}

fn rate_area(
    segment_id: usize,
    first: &Segment,
    partner: &Segment,
    area: FreeArea,
    segments: &[Segment],
    dependencies: &[Dependency],
) -> AreaRating {
    let center = (area.start + area.end) / 2.0;
    let mut first = first.clone();
    let mut partner = partner.clone();
    first.outgoing_coordinates.clear();
    first.outgoing_coordinates.push(center);
    partner.incoming_coordinates.clear();
    partner.incoming_coordinates.push(center);
    first.recompute_extent();
    partner.recompute_extent();
    let mut rating = AreaRating::default();
    for dependency_id in &segments[segment_id].incoming_dependencies {
        let other = dependencies[*dependency_id].source.unwrap();
        update_considering_both_orderings(&mut rating, &first, &segments[other]);
        update_considering_both_orderings(&mut rating, &partner, &segments[other]);
    }
    for dependency_id in &segments[segment_id].outgoing_dependencies {
        let other = dependencies[*dependency_id].target.unwrap();
        update_considering_both_orderings(&mut rating, &first, &segments[other]);
        update_considering_both_orderings(&mut rating, &partner, &segments[other]);
    }
    rating.dependencies += 2;
    let cause = segments[segment_id].split_by.unwrap();
    rating.crossings += count_crossings_for_single_ordering(&first, &segments[cause]);
    rating.crossings += count_crossings_for_single_ordering(&segments[cause], &partner);
    rating
}

fn update_considering_both_orderings(rating: &mut AreaRating, first: &Segment, second: &Segment) {
    let first_left = count_crossings_for_single_ordering(first, second);
    let second_left = count_crossings_for_single_ordering(second, first);
    if first_left == second_left {
        if first_left > 0 {
            rating.dependencies += 2;
            rating.crossings += first_left;
        }
    } else {
        rating.dependencies += 1;
        rating.crossings += first_left.min(second_left);
    }
}

fn count_crossings_for_single_ordering(left: &Segment, right: &Segment) -> i32 {
    count_crossings(&left.outgoing_coordinates, right.start, right.end)
        + count_crossings(&right.incoming_coordinates, left.start, left.end)
}

fn is_better_area(
    current: FreeArea,
    current_rating: AreaRating,
    best: FreeArea,
    best_rating: AreaRating,
) -> bool {
    current_rating.crossings < best_rating.crossings
        || current_rating.crossings == best_rating.crossings
            && (current_rating.dependencies < best_rating.dependencies
                || current_rating.dependencies == best_rating.dependencies
                    && current.size > best.size)
}

fn use_area(areas: &mut Vec<FreeArea>, index: usize, critical_threshold: f64) {
    let old = areas.remove(index);
    if old.size / 2.0 < critical_threshold {
        return;
    }
    let old_center = (old.start + old.end) / 2.0;
    let first_end = old_center - critical_threshold;
    let second_start = old_center + critical_threshold;
    let mut replacements = Vec::new();
    if old.start <= first_end {
        replacements.push(FreeArea {
            start: old.start,
            end: first_end,
            size: first_end - old.start,
        });
    }
    if second_start <= old.end {
        replacements.push(FreeArea {
            start: second_start,
            end: old.end,
            size: old.end - second_start,
        });
    }
    for (offset, area) in replacements.into_iter().enumerate() {
        areas.insert(index + offset, area);
    }
}

fn split_segment(
    segment_id: usize,
    position: f64,
    segments: &mut Vec<Segment>,
    dependencies: &mut [Dependency],
) -> usize {
    let mut partner = Segment::new();
    let partner_id = segments.len();
    partner.split_partner = Some(segment_id);
    partner
        .outgoing_coordinates
        .clone_from(&segments[segment_id].outgoing_coordinates);
    segments[segment_id].split_partner = Some(partner_id);
    segments[segment_id].outgoing_coordinates.clear();
    segments[segment_id].outgoing_coordinates.push(position);
    partner.incoming_coordinates.push(position);
    segments[segment_id].recompute_extent();
    partner.recompute_extent();
    while let Some(dependency_id) = segments[segment_id].incoming_dependencies.first().copied() {
        remove_dependency(dependency_id, segments, dependencies);
    }
    while let Some(dependency_id) = segments[segment_id].outgoing_dependencies.first().copied() {
        remove_dependency(dependency_id, segments, dependencies);
    }
    segments.push(partner);
    partner_id
}
