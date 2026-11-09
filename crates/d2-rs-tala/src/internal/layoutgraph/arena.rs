// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Conversion from the public [`crate::Graph`] into the mutable engine arena.
//!
//! This boundary establishes stable identities, container maps, directions,
//! metadata side tables, and the order-sensitive views used by later stages.

use super::*;

impl ArenaGraph {
    pub(super) fn from_input(input: &Graph) -> Self {
        let mut graph = Self::from_input_without_tree_routing(input);
        graph.initialize_tree_routing_state();
        graph
    }

    /// Construct a fresh graph for callers that do not run tree preprocessing,
    /// such as hierarchy.PlaceCompound's isolated placement proxy.
    pub(super) fn from_input_without_tree_routing(input: &Graph) -> Self {
        if crate::internal::trace_env_enabled("TALA_TRACE_SOURCE_EDGES") {
            eprintln!(
                "SOURCE_EDGES_ARENA input_edges={} vec={} map={} first={:?}",
                input.edges().count(),
                input.source_edge_endpoints.len(),
                input.source_edge_endpoints_by_id.len(),
                input
                    .source_edge_endpoints
                    .iter()
                    .take(20)
                    .collect::<Vec<_>>()
            );
        }
        let containers = ContainerMap::new();
        let root_direction = input.direction;
        let mut directions = BTreeMap::from([(None, root_direction)]);
        let mut scoring_directions = input
            .explicit_direction
            .map(|direction| BTreeMap::from([(None, direction)]))
            .unwrap_or_default();
        let mut nodes: Vec<_> = input
            .nodes()
            .map(|(id, node)| {
                let childless_grid = (node.grid_rows.is_some() || node.grid_columns.is_some())
                    && !input.nodes().any(|(_, child)| child.parent == Some(id));
                // translateGraph keeps authored integer constraints separate
                // from the measured Box. Public engine validation has already
                // checked these raw scalar strings before arena construction.
                let authored_number = |bytes: &Option<Vec<u8>>| {
                    bytes.as_deref().map(|bytes| {
                        std::str::from_utf8(bytes)
                            .expect("validated D2 geometry bytes")
                            .parse::<i64>()
                            .expect("validated D2 geometry integer") as f64
                    })
                };
                let (desired_width, desired_height) = match &node.d2_authored_geometry {
                    Some(attrs) => (
                        authored_number(&attrs.width),
                        authored_number(&attrs.height),
                    ),
                    None => input.tala_desired_dimensions.get(&id).copied().map_or_else(
                        || {
                            (
                                node.fixed_width.then_some(node.size.width),
                                node.fixed_height.then_some(node.size.height),
                            )
                        },
                        |[width, height]| (width, height),
                    ),
                };
                let fixed_top_left = match &node.d2_authored_geometry {
                    Some(attrs) => authored_number(&attrs.left)
                        .zip(authored_number(&attrs.top))
                        .map(|(x, y)| Point { x, y }),
                    None => node.locked_position.or_else(|| {
                        Some(Point {
                            x: node.constrained_x?,
                            y: node.constrained_y?,
                        })
                    }),
                };
                let icon_position_fixed = node.has_icon
                    && node
                        .icon_position
                        .is_some_and(|position| position != LabelPosition::Unset);
                // adapter.translateGraph creates a TALA Label only when
                // Object.HasLabel is true. Graph label dimensions also exist
                // for text/content shapes and blank boxes, but are not a
                // retained layout label on those objects.
                let label_text_present = input
                    .tala_decoded_label_presence
                    .get(&id)
                    .copied()
                    .unwrap_or_else(|| {
                        !matches!(
                            node.shape,
                            crate::ShapeKind::Text
                                | crate::ShapeKind::Code
                                | crate::ShapeKind::Class
                                | crate::ShapeKind::SqlTable
                        ) && input
                            .node_labels
                            .get(&id)
                            .map_or(!node.external_id.is_empty(), |text| !text.is_empty())
                    });
                let authored_label_position = node
                    .d2_authored_geometry
                    .as_ref()
                    .and_then(|geometry| geometry.attributes.as_deref())
                    .is_some_and(|attributes| attributes.label_position.is_some());
                let label_position_fixed = label_text_present
                    && node.label_position != LabelPosition::Unset
                    && (node.d2_authored_geometry.is_none() || authored_label_position)
                    && !node.external_label.is_some_and(|label| label.automatic);
                let mut node_padding = Insets::uniform(0.0);
                let mut layout_margins = node.layout_margins;
                if childless_grid
                    && let Some(label) = node.external_label.filter(|label| label.automatic)
                {
                    match label.side {
                        ExternalSide::Top => layout_margins.top = 0.0,
                        ExternalSide::Right => layout_margins.right = 0.0,
                        ExternalSide::Bottom => layout_margins.bottom = 0.0,
                        ExternalSide::Left => layout_margins.left = 0.0,
                    }
                }
                if label_position_fixed && let Some(label) = node.label_size {
                    let width = label.width + 10.0;
                    let height = label.height + 10.0;
                    match node.label_position {
                        LabelPosition::OutsideTopLeft
                        | LabelPosition::OutsideTopCenter
                        | LabelPosition::OutsideTopRight => layout_margins.top = height,
                        LabelPosition::OutsideBottomLeft
                        | LabelPosition::OutsideBottomCenter
                        | LabelPosition::OutsideBottomRight => layout_margins.bottom = height,
                        LabelPosition::OutsideLeftTop
                        | LabelPosition::OutsideLeftMiddle
                        | LabelPosition::OutsideLeftBottom => layout_margins.left = width,
                        LabelPosition::OutsideRightTop
                        | LabelPosition::OutsideRightMiddle
                        | LabelPosition::OutsideRightBottom => layout_margins.right = width,
                        LabelPosition::InsideTopLeft
                        | LabelPosition::InsideTopCenter
                        | LabelPosition::InsideTopRight => node_padding.top = height,
                        LabelPosition::InsideBottomLeft
                        | LabelPosition::InsideBottomCenter
                        | LabelPosition::InsideBottomRight => node_padding.bottom = height,
                        LabelPosition::InsideMiddleLeft => node_padding.left = width,
                        LabelPosition::InsideMiddleRight => node_padding.right = width,
                        _ => {}
                    }
                }
                // Mirror Graph.ComputeNodeSpacing -> Node.UpdateSpacing.
                // Explicit outside icon positions reserve the icon's maximum
                // rendered extent in the node margin, including image shapes.
                // Inside positions reserve that extent as node padding.
                if icon_position_fixed && let Some(position) = node.icon_position {
                    match position {
                        LabelPosition::InsideTopLeft
                        | LabelPosition::InsideTopCenter
                        | LabelPosition::InsideTopRight => {
                            node_padding.top = node_padding.top.max(74.0)
                        }
                        LabelPosition::InsideBottomLeft
                        | LabelPosition::InsideBottomCenter
                        | LabelPosition::InsideBottomRight => {
                            node_padding.bottom = node_padding.bottom.max(74.0)
                        }
                        LabelPosition::InsideMiddleLeft => {
                            node_padding.left = node_padding.left.max(74.0)
                        }
                        LabelPosition::InsideMiddleRight => {
                            node_padding.right = node_padding.right.max(74.0)
                        }
                        LabelPosition::OutsideTopLeft
                        | LabelPosition::OutsideTopCenter
                        | LabelPosition::OutsideTopRight => {
                            layout_margins.top = layout_margins.top.max(74.0)
                        }
                        LabelPosition::OutsideBottomLeft
                        | LabelPosition::OutsideBottomCenter
                        | LabelPosition::OutsideBottomRight => {
                            layout_margins.bottom = layout_margins.bottom.max(74.0)
                        }
                        LabelPosition::OutsideLeftTop
                        | LabelPosition::OutsideLeftMiddle
                        | LabelPosition::OutsideLeftBottom => {
                            layout_margins.left = layout_margins.left.max(74.0)
                        }
                        LabelPosition::OutsideRightTop
                        | LabelPosition::OutsideRightMiddle
                        | LabelPosition::OutsideRightBottom => {
                            layout_margins.right = layout_margins.right.max(74.0)
                        }
                        _ => {}
                    }
                }
                if input.tala_decoded_members.is_none() {
                    containers.append_node(node.parent, id);
                }
                if let Some(direction) = node.direction {
                    directions.insert(Some(id), direction);
                    scoring_directions.insert(Some(id), direction);
                }
                ArenaNode {
                    label_text_present,
                    input_id: id,
                    external_id: node.external_id.clone(),
                    d2_id: node
                        .d2_authored_geometry
                        .as_ref()
                        .map(|_| node.external_id.clone()),
                    tala_id: input
                        .tala_id_overrides
                        .get(id.0 as usize)
                        .and_then(|override_id| *override_id)
                        .unwrap_or_else(|| fnv1a32(node.external_id.as_bytes())),
                    rect: Rect {
                        origin: Point::default(),
                        size: node.size,
                    },
                    declared_size: node.declared_size,
                    layout_margins,
                    position: fixed_top_left.or_else(|| {
                        input
                            .initial_positions
                            .get(id.0 as usize)
                            .copied()
                            .flatten()
                    }),
                    initial_position: input
                        .initial_positions
                        .get(id.0 as usize)
                        .copied()
                        .flatten(),
                    unpositioned_scope_translation: None,
                    scope_translation_materialized: false,
                    edges: IncidentEdges::default(),
                    loop_offsets: input
                        .tala_node_loop_offsets
                        .get(id.0 as usize)
                        .cloned()
                        .flatten(),
                    nears: NodeNearMap::allocated(),
                    herd_assignment: None,
                    container: node.parent,
                    fixed_top_left,
                    force_hierarchy: node.force_hierarchy,
                    hierarchy: None,
                    sequence: None,
                    cluster: None,
                    is_sequence_diagram: input.is_sequence_diagram(id),
                    desired_width,
                    desired_height,
                    folded_label_min_size: None,
                    label_size: node.label_size.filter(|_| label_text_present),
                    font_size: input.tala_text_font_size(id),
                    label_position: if label_text_present {
                        node.label_position
                    } else {
                        LabelPosition::Unset
                    },
                    label_position_fixed,
                    shape: node.shape,
                    is_invisible: input.node_is_invisible(id),
                    is_container: input
                        .container_flags
                        .get(id.0 as usize)
                        .copied()
                        .unwrap_or(false),
                    scoring_is_container: false,
                    scoring_container_parent: None,
                    scoring_cluster_arrangement: None,
                    scoring_is_aggregate_vessel: false,
                    scoring_cluster_vessel: None,
                    scoring_container_ancestors: Vec::new(),
                    owner_geometry_source: None,
                    owner_geometry_anchor: None,
                    transaction_anchor_tala_id: None,
                    transaction_anchor_offset: None,
                    transaction_position_was_nil: false,
                    transaction_validation_only: false,
                    content_insets: node.content_insets,
                    node_padding,
                    // LayoutNested leaves an empty grid in its ordinary parent
                    // graph, and translateGraph does not copy GridRows or
                    // GridColumns into the internal TALA node. Keep the D2
                    // object's container label metrics above, but do not let
                    // recovered nonempty-grid packing reinterpret this leaf.
                    grid_rows: (!childless_grid).then_some(node.grid_rows).flatten(),
                    grid_columns: (!childless_grid).then_some(node.grid_columns).flatten(),
                    canvas_position: node.canvas_position,
                    label_aware_grid: node.label_aware_grid,
                    packed_grid: node.packed_grid,
                    is_3d: node.is_3d,
                    is_multiple: node.is_multiple,
                    external_label: node
                        .external_label
                        .filter(|label| !(childless_grid && label.automatic)),
                    icon_position: node.icon_position,
                    icon_position_fixed,
                    has_icon: node.has_icon,
                    table_column_count: input.table_column_count(id),
                }
            })
            .collect();
        // Node.UpdateSpacing adds modifier protrusions after fixed labels and
        // icons have established the base margins. These are collision
        // clearances as well as bounding-box extents.
        for node in &mut nodes {
            let (dx, dy) = super::bounds::modifier_element_adjustments(node);
            if dx != 0.0 || dy != 0.0 {
                node.layout_margins.top += dy;
                node.layout_margins.right += dx;
            }
        }
        // The Go adapter inserts siblings from `ChildrenArray`, while the
        // serialized object records are commonly parent-first rather than
        // sibling-ordered. Reapply the explicit source order before every
        // reverse-DFS traversal; unspecified synthetic/test nodes retain
        // their existing insertion order.
        for (parent, desired) in &input.hierarchy_children_order {
            if input.tala_decoded_members.is_some() {
                // codec.go appends exactly the supplied children. The pinned
                // Go compiler uses growsliceBuf(..., 4) for this local slice.
                let header = SharedNodeSlice::from_go127_buffered_appends(desired)
                    .expect("decoded container capacity");
                containers.replace_header(*parent, header);
                continue;
            }
            // codec.go preserves every explicitly supplied container key,
            // including an allocated empty child slice. Sorting alone does
            // nothing when no node append created that map entry.
            if containers.get_header(parent).is_none() {
                containers.insert(*parent, Vec::new());
            }
            containers.sort_by_key(parent, |child| {
                desired
                    .iter()
                    .position(|candidate| candidate == child)
                    .unwrap_or(usize::MAX)
            });
        }
        for children in containers.values() {
            for child in children {
                if let Some(parent) = nodes[child.0 as usize].container {
                    nodes[parent.0 as usize].is_container = true;
                }
            }
        }
        for index in 0..nodes.len() {
            nodes[index].scoring_container_parent = nodes[index]
                .container
                .map(|parent| nodes[parent.0 as usize].tala_id);
            let mut current = nodes[index].container;
            while let Some(container) = current {
                let tala_id = nodes[container.0 as usize].tala_id;
                let parent = nodes[container.0 as usize].container;
                nodes[index].scoring_container_ancestors.push(tala_id);
                current = parent;
            }
        }
        let flat_root_scope = nodes.iter().all(|node| node.container.is_none());
        let source_edge_endpoints = input
            .edges()
            .map(|(edge_id, edge)| {
                let edge_tala_id = input
                    .tala_edge_ids
                    .get(edge_id.0 as usize)
                    .copied()
                    .flatten()
                    .unwrap_or(edge_id.0 as i64);
                let source = input
                    .source_edge_endpoints_by_id
                    .get(&edge_tala_id)
                    .copied()
                    .or_else(|| input.source_edge_endpoints.get(edge_id.0 as usize).copied())
                    .unwrap_or_else(|| {
                        (
                            nodes[edge.source.0 as usize].tala_id,
                            nodes[edge.target.0 as usize].tala_id,
                        )
                    });
                if crate::internal::trace_env_enabled("TALA_TRACE_SOURCE_EDGES") && edge_id.0 < 8 {
                    eprintln!(
                        "SOURCE_EDGE_PICK edge={} tala={} current=({}, {}) source={:?}",
                        edge_id.0,
                        edge_tala_id,
                        nodes[edge.source.0 as usize].tala_id,
                        nodes[edge.target.0 as usize].tala_id,
                        source
                    );
                }
                source
            })
            .collect::<Vec<_>>();
        let decoded_edges = input
            .tala_decoded_members
            .as_ref()
            .map(|(_, edges)| edges.iter().copied().collect::<BTreeSet<_>>());
        let edges: Vec<_> = input
            .edges()
            .map(|(id, edge)| {
                // Only graph.Edges entries call AddDecodedEdge. Tree-only
                // sentinel records are interned without endpoint incidence.
                if decoded_edges
                    .as_ref()
                    .is_none_or(|members| members.contains(&id))
                {
                    nodes[edge.source.0 as usize].edges.push(id);
                    if edge.source != edge.target {
                        nodes[edge.target.0 as usize].edges.push(id);
                    }
                }
                let arrowhead_labels = input.edge_arrowhead_labels(id);
                let mut arrowheads = input.edge_arrowheads(id);
                let arrows = input.edge_arrows(id);
                // The Go adapter materializes default identities even when
                // there is no authored arrowhead record. This matters to
                // cluster signatures and shared-port shape comparisons.
                arrowheads
                    .source
                    .get_or_insert_with(|| if arrows.source { "triangle" } else { "none" }.into());
                arrowheads
                    .target
                    .get_or_insert_with(|| if arrows.target { "triangle" } else { "none" }.into());
                let table_columns = input.edge_table_columns(id);
                let main_label = input.edge_label(id).cloned();
                let style = input.edge_style(id);
                let opacity_zero = style
                    .opacity
                    .as_deref()
                    .and_then(d2_rs_core::parse_d2_float)
                    .is_some_and(|value| value == 0.0);
                let invisible = input
                    .tala_edge_invisible
                    .get(id.0 as usize)
                    .copied()
                    .flatten()
                    .unwrap_or_else(|| {
                        opacity_zero
                            || style
                                .stroke
                                .as_deref()
                                .is_some_and(|value| value.eq_ignore_ascii_case("transparent"))
                    });
                // Recovered d2transpiler behavior: main label dimensions plus
                // ten units, followed by each arrowhead label's largest extent
                // plus five. These dimensions belong to the edge and survive
                // abduction into temporary hierarchy and cluster owners.
                let mut min_width = input.d2_edge_validation(id).map_or_else(
                    || {
                        main_label
                            .as_ref()
                            .map_or(10.0, |label| label.size.width + 10.0)
                    },
                    |attrs| attrs.attributes.label_dimensions[0] as f64 + 10.0,
                );
                let mut min_height = input.d2_edge_validation(id).map_or_else(
                    || {
                        main_label
                            .as_ref()
                            .map_or(10.0, |label| label.size.height + 10.0)
                    },
                    |attrs| attrs.attributes.label_dimensions[1] as f64 + 10.0,
                );
                for label in [
                    arrowhead_labels.source.as_ref(),
                    arrowhead_labels.target.as_ref(),
                ]
                .into_iter()
                .flatten()
                {
                    let extent = label.size.width.max(label.size.height) + 5.0;
                    min_width += extent;
                    min_height += extent;
                }
                if opacity_zero {
                    min_width = 0.0;
                    min_height = 0.0;
                }
                if let Some(width) = input.edge_min_widths.get(id.0 as usize).copied().flatten() {
                    min_width = width;
                }
                if let Some(height) = input.edge_min_heights.get(id.0 as usize).copied().flatten() {
                    min_height = height;
                }
                ArenaEdge {
                    input_id: id,
                    source_label_positions: input
                        .tala_edge_label_positions
                        .get(&id)
                        .cloned()
                        .unwrap_or_default(),
                    invisible,
                    label_position_fixed: input
                        .tala_edge_label_fixed
                        .get(id.0 as usize)
                        .copied()
                        .unwrap_or(false),
                    tala_id: input
                        .tala_edge_ids
                        .get(id.0 as usize)
                        .copied()
                        .flatten()
                        .unwrap_or(0),
                    from: edge.source,
                    to: edge.target,
                    route: input.tala_decoded_routes.get(&id).map_or_else(
                        || RoutePoints::decoded(input.edge_route(id).iter().copied().map(Some)),
                        |points| RoutePoints::decoded(points.iter().copied()),
                    ),
                    is_curve: input.edge_is_curve(id),
                    // D2's TALA adapter applies explicit arrowhead records
                    // before HasSourceArrow/HasTargetArrow classify direction.
                    // Keep syntax flags in the public graph for other stages.
                    source_arrow: arrowheads
                        .source
                        .as_deref()
                        .map_or(input.edge_arrows(id).source, |shape| {
                            !shape.is_empty() && shape != "none"
                        }),
                    target_arrow: arrowheads
                        .target
                        .as_deref()
                        .map_or(input.edge_arrows(id).target, |shape| {
                            !shape.is_empty() && shape != "none"
                        }),
                    source_arrowhead: arrowheads.source,
                    target_arrowhead: arrowheads.target,
                    label: main_label,
                    source_arrowhead_label: arrowhead_labels.source,
                    target_arrowhead_label: arrowhead_labels.target,
                    style,
                    source_table_column: table_columns.source,
                    target_table_column: table_columns.target,
                    source_table_column_count: input.table_column_count(edge.source),
                    target_table_column_count: input.table_column_count(edge.target),
                    min_width,
                    min_height,
                }
            })
            .collect();
        let max_edge_delta = edges
            .iter()
            .map(|edge| edge.min_width.max(edge.min_height))
            .fold(0.0_f64, f64::max);
        let max_loop_extent = edges
            .iter()
            .filter(|edge| edge.from == edge.to)
            .map(|edge| {
                edge.label
                    .as_ref()
                    .map_or(30.0, |label| label.size.width + 37.0)
            })
            .fold(0.0_f64, f64::max);
        // Graph.Edges contains canonical allocation IDs, independent of
        // membership position and the public ArenaEdge.input_id metadata.
        let edge_order: Vec<EdgeId> = input.tala_decoded_members.as_ref().map_or_else(
            || (0..edges.len()).map(|index| EdgeId(index as u32)).collect(),
            |(_, members)| members.clone(),
        );
        let edge_order = if input.tala_decoded_members.is_some() {
            GraphEdges::from_header(
                owner_pointer_slices::SharedEdgeSlice::from_go127_buffered_appends(&edge_order)
                    .expect("decoded Graph.Edges"),
            )
        } else {
            GraphEdges::from_appends(&edge_order)
        };
        let mut directions_by_tala = BTreeMap::from([(None, root_direction)]);
        let mut scoring_directions_by_tala = input
            .explicit_direction
            .map(|direction| BTreeMap::from([(None, direction)]))
            .unwrap_or_default();
        for (id, input_node) in input.nodes() {
            let tala_id = nodes[id.0 as usize].tala_id;
            if let Some(direction) = input_node.direction {
                directions_by_tala.insert(Some(tala_id), direction);
                scoring_directions_by_tala.insert(Some(tala_id), direction);
            }
        }
        let source_container_child_order: BTreeMap<i64, Vec<i64>> = input
            .hierarchy_children_order
            .iter()
            .filter_map(|(parent, children)| {
                let parent = (*parent)?.0 as usize;
                Some((
                    nodes[parent].tala_id,
                    children
                        .iter()
                        .map(|child| nodes[child.0 as usize].tala_id)
                        .collect(),
                ))
            })
            .collect();
        let connected_container_child_order = source_container_child_order.clone();
        let source_positions_by_tala = if input.source_positions_by_tala.is_empty() {
            nodes
                .iter()
                .filter_map(|node| node.position.map(|position| (node.tala_id, position)))
                .collect::<BTreeMap<_, _>>()
        } else {
            input.source_positions_by_tala.clone()
        };
        let source_parent_tala_by_tala = if input.source_parent_tala_by_tala.is_empty() {
            nodes
                .iter()
                .map(|node| {
                    (
                        node.tala_id,
                        node.container
                            .and_then(|parent| nodes.get(parent.0 as usize))
                            .map(|parent| parent.tala_id),
                    )
                })
                .collect::<BTreeMap<_, _>>()
        } else {
            input.source_parent_tala_by_tala.clone()
        };
        // Graphjson inputs carry the stable edge IDs in the parallel TALA
        // metadata, while ordinary measured graphs only retain the indexed
        // endpoint vector. Rebuild the stable lookup at this boundary so
        // temporary placement graphs can resolve endpoints after densifying
        // their local `input_id` values.
        let mut source_edge_endpoints_by_id = input.source_edge_endpoints_by_id.clone();
        if source_edge_endpoints_by_id.is_empty() {
            for (edge, endpoints) in edges.iter().zip(&source_edge_endpoints) {
                if edge.tala_id != 0 {
                    source_edge_endpoints_by_id.insert(edge.tala_id, *endpoints);
                }
            }
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_SOURCE_POSITIONS") {
            eprintln!(
                "SOURCE_POSITIONS_RUST {:?}",
                source_positions_by_tala
                    .iter()
                    .map(|(id, position)| (*id, *position))
                    .collect::<Vec<_>>()
            );
        }
        let mut graph = Self {
            nodes,
            shared_topology: SharedTopology::default(),
            node_order: Vec::new(),
            node_order_membership: Vec::new(),
            node_order_membership_valid: false,
            active_sequence_flags: Vec::new(),
            active_cluster_flags: Vec::new(),
            active_sequence_indices: Vec::new(),
            active_cluster_indices: Vec::new(),
            active_sequence_owners: Vec::new(),
            active_cluster_owners: Vec::new(),
            active_node_containers: Arc::new(Vec::new()),
            active_aggregate_vessels_by_member: Arc::new(HashMap::new()),
            active_aggregate_vessel_nodes: Arc::new(HashMap::new()),
            node_order_tala_carriers: None,
            active_graph_node_order: Vec::new(),
            #[cfg(feature = "diagnostic-traces")]
            trace_owner_node_orders: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            #[cfg(feature = "diagnostic-traces")]
            trace_root_owner_node_orders: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            #[cfg(feature = "diagnostic-traces")]
            trace_root_owner_container_children: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            #[cfg(feature = "diagnostic-traces")]
            trace_container_placement_ranks: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            reachable_container_tala_ids: None,
            source_hierarchy_ordered: !input.hierarchy_children_order.is_empty(),
            source_container_child_order,
            connected_container_child_order,
            source_positions_by_tala,
            source_parent_tala_by_tala,
            edge_records: edges,
            source_edge_endpoints,
            source_edge_endpoints_by_id,
            edge_order,
            incident_edge_order: BTreeMap::new(),
            external_edge_counts: input.external_edge_counts.clone(),
            placement_scope_owned: flat_root_scope,
            root_hierarchy: input.root_hierarchy,
            containers,
            descendant_cache: Vec::new(),
            directions,
            scoring_directions,
            directions_by_tala,
            scoring_directions_by_tala,
            cell_size: 0.0,
            #[cfg(feature = "diagnostic-traces")]
            source_cell_size: input.serialized_cell_size.unwrap_or(0.0),
            cost_cache: Arc::new(GraphCostCache::new((0.0, 0.0, 0.0))),
            routing_costs: (0.0, 0.0, 0.0),
            max_spacing_delta: 120.0_f64
                .max(max_edge_delta)
                .max(20.0 + 2.0 * max_loop_extent),
            placement_component: vec![None; input.nodes().count()],
            common_uncle_siblings: GraphNodeMap::nil(),
            hubs: GraphNodeMap::new(),
            sized_adjacent_overrides: Default::default(),
            sized_cluster_distance_boxes: Default::default(),
            sized_edge_abductions: Default::default(),
            sized_edge_abductions_present: false,
            long_distance_neighbor_data: vec![None; input.nodes().count()],
            sized_projected_obstructions: Default::default(),
            sized_projected_obstruction_sets: Default::default(),
            sized_collapsed_symmetry_neighbors: Default::default(),
            projection_memo: ProjectionMemo::default(),
            suppress_aggregate_projection: false,
            swap_scoring_vessel_boxes: false,
            transaction_external_containers: Vec::new(),
            transaction_validation_external_containers: Vec::new(),
            transaction_external_container_children: BTreeMap::new(),
            trace_container_children: BTreeMap::new(),
            transaction_external_aggregate_children: BTreeMap::new(),
            transaction_external_cluster_layouts: BTreeMap::new(),
            projected_transaction_nodes: Vec::new(),
            root_grid_bin_pack_completed: false,
            sequences: Vec::new(),
            sequence_backlinks_complete: false,
            clusters: Vec::new(),
            cluster_vessel_sizes: Vec::new(),
            serialized_vessel_sizes_by_tala: BTreeMap::new(),
            serialized_vessel_positions_by_tala: BTreeMap::new(),
            serialized_cluster_arrangements_by_tala: BTreeMap::new(),
            serialized_cluster_edge_abductions: BTreeMap::new(),
            serialized_sequence_edge_abductions: BTreeMap::new(),
            serialized_cluster_containers_by_tala: BTreeMap::new(),
            pending_cluster_vessel_positions: BTreeMap::new(),
            cluster_backlinks_complete: false,
            preprocessed_tree_children: BTreeMap::new(),
            restored_tree_edges: BTreeSet::new(),
            published_tree_node_edge_order: Vec::new(),
            restored_hierarchy_endpoint_order: Vec::new(),
            tree_routing_nodes: BTreeMap::new(),
            tree_sentinels: BTreeSet::new(),
        };
        #[cfg(feature = "diagnostic-traces")]
        for (parent, children) in &graph.containers {
            let Some(parent_tala_id) = parent.map(|id| graph.nodes[id.0 as usize].tala_id) else {
                continue;
            };
            let projected = children
                .iter()
                .map(|child| graph.nodes[child.0 as usize].clone())
                .collect::<Vec<_>>();
            if !projected.is_empty() {
                graph
                    .trace_container_children
                    .insert(parent_tala_id, projected);
            }
        }
        #[cfg(feature = "diagnostic-traces")]
        if let Some(children) = graph.containers.get(&None) {
            let projected = children
                .iter()
                .map(|child| graph.nodes[child.0 as usize].clone())
                .collect::<Vec<_>>();
            if !projected.is_empty() {
                // The Go graph uses a nil container key for the source root;
                // zero is the stable diagnostic sentinel used by GraphJSON.
                graph.trace_container_children.insert(0, projected);
            }
        }
        graph.external_edge_counts.resize(graph.nodes.len(), 0);
        graph.rebuild_descendant_cache();
        // Serialized graphjson carries the owning Go graph's `Nodes` slice
        // separately from its map-backed container member order. Preserve
        // that explicit order when supplied.  For ordinary input, `Graph.Nodes`
        // is the insertion/source order produced by the adapter; hierarchy
        // children are a separate ordering used by container-local walks.
        graph.node_order = if let Some((members, _)) = &input.tala_decoded_members {
            members.clone()
        } else if input.node_order_override.is_empty() {
            (0..graph.nodes.len())
                .map(|index| NodeId(index as u32))
                .collect()
        } else {
            let mut seen = BTreeSet::new();
            let mut order = Vec::with_capacity(graph.nodes.len());
            for node in input.node_order_override.iter().copied() {
                if (node.0 as usize) < graph.nodes.len() && seen.insert(node) {
                    order.push(node);
                }
            }
            for node in graph.hierarchy_node_order() {
                if seen.insert(node) {
                    order.push(node);
                }
            }
            order
        };
        #[cfg(feature = "diagnostic-traces")]
        {
            let owner_nodes = graph
                .node_order
                .iter()
                .map(|node| graph.nodes[node.0 as usize].tala_id)
                .collect::<Vec<_>>();
            if let Ok(mut owners) = graph.trace_owner_node_orders.lock() {
                owners.clear();
                owners.extend(
                    graph
                        .nodes
                        .iter()
                        .map(|node| (node.tala_id, owner_nodes.clone())),
                );
            }
        }
        graph.node_order_membership = vec![false; graph.nodes.len()];
        graph.node_order_membership_valid = false;
        // Graphjson snapshots may already be in TALA's post-grouping state:
        // defining sequence edges have been consumed and aggregate vessels
        // live only in their dedicated maps. Restore those ownership maps
        // before the normal pipeline stages so their preprocessing observes
        // the same active carriers as Go's decoder.
        graph.load_serialized_sequences(&input.serialized_sequences);
        graph.load_serialized_clusters(&input.serialized_clusters);
        // codec.go's final publication filters members of active decoded
        // groups. A decoded vessel is active exactly when its Node.Graph is
        // nonnil: it came from Graph.Nodes, rather than a map-only record.
        // Keep the concrete vessel and retained member pointers separate; do
        // not append a numerical carrier or reconnect their actual edges.
        let decoded_nodes = graph
            .node_order
            .iter()
            .map(|node| graph.nodes[node.0 as usize].tala_id)
            .collect::<BTreeSet<_>>();
        // Node.Cluster/Sequence are overwritten in sorted decoder order.
        // An earlier active group must not hide a member whose final pointer
        // names a later inactive group.
        let mut cluster_active = BTreeMap::new();
        let mut sequence_active = BTreeMap::new();
        for group in &graph.clusters {
            for &member in &group.members {
                cluster_active.insert(member, decoded_nodes.contains(&group.vessel_tala_id));
            }
        }
        for group in &graph.sequences {
            for &member in &group.members {
                sequence_active.insert(member, decoded_nodes.contains(&group.vessel_tala_id));
            }
        }
        let filtered_members = cluster_active
            .into_iter()
            .chain(sequence_active)
            .filter_map(|(node, active)| active.then_some(node))
            .collect::<BTreeSet<_>>();
        let mut member_header = if input.tala_decoded_members.is_some() {
            // codec.go's final make uses original node count, before filtering.
            SharedNodeSlice::with_len_cap(0, graph.node_order.len())
                .expect("decoded graph node capacity")
        } else {
            SharedNodeSlice::allocated_empty()
        };
        graph
            .node_order
            .retain(|node| !filtered_members.contains(node));
        for &node in &graph.node_order {
            member_header = member_header
                .append_node(node)
                .expect("initial graph node append");
        }
        // codec.go 1270–1294 publishes allocated map and append-built values.
        for hub in &input.serialized_hubs {
            let resolve = |tala_id| {
                graph
                    .nodes
                    .iter()
                    .position(|node| node.tala_id == tala_id)
                    .map(|index| NodeId(index as u32))
                    .expect("validated decoded hub node")
            };
            let spokes = hub
                .spoke_tala_ids
                .iter()
                .map(|&id| resolve(id))
                .collect::<Vec<_>>();
            graph
                .hubs
                .header()
                .replace(
                    Some(resolve(hub.hub_tala_id)),
                    SharedNodeSlice::from_go127_buffered_appends(&spokes)
                        .expect("decoded hub append"),
                )
                .expect("allocated decoded hub map");
        }
        graph.initialize_shared_topology_with_members(member_header);
        for &node in &input.tala_decoded_nil_owners {
            graph
                .set_owner_node_graph_reference(node, None)
                .expect("interned decoded vessel");
        }
        graph.resolve_serialized_aggregate_edge_abductions(
            &input.serialized_clusters,
            &input.serialized_sequences,
            &input.tala_decoded_edge_records,
        );
        if input.tala_decoded_members.is_some() {
            graph.publish_decoded_owner_records(input);
        }
        // Serialized cluster ownership is now restored at the same boundary
        // as sequences. AddClusters remains responsible for ordinary source
        // graphs, while graphjson snapshots retain the decoded vessel state
        // and its member backlinks before any placement scope is entered.
        // D2's serialized node attribute names one peer, but TALA's
        // d2transpiler materializes it through Node.AddNear on both nodes.
        // Original D2 keys are re-resolved against this graph's current
        // tree, exactly as adapter.findD2Object -> Node.AddNear. Extraction
        // can change their target; a compiler-cached Node.near is not enough.
        let mut explicit_nears =
            super::validation::resolve_d2_near_keys(input).expect("validated D2 near keys");
        explicit_nears.extend(
            input
                .nodes()
                .filter(|(_, node)| node.d2_near_key.is_none())
                .flat_map(|(id, node)| {
                    node.near
                        .into_iter()
                        .chain(
                            input
                                .near_targets
                                .get(id.0 as usize)
                                .into_iter()
                                .flatten()
                                .copied(),
                        )
                        .map(move |near| (id, near))
                }),
        );
        for (first, second) in explicit_nears {
            graph
                .add_owner_near(Some(first), Some(second))
                .expect("actual input Near nodes");
        }
        graph.compute_cell_size();
        // GraphJSON snapshots preserve the owning Go graph's already
        // materialized cell lattice and routing costs.  An input fixture has
        // CellSize == 0 and must be computed from its node sizes; an expected
        // snapshot has a nonzero value that the Go decoder publishes directly
        // before layout.  Recomputing that value changes the placement lattice
        // on replay and makes graph.exp fixtures non-idempotent.
        if let Some(cell_size) = input.serialized_cell_size.filter(|value| *value > 0.0) {
            graph.cell_size = cell_size;
        }
        if let Some((crossing, turn, non_center_port)) = input.serialized_routing_costs {
            graph.restore_routing_costs((crossing, turn, non_center_port));
            graph.routing_costs = (crossing, turn, non_center_port);
        }
        graph.refresh_placement_components();
        // Unmarshal publishes decoded Hierarchy pointers without running
        // AssignNodeHierarchy. The pipeline performs assignment later; doing
        // it here would erase the just-decoded records and their shared maps.
        if input.tala_decoded_members.is_none() {
            graph.assign_forced_hierarchies();
        }
        graph
    }

    pub(super) fn restore_rejoined_scoring_container_identity(&mut self) {
        // TALA's independently placed Graph values retain Node.isContainer
        // when their nodes are translated back into the main graph. The Rust
        // arena uses a separate carrier while constructing temporary scopes,
        // so restore that retained identity only at the corresponding rejoin
        // boundary rather than exposing it to earlier hierarchy operations.
        for node in &mut self.nodes {
            node.scoring_is_container = node.is_container;
        }
    }

    // Direct translation of recovered Graph.ComputeCellSize.
    pub(super) fn compute_cell_size(&mut self) {
        let current_nodes = self.graph_node_order();
        if current_nodes.is_empty() {
            self.cell_size = 10.0;
            return;
        }
        // TALA computes over the current Graph.Nodes slice. While sequence or
        // cluster vessels are active, their retired members are absent and
        // the vessel's aggregate dimensions participate as one node.
        let min_width = current_nodes
            .iter()
            .map(|node| self.active_node_size(*node).width)
            .fold(f64::INFINITY, f64::min);
        let min_height = current_nodes
            .iter()
            .map(|node| self.active_node_size(*node).height)
            .fold(f64::INFINITY, f64::min);
        let max_width = current_nodes
            .iter()
            .map(|node| self.active_node_size(*node).width)
            .fold(f64::NEG_INFINITY, f64::max);
        let max_height = current_nodes
            .iter()
            .map(|node| self.active_node_size(*node).height)
            .fold(f64::NEG_INFINITY, f64::max);
        let min_length = min_width.min(min_height);
        let max_length = max_width.max(max_height);
        self.cell_size = if 3.0 * min_length > max_length {
            max_length.ceil()
        } else {
            (3.0 * min_length * 0.5).ceil()
        }
        .max(10.0);
    }

    pub(super) fn member_edge_indices(&self) -> impl DoubleEndedIterator<Item = usize> + use<> {
        self.edge_order
            .read()
            .into_iter()
            .map(|edge| edge.0 as usize)
    }

    /// Enumerate Graph.Edges; retained canonical records are not members.
    pub(super) fn member_edges(&self) -> impl DoubleEndedIterator<Item = &ArenaEdge> {
        self.edge_order
            .read()
            .into_iter()
            .map(|edge| &self.edge_records[edge.0 as usize])
    }

    pub(super) fn member_edge_entries(
        &self,
    ) -> impl DoubleEndedIterator<Item = (usize, &ArenaEdge)> {
        self.edge_order
            .read()
            .into_iter()
            .map(|edge| (edge.0 as usize, &self.edge_records[edge.0 as usize]))
    }

    pub(super) fn adjacent(&self, node: NodeId, edge: EdgeId) -> NodeId {
        let edge = &self.edge_records[edge.0 as usize];
        // While a sequence/cluster is active, Graph.Edges point at stable
        // members but the recovered Go graph has reconnected them to the
        // temporary aggregate vessel. Project both endpoints before choosing
        // the opposite side so callers that retain the raw Node.Edges walk
        // the same live graph.
        let node_owner = self.active_aggregate_owner(node);
        let from = self.active_aggregate_owner(edge.from);
        let to = self.active_aggregate_owner(edge.to);
        if from == node_owner { to } else { from }
    }

    pub(super) fn position(&self, node: NodeId) -> Option<Point> {
        self.owner_bound_point_value(node)
            .unwrap_or_else(|| self.nodes[node.0 as usize].position)
    }

    #[track_caller]
    pub(super) fn set_position(&mut self, node: NodeId, position: Point) {
        if crate::internal::trace_env_value("TALA_TRACE_POSITION_NODE")
            .and_then(|value| value.parse::<i64>().ok())
            == Some(self.nodes[node.0 as usize].tala_id)
        {
            eprintln!(
                "SET_POSITION_RUST caller={} node={} previous={:?} next={},{} graphNodes={} placementOwned={} container={:?}",
                std::panic::Location::caller(),
                self.nodes[node.0 as usize].tala_id,
                self.nodes[node.0 as usize].position,
                position.x,
                position.y,
                self.nodes.len(),
                self.placement_scope_owned,
                self.nodes[node.0 as usize].container
            );
        }
        if !self.replace_bound_point_value(node, Some(position)) {
            self.nodes[node.0 as usize].position = Some(position);
        }
        self.nodes[node.0 as usize].rect.origin = position;
        self.nodes[node.0 as usize].unpositioned_scope_translation = None;
    }

    pub(super) fn clear_position(&mut self, node: NodeId) {
        self.replace_bound_point_value(node, None);
        self.nodes[node.0 as usize].position = None;
    }

    pub(super) fn rebuild_descendant_cache(&mut self) {
        // Shared header/backing writes cannot be invalidated by a local graph
        // cache. Keep no authoritative descendant image.
        self.descendant_cache.clear();
    }

    pub(super) fn descendants(&self, root: NodeId) -> Vec<NodeId> {
        let mut result = Vec::new();
        let mut pending = self.containers.get(&Some(root)).unwrap_or_default();
        while let Some(node) = pending.pop() {
            result.push(node);
            if let Some(children) = self.containers.get(&Some(node)) {
                pending.extend(children);
            }
        }
        result
    }

    /// Whether a subtree carries one of TALA's aggregate placement states.
    ///
    /// The cloned Rust scope needs frame replay for ordinary shared-pointer
    /// children, but aggregate vessels already publish their projected
    /// coordinates through their own materialization lifecycle.
    pub(super) fn subtree_has_special_placement_state(&self, root: NodeId) -> bool {
        std::iter::once(root)
            .chain(self.descendants(root))
            .any(|node| {
                let node_ref = &self.nodes[node.0 as usize];
                self.active_node_is_aggregate(node)
                    || node_ref.sequence.is_some()
                    || node_ref.cluster.is_some()
                    || node_ref.scoring_is_aggregate_vessel
                    || node_ref.grid_rows.is_some()
                    || node_ref.grid_columns.is_some()
                    || node_ref.label_aware_grid
                    || node_ref.packed_grid
            })
    }

    pub(super) fn subtree_has_sequence_or_grid_state(&self, root: NodeId) -> bool {
        std::iter::once(root)
            .chain(self.descendants(root))
            .any(|node| {
                let node_ref = &self.nodes[node.0 as usize];
                node_ref.sequence.is_some()
                    || node_ref.grid_rows.is_some()
                    || node_ref.grid_columns.is_some()
                    || node_ref.label_aware_grid
                    || node_ref.packed_grid
            })
    }

    /// The shared-pointer frame is also observable for TALA's queue fan-out
    /// composition, whose branching graph is not a simple path.
    pub(super) fn has_queue_shape(&self) -> bool {
        self.nodes.iter().any(|node| node.shape == ShapeKind::Queue)
    }

    pub(super) fn graph_is_path_like(&self) -> bool {
        let mut degrees = vec![0usize; self.nodes.len()];
        for edge in self.member_edges() {
            degrees[edge.from.0 as usize] += 1;
            degrees[edge.to.0 as usize] += 1;
        }
        degrees.into_iter().max().unwrap_or(0) <= 2
    }

    pub(super) fn is_descendant_of_scope(&self, node: NodeId, scope: Option<NodeId>) -> bool {
        let Some(scope) = scope else {
            return true;
        };
        // Recovered Node.isDescendentOf checks identity before following the
        // active ownership chain. Keep that loop allocation-free: the release
        // implementation has no visited-set guard, and valid TALA ownership
        // graphs are acyclic.
        self.is_descendant_of_any_scope(node, &[scope])
    }

    /// `is_descendant_of_scope(node, Some(scope))` for any of `scopes`, with
    /// one walk of `node`'s ownership chain.
    pub(super) fn is_descendant_of_any_scope(&self, node: NodeId, scopes: &[NodeId]) -> bool {
        let mut current = node;
        loop {
            if scopes.contains(&current) {
                return true;
            }
            let Some(parent) = self.scope_parent(current) else {
                return false;
            };
            current = parent;
        }
    }

    /// The ownership chain `node, parent, ...` that `is_descendant_of_scope`
    /// walks: `is_descendant_of_scope(node, Some(scope))` holds exactly when
    /// `scope` is in it.
    pub(super) fn scope_ancestor_chain(&self, node: NodeId) -> Vec<NodeId> {
        let mut chain = vec![node];
        let mut current = node;
        while let Some(parent) = self.scope_parent(current) {
            chain.push(parent);
            current = parent;
        }
        chain
    }

    /// One step of the recovered `Node.isDescendentOf` ownership chain.
    fn scope_parent(&self, current: NodeId) -> Option<NodeId> {
        let node_ref = &self.nodes[current.0 as usize];
        if let Some(container) = node_ref.container {
            Some(container)
        } else if let Some(cluster) = node_ref.cluster {
            self.nodes
                .iter()
                .find(|candidate| candidate.tala_id == self.clusters[cluster].vessel_tala_id)
                .map(|candidate| candidate.input_id)
        } else if let Some(sequence) = node_ref.sequence {
            self.nodes
                .iter()
                .find(|candidate| candidate.tala_id == self.sequences[sequence].vessel_tala_id)
                .map(|candidate| candidate.input_id)
        } else {
            None
        }
    }

    /// Reconstructs the source graph's `Node.isDescendentOf` relation by
    /// stable TALA identity.  Placement scopes are dense Rust projections,
    /// while Go's `Graph.CopyEntitiesFrom` keeps the original node pointers
    /// (and therefore their Container links) alive even when the parent is
    /// outside the projected `Nodes` slice.  The serialized parent map is the
    /// durable equivalent of those pointer links.
    pub(super) fn source_is_descendant_of_tala(&self, descendant: i64, ancestor: i64) -> bool {
        if descendant == ancestor {
            return true;
        }
        let mut current = descendant;
        let mut seen = BTreeSet::new();
        while seen.insert(current) {
            let Some(parent) = self
                .source_parent_tala_by_tala
                .get(&current)
                .copied()
                .flatten()
            else {
                return false;
            };
            if parent == ancestor {
                return true;
            }
            current = parent;
        }
        false
    }

    /// Recovered `Graph.getContainerFixedOrigin`.
    ///
    /// The origin is not a fixed node's requested top-left. TALA scans nodes
    /// in graph order for the first node in the requested placement container
    /// whose current and requested positions are both set, then returns their
    /// difference. During sizeless placement those positions intentionally
    /// occupy different coordinate domains; the optimizer subsequently
    /// divides this result by the graph cell size.
    pub(super) fn container_fixed_origin(&self, container: Option<NodeId>) -> Option<Point> {
        self.nodes.iter().find_map(|node| {
            if node.container != container {
                return None;
            }
            let current = node.position?;
            let fixed = node.fixed_top_left?;
            Some(Point {
                x: current.x - fixed.x,
                y: current.y - fixed.y,
            })
        })
    }

    pub(super) fn can_optimize_node(&self, node: NodeId) -> bool {
        let node_ref = &self.nodes[node.0 as usize];
        // sizeless_optimizer.go598-625 reads Node.Edges itself. An edge can
        // remain incident after Graph.Edges has been temporarily narrowed.
        // The guarded owner path supplies source work/errors for ancestry;
        // this convenience predicate supplies its ordinary eligibility value.
        let connected = self.owner_node_incident_count(node) != 0
            || node_ref
                .nears
                .raw_keys()
                .into_iter()
                .any(|near| match near {
                    Some(near) => self.is_descendant_of_scope(near, node_ref.container),
                    None => node_ref.container.is_none(),
                });
        if !connected || node_ref.fixed_top_left.is_some() {
            return false;
        }
        // The selected graph is this arena's root graph, independently of
        // node.Graph. Presence excludes the node even when the value is nil.
        // Before ownership initialization its source map is genuinely nil.
        if self.shared_topology_initialized() {
            return self
                .owner_graph_node_to_tree_entry(OwnerGraphId(0), Some(node))
                .expect("initialized arena retains its root graph")
                .is_none();
        }
        true
    }

    // Direct translation of Node.HasLeakyEdge. A container with no edge
    // attached to the container node itself still participates in
    // SwapOptimize when an edge crosses from one of its descendants to a node
    // outside the container.
    pub(super) fn has_leaky_edge(&self, node: NodeId) -> bool {
        if !self.nodes[node.0 as usize].is_container {
            return false;
        }
        self.descendants(node).into_iter().any(|descendant| {
            self.nodes[descendant.0 as usize]
                .edges
                .read()
                .iter()
                .copied()
                .any(|edge_id| {
                    let adjacent = self.adjacent(descendant, edge_id);
                    adjacent != node && !self.is_descendant_of_scope(adjacent, Some(node))
                })
        })
    }

    pub(super) fn optimizable_components(&self) -> Vec<Vec<NodeId>> {
        let nodes: Vec<_> = self
            .node_order
            .iter()
            .copied()
            .filter(|node| self.can_optimize_node(*node))
            .collect();
        let active: BTreeSet<_> = nodes.iter().copied().collect();
        let mut unseen = active.clone();
        let mut components: Vec<Vec<NodeId>> = Vec::new();
        for start in nodes.iter().copied() {
            if !unseen.remove(&start) {
                continue;
            }
            let mut members = BTreeSet::from([start]);
            let mut queue = VecDeque::from([start]);
            while let Some(node) = queue.pop_front() {
                let node_ref = &self.nodes[node.0 as usize];
                let mut adjacent: Vec<_> = node_ref
                    .nears
                    .keys()
                    .into_iter()
                    .filter(|candidate| active.contains(candidate))
                    .collect();
                adjacent.extend(
                    self.active_edge_ids(node)
                        .into_iter()
                        .map(|edge| self.active_adjacent(node, edge))
                        .filter(|candidate| active.contains(candidate)),
                );
                adjacent.extend(
                    nodes
                        .iter()
                        .copied()
                        .filter(|candidate| self.nodes[candidate.0 as usize].nears.contains(&node)),
                );
                for candidate in adjacent {
                    if unseen.remove(&candidate) {
                        members.insert(candidate);
                        queue.push_back(candidate);
                    }
                }
            }
            components.push(
                nodes
                    .iter()
                    .copied()
                    .filter(|node| members.contains(node))
                    .collect(),
            );
        }

        components
    }

    /// Node order observed by NewSizelessOptimizer/NewSizedOptimizer after
    /// SplitSubgraphs. The recovered flat path uses FIFO connectivity order.
    /// Nested placement still needs child-graph materialization; applying this
    /// traversal to today's whole-arena nested representation conflates graph
    /// scopes and is not equivalent to the Go pipeline.
    pub(super) fn optimizer_nodes(&self) -> Vec<NodeId> {
        let declaration_order = self
            .node_order
            .iter()
            .copied()
            .filter(|node| self.can_optimize_node(*node))
            .collect();
        self.placement_fifo_order(declaration_order)
    }

    /// Recovered `Graph.Nodes` order seen by `sizedOptimizer.optimize`.
    ///
    /// Unlike `NewSizelessOptimizer`, `NewSizedOptimizer` retains the complete
    /// `SplitSubgraphs` node slice. Its optimize loop shuffles every index and
    /// only then skips fixed, tree-owned, or otherwise unusable nodes, so those
    /// entries must remain in the worklist to preserve the shared RNG stream.
    pub(super) fn sized_optimizer_nodes(&self) -> Vec<NodeId> {
        let declaration_order = self.node_order.clone();
        if crate::internal::trace_env_enabled("TALA_TRACE_NODE_ORDER") {
            eprintln!(
                "NODE_ORDER_RUST placement_owned={} node_order={:?} containers={:?}",
                self.placement_scope_owned,
                declaration_order
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
                self.containers
                    .iter()
                    .map(|(scope, children)| (
                        scope.map(|node| self.nodes[node.0 as usize].tala_id),
                        children
                            .iter()
                            .map(|node| self.nodes[node.0 as usize].tala_id)
                            .collect::<Vec<_>>(),
                    ))
                    .collect::<Vec<_>>(),
            );
        }
        self.placement_fifo_order(declaration_order)
    }

    fn placement_fifo_order(&self, declaration_order: Vec<NodeId>) -> Vec<NodeId> {
        if !self.placement_scope_owned {
            return declaration_order;
        }

        let active = declaration_order.iter().copied().collect::<BTreeSet<_>>();
        let mut unseen = active.clone();
        let mut ordered = Vec::with_capacity(declaration_order.len());
        for start in declaration_order.iter().copied() {
            if !unseen.remove(&start) {
                continue;
            }
            let mut queue = VecDeque::from([start]);
            while let Some(node) = queue.pop_front() {
                ordered.push(node);
                let node_ref = &self.nodes[node.0 as usize];
                let mut adjacent = node_ref
                    .edges
                    .read()
                    .iter()
                    .copied()
                    .map(|edge| self.adjacent(node, edge))
                    .filter(|candidate| active.contains(candidate))
                    .collect::<Vec<_>>();
                let mut nears = node_ref.nears.keys();
                nears.sort_by_key(|near| self.nodes[near.0 as usize].tala_id);
                adjacent.extend(
                    nears
                        .into_iter()
                        .filter(|candidate| active.contains(candidate)),
                );
                adjacent.extend(
                    declaration_order
                        .iter()
                        .copied()
                        .filter(|candidate| self.nodes[candidate.0 as usize].nears.contains(&node)),
                );
                for candidate in adjacent {
                    if unseen.remove(&candidate) {
                        queue.push_back(candidate);
                    }
                }
            }
        }
        ordered
    }

    pub(super) fn largest_optimizable_component_size(&self) -> usize {
        self.optimizable_components()
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or(0)
    }

    /// Recovered `Graph.SplitSubgraphs(false, true, true)` partition.
    ///
    /// Fixed nodes and everything reachable from any of them are coalesced
    /// into the leading subgraph. Remaining subgraphs start in concrete node
    /// slice order and use FIFO edge-then-ordered-near traversal.
    pub(super) fn split_placement_subgraphs(&self) -> Vec<Vec<NodeId>> {
        if crate::internal::trace_env_enabled("TALA_TRACE_PLACEMENT_SUBGRAPHS") {
            for node in &self.node_order {
                let node_ref = &self.nodes[node.0 as usize];
                eprint!("PLACEMENT_ADJ_RUST node={} edges=", node_ref.tala_id);
                for edge in &node_ref.edges.read() {
                    let adjacent = self.adjacent(*node, *edge);
                    eprint!(
                        "{}:{},",
                        self.edge_records[edge.0 as usize].tala_id,
                        self.nodes[adjacent.0 as usize].tala_id
                    );
                }
                eprint!(" nears=");
                for near in node_ref.nears.keys() {
                    eprint!("{},", self.nodes[near.0 as usize].tala_id);
                }
                eprintln!();
            }
        }
        let mut visited = BTreeSet::new();
        let mut result = Vec::new();
        // PreprocessTrees physically removes retained branching-tree nodes
        // from Graph.Nodes. `traverseTrees=true` lets reachability inspect the
        // tree carrier, but those removed nodes are not members of the
        // SplitSubgraphs result; Graph.PlaceTrees reconnects them only after
        // ordinary placement. The stable Rust arena therefore needs an
        // explicit live-node filter here.
        // Aggregate carriers retained in the shared container map are
        // deliberately absent from the owning Graph.Nodes order. They remain
        // available to the direct/mirroring transaction views, but must not
        // become independent placement components.
        let live_nodes = self.node_order.iter().copied().collect::<BTreeSet<_>>();
        if crate::internal::trace_env_enabled("TALA_TRACE_TOPOLOGY") {
            for node in self.node_order.iter().copied() {
                let node_ref = &self.nodes[node.0 as usize];
                eprintln!(
                    "PLACEMENT_TOPOLOGY_RUST node={} container={} source_parent={:?} active_container={:?} aggregate={}",
                    node_ref.tala_id,
                    node_ref
                        .container
                        .map(|parent| self.nodes[parent.0 as usize].tala_id)
                        .map_or_else(|| "<nil>".to_owned(), |id| id.to_string()),
                    self.source_parent_tala_by_tala.get(&node_ref.tala_id),
                    self.active_node_container(node)
                        .map(|parent| self.nodes[parent.0 as usize].tala_id),
                    self.active_node_is_aggregate(node),
                );
            }
        }
        let fixed = self
            .node_order
            .iter()
            .copied()
            .filter(|node| self.nodes[node.0 as usize].fixed_top_left.is_some())
            .collect::<Vec<_>>();
        if crate::internal::trace_env_enabled("TALA_TRACE_PLACEMENT_SUBGRAPHS") {
            for wanted in [3677471320_i64, 1010209959_i64] {
                if let Some((index, _)) = self
                    .nodes
                    .iter()
                    .enumerate()
                    .find(|(_, node)| node.tala_id == wanted)
                {
                    eprintln!(
                        "PLACEMENT_ACTIVE_DESC_RUST node={} is_container={} scoring={} descendants={:?}",
                        wanted,
                        self.nodes[index].is_container,
                        self.nodes[index].scoring_is_container,
                        self.active_descendants(NodeId(index as u32))
                            .iter()
                            .map(|node| self.nodes[node.0 as usize].tala_id)
                            .collect::<Vec<_>>()
                    );
                    eprintln!(
                        "PLACEMENT_ANCESTORS_RUST node={} {:?}",
                        wanted, self.nodes[index].scoring_container_ancestors
                    );
                }
            }
        }

        let reachable = |starts: &[NodeId], already: &BTreeSet<NodeId>| {
            let mut members = Vec::new();
            let mut queued = already.clone();
            let mut queue = VecDeque::new();
            for start in starts {
                if queued.insert(*start) {
                    queue.push_back(*start);
                }
            }
            while let Some(current) = queue.pop_front() {
                members.push(current);
                let node = &self.nodes[current.0 as usize];
                let mut adjacent = node
                    .edges
                    .read()
                    .iter()
                    .copied()
                    .map(|edge| self.adjacent(current, edge))
                    .collect::<Vec<_>>();
                let mut nears = node.nears.keys();
                nears.sort_by_key(|near| self.nodes[near.0 as usize].tala_id);
                adjacent.extend(nears);
                for candidate in adjacent {
                    if live_nodes.contains(&candidate) && queued.insert(candidate) {
                        queue.push_back(candidate);
                    }
                }
                // SplitSubgraphs is called with includeContainers=true by
                // placeNodes. Go's reachable walk scans the owning
                // Graph.Nodes slice for every current node and queues
                // descendants in that slice order. The stable arena keeps
                // absorbed members allocated and has a separate child-map
                // traversal, but expanding that map here batches descendants
                // ahead of siblings and changes the induced Graph.Nodes order
                // (which in turn changes the seeded optimizer stream). Use the
                // current live node slice as the primary queue order and keep
                // aggregate child aliases as a fallback below.
                let current_tala_id = self.nodes[current.0 as usize].tala_id;
                for candidate in self.node_order.iter().copied() {
                    if !live_nodes.contains(&candidate) || queued.contains(&candidate) {
                        continue;
                    }
                    let candidate_tala_id = self.nodes[candidate.0 as usize].tala_id;
                    let source_related = self
                        .source_is_descendant_of_tala(current_tala_id, candidate_tala_id)
                        || self.source_is_descendant_of_tala(candidate_tala_id, current_tala_id);
                    if crate::internal::trace_env_enabled("TALA_TRACE_PLACEMENT_SUBGRAPHS")
                        && current_tala_id == 3677471320
                        && candidate_tala_id == 1010209959
                    {
                        eprintln!(
                            "PLACEMENT_SOURCE_RELATED_RUST current={} candidate={} value={} parents={:?}",
                            current_tala_id,
                            candidate_tala_id,
                            source_related,
                            self.source_parent_tala_by_tala
                        );
                    }
                    if source_related {
                        queued.insert(candidate);
                        queue.push_back(candidate);
                    }
                }
                // A synthetic aggregate vessel is the only Go node whose
                // hidden members are not present in Graph.Nodes. Preserve
                // their nested descendants for the owning live vessel after
                // the owning-slice scan, without letting the retired members
                // themselves become independent placement nodes.
                for candidate in self.active_descendants(current) {
                    if !live_nodes.contains(&candidate) || queued.contains(&candidate) {
                        continue;
                    }
                    let candidate_tala_id = self.nodes[candidate.0 as usize].tala_id;
                    let source_related = self
                        .source_is_descendant_of_tala(current_tala_id, candidate_tala_id)
                        || self.source_is_descendant_of_tala(candidate_tala_id, current_tala_id);
                    if source_related {
                        queued.insert(candidate);
                        queue.push_back(candidate);
                    }
                }
            }
            members
        };

        if !fixed.is_empty() {
            // Go's fixed-node branch calls addReachable once per fixed node
            // against one shared subgraph. That preserves discovery order:
            // an edge-connected target found from the first fixed node is
            // appended before a later isolated fixed node. Seeding one BFS
            // queue with every fixed node instead puts all fixed roots ahead
            // of their edge neighbors and changes Graph.Nodes order.
            let mut members = Vec::new();
            for start in &fixed {
                let discovered = reachable(&[*start], &visited);
                visited.extend(discovered.iter().copied());
                members.extend(discovered);
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_PLACEMENT_SUBGRAPHS") {
                eprintln!(
                    "PLACEMENT_REACH_RUST start=fixed members={:?}",
                    members
                        .iter()
                        .map(|node| self.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>()
                );
            }
            result.push(members);
        }
        for node in self.node_order.iter().copied() {
            if visited.contains(&node) {
                continue;
            }
            let members = reachable(&[node], &visited);
            if crate::internal::trace_env_enabled("TALA_TRACE_PLACEMENT_SUBGRAPHS") {
                eprintln!(
                    "PLACEMENT_REACH_RUST start={} members={:?}",
                    self.nodes[node.0 as usize].tala_id,
                    members
                        .iter()
                        .map(|member| self.nodes[member.0 as usize].tala_id)
                        .collect::<Vec<_>>()
                );
            }
            visited.extend(members.iter().copied());
            result.push(members);
        }
        #[cfg(feature = "diagnostic-traces")]
        if let Ok(mut owners) = self.trace_owner_node_orders.lock() {
            // SplitSubgraphs' AddNodeUnchecked calls assign every returned
            // member to its new Graph. placeNodes then calls
            // Nodes(subgraph.Nodes).SetGraphReference, which assigns that
            // same owner to each live container/aggregate descendant.
            for members in &result {
                let owner_order = members
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>();
                let mut reassigned = BTreeSet::new();
                for member in members {
                    reassigned.insert(*member);
                    reassigned.extend(self.active_descendants(*member));
                }
                for node in reassigned {
                    owners.insert(self.nodes[node.0 as usize].tala_id, owner_order.clone());
                }
            }
        }
        result
    }

    pub(super) fn abduction_current_tala_id(
        &self,
        endpoint: AbductionCurrentNodeRef,
    ) -> Option<i64> {
        endpoint
            .local_node()
            .map(|node| self.active_node_tala_id(node))
            .or(endpoint.tala_id)
    }

    pub(super) fn abduction_current_is_aggregate(&self, endpoint: AbductionCurrentNodeRef) -> bool {
        endpoint
            .local_node()
            .map_or(endpoint.is_aggregate_vessel, |node| {
                self.active_node_is_aggregate(node)
                    || self.nodes[node.0 as usize].scoring_is_aggregate_vessel
            })
    }

    /// Materializes one temporary graph produced by recovered
    /// `SplitSubgraphs`. IDs are made dense inside the temporary graph and the
    /// returned vector maps each new node back to its owner in this graph.
    pub(super) fn induced_placement_subgraph(&self, members: &[NodeId]) -> (Self, Vec<NodeId>) {
        let (graph, nodes, _) = self.induced_placement_subgraph_with_edge_mapping(members);
        (graph, nodes)
    }

    /// SplitSubgraphs first redirects every component's owning pointers.
    /// Preparing those bindings needs only the source correspondence, not a
    /// second materialization of all node geometry and projection inventories.
    pub(super) fn induced_placement_correspondence(
        &self,
        members: &[NodeId],
    ) -> (Vec<NodeId>, Vec<EdgeId>, Vec<NodeId>) {
        // Aggregate materialization may change which dense nodes are live.
        // Keep that path until its membership can be derived independently.
        if !self.clusters.is_empty() || !self.sequences.is_empty() {
            let (graph, nodes, edges) = self.induced_placement_subgraph_with_edge_mapping(members);
            return (nodes, edges, graph.graph_node_order());
        }
        let old_to_new = members
            .iter()
            .enumerate()
            .map(|(index, old)| (*old, NodeId(index as u32)))
            .collect::<crate::internal::IdHashMap<_, _>>();
        // Keep the same last-occurrence mapping as materialization, including
        // repeated edge pointers in a serialized Graph.Edges slice.
        let mut edge_indices = crate::internal::IdHashMap::default();
        for (index, (edge, _, _)) in self.induced_placement_edges(&old_to_new).enumerate() {
            edge_indices.insert(edge, index);
        }
        let mut edges = edge_indices.into_iter().collect::<Vec<_>>();
        edges.sort_by_key(|&(_, index)| index);
        (
            members.to_vec(),
            edges.into_iter().map(|(edge, _)| edge).collect(),
            (0..members.len())
                .map(|index| NodeId(index as u32))
                .collect(),
        )
    }

    fn induced_placement_edges<'a>(
        &'a self,
        old_to_new: &'a crate::internal::IdHashMap<NodeId, NodeId>,
    ) -> impl Iterator<Item = (EdgeId, NodeId, NodeId)> + 'a {
        // Edges incident to hidden aggregate members are reattached to their
        // temporary vessel. Preparation and materialization must use the same
        // owning-node correspondence, independently of numeric metadata.
        let endpoint = move |node| {
            let owner = self.active_aggregate_owner(node);
            old_to_new
                .get(&owner)
                .copied()
                .or_else(|| old_to_new.get(&node).copied())
        };
        self.edge_order
            .read()
            .into_iter()
            .filter_map(move |old_edge| {
                let edge = &self.edge_records[old_edge.0 as usize];
                Some((old_edge, endpoint(edge.from)?, endpoint(edge.to)?))
            })
    }

    /// Keep the independently constructed edge mapping alongside the node
    /// mapping. Source SplitSubgraphs retains the actual edge pointers; a
    /// compact input_id or repeated Tala ID cannot identify their owner.
    pub(super) fn induced_placement_subgraph_with_edge_mapping(
        &self,
        members: &[NodeId],
    ) -> (Self, Vec<NodeId>, Vec<EdgeId>) {
        if crate::internal::trace_env_enabled("TALA_TRACE_INDUCED_ALL")
            || (crate::internal::trace_env_enabled("TALA_TRACE_INDUCED") && members.len() == 4)
        {
            eprintln!(
                "INDUCED_FOUR_RUST members={:?}",
                members
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_INDUCED_ALL")
            || (crate::internal::trace_env_enabled("TALA_TRACE_INDUCED") && members.len() == 10)
        {
            eprintln!(
                "INDUCED_INPUT members={:?}",
                members
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
            for edge in self.member_edges() {
                if edge.tala_id == 1694388047
                    || edge.tala_id == 1761645618
                    || edge.tala_id == 3659036806
                    || edge.tala_id == 1888486037
                {
                    eprintln!(
                        "INDUCED_PARENT_EDGE t{} {}->{}",
                        edge.tala_id,
                        self.nodes[edge.from.0 as usize].tala_id,
                        self.nodes[edge.to.0 as usize].tala_id
                    );
                }
            }
        }
        let member_set = members
            .iter()
            .copied()
            .collect::<crate::internal::IdHashSet<_>>();
        let old_to_new = members
            .iter()
            .enumerate()
            .map(|(index, old)| (*old, NodeId(index as u32)))
            .collect::<crate::internal::IdHashMap<_, _>>();

        let mut nodes = members
            .iter()
            .copied()
            .map(|old| {
                // SplitSubgraphs receives the live Graph.Nodes pointers.  At
                // this point AddCluster/AddSequence has replaced every
                // aggregate member group with a fresh vessel pointer.  The
                // stable arena keeps the first member as the carrier, so
                // materialize the same plain vessel node in the induced
                // graph before direct placement.  Keeping the member's
                // original box here makes an enclosing graph optimize a
                // 128x128 child where TALA sees (for example) a 128x276
                // cluster vessel, changing every subsequent pack decision.
                let is_aggregate_owner =
                    self.active_node_is_aggregate(old) && self.active_aggregate_owner(old) == old;
                let induced_nears = |nears: &NodeNearMap| {
                    nears
                        .keys()
                        .iter()
                        .filter_map(|near| {
                            let owner = self.active_aggregate_owner(*near);
                            old_to_new.get(&owner).copied()
                        })
                        .collect::<NodeNearMap>()
                };
                // Both handles are replaced here (edges again below), so the
                // ordinary copy takes fresh ones instead of forking storage.
                let mut node = if is_aggregate_owner {
                    let mut node = self.projected_transaction_node_from_current(old).node;
                    node.edges.clear();
                    node.nears = induced_nears(&node.nears);
                    node
                } else {
                    let source = &self.nodes[old.0 as usize];
                    source
                        .clone_with_handles(IncidentEdges::default(), induced_nears(&source.nears))
                };
                // GraphJSON inputs may carry a prior (non-fixed) TopLeft.
                // Go's SplitSubgraphs keeps that live pointer when it adds a
                // node to an induced graph.  Preserve the current position
                // here as well; using only FixedTopLeft reset existing
                // layouts to the origin before the first placement pass.
                let prior_position = node.position;
                node.input_id = old_to_new[&old];
                node.container = None;
                node.position = if is_aggregate_owner {
                    self.active_node_position(old)
                } else {
                    prior_position.or(node.fixed_top_left)
                };
                node.rect.origin = node.position.unwrap_or_default();
                node
            })
            .collect::<Vec<_>>();

        let mut old_edge_to_new = crate::internal::IdHashMap::default();
        let mut edges = Vec::new();
        let mut induced_source_edge_endpoints = Vec::new();
        for (old_edge, from, to) in self.induced_placement_edges(&old_to_new) {
            let edge = &self.edge_records[old_edge.0 as usize];
            let new_edge = EdgeId(edges.len() as u32);
            old_edge_to_new.insert(old_edge, new_edge);
            // The temporary graph densifies `input_id`; carry the matching
            // serialized endpoint pair along with that new edge index. A
            // cloned global source table would make the second split scope
            // report the first scope's edge in diagnostic traces.
            let source_endpoints = self
                .source_edge_endpoints
                .get(edge.input_id.0 as usize)
                .copied()
                .unwrap_or_else(|| {
                    (
                        self.nodes[edge.from.0 as usize].tala_id,
                        self.nodes[edge.to.0 as usize].tala_id,
                    )
                });
            induced_source_edge_endpoints.push(source_endpoints);
            let mut edge = edge.clone();
            edge.input_id = new_edge;
            edge.from = from;
            edge.to = to;
            edges.push(edge);
        }
        // SplitSubgraphs adds the existing Node pointers to a fresh Graph.
        // Their endpoint-local Edges slices therefore retain reconnect order;
        // only Graph.Edges is rebuilt by the owning graph scan. Preserve that
        // distinction when dense Rust IDs materialize the temporary graph.
        for (new_index, old) in members.iter().copied().enumerate() {
            let edge_ids =
                if self.active_node_is_aggregate(old) && self.active_aggregate_owner(old) == old {
                    self.active_edge_ids(old).into_iter().collect::<Vec<_>>()
                } else {
                    self.nodes[old.0 as usize].edges.read()
                };
            nodes[new_index].edges = edge_ids
                .iter()
                .filter_map(|edge| old_edge_to_new.get(edge).copied())
                .collect();
        }

        let remap_adjacent = |adjacent: ProjectedAdjacent| {
            old_to_new
                .get(&adjacent.owner)
                .copied()
                .map(|owner| ProjectedAdjacent { owner, ..adjacent })
        };
        let remap_abducted_ref = |original: AbductedNodeRef| AbductedNodeRef {
            owner_tala_id: original
                .owner
                .map(|owner| self.active_node_tala_id(owner))
                .or(original.owner_tala_id),
            owner: original
                .owner
                .and_then(|owner| old_to_new.get(&owner).copied()),
            ..original
        };
        let remap_current_ref = |mut endpoint: AbductionCurrentNodeRef| {
            if let Some(node) = endpoint.local_node() {
                endpoint.tala_id = Some(self.active_node_tala_id(node));
                endpoint.container_tala_id = self.nodes[node.0 as usize]
                    .container
                    .map(|container| self.nodes[container.0 as usize].tala_id);
                endpoint.position = self.active_node_position(node);
                endpoint.size = Some(self.active_node_size(node));
                endpoint.is_aggregate_vessel = self.active_node_is_aggregate(node)
                    || self.nodes[node.0 as usize].scoring_is_aggregate_vessel;
                endpoint.location = old_to_new
                    .get(&node)
                    .copied()
                    .map(AbductionNodeLocation::Local)
                    .unwrap_or(AbductionNodeLocation::Foreign);
            }
            endpoint
        };
        let common_uncle_siblings = self
            .common_uncle_siblings
            .iter()
            .filter_map(|(old, siblings)| {
                let new = old_to_new.get(&old).copied()?;
                let siblings = siblings
                    .iter()
                    .filter_map(|sibling| old_to_new.get(sibling).copied())
                    .collect::<Vec<_>>();
                (!siblings.is_empty()).then_some((new, siblings))
            })
            .collect();
        let hubs = self
            .hubs
            .iter()
            .filter_map(|(old_hub, old_spokes)| {
                let hub = old_to_new.get(&old_hub).copied()?;
                let spokes = old_spokes
                    .iter()
                    .filter_map(|spoke| old_to_new.get(spoke).copied())
                    .collect::<Vec<_>>();
                (!spokes.is_empty()).then_some((hub, spokes))
            })
            .collect();
        let sized_adjacent_overrides = self
            .sized_adjacent_overrides
            .iter()
            .filter_map(|(&(old_node, old_edge), &adjacent)| {
                Some((
                    (
                        *old_to_new.get(&old_node)?,
                        *old_edge_to_new.get(&old_edge)?,
                    ),
                    remap_adjacent(adjacent)?,
                ))
            })
            .collect();
        let sized_cluster_distance_boxes = self
            .sized_cluster_distance_boxes
            .iter()
            .filter_map(|(&(old_owner, tala_id), geometry)| {
                let mut geometry = geometry.clone();
                geometry.external_connected = geometry
                    .external_connected
                    .iter()
                    .copied()
                    .filter_map(remap_adjacent)
                    .collect();
                Some(((*old_to_new.get(&old_owner)?, tala_id), geometry))
            })
            .collect();
        let remap_obstruction_inventory = |inventory: &OrderedObstructionInventory| {
            let mut inventory = inventory.clone();
            // SplitSubgraphs retains the source pointers. A dense component
            // whose local identities stay unchanged can retain this snapshot
            // too, rather than copying every obstruction in every edge view.
            let needs_remap = inventory.sets.iter().any(|set| {
                set.items.iter().flatten().any(|obstruction| {
                    if let AbductionNodeLocation::Local(owner) = obstruction.location {
                        if old_to_new.get(&owner).copied() != Some(owner)
                            || (obstruction.source_owner.is_some()
                                && obstruction.same_graph_as_replacement
                                    != old_to_new.contains_key(&owner))
                        {
                            return true;
                        }
                    }
                    obstruction.projection.is_some_and(|projection| {
                        old_to_new.get(&projection.owner).copied() != Some(projection.owner)
                    })
                })
            });
            if !needs_remap {
                return inventory;
            }
            inventory.projection_index = Default::default();
            for set in std::sync::Arc::make_mut(&mut inventory.sets) {
                for obstruction in set.items.iter_mut().flatten() {
                    if let AbductionNodeLocation::Local(owner) = obstruction.location {
                        if obstruction.source_owner.is_some() {
                            obstruction.same_graph_as_replacement = old_to_new.contains_key(&owner);
                        }
                        obstruction.location = old_to_new
                            .get(&owner)
                            .copied()
                            .map(AbductionNodeLocation::Local)
                            .unwrap_or(AbductionNodeLocation::Foreign);
                    }
                    obstruction.projection = obstruction.projection.and_then(remap_adjacent);
                }
            }
            inventory
        };
        let sized_edge_abductions: Vec<SizedEdgeAbduction> = self
            .sized_edge_abductions
            .iter()
            .map(|abduction| {
                // Go passes the owning slice unchanged to each component.
                // Foreign pointers stay nonnil; only their local materialized
                // edge/carrier is absent. Preserve every record and its order.
                SizedEdgeAbduction {
                    edge: abduction
                        .edge
                        .and_then(|edge| old_edge_to_new.get(&edge).copied()),
                    source_edge: abduction.source_edge,
                    edge_tala_id: abduction.edge_tala_id,
                    current_from: abduction.current_from.map(remap_current_ref),
                    current_to: abduction.current_to.map(remap_current_ref),
                    originally_from: abduction.originally_from.map(remap_abducted_ref),
                    originally_to: abduction.originally_to.map(remap_abducted_ref),
                    originally_from_table_neighbors: abduction
                        .originally_from_table_neighbors
                        .iter()
                        .filter_map(|neighbor| {
                            Some(ProjectedTableColumnNeighbor {
                                other: remap_adjacent(neighbor.other)?,
                                column_index: neighbor.column_index,
                            })
                        })
                        .collect(),
                    originally_to_table_neighbors: abduction
                        .originally_to_table_neighbors
                        .iter()
                        .filter_map(|neighbor| {
                            Some(ProjectedTableColumnNeighbor {
                                other: remap_adjacent(neighbor.other)?,
                                column_index: neighbor.column_index,
                            })
                        })
                        .collect(),
                    originally_from_container: abduction.originally_from_container,
                    originally_to_container: abduction.originally_to_container,
                    sequence_abduction: abduction.sequence_abduction,
                    obstructions_from_to: abduction
                        .obstructions_from_to
                        .iter()
                        .copied()
                        .filter_map(remap_adjacent)
                        .collect(),
                    obstructions_to_from: abduction
                        .obstructions_to_from
                        .iter()
                        .copied()
                        .filter_map(remap_adjacent)
                        .collect(),
                    obstruction_sets_from_to: remap_obstruction_inventory(
                        &abduction.obstruction_sets_from_to,
                    ),
                    obstruction_sets_to_from: remap_obstruction_inventory(
                        &abduction.obstruction_sets_to_from,
                    ),
                }
            })
            .collect();
        let sized_projected_obstructions = self
            .sized_projected_obstructions
            .iter()
            .filter_map(|(&(old_node, old_edge), obstructions)| {
                let new_node = *old_to_new.get(&old_node)?;
                let new_edge = *old_edge_to_new.get(&old_edge)?;
                let obstructions = if obstructions.iter().all(|projection| {
                    old_to_new.get(&projection.owner).copied() == Some(projection.owner)
                }) {
                    obstructions.clone()
                } else {
                    obstructions
                        .iter()
                        .copied()
                        .filter_map(remap_adjacent)
                        .collect::<SharedProjections>()
                };
                // An explicit empty inventory is material: recovered
                // edgeLength walked the restored endpoint-container chains
                // and found no eligible boxes. Dropping the key would fall
                // back to scanning every carrier in the temporary component.
                Some(((new_node, new_edge), obstructions))
            })
            .collect();
        let sized_projected_obstruction_sets = self
            .sized_projected_obstruction_sets
            .iter()
            .filter_map(|(&(old_node, old_edge), inventory)| {
                Some((
                    (
                        *old_to_new.get(&old_node)?,
                        *old_edge_to_new.get(&old_edge)?,
                    ),
                    remap_obstruction_inventory(inventory),
                ))
            })
            .collect();
        let sized_collapsed_symmetry_neighbors = self
            .sized_collapsed_symmetry_neighbors
            .iter()
            .filter_map(|(&original, neighbors)| {
                let neighbors = neighbors
                    .iter()
                    .copied()
                    .filter_map(remap_adjacent)
                    .collect::<Vec<_>>();
                (!neighbors.is_empty()).then_some((original, neighbors))
            })
            .collect();
        let member_tala_ids = members
            .iter()
            .map(|member| self.nodes[member.0 as usize].tala_id)
            .collect::<BTreeSet<_>>();
        if crate::internal::trace_env_value("TALA_TRACE_ABDUCTION_TARGET").is_some() {
            let target = crate::internal::trace_env_value("TALA_TRACE_ABDUCTION_TARGET")
                .and_then(|value| value.parse::<i64>().ok());
            eprintln!(
                "INDUCED_ABDUCTIONS_RUST members={:?} {:?}",
                member_tala_ids,
                sized_edge_abductions
                    .iter()
                    .filter(|abduction| {
                        target.is_none_or(|target| {
                            abduction
                                .originally_from
                                .is_some_and(|projected| projected.tala_id == target)
                                || abduction
                                    .originally_to
                                    .is_some_and(|projected| projected.tala_id == target)
                        })
                    })
                    .map(|abduction| (
                        abduction.edge,
                        abduction.current_from,
                        abduction.current_to,
                        abduction.originally_from,
                        abduction.originally_to,
                    ))
                    .collect::<Vec<_>>()
            );
        }
        let tree_sentinels = self
            .tree_sentinels
            .iter()
            .filter_map(|sentinel| old_to_new.get(sentinel).copied())
            .collect();
        let mut transaction_external_containers = self
            .transaction_external_containers
            .iter()
            .filter(|container| !member_tala_ids.contains(&container.tala_id))
            .cloned()
            .collect::<Vec<_>>();
        for node in &self.nodes {
            if node.is_container
                && node.position.is_some()
                && !member_set.contains(&node.input_id)
                && !transaction_external_containers
                    .iter()
                    .any(|container| container.tala_id == node.tala_id)
            {
                transaction_external_containers.push(node.clone());
            }
        }
        // `Graph.CopyEntitiesFrom` retains every shared container pointer in
        // the owning Graph.Nodes projection, even when a temporary placement
        // scope's local node slice contains only the direct members being
        // optimized.  Preserve those positioned container boxes as external
        // transaction carriers so `repositionContainers` and its overlap
        // validation can reject a candidate that collides with a sibling
        // hierarchy outside the local scope.
        for entry in &self.projected_transaction_nodes {
            let node = &entry.node;
            if node.is_container
                && node.position.is_some()
                && !member_set.contains(&node.input_id)
                && !transaction_external_containers
                    .iter()
                    .any(|container| container.tala_id == node.tala_id)
            {
                transaction_external_containers.push(node.clone());
            }
        }
        // `projected_transaction_nodes` mirrors the owning Graph.Nodes slice,
        // but its dense input IDs belong to the source graph.  During a split
        // scope an input ID can therefore collide with a local NodeId and the
        // membership check above would incorrectly discard an external
        // container.  Graph.CopyEntitiesFrom keeps every concrete
        // Graph.Containers entry, so recover those parents by their stable
        // container-map key instead. A validation-only materialized alias is
        // copied into a small induced graph so Go's shared pointer remains
        // visible during container validation. These aliases remain outside
        // the active node slice: they participate in transaction refitting
        // and overlap validation without becoming placement candidates.
        for &container_tala_id in self.transaction_external_container_children.keys() {
            if self
                .nodes
                .iter()
                .any(|node| node.tala_id == container_tala_id)
                || transaction_external_containers
                    .iter()
                    .any(|container| container.tala_id == container_tala_id)
            {
                continue;
            }
            let Some(mut container) = self
                .projected_transaction_nodes
                .iter()
                .find(|projected| projected.node.tala_id == container_tala_id)
                .map(|projected| projected.node.clone())
            else {
                continue;
            };
            // A concrete key in Go's Containers map remains a positioned
            // transaction container even when its source input index collides
            // with a member index in this induced arena. Preserve the map-key
            // identity and shared child projection instead of dropping it on
            // the dense-ID membership check above.
            if container.position.is_some() {
                container.is_container = true;
                container.scoring_is_container = true;
                container.transaction_validation_only = false;
                transaction_external_containers.push(container);
            }
        }
        transaction_external_containers.retain(|container| {
            if !container.transaction_validation_only {
                return true;
            }
            // A positioned alias whose ID is a concrete Containers-map key
            // still participates in Go's container validation even when its
            // split-scope coordinates are carried by the child projections.
            // Keep it as a validation-only carrier; transaction code resolves
            // its local box from those children below.
            if self
                .transaction_external_container_children
                .contains_key(&container.tala_id)
            {
                return true;
            }
            let Some(container_position) = self
                .source_positions_by_tala
                .get(&container.tala_id)
                .copied()
                .or(container.position)
            else {
                return false;
            };
            members.iter().any(|member| {
                let node = &self.nodes[member.0 as usize];
                // A materialized container is a validation obstacle for
                // ordinary nodes. Container-to-container spacing is handled
                // by the enclosing pack and is not copied into each sibling
                // induced graph; retaining it here changes candidate order.
                if node.is_container {
                    return false;
                }
                let Some(position) = self
                    .source_positions_by_tala
                    .get(&node.tala_id)
                    .copied()
                    .or(node.position)
                else {
                    return false;
                };
                let horizontal_gap = if position.x + node.rect.size.width < container_position.x {
                    container_position.x - (position.x + node.rect.size.width)
                } else if container_position.x + container.rect.size.width < position.x {
                    position.x - (container_position.x + container.rect.size.width)
                } else {
                    0.0
                };
                let vertical_gap = if position.y + node.rect.size.height < container_position.y {
                    container_position.y - (position.y + node.rect.size.height)
                } else if container_position.y + container.rect.size.height < position.y {
                    position.y - (container_position.y + container.rect.size.height)
                } else {
                    0.0
                };
                horizontal_gap <= 20.0 && vertical_gap <= 20.0
            })
        });
        for container in &mut transaction_external_containers {
            if self
                .transaction_external_container_children
                .contains_key(&container.tala_id)
                && self.projected_transaction_nodes.len() <= 64
            {
                container.transaction_validation_only = false;
            }
        }
        // The aggregate-member map is a flattened view of the shared
        // Graph.Containers hierarchy. Retain aggregate entries below any
        // induced member by walking the ordinary map-key edges; synthetic
        // aggregate vessels do not necessarily have a corresponding
        // ArenaNode in `self.nodes`.
        let mut aggregate_container_keys = member_tala_ids.clone();
        // `Graph.CopyEntitiesFrom` retains the complete Containers and
        // Clusters maps in every induced placement graph. A temporary
        // aggregate vessel is therefore a valid map root even when its
        // stable carrier is the only local node. Keep those vessel keys in
        // the projected map so SyncClusters can mutate the shared member
        // pointers at the same lifecycle boundary as the Go implementation.
        aggregate_container_keys.extend(
            nodes
                .iter()
                .filter(|node| node.scoring_is_aggregate_vessel)
                .map(|node| node.tala_id),
        );
        loop {
            let mut changed = false;
            for (&parent, children) in &self.transaction_external_container_children {
                if !aggregate_container_keys.contains(&parent) {
                    continue;
                }
                for child in children {
                    // Walk through ordinary container keys as well as
                    // aggregate vessels.  A nested cluster may sit behind
                    // one or more ordinary containers; Go's shared
                    // Containers map keeps that whole pointer chain visible
                    // to moveNodeWithChildren, so stopping at the first
                    // non-aggregate child loses the nested member boxes.
                    if self
                        .transaction_external_container_children
                        .contains_key(&child.tala_id)
                        || self
                            .transaction_external_aggregate_children
                            .contains_key(&child.tala_id)
                    {
                        changed |= aggregate_container_keys.insert(child.tala_id);
                    }
                }
            }
            if !changed {
                break;
            }
        }
        // `Graph.CopyEntitiesFrom` aliases the complete Containers map into
        // every split placement graph. Keep even sibling/unpositioned
        // container entries: Transaction's prior overlap state can reject a
        // move against one of those shared child pointers before the sibling
        // component itself is combined.
        let transaction_external_container_children =
            self.transaction_external_container_children.clone();
        let transaction_external_aggregate_children: BTreeMap<i64, Vec<ArenaNode>> = self
            .transaction_external_aggregate_children
            .iter()
            .filter(|(container, _)| aggregate_container_keys.contains(*container))
            .map(|(&container, children)| (container, children.clone()))
            .collect();
        let transaction_external_cluster_layouts = self
            .transaction_external_cluster_layouts
            .iter()
            .filter(|(vessel, _)| aggregate_container_keys.contains(*vessel))
            .map(|(&vessel, &layout)| (vessel, layout))
            .collect();
        if crate::internal::trace_env_value("TALA_TRACE_INDUCED_AGGREGATES").is_some() {
            eprintln!(
                "INDUCED_AGGREGATES_RUST members={:?} layouts={:?} aggregates={:?}",
                member_tala_ids,
                transaction_external_cluster_layouts,
                transaction_external_aggregate_children
                    .iter()
                    .map(|(vessel, children)| (
                        *vessel,
                        children
                            .iter()
                            .map(|child| (child.tala_id, child.position, child.rect.size))
                            .collect::<Vec<_>>(),
                    ))
                    .collect::<Vec<_>>()
            );
        }
        // `Graph.CopyEntitiesFrom` and `SplitSubgraphs` retain the owning
        // graph behind Cluster.Graph. Every induced component therefore sees
        // the same complete ordered Graph.Nodes carrier; only live aliases
        // are component-local.
        let projected_transaction_nodes = self.projected_transaction_nodes.clone();
        if crate::internal::trace_env_enabled("TALA_TRACE_MOVE_PROJECTION") {
            eprintln!(
                "MOVE_PROJECTION_INDUCED_RUST members={:?} hierarchy_keys={:?} source={:?} retained={:?} aggregate={:?}",
                member_tala_ids,
                aggregate_container_keys,
                self.transaction_external_container_children
                    .iter()
                    .map(|(key, children)| (
                        *key,
                        children
                            .iter()
                            .map(|child| (child.tala_id, child.transaction_anchor_tala_id))
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>(),
                transaction_external_container_children
                    .keys()
                    .collect::<Vec<_>>(),
                transaction_external_aggregate_children
                    .keys()
                    .collect::<Vec<_>>()
            );
            eprintln!(
                "INDUCED_EXTERNAL_POS_RUST members={:?} maps={:?}",
                member_tala_ids,
                transaction_external_container_children
                    .iter()
                    .map(|(parent, children)| (
                        *parent,
                        children
                            .iter()
                            .map(|child| (child.tala_id, child.position))
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>()
            );
        }
        // This is a compact numerical projection, not CopyEntitiesFrom.
        // Canonical owner headers cannot be indexed using these dense IDs.
        // Preserve source child order for the local traversal adapter; live
        // owner scoring requires an explicitly borrowed canonical frame.
        let induced_root_children = self
            .containers
            .get(&None)
            .into_iter()
            .flatten()
            .filter_map(|old| old_to_new.get(&old).copied())
            .chain(
                members
                    .iter()
                    .copied()
                    .filter(|old| {
                        !self
                            .containers
                            .get(&None)
                            .is_some_and(|children| children.contains(old))
                    })
                    .filter_map(|old| old_to_new.get(&old).copied()),
            )
            .collect::<Vec<_>>();
        // SplitSubgraphs keeps every reachable pointer in Graph.Containers,
        // but Node.Graph ownership remains the source graph's live Nodes
        // slice. Preserve that ownership set across this dense projection so
        // Graph.direct can ignore tree/child aliases exactly as Go does.
        let source_owner_order = if self.active_graph_node_order.is_empty() {
            &self.node_order
        } else {
            &self.active_graph_node_order
        };
        let source_owner_tala_ids = source_owner_order
            .iter()
            .map(|node| self.nodes[node.0 as usize].tala_id)
            .collect::<BTreeSet<_>>();
        // Retain the tree-node set separately: same-graph placement may have
        // reconnected these descendants before `Graph.direct`, while a split
        // component can still expose them in its shared Containers slice with
        // their Node.Graph pointer owned by the parent graph.
        let source_tree_owner_tala_ids = self
            .tree_routing_nodes
            .keys()
            .map(|node| self.nodes[node.0 as usize].tala_id)
            .collect::<BTreeSet<_>>();
        if crate::internal::trace_env_enabled("TALA_TRACE_DIRECT_ROOT") {
            eprintln!(
                "INDUCED_DIRECT_OWNERS source={:?} tree={:?} members={:?}",
                source_owner_tala_ids, source_tree_owner_tala_ids, member_tala_ids
            );
        }
        let direction = self
            .directions
            .get(&None)
            .copied()
            .unwrap_or(Direction::Down);
        let scoring_direction = self.scoring_directions.get(&None).copied();
        let mut graph = Self {
            nodes,
            shared_topology: SharedTopology::default(),
            node_order: (0..members.len())
                .map(|index| NodeId(index as u32))
                .collect(),
            node_order_membership: Vec::new(),
            node_order_membership_valid: false,
            active_sequence_flags: Vec::new(),
            active_cluster_flags: Vec::new(),
            active_sequence_indices: Vec::new(),
            active_cluster_indices: Vec::new(),
            active_sequence_owners: Vec::new(),
            active_cluster_owners: Vec::new(),
            active_node_containers: Arc::new(Vec::new()),
            active_aggregate_vessels_by_member: Arc::new(HashMap::new()),
            active_aggregate_vessel_nodes: Arc::new(HashMap::new()),
            node_order_tala_carriers: None,
            active_graph_node_order: Vec::new(),
            #[cfg(feature = "diagnostic-traces")]
            trace_owner_node_orders: self.trace_owner_node_orders.clone(),
            #[cfg(feature = "diagnostic-traces")]
            trace_root_owner_node_orders: self.trace_root_owner_node_orders.clone(),
            #[cfg(feature = "diagnostic-traces")]
            trace_root_owner_container_children: self.trace_root_owner_container_children.clone(),
            #[cfg(feature = "diagnostic-traces")]
            trace_container_placement_ranks: self.trace_container_placement_ranks.clone(),
            reachable_container_tala_ids: None,
            // SplitSubgraphs already supplies the live Graph.Nodes slice in
            // its recovered order. This temporary graph is not a serialized
            // D2 adapter boundary, so do not reinterpret copied parent links
            // as a fresh ChildrenArray hierarchy.
            source_hierarchy_ordered: false,
            source_container_child_order: self.source_container_child_order.clone(),
            connected_container_child_order: self.connected_container_child_order.clone(),
            source_positions_by_tala: self.source_positions_by_tala.clone(),
            source_parent_tala_by_tala: self.source_parent_tala_by_tala.clone(),
            edge_order: edges.iter().map(|edge| edge.input_id).collect(),
            edge_records: edges,
            source_edge_endpoints: induced_source_edge_endpoints,
            source_edge_endpoints_by_id: self.source_edge_endpoints_by_id.clone(),
            incident_edge_order: BTreeMap::new(),
            external_edge_counts: vec![0; members.len()],
            placement_scope_owned: true,
            root_hierarchy: self.root_hierarchy,
            containers: ContainerMap::from([(None, induced_root_children)]),
            descendant_cache: Vec::new(),
            directions: BTreeMap::from([(None, direction)]),
            scoring_directions: scoring_direction
                .map(|direction| BTreeMap::from([(None, direction)]))
                .unwrap_or_default(),
            directions_by_tala: self.directions_by_tala.clone(),
            scoring_directions_by_tala: self.scoring_directions_by_tala.clone(),
            cell_size: 0.0,
            #[cfg(feature = "diagnostic-traces")]
            source_cell_size: self.source_cell_size,
            cost_cache: Arc::new(GraphCostCache::new((0.0, 0.0, 0.0))),
            routing_costs: self.routing_costs,
            max_spacing_delta: self.max_spacing_delta,
            placement_component: vec![None; members.len()],
            common_uncle_siblings,
            hubs,
            sized_adjacent_overrides,
            sized_cluster_distance_boxes,
            sized_edge_abductions: sized_edge_abductions.into(),
            sized_edge_abductions_present: self.sized_edge_abductions_present,
            long_distance_neighbor_data: vec![None; members.len()],
            sized_projected_obstructions,
            sized_projected_obstruction_sets,
            sized_collapsed_symmetry_neighbors,
            projection_memo: ProjectionMemo::default(),
            suppress_aggregate_projection: false,
            swap_scoring_vessel_boxes: false,
            transaction_external_containers,
            transaction_validation_external_containers: Vec::new(),
            transaction_external_container_children,
            trace_container_children: self.trace_container_children.clone(),
            transaction_external_aggregate_children,
            transaction_external_cluster_layouts,
            projected_transaction_nodes,
            root_grid_bin_pack_completed: false,
            sequences: Vec::new(),
            sequence_backlinks_complete: false,
            clusters: Vec::new(),
            cluster_vessel_sizes: Vec::new(),
            serialized_vessel_sizes_by_tala: self.serialized_vessel_sizes_by_tala.clone(),
            serialized_vessel_positions_by_tala: self.serialized_vessel_positions_by_tala.clone(),
            serialized_cluster_arrangements_by_tala: self
                .serialized_cluster_arrangements_by_tala
                .clone(),
            serialized_cluster_edge_abductions: self.serialized_cluster_edge_abductions.clone(),
            serialized_sequence_edge_abductions: self.serialized_sequence_edge_abductions.clone(),
            serialized_cluster_containers_by_tala: self
                .serialized_cluster_containers_by_tala
                .clone(),
            pending_cluster_vessel_positions: BTreeMap::new(),
            cluster_backlinks_complete: false,
            preprocessed_tree_children: BTreeMap::new(),
            restored_tree_edges: BTreeSet::new(),
            published_tree_node_edge_order: Vec::new(),
            restored_hierarchy_endpoint_order: Vec::new(),
            tree_routing_nodes: BTreeMap::new(),
            tree_sentinels,
        };
        graph.active_graph_node_order = graph
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                source_owner_tala_ids
                    .contains(&node.tala_id)
                    .then_some(NodeId(index as u32))
            })
            .collect();
        if let Some(trace_target) = crate::internal::trace_env_value("TALA_TRACE_INDUCED_TARGET") {
            let wanted = trace_target == "all"
                || trace_target.parse::<i64>().ok().is_some_and(|target| {
                    member_tala_ids.contains(&target)
                        || self
                            .transaction_external_container_children
                            .values()
                            .flatten()
                            .any(|child| child.tala_id == target)
                });
            if wanted {
                let compact = |entries: &BTreeMap<i64, Vec<ArenaNode>>| {
                    entries
                        .iter()
                        .filter(|(parent, children)| {
                            **parent == 4254926550
                                || children.iter().any(|child| {
                                    matches!(child.tala_id, 4254926550 | 1369972579 | 3471367806)
                                })
                        })
                        .map(|(parent, children)| {
                            (
                                *parent,
                                children
                                    .iter()
                                    .map(|child| (child.tala_id, child.position))
                                    .collect::<Vec<_>>(),
                            )
                        })
                        .collect::<Vec<_>>()
                };
                eprintln!(
                    "INDUCED_TARGET_RUST members={:?} old_to_new={:?} source={:?} induced={:?}",
                    member_tala_ids,
                    old_to_new
                        .iter()
                        .collect::<BTreeMap<_, _>>()
                        .into_iter()
                        .filter_map(|(old, new)| {
                            matches!(
                                self.nodes[old.0 as usize].tala_id,
                                4254926550 | 1369972579 | 3471367806
                            )
                            .then_some((
                                self.nodes[old.0 as usize].tala_id,
                                graph.nodes[new.0 as usize].tala_id,
                            ))
                        })
                        .collect::<Vec<_>>(),
                    compact(&self.transaction_external_container_children),
                    compact(&graph.transaction_external_container_children),
                );
            }
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_INDUCED") && members.len() == 10 {
            eprintln!("INDUCED_OUTPUT edges={}", graph.edge_records.len());
            for (index, edge) in graph.member_edge_entries() {
                eprintln!(
                    "INDUCED_EDGE {} t{} {}->{}",
                    index,
                    edge.tala_id,
                    graph.nodes[edge.from.0 as usize].tala_id,
                    graph.nodes[edge.to.0 as usize].tala_id
                );
            }
        }
        graph.rebuild_descendant_cache();
        graph.compute_cell_size();
        graph.refresh_placement_components();
        if crate::internal::trace_env_enabled("TALA_TRACE_INDUCED_ORDER") {
            eprintln!(
                "INDUCED_ORDER_RUST members={:?} node_order={:?} nodes={:?}",
                member_tala_ids,
                graph
                    .node_order
                    .iter()
                    .map(|node| graph.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
                graph
                    .nodes
                    .iter()
                    .map(|node| node.tala_id)
                    .collect::<Vec<_>>()
            );
        }
        let mut parent_edges = old_edge_to_new
            .iter()
            .map(|(&parent, &local)| (local, parent))
            .collect::<Vec<_>>();
        parent_edges.sort_by_key(|&(local, _)| local);
        // A materialized component must expose its Node.Graph and graph map
        // fields before InitializeNodes, including readonly median lookups.
        // A later mutable owner API must not be needed to create this view.
        graph.initialize_shared_topology();
        (
            graph,
            members.to_vec(),
            parent_edges.into_iter().map(|(_, parent)| parent).collect(),
        )
    }

    /// Synchronize the stable arena's endpoint slices after recovered
    /// `putBackNonBranchingTrees` and `reconnectTree` have rebuilt the exact
    /// `Graph.Edges` order.
    pub(super) fn sync_edge_adjacency_order(&mut self) {
        // Disconnected canonical allocations are intentionally absent from
        // the published member list and must never be renumbered or restored.
        let recovered_rank = self
            .edge_order
            .read()
            .iter()
            .enumerate()
            .map(|(index, edge)| (*edge, index))
            .collect::<BTreeMap<_, _>>();
        for node in &mut self.nodes {
            node.edges.sort_by_key(|edge| recovered_rank[edge]);
        }
        // A temporary placeNodes graph shares Node pointers with its owner.
        // reconnectTree therefore leaves endpoint slices in root-first
        // reconnect order even though Graph.placeNodes publishes Graph.Edges
        // descendant-first. Replay that distinct pointer-visible lifecycle
        // after synchronizing all ordinary and non-branching-tree edges.
        for edge in self.published_tree_node_edge_order.clone() {
            self.reconnect_edge_adjacency_order(edge);
        }
        // RestoreEdgeAbductions appends only the endpoint that changed.
        // Replay completed scopes in recursive return order, after tree
        // publication has rebuilt the global edge inventory.
        for (node, edge) in self.restored_hierarchy_endpoint_order.clone() {
            let incident = &mut self.nodes[node.0 as usize].edges;
            incident.remove_first(edge);
            incident.push(edge);
        }
    }

    pub(super) fn reconnect_edge_adjacency_order(&mut self, edge: EdgeId) {
        let arena_edge = &self.edge_records[edge.0 as usize];
        let from = arena_edge.from;
        let to = arena_edge.to;
        self.nodes[from.0 as usize].edges.remove_first(edge);
        self.nodes[from.0 as usize].edges.push(edge);
        if to != from {
            self.nodes[to.0 as usize].edges.remove_first(edge);
            self.nodes[to.0 as usize].edges.push(edge);
        }
    }

    pub(super) fn refresh_placement_components(&mut self) {
        if crate::internal::trace_env_enabled("TALA_TRACE_REFRESH") {
            eprintln!(
                "REFRESH_RUST before nodes={} order={}",
                self.nodes.len(),
                self.node_order.len()
            );
        }
        self.placement_component.fill(None);
        let components = self.optimizable_components();
        if crate::internal::trace_env_enabled("TALA_TRACE_REFRESH") {
            eprintln!("REFRESH_RUST components={}", components.len());
        }
        for (index, component) in components.iter().enumerate() {
            for node in component {
                self.placement_component[node.0 as usize] = Some(index);
            }
        }
    }

    pub(super) fn same_placement_component(&self, left: NodeId, right: NodeId) -> bool {
        self.placement_component[left.0 as usize].is_some()
            && self.placement_component[left.0 as usize]
                == self.placement_component[right.0 as usize]
    }

    /// Recovered fixed-node anchoring in `placeNodesOrthogonally`.
    ///
    /// Sizeless placement temporarily represents fixed nodes on its coarse
    /// cell lattice. Before sized optimization, and again after it, TALA
    /// translates the complete subgraph by the first fixed node's displacement
    /// and restores every fixed node to its exact requested top-left.
    pub(super) fn anchor_fixed_positions(&mut self, move_fixed_children: bool) {
        let Some(anchor) = self
            .nodes
            .iter()
            .find(|node| node.fixed_top_left.is_some() && node.position.is_some())
            .map(|node| node.input_id)
        else {
            return;
        };
        let requested = self.nodes[anchor.0 as usize].fixed_top_left.unwrap();
        let current = self.position(anchor).unwrap();
        let delta = Point {
            x: requested.x - current.x,
            y: requested.y - current.y,
        };
        let nodes = self
            .nodes
            .iter()
            .map(|node| node.input_id)
            .collect::<Vec<_>>();
        for node in nodes.iter().copied() {
            // placeNodesOrthogonally moves each node with its live descendants,
            // including children retained outside the temporary scope graph.
            self.translate_node_with_children(node, delta);
        }
        for node in nodes {
            if let Some(requested) = self.nodes[node.0 as usize].fixed_top_left {
                if move_fixed_children {
                    self.move_node_abs_with_children(node, requested);
                } else {
                    self.set_position(node, requested);
                }
            }
        }
    }
}

#[cfg(feature = "diagnostic-traces")]
impl ArenaGraph {
    pub(super) fn observe_decoded_membership(&self, name: &str) -> serde_json::Value {
        use serde_json::json;
        let node_id = |id: Option<NodeId>| id.map(|id| self.nodes[id.0 as usize].tala_id);
        let header = self.owner_graph_member_header(OwnerGraphId(0)).unwrap();
        let mut inventory = self.nodes.iter().map(|node| json!({
            "id":node.tala_id,"owner":self.topology_node_owner(node.input_id)==Some(OwnerGraphId(0)),
            "parent":node_id(node.container),"is_container":node.is_container,
            "incidents":node.edges.read().iter().map(|edge|self.edge_records[edge.0 as usize].tala_id).collect::<Vec<_>>(),
            "incident_header":{"nil":node.edges.header().is_nil(),"len":node.edges.len(),"cap":node.edges.capacity(),
                "slots":(0..node.edges.capacity()).map(|i|node.edges.header().read_capacity_slot(i).unwrap()
                    .map(|edge|self.edge_records[edge.0 as usize].tala_id)).collect::<Vec<_>>()}
        })).collect::<Vec<_>>();
        inventory.sort_by_key(|node| node["id"].as_i64().unwrap());
        let mut containers = self.containers.keys().map(|key| {
            let h=self.containers.get_header(&key).unwrap();
            json!({"key":node_id(key),"nil":h.is_nil(),"len":h.len(),"cap":h.capacity(),
                "slots":(0..h.capacity()).map(|i|node_id(h.read_capacity_slot(i).unwrap())).collect::<Vec<_>>()})
        }).collect::<Vec<_>>();
        containers.sort_by_key(|row| row["key"].as_i64());
        json!({"name":name,"nodes":{"nil":header.is_nil(),"len":header.len(),"cap":header.capacity(),
            "slots":(0..header.capacity()).map(|i|node_id(header.read_capacity_slot(i).unwrap())).collect::<Vec<_>>()},
            "edges":self.edge_order.read().iter().map(|id|self.edge_records[id.0 as usize].tala_id).collect::<Vec<_>>(),
            "edge_header":{"nil":self.edge_order.header().is_nil(),"len":self.edge_order.len(),"cap":self.edge_order.header().capacity(),
                "slots":self.edge_order.header().capacity_slots().into_iter().map(|id|id.map(|id|self.edge_records[id.0 as usize].tala_id)).collect::<Vec<_>>()},
            "containers":containers,"inventory":inventory})
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn childless_grid_keeps_label_metrics_without_internal_grid_metadata() {
        let mut input = Graph::default();
        let outer = input.add_node(crate::Node::new("outer", Size::default()));
        let mut empty = crate::Node::new("outer.empty", Size::default());
        empty.parent = Some(outer);
        empty.grid_rows = Some(1);
        empty.layout_margins.top = 41.0;
        empty.external_label = Some(crate::ExternalLabel {
            size: Size {
                width: 8.0,
                height: 31.0,
            },
            side: crate::ExternalSide::Top,
            alignment: crate::ExternalAlignment::Center,
            automatic: true,
            reserve_space: true,
        });
        let empty = input.add_node(empty);
        input.node_font_size_declarations.insert(empty, None);
        input.node_label_levels.insert(empty, 2);

        let arena = ArenaGraph::from_input(&input);
        assert_eq!(arena.nodes[empty.0 as usize].font_size, Some(24));
        assert_eq!(arena.nodes[empty.0 as usize].grid_rows, None);
        assert_eq!(arena.nodes[empty.0 as usize].layout_margins.top, 0.0);
        assert_eq!(arena.nodes[empty.0 as usize].external_label, None);

        let child = input.add_node(crate::Node {
            parent: Some(empty),
            ..crate::Node::new("outer.empty.child", Size::default())
        });
        let arena = ArenaGraph::from_input(&input);
        assert_eq!(arena.nodes[empty.0 as usize].grid_rows, Some(1));
        assert_eq!(arena.nodes[child.0 as usize].container, Some(empty));
    }

    #[test]
    fn computed_d2_label_position_is_not_an_authored_tala_constraint() {
        let mut input = Graph::default();
        let mut node = crate::Node::new("container", Size::default());
        node.label_size = Some(Size {
            width: 80.0,
            height: 31.0,
        });
        node.label_position = LabelPosition::InsideTopCenter;
        node.d2_authored_geometry = Some(Box::new(crate::D2AuthoredGeometry {
            attributes: Some(Box::new(crate::D2ObjectValidation::default())),
            ..crate::D2AuthoredGeometry::default()
        }));
        let id = input.add_node(node);

        let arena = ArenaGraph::from_input(&input);
        assert_eq!(arena.nodes[id.0 as usize].node_padding.top, 0.0);

        input
            .node_mut(id)
            .unwrap()
            .d2_authored_geometry
            .as_mut()
            .unwrap()
            .attributes
            .as_mut()
            .unwrap()
            .label_position = Some(b"INSIDE_TOP_CENTER".to_vec());
        let arena = ArenaGraph::from_input(&input);
        assert_eq!(arena.nodes[id.0 as usize].node_padding.top, 41.0);
    }

    #[test]
    fn induced_node_and_edge_lineage_survives_noncompact_ids_and_repeated_tala_metadata() {
        // Source graph.go763–831 creates a fresh graph, adds actual Node
        // pointers, then scans original Edge pointers in Graph.Edges order.
        // Numeric IDs/DebugIDs do not supply that correspondence. Dense Rust
        // views must therefore return independent explicit maps for both.
        let mut input = Graph::default();
        let nodes = (0..7)
            .map(|index| {
                input.add_node(crate::Node::new(
                    format!("n{index}"),
                    Size {
                        width: 10.0,
                        height: 10.0,
                    },
                ))
            })
            .collect::<Vec<_>>();
        let edges = [(0, 1), (4, 6), (1, 3), (0, 1), (6, 2), (4, 6)]
            .into_iter()
            .map(|(from, to)| {
                input.add_edge(crate::Edge {
                    source: nodes[from],
                    target: nodes[to],
                })
            })
            .collect::<Vec<_>>();
        let mut parent = ArenaGraph::from_input(&input);
        for node in &mut parent.nodes {
            node.tala_id = 777;
        }
        for edge in &mut parent.edge_records {
            edge.tala_id = 888;
            // A compact input_id is metadata, not an owning edge handle.
            edge.input_id = EdgeId(0);
        }
        parent
            .edge_order
            .replace_values(&[edges[4], edges[0], edges[5], edges[2], edges[1], edges[3]]);
        // Actual endpoint incidence order need not equal Graph.Edges order.
        parent.nodes[nodes[6].0 as usize].edges = vec![edges[5], edges[1], edges[4]].into();
        parent.nodes[nodes[4].0 as usize].edges = vec![edges[1], edges[5]].into();
        let members = [nodes[6], nodes[2], nodes[4]];
        let (first, parent_nodes, parent_edges) =
            parent.induced_placement_subgraph_with_edge_mapping(&members);
        let (prepared_nodes, prepared_edges, prepared_order) =
            parent.induced_placement_correspondence(&members);
        assert_eq!(prepared_nodes, parent_nodes);
        assert_eq!(prepared_edges, parent_edges);
        assert_eq!(prepared_order, first.graph_node_order());
        assert_eq!(parent_nodes, members);
        assert_eq!(parent_edges, [edges[4], edges[5], edges[1]]);
        assert!(
            parent_edges
                .iter()
                .enumerate()
                .all(|(local, parent)| parent.0 as usize != local)
        );
        assert_eq!(first.nodes[0].edges, [EdgeId(1), EdgeId(2), EdgeId(0)]);
        assert_eq!(first.nodes[2].edges, [EdgeId(2), EdgeId(1)]);
        for (local, &canonical) in parent_edges.iter().enumerate() {
            let dense = &first.edge_records[local];
            let original = &parent.edge_records[canonical.0 as usize];
            assert_eq!(parent_nodes[dense.from.0 as usize], original.from);
            assert_eq!(parent_nodes[dense.to.0 as usize], original.to);
            assert_eq!(dense.input_id, EdgeId(local as u32));
            assert_eq!(dense.tala_id, 888);
        }
        let (second, intermediate_nodes, intermediate_edges) =
            first.induced_placement_subgraph_with_edge_mapping(&[NodeId(2), NodeId(0)]);
        assert_eq!(intermediate_nodes, [NodeId(2), NodeId(0)]);
        assert_eq!(intermediate_edges, [EdgeId(1), EdgeId(2)]);
        let canonical_nodes = intermediate_nodes
            .iter()
            .map(|local| parent_nodes[local.0 as usize])
            .collect::<Vec<_>>();
        let canonical_edges = intermediate_edges
            .iter()
            .map(|local| parent_edges[local.0 as usize])
            .collect::<Vec<_>>();
        assert_eq!(canonical_nodes, [nodes[4], nodes[6]]);
        assert_eq!(canonical_edges, [edges[5], edges[1]]);
        for (local, &canonical) in canonical_edges.iter().enumerate() {
            let dense = &second.edge_records[local];
            let original = &parent.edge_records[canonical.0 as usize];
            assert_eq!(canonical_nodes[dense.from.0 as usize], original.from);
            assert_eq!(canonical_nodes[dense.to.0 as usize], original.to);
        }
        let (legacy, legacy_nodes) = parent.induced_placement_subgraph(&members);
        assert_eq!(legacy_nodes, parent_nodes);
        assert_eq!(legacy.edge_records.len(), first.edge_records.len());
        assert_eq!(legacy.nodes[0].edges, first.nodes[0].edges);

        // A repeated serialized edge pointer keeps its final local slot in
        // the owning correspondence, independently of repeated Tala IDs.
        parent.edge_order.push(edges[4]);
        let (repeated, parent_nodes, parent_edges) =
            parent.induced_placement_subgraph_with_edge_mapping(&members);
        let (prepared_nodes, prepared_edges, prepared_order) =
            parent.induced_placement_correspondence(&members);
        assert_eq!(prepared_nodes, parent_nodes);
        assert_eq!(prepared_edges, parent_edges);
        assert_eq!(prepared_order, repeated.graph_node_order());
        assert_eq!(prepared_edges, [edges[5], edges[1], edges[4]]);
    }

    #[test]
    fn effective_arrowheads_override_syntax_only_in_tala_arena() {
        let mut input = Graph::default();
        let from = input.add_node(crate::Node::new("a", Size::default()));
        let to = input.add_node(crate::Node::new("b", Size::default()));
        // The language adapter resolves a label-only arrowhead record to
        // triangle. TALA classifies that endpoint independently of the arrow
        // written in the edge expression (adapter.go / HasSourceArrow).
        for (syntax, source, target, expected) in [
            ((false, true), None, None, (false, true)),
            ((false, true), Some("triangle"), None, (true, true)),
            ((true, true), Some("none"), None, (false, true)),
            ((true, true), None, Some("none"), (true, false)),
            (
                (false, false),
                Some("filled-diamond"),
                Some("unfilled-triangle"),
                (true, true),
            ),
            ((true, true), Some(""), Some(""), (false, false)),
        ] {
            let id = input.add_edge(crate::Edge {
                source: from,
                target: to,
            });
            input.set_edge_arrows(
                id,
                crate::EdgeArrows {
                    source: syntax.0,
                    target: syntax.1,
                },
            );
            input.set_edge_arrowheads(
                id,
                crate::EdgeArrowheads {
                    source: source.map(str::to_owned),
                    target: target.map(str::to_owned),
                },
            );
            let arena = ArenaGraph::from_input(&input);
            let edge = &arena.edge_records[id.0 as usize];
            assert_eq!(
                (edge.source_arrow, edge.target_arrow),
                expected,
                "{source:?}/{target:?}"
            );
            assert_eq!(
                input.edge_arrows(id),
                crate::EdgeArrows {
                    source: syntax.0,
                    target: syntax.1
                }
            );
            assert_eq!(
                edge.source_arrowhead.as_deref(),
                source.or(Some(if syntax.0 { "triangle" } else { "none" }))
            );
            assert_eq!(
                edge.target_arrowhead.as_deref(),
                target.or(Some(if syntax.1 { "triangle" } else { "none" }))
            );
            assert_eq!(input.edge_arrowheads(id).source.as_deref(), source);
            assert_eq!(input.edge_arrowheads(id).target.as_deref(), target);
        }
    }

    #[test]
    fn modifier_spacing_follows_label_and_icon_margins() {
        for (shape, three_d, multiple, dx, dy) in [
            (ShapeKind::Rectangle, false, false, 0.0, 0.0),
            (ShapeKind::Rectangle, true, false, 15.0, 15.0),
            (ShapeKind::Hexagon, true, false, 15.0, 7.5),
            (ShapeKind::Hexagon, true, true, 15.0, 7.5),
            (ShapeKind::Hexagon, false, true, 10.0, 10.0),
        ] {
            for (label_position, icon_position, top, right) in [
                (LabelPosition::Unset, None, 0.0, 0.0),
                (LabelPosition::OutsideTopCenter, None, 23.3, 0.0),
                (LabelPosition::OutsideRightMiddle, None, 0.0, 49.4),
                (
                    LabelPosition::OutsideTopCenter,
                    Some(LabelPosition::OutsideTopCenter),
                    74.0,
                    0.0,
                ),
                (
                    LabelPosition::OutsideRightMiddle,
                    Some(LabelPosition::OutsideRightMiddle),
                    0.0,
                    74.0,
                ),
            ] {
                let mut input = Graph::default();
                let mut node = crate::Node::new(
                    "decorated",
                    Size {
                        width: 100.0,
                        height: 80.0,
                    },
                );
                node.shape = shape;
                node.is_3d = three_d;
                node.is_multiple = multiple;
                node.label_size = Some(Size {
                    width: 39.4,
                    height: 13.3,
                });
                node.label_position = label_position;
                node.has_icon = icon_position.is_some();
                node.icon_position = icon_position;
                input.add_node(node);
                let arena = ArenaGraph::from_input(&input);
                assert_eq!(arena.nodes[0].layout_margins.top, top + dy);
                assert_eq!(arena.nodes[0].layout_margins.right, right + dx);
            }
        }
    }

    #[test]
    fn explicit_icon_positions_populate_tala_spacing_for_image_shapes() {
        let mut input = Graph::default();
        let mut outside = crate::Node::new(
            "outside-image-icon",
            Size {
                width: 100.0,
                height: 80.0,
            },
        );
        outside.shape = ShapeKind::Image;
        outside.has_icon = true;
        outside.icon_position = Some(LabelPosition::OutsideTopCenter);
        input.add_node(outside);

        let mut inside = crate::Node::new(
            "inside-image-icon",
            Size {
                width: 100.0,
                height: 80.0,
            },
        );
        inside.shape = ShapeKind::Image;
        inside.has_icon = true;
        inside.icon_position = Some(LabelPosition::InsideMiddleRight);
        input.add_node(inside);

        let arena = ArenaGraph::from_input(&input);
        assert_eq!(arena.nodes[0].layout_margins.top, 74.0);
        assert_eq!(arena.nodes[1].node_padding.right, 74.0);
    }

    #[test]
    fn authored_outside_label_clearance_participates_in_component_collision() {
        let mut input = Graph::default();
        let mut left_label = crate::Node::new(
            "left-label",
            Size {
                width: 156.0,
                height: 66.0,
            },
        );
        left_label.label_size = Some(Size {
            width: 111.0,
            height: 21.0,
        });
        left_label.label_position = LabelPosition::OutsideLeftTop;
        let a = input.add_node(left_label);
        let b = input.add_node(crate::Node::new(
            "other",
            Size {
                width: 295.0,
                height: 66.0,
            },
        ));
        let mut graph = ArenaGraph::from_input(&input);
        graph.nodes[b.0 as usize].position = Some(Point { x: 0.0, y: 441.0 });
        graph.nodes[b.0 as usize].rect.origin = Point { x: 0.0, y: 441.0 };
        // A 28px box gap is insufficient for this authored 111px left label:
        // source UpdateSpacing reserves its width plus two 5px paddings.
        let candidate = Point { x: 323.0, y: 441.0 };
        assert_eq!(graph.spacing_delta_with_loops(a, b, candidate), 121.0);
        assert!(
            candidate.x
                < graph.nodes[b.0 as usize].rect.size.width
                    + graph.spacing_delta_with_loops(a, b, candidate)
        );
        assert_eq!(
            input.node(a).unwrap().label_position,
            LabelPosition::OutsideLeftTop
        );
    }

    #[test]
    fn explicit_edge_visibility_is_independent_of_style_and_stays_stored() {
        let mut input = Graph::default();
        let from = input.add_node(crate::Node::new(
            "a",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let to = input.add_node(crate::Node::new(
            "b",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let visible = input.add_edge(crate::Edge {
            source: from,
            target: to,
        });
        input.set_edge_style(
            visible,
            crate::EdgeStyle {
                stroke: Some("transparent".into()),
                ..Default::default()
            },
        );
        input.tala_edge_invisible[visible.0 as usize] = Some(false);
        let invisible = input.add_edge(crate::Edge {
            source: to,
            target: from,
        });
        input.tala_edge_invisible[invisible.0 as usize] = Some(true);
        let mut arena = ArenaGraph::from_input(&input);
        assert!(!arena.edge_records[0].is_invisible());
        assert!(arena.edge_records[1].is_invisible());
        assert_eq!(arena.edge_records[1].min_width, 10.0);
        arena.edge_records[0].style.opacity = Some("0".into());
        assert!(!arena.edge_records[0].is_invisible());
        arena.edge_records[1].style.stroke = Some("red".into());
        assert!(arena.edge_records[1].is_invisible());
    }
    #[test]
    fn go_zero_opacity_spellings_clear_tala_edge_minimums_without_rewriting_style() {
        for value in ["0", "-0", "0x0p0", "0x_0.0p+0", "1e-4000", "0x1p-1075"] {
            let mut input = Graph::default();
            let from = input.add_node(crate::Node::new(
                "from",
                Size {
                    width: 40.0,
                    height: 40.0,
                },
            ));
            let to = input.add_node(crate::Node::new(
                "to",
                Size {
                    width: 40.0,
                    height: 40.0,
                },
            ));
            let edge = input.add_edge(crate::Edge {
                source: from,
                target: to,
            });
            input.set_edge_style(
                edge,
                d2_rs_core::EdgeStyle {
                    opacity: Some(value.into()),
                    ..Default::default()
                },
            );
            let arena = ArenaGraph::from_input(&input);
            assert!(arena.edge_records[0].is_invisible(), "{value}");
            assert_eq!(arena.edge_records[0].min_width, 0.0, "{value}");
            assert_eq!(arena.edge_records[0].min_height, 0.0, "{value}");
            assert_eq!(arena.edge_records[0].style.opacity.as_deref(), Some(value));
        }
    }

    #[test]
    fn adapter_container_headers_retain_append_capacity_and_source_sibling_order() {
        let mut input = Graph::default();
        let nodes: Vec<_> = (0..3)
            .map(|index| {
                input.add_node(crate::Node::new(
                    index.to_string(),
                    Size {
                        width: 10.0,
                        height: 10.0,
                    },
                ))
            })
            .collect();
        input
            .hierarchy_children_order
            .insert(None, vec![nodes[2], nodes[0], nodes[1]]);
        let mut arena = ArenaGraph::from_input(&input);
        let header = arena.containers.get_header(&None).unwrap();
        assert_eq!(
            header.require_nodes().collect::<Vec<_>>(),
            [nodes[2], nodes[0], nodes[1]]
        );
        assert_eq!((header.len(), header.capacity()), (3, 4));
        arena.initialize_shared_topology();
        let owner = arena.owner_container_map(OwnerGraphId(0)).unwrap();
        let map = arena.containers.share_map();
        assert!(owner.same_map_allocation(&arena.owner_container_map(OwnerGraphId(0)).unwrap()));
        let live = map.get(&None).unwrap();
        assert!(live.same_backing_allocation(&header));
        assert_eq!(live.capacity(), 4);
    }

    #[test]
    fn signed_node_ids_drive_container_order_and_projected_lookup() {
        let mut input = Graph::default();
        for (index, identity) in [i64::MAX, -1, i64::MIN].into_iter().enumerate() {
            let mut node = crate::Node::new(
                index.to_string(),
                Size {
                    width: 10.0,
                    height: 10.0,
                },
            );
            node.locked_position = Some(Point {
                x: index as f64 * 100.0,
                y: 0.0,
            });
            let id = input.add_node(node);
            input.tala_id_overrides[id.0 as usize] = Some(identity);
        }
        let mut arena = ArenaGraph::from_input(&input);
        arena.infer_prearranged_containers();
        assert_eq!(
            arena.containers.nodes(&None),
            [NodeId(2), NodeId(1), NodeId(0)]
        );
        assert_eq!(arena.active_node_tala_id(NodeId(2)), i64::MIN);
        assert_eq!(
            arena.projected_node_by_tala(i64::MIN).unwrap().input_id,
            NodeId(2)
        );
        assert_eq!(
            arena.projected_node_by_tala(-1).unwrap().input_id,
            NodeId(1)
        );
    }
    #[test]
    fn edge_entity_id_is_signed_and_independent_of_arena_index() {
        let mut input = Graph::default();
        let from = input.add_node(crate::Node::new(
            "a",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let to = input.add_node(crate::Node::new(
            "b",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let first = input.add_edge(crate::Edge {
            source: from,
            target: to,
        });
        let second = input.add_edge(crate::Edge {
            source: to,
            target: from,
        });
        input.tala_edge_ids[first.0 as usize] = Some(i64::MIN);
        input.tala_edge_ids[second.0 as usize] = Some(1i64 << 32);
        let arena = ArenaGraph::from_input(&input);
        assert_eq!(arena.edge_records[0].input_id, first);
        assert_eq!(arena.edge_records[0].tala_id, i64::MIN);
        assert_eq!(arena.edge_records[1].input_id, second);
        assert_eq!(arena.edge_records[1].tala_id, 1i64 << 32);
    }

    #[test]
    fn decoded_label_presence_precedes_object_adapter_shape_and_text_rules() {
        // graphjson.deserializeNode installs the decoded Label pointer even
        // for content shapes and empty label text. A nil Label stays nil.
        for shape in ["Table", "Class", "Text", "Code", "Square"] {
            for present in [false, true] {
                let label = if present {
                    serde_json::json!({"Text":"", "Position":"OUTSIDE_LEFT_MIDDLE", "Width":150.5, "Height":41.25})
                } else {
                    serde_json::Value::Null
                };
                let bytes=serde_json::to_vec(&serde_json::json!({
                    "nodes":[{"id":1,"width":10.5,"height":10.25,"topLeft":{"x":-0.5,"y":0.25},"shapeType":shape,"label":label}],
                    "containers":{"0":[1]}
                })).unwrap();
                let input = graphjson::graphjson_to_graph(&bytes).unwrap();
                let arena = ArenaGraph::from_input(&input);
                let node = &arena.nodes[0];
                assert_eq!(node.label_text_present, present, "{shape}");
                assert_eq!(node.label_size.is_some(), present, "{shape}");
                assert_eq!(
                    node.label_position,
                    if present {
                        LabelPosition::OutsideLeftMiddle
                    } else {
                        LabelPosition::Unset
                    }
                );
            }
        }
    }

    #[test]
    fn source_adapter_omits_labels_on_content_shapes_and_empty_text() {
        let mut input = Graph::default();
        let size = Size {
            width: 100.0,
            height: 60.0,
        };
        let mut ids = Vec::new();
        for shape in [
            ShapeKind::Text,
            ShapeKind::Code,
            ShapeKind::Class,
            ShapeKind::SqlTable,
            ShapeKind::Rectangle,
        ] {
            let id = input.add_node(crate::Node {
                shape,
                label_size: Some(Size {
                    width: 80.0,
                    height: 20.0,
                }),
                label_position: LabelPosition::OutsideTopCenter,
                ..crate::Node::new(format!("node-{}", ids.len()), size)
            });
            input.set_node_label(
                id,
                if shape == ShapeKind::Rectangle {
                    ""
                } else {
                    "content"
                },
            );
            ids.push(id);
        }
        let arena = ArenaGraph::from_input(&input);
        for id in ids {
            let value = &arena.nodes[id.0 as usize];
            assert!(!value.label_text_present);
            assert!(value.label_size.is_none());
            assert_eq!(value.label_position, LabelPosition::Unset);
            assert!(!value.label_position_fixed);
            assert_eq!(value.rect.size, size);
            assert_eq!(value.layout_margins.top, 0.0);
            assert!(input.node(id).unwrap().label_size.is_some());
        }
    }

    #[test]
    fn source_adapter_retains_nonempty_ordinary_label_including_whitespace() {
        let mut input = Graph::default();
        for text in ["label", " "] {
            let id = input.add_node(crate::Node {
                label_size: Some(Size {
                    width: 80.0,
                    height: 20.0,
                }),
                label_position: LabelPosition::OutsideTopCenter,
                ..crate::Node::new(
                    text,
                    Size {
                        width: 100.0,
                        height: 60.0,
                    },
                )
            });
            input.set_node_label(id, text);
        }
        let arena = ArenaGraph::from_input(&input);
        for value in &arena.nodes {
            assert!(value.label_text_present);
            assert!(value.label_size.is_some());
            assert!(value.label_position_fixed);
            assert_eq!(value.layout_margins.top, 30.0);
        }
    }
}
