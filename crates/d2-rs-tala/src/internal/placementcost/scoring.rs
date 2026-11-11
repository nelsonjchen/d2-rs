// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Placement cost functions and symmetry analysis.
//!
//! Scores intentionally preserve ordered reductions and open floating-point
//! comparison boundaries because equal candidates are resolved by visit order.

use super::scoring_context::{ScoreContext, score_interval, score_poll};
use super::*;
use crate::internal::limits;

#[path = "../placement/owner_sizeless_topology.rs"]
mod owner_sizeless_topology;

type ProjectedEndpoints = ((Point, Size, i64), (Point, Size, i64));

fn owner_score_error(error: impl std::fmt::Display) -> limits::Error {
    limits::Error::Invalid(format!("TALA owner scoring: {error}"))
}

fn owner_score_incidents(
    owner: &ArenaGraph,
    node: NodeId,
) -> Result<Vec<Option<EdgeId>>, limits::Error> {
    match owner.owner_node_incident_slots(node) {
        Ok(slots) => Ok(slots),
        Err(OwnerTopologyError::UnboundIncidentHeader(_)) => Ok(owner.nodes[node.0 as usize]
            .edges
            .read()
            .iter()
            .copied()
            .map(Some)
            .collect()),
        Err(error) => Err(owner_score_error(error)),
    }
}

#[derive(Debug, Eq, PartialEq)]
struct OwnerSizelessNodeTopology {
    node: NodeId,
    tala_id: i64,
    container: Option<NodeId>,
    receiver: Option<OwnerGraphId>,
    direction_entry: Option<Orientation>,
    is_table: bool,
    edges: Vec<Option<EdgeId>>,
}

#[derive(Debug, Eq, PartialEq)]
struct OwnerSizelessEdgeTopology {
    edge: EdgeId,
    tala_id: i64,
    from: Option<NodeId>,
    to: Option<NodeId>,
    source_arrow: bool,
    target_arrow: bool,
    invisible: bool,
}

#[derive(Debug, Eq, PartialEq)]
struct OwnerSizelessTopology {
    arena: OwnerArenaId,
    mapping_view: OwnerGraphId,
    node: NodeId,
    nodes: Vec<OwnerSizelessNodeTopology>,
    edges: Vec<OwnerSizelessEdgeTopology>,
    node_mapping: Vec<NodeId>,
    edge_mapping: Vec<EdgeId>,
}

struct OwnerSizelessPreparation {
    topology: OwnerSizelessTopology,
    incident: Vec<Option<EdgeId>>,
    adjacent: Vec<Option<NodeId>>,
    direction: Orientation,
    direction_factor: f64,
    checks: usize,
}

/// Source edge_length_scorer.go22–90: one lazy, mutable scorer per movement
/// sweep. Preparation retains identities and direction setup, never positions
/// or a borrowed owner frame. Each Score borrows the current owner again.
///
/// Options match sizeless_optimizer.go401–407: nil abductions, false sizes,
/// false minimum gap. Raw incidents/endpoints, group abduction scans/policy,
/// Cluster.Vessel distance, current Near keys and common-uncle geometry read
/// the actual owner. This entry never invokes the dense scorer.
pub(super) struct OwnerSizelessEdgeLengthScorer {
    node: Option<NodeId>,
    direction_penalty: bool,
    prepared: Option<OwnerSizelessPreparation>,
    /// Set by a caller whose owner changes only geometry between scores,
    /// under contexts whose polls leave it unchanged. Then the retained
    /// topology still matches the owner, exactly as `unchanged` would find.
    stable_topology: bool,
    shared_admission: bool,
}

// A scoring poll releases its borrowed frame before an exclusive callback,
// then borrows current owner fields again. Both entries use one kernel.
enum OwnerSizelessAccess<'a, 'b> {
    Borrowed(&'a OwnerScoreFrame<'b>),
    Exclusive {
        owner: &'a mut ArenaGraph,
        mapping: &'a CanonicalScoreMapping,
        view: OwnerGraphId,
        /// The owner can change only through `check`'s exclusive callback,
        /// and not even then under a stable context. Until then, a frame
        /// these inputs already produced is rebuilt without repeating its
        /// validation.
        validated: std::cell::Cell<bool>,
    },
}
impl OwnerSizelessAccess<'_, '_> {
    fn frame(&self) -> Result<OwnerScoreFrame<'_>, limits::Error> {
        match self {
            Self::Borrowed(frame) => Ok((*frame).clone()),
            Self::Exclusive {
                owner,
                mapping,
                view,
                validated,
            } => {
                if validated.get() {
                    return Ok(OwnerScoreFrame::validated_edge_mapping(
                        owner,
                        mapping.arena(),
                        *view,
                        mapping.nodes(),
                        mapping.edges(),
                    ));
                }
                let frame = OwnerScoreFrame::new_with_edge_mapping(
                    owner,
                    mapping.arena(),
                    *view,
                    mapping.nodes(),
                    mapping.edges(),
                )
                .map_err(owner_score_error)?;
                validated.set(true);
                Ok(frame)
            }
        }
    }
    fn check(&mut self, context: ScoreContext<'_>) -> Result<(), limits::Error> {
        match self {
            Self::Borrowed(_) => context.check(),
            Self::Exclusive {
                owner, validated, ..
            } => {
                // A stable context's poll leaves the validated owner as it is.
                if !context.stable_owner() {
                    validated.set(false);
                }
                context.check_with_owner(owner)
            }
        }
    }
    #[cfg(feature = "diagnostic-traces")]
    fn observe(
        &self,
        context: ScoreContext<'_>,
        site: &'static str,
        node: Option<NodeId>,
        iteration: Option<usize>,
    ) -> Result<(), limits::Error> {
        let frame = self.frame()?;
        context.observe(frame.owner(), site, node, iteration);
        Ok(())
    }
}
macro_rules! owner_sizeless_poll {
    ($context:expr,$access:expr,$frame:ident,$site:expr,$node:expr,$iteration:expr) => {
        #[cfg(feature = "diagnostic-traces")]
        let diagnostic_args = ($site, $node, $iteration);
        drop($frame);
        #[cfg(feature = "diagnostic-traces")]
        $access.observe(
            $context,
            diagnostic_args.0,
            diagnostic_args.1,
            diagnostic_args.2,
        )?;
        $access.check($context)?;
    };
}
macro_rules! owner_sizeless_interval {
    ($context:expr,$access:expr,$frame:ident,$site:expr,$node:expr,$iteration:expr) => {
        #[cfg(feature = "diagnostic-traces")]
        let diagnostic_args = ($site, $node, Some($iteration));
        drop($frame);
        if $iteration % 64 == 0 {
            #[cfg(feature = "diagnostic-traces")]
            $access.observe(
                $context,
                diagnostic_args.0,
                diagnostic_args.1,
                diagnostic_args.2,
            )?;
            $access.check($context)?;
        }
        let $frame = $access.frame()?;
    };
}

impl OwnerSizelessEdgeLengthScorer {
    pub(super) fn new(node: NodeId, direction_penalty: bool) -> Self {
        Self {
            node: Some(node),
            direction_penalty,
            prepared: None,
            stable_topology: false,
            shared_admission: false,
        }
    }

    pub(super) fn close(&mut self) {
        self.prepared = None;
        self.node = None;
        self.direction_penalty = false;
        self.stable_topology = false;
        self.shared_admission = false;
    }

    /// The caller guarantees that, between this scorer's scores, it changes
    /// only the owner's geometry, the workspace and mapping stay the same,
    /// and every context it polls has `stable_sized_owner()`.
    pub(super) fn assume_stable_topology(&mut self) {
        self.stable_topology = true;
    }

    /// Another scorer has successfully admitted this same owner, workspace,
    /// view and mapping. The caller keeps all four topologically unchanged,
    /// including through every context poll, as for `assume_stable_topology`.
    /// Mutation-capable scorers still perform their own complete admission.
    pub(super) fn reuse_admission(&mut self) {
        self.shared_admission = self.stable_topology;
    }

    fn admitted_topology(frame: &OwnerScoreFrame<'_>, node: NodeId) -> OwnerSizelessTopology {
        OwnerSizelessTopology {
            arena: frame.source_arena(),
            mapping_view: frame.mapping_view(),
            node,
            nodes: Vec::new(),
            edges: Vec::new(),
            node_mapping: Vec::new(),
            edge_mapping: Vec::new(),
        }
    }

    // Stable sweeps need all initial validation, but only their incident-edge
    // preparation is used afterwards (edge_length_scorer.go:16-76). Retain a
    // whole-owner comparison snapshot only for mutation-capable callers.
    fn topology<const RETAIN: bool>(
        workspace: &ArenaGraph,
        frame: &OwnerScoreFrame<'_>,
        local_node: NodeId,
    ) -> Result<OwnerSizelessTopology, limits::Error> {
        if frame.abductions().is_some_and(|slice| slice.present) {
            return Err(owner_score_error(
                "sizeless sweep requires the source nil abduction option",
            ));
        }
        if workspace.nodes.len() != frame.local_to_canonical().len()
            || workspace.edge_records.len() != frame.local_to_canonical_edges().len()
        {
            return Err(owner_score_error(
                "sizeless workspace mapping length differs",
            ));
        }
        let canonical_scoring = frame
            .canonical_node(local_node)
            .map_err(owner_score_error)?;
        let mut nodes = Vec::new();
        // Membership of the recorded nodes. Each is recorded only after its
        // own frame lookup succeeds, so its id indexes the owner's nodes.
        let mut seen = vec![false; frame.owner().nodes.len()];
        // One node's ancestry, as membership only: short chains in a reused
        // buffer, any deeper remainder in the ordered set.
        let mut ancestry = Vec::new();
        let mut deep_ancestry = BTreeSet::new();
        for (local, canonical) in frame.local_to_canonical().iter().copied().enumerate() {
            let actual = frame.node(canonical).map_err(owner_score_error)?;
            let carrier = &workspace.nodes[local];
            if carrier.tala_id != actual.tala_id {
                return Err(owner_score_error(
                    "sizeless workspace mapping identifies a different node",
                ));
            }
            // Record raw ancestry as well as mapped carriers. Geometry may
            // change between scores; membership/directions may not.
            let mut current = Some(canonical);
            ancestry.clear();
            deep_ancestry.clear();
            while let Some(node) = current {
                if ancestry.contains(&node) || deep_ancestry.contains(&node) {
                    return Err(owner_score_error("sizeless container ancestry is cyclic"));
                }
                // A node recorded by an earlier walk had its whole ancestry
                // walked then, with every lookup below succeeding and no cycle
                // (a cycle through it would have failed that walk). Repeating
                // those pure reads records nothing new and cannot fail.
                if seen.get(node.0 as usize).is_some_and(|&recorded| recorded) {
                    break;
                }
                if ancestry.len() < 32 {
                    ancestry.push(node);
                } else {
                    deep_ancestry.insert(node);
                }
                let actual = frame.node(node).map_err(owner_score_error)?;
                let receiver = frame.graph_id(node).map_err(owner_score_error)?;
                let parent = actual.container;
                let direction_entry = receiver
                    .map(|receiver| {
                        frame
                            .owner()
                            .owner_graph_direction_entry(receiver, parent)
                            .map_err(owner_score_error)
                    })
                    .transpose()?
                    .flatten();
                seen[node.0 as usize] = true;
                if RETAIN {
                    nodes.push(OwnerSizelessNodeTopology {
                        node,
                        tala_id: actual.tala_id,
                        container: actual.container,
                        receiver,
                        direction_entry,
                        is_table: actual.shape == ShapeKind::SqlTable,
                        edges: owner_score_incidents(frame.owner(), node)?,
                    });
                } else {
                    // Same incident-header admission without copying
                    // unrelated nodes' incidence into every scorer.
                    frame
                        .owner()
                        .for_each_owner_node_incident(node, |_| {})
                        .map_err(owner_score_error)?;
                }
                current = actual.container;
            }
        }
        let mut edges = if RETAIN {
            Vec::with_capacity(workspace.edge_records.len())
        } else {
            Vec::new()
        };
        for (local, canonical) in frame.local_to_canonical_edges().iter().copied().enumerate() {
            let actual = frame.edge(canonical).map_err(owner_score_error)?;
            let carrier = &workspace.edge_records[local];
            if carrier.tala_id != actual.tala_id
                || carrier.source_arrow != actual.source_arrow
                || carrier.target_arrow != actual.target_arrow
                || carrier.is_invisible() != actual.is_invisible()
            {
                return Err(owner_score_error(
                    "sizeless workspace edge differs from its canonical owner edge",
                ));
            }
            let from = frame
                .owner()
                .owner_edge_endpoint(canonical, false)
                .map_err(owner_score_error)?;
            let to = frame
                .owner()
                .owner_edge_endpoint(canonical, true)
                .map_err(owner_score_error)?;
            if RETAIN {
                edges.push(OwnerSizelessEdgeTopology {
                    edge: canonical,
                    tala_id: actual.tala_id,
                    from,
                    to,
                    source_arrow: actual.source_arrow,
                    target_arrow: actual.target_arrow,
                    invisible: actual.is_invisible(),
                });
            }
        }
        // Only identity/topology is retained. In particular, stale workspace
        // positions and sizes never become actual owner scoring operands.
        Ok(OwnerSizelessTopology {
            nodes,
            edges,
            node_mapping: if RETAIN {
                frame.local_to_canonical().to_vec()
            } else {
                Vec::new()
            },
            edge_mapping: if RETAIN {
                frame.local_to_canonical_edges().to_vec()
            } else {
                Vec::new()
            },
            ..Self::admitted_topology(frame, canonical_scoring)
        })
    }

    fn prepare(
        topology: OwnerSizelessTopology,
        direction_penalty: bool,
        access: &mut OwnerSizelessAccess<'_, '_>,
        context: ScoreContext<'_>,
    ) -> Result<OwnerSizelessPreparation, limits::Error> {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(NodeScorePrepare);
        let frame = access.frame()?;
        let node = topology.node;
        let incident = owner_score_incidents(frame.owner(), node)?;
        let receiver = frame
            .graph_id(node)
            .map_err(owner_score_error)?
            .expect("runtime error: invalid memory address or nil pointer dereference");
        let map = frame
            .owner()
            .owner_graph_group_map(receiver, super::owner_groups::OwnerGroupKind::Cluster)
            .map_err(owner_score_error)?;
        let mut vessels = frame
            .owner()
            .owner_group_map_keys(&map)
            .map_err(owner_score_error)?;
        super::go_sort::sort_by(&mut vessels, |a, b| {
            let id = |node: &Option<NodeId>| {
                node.map_or(0, |node| {
                    frame.node(node).expect("validated vessel").tala_id
                })
            };
            id(a) < id(b)
        });
        let mut checks = 0;
        let mut abduction_count = 0usize;
        drop(frame);
        for (index, vessel) in vessels.into_iter().enumerate() {
            if index % 64 == 0 {
                let frame = access.frame()?;
                owner_sizeless_poll!(
                    context,
                    access,
                    frame,
                    "node-prepare-cluster-abductions",
                    Some(node),
                    Some(index)
                );
                checks += 1;
            }
            let frame = access.frame()?;
            let group = frame
                .owner()
                .owner_graph_group_entry(
                    frame
                        .graph_id(node)
                        .map_err(owner_score_error)?
                        .expect("runtime error: invalid memory address or nil pointer dereference"),
                    super::owner_groups::OwnerGroupKind::Cluster,
                    vessel,
                )
                .map_err(owner_score_error)?
                .flatten()
                .expect("runtime error: invalid memory address or nil pointer dereference");
            abduction_count += frame
                .owner()
                .owner_cluster_policy(group)
                .map_err(owner_score_error)?
                .edge_abductions
                .as_ref()
                .map_or(0, |header| header.len());
        }
        for (_site, length) in [
            ("node-prepare-used-abductions", abduction_count),
            ("node-prepare-node-replacements", incident.len()),
            ("node-prepare-adjacent-replacements", incident.len()),
        ] {
            for _index in (0..length).step_by(64) {
                let frame = access.frame()?;
                owner_sizeless_poll!(context, access, frame, _site, Some(node), Some(_index));
                checks += 1;
            }
        }
        let mut adjacent = Vec::with_capacity(incident.len());
        for (index, edge) in incident.iter().copied().enumerate() {
            if index % 64 == 0 {
                let frame = access.frame()?;
                owner_sizeless_poll!(
                    context,
                    access,
                    frame,
                    "node-prepare-edge",
                    Some(node),
                    Some(index)
                );
                checks += 1;
            }
            let frame = access.frame()?;
            let edge =
                edge.expect("runtime error: invalid memory address or nil pointer dereference");
            let from = frame
                .owner()
                .owner_edge_endpoint(edge, false)
                .map_err(owner_score_error)?;
            adjacent.push(if from == Some(node) {
                frame
                    .owner()
                    .owner_edge_endpoint(edge, true)
                    .map_err(owner_score_error)?
            } else {
                from
            });
        }
        let frame = access.frame()?;
        let mut direction = frame.container_direction(node).map_err(owner_score_error)?;
        let mut direction_factor = 0.0;
        if direction_penalty {
            if direction != Orientation::None {
                direction_factor = 1.5;
            } else if frame.node(node).map_err(owner_score_error)?.shape == ShapeKind::SqlTable {
                direction = Orientation::Right;
                direction_factor = 0.5;
            } else {
                direction = Orientation::BottomRight;
                direction_factor = 0.3;
            }
        }
        if direction_penalty
            && frame.container_direction(node).map_err(owner_score_error)? == Orientation::None
            && frame.owner().owner_is_cluster_vessel(node)
        {
            direction_factor = 0.2;
            let group = frame
                .owner()
                .owner_graph_group_entry(
                    receiver,
                    super::owner_groups::OwnerGroupKind::Cluster,
                    Some(node),
                )
                .map_err(owner_score_error)?
                .flatten()
                .expect("runtime error: invalid memory address or nil pointer dereference");
            match frame
                .owner()
                .owner_cluster_policy(group)
                .map_err(owner_score_error)?
                .arrangement
                .as_str()
            {
                "Row" => direction = Orientation::Bottom,
                "Column" => direction = Orientation::Right,
                _ => {}
            }
        }
        drop(frame);
        for index in 0..incident.len() {
            if index % 64 == 0 {
                let frame = access.frame()?;
                owner_sizeless_poll!(
                    context,
                    access,
                    frame,
                    "node-prepare-label-edge",
                    Some(node),
                    Some(index)
                );
                checks += 1;
            }
        }
        Ok(OwnerSizelessPreparation {
            topology,
            incident,
            adjacent,
            direction,
            direction_factor,
            checks,
        })
    }

    pub(super) fn score(
        &mut self,
        workspace: &ArenaGraph,
        frame: &OwnerScoreFrame<'_>,
        context: ScoreContext<'_>,
    ) -> Result<f64, limits::Error> {
        self.score_access(workspace, OwnerSizelessAccess::Borrowed(frame), context)
    }
    pub(super) fn score_with_owner(
        &mut self,
        workspace: &ArenaGraph,
        owner: &mut ArenaGraph,
        mapping: &CanonicalScoreMapping,
        view: OwnerGraphId,
        context: ScoreContext<'_>,
    ) -> Result<f64, limits::Error> {
        self.score_access(
            workspace,
            OwnerSizelessAccess::Exclusive {
                owner,
                mapping,
                view,
                // A successful preparation admitted this same owner/mapping.
                // The stable sweep contract keeps both valid across scores;
                // polls still run at every original boundary.
                validated: std::cell::Cell::new(
                    self.stable_topology && (self.prepared.is_some() || self.shared_admission),
                ),
            },
            context,
        )
    }
    fn score_access(
        &mut self,
        workspace: &ArenaGraph,
        mut access: OwnerSizelessAccess<'_, '_>,
        context: ScoreContext<'_>,
    ) -> Result<f64, limits::Error> {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(NodeScore);
        let frame = access.frame()?;
        // Source entry cancellation precedes closed/scorer/topology checks.
        // Canonical diagnostic arguments compile out in ordinary builds.
        owner_sizeless_poll!(
            context,
            access,
            frame,
            "node-edge-length-entry",
            self.node.and_then(|node| frame.canonical_node(node).ok()),
            None
        );
        let frame = access.frame()?;
        let local = self
            .node
            .ok_or_else(|| limits::Error::Invalid("EdgeLength: scorer is closed".into()))?;
        let topology = if self.prepared.as_ref().is_some_and(|prepared| {
            self.stable_topology
                || owner_sizeless_topology::unchanged(&prepared.topology, workspace, &frame, local)
        }) {
            None
        } else {
            Some(if self.shared_admission {
                // NodesEdgeLength scores one fixed-topology subgraph in order
                // (compaction.go:233-294). The first successful scorer already
                // checked every mapped carrier and ancestor. Prepare only this
                // node's incidents, as edge_length_scorer.go does, retaining
                // its original entry and preparation cancellation boundaries.
                Self::admitted_topology(
                    &frame,
                    frame.canonical_node(local).map_err(owner_score_error)?,
                )
            } else if self.stable_topology {
                Self::topology::<false>(workspace, &frame, local)?
            } else {
                Self::topology::<true>(workspace, &frame, local)?
            })
        };
        drop(frame);
        if let Some(prepared) = &self.prepared {
            if topology
                .as_ref()
                .is_some_and(|topology| prepared.topology != *topology)
            {
                return Err(owner_score_error(
                    "sizeless scorer topology changed during its movement sweep",
                ));
            }
            for _index in 0..prepared.checks {
                let frame = access.frame()?;
                owner_sizeless_poll!(
                    context,
                    access,
                    frame,
                    "scorer-replay-preparation",
                    Some(prepared.topology.node),
                    Some(_index)
                );
            }
        } else {
            // On cancellation/error, no prepared scratch is published; a
            // later Score prepares again. A successful preparation remains
            // owned even if evaluation subsequently errors.
            self.prepared = Some(Self::prepare(
                topology.expect("unprepared scorer topology"),
                self.direction_penalty,
                &mut access,
                context,
            )?);
        }
        let prepared = self.prepared.as_ref().expect("successful lazy preparation");
        let node = prepared.topology.node;
        let mut total = 0.0;
        for (index, &edge) in prepared.incident.iter().enumerate() {
            // The interval rebuilds the frame it uses. An entry frame is
            // validated before it only at index 0: a later entry would see
            // exactly the state the previous iteration's frame validated,
            // since nothing polls or writes in between.
            let frame = if index == 0 {
                Some(access.frame()?)
            } else {
                None
            };
            owner_sizeless_interval!(
                context,
                access,
                frame,
                "node-evaluate-edge",
                Some(node),
                index
            );
            let edge_id =
                edge.expect("runtime error: invalid memory address or nil pointer dereference");
            let edge = frame.edge(edge_id).map_err(owner_score_error)?;
            let from = frame
                .owner()
                .owner_edge_endpoint(edge_id, false)
                .map_err(owner_score_error)?;
            let adjacent = if from == Some(node) {
                frame
                    .owner()
                    .owner_edge_endpoint(edge_id, true)
                    .map_err(owner_score_error)?
            } else {
                from
            };
            let adjacent =
                adjacent.expect("runtime error: invalid memory address or nil pointer dereference");
            // Source skips a nil adjacent before any meaningful read of the
            // receiver TopLeft. Thus a nil isolated receiver scores zero.
            let Some(second) = frame.position(adjacent).map_err(owner_score_error)? else {
                continue;
            };
            let replacement = prepared.adjacent[index];
            let replacement = replacement
                .expect("runtime error: invalid memory address or nil pointer dereference");
            let cluster = frame
                .owner()
                .owner_node_group_vessel(node, super::owner_groups::OwnerGroupKind::Cluster)
                .map_err(owner_score_error)?;
            let adjacent_cluster = frame
                .owner()
                .owner_node_group_vessel(replacement, super::owner_groups::OwnerGroupKind::Cluster)
                .map_err(owner_score_error)?;
            let mut distance = if let Some(vessel) = cluster {
                super::owner_sizeless_geometry::placement_distance(
                    &frame,
                    vessel,
                    Some(replacement),
                )
            } else if let Some(vessel) = adjacent_cluster {
                super::owner_sizeless_geometry::placement_distance(&frame, vessel, Some(node))
            } else {
                super::owner_sizeless_geometry::placement_distance(
                    &frame,
                    Some(node),
                    Some(replacement),
                )
            };
            distance = super::owner_median::source_math_max(distance, 0.0);
            if self.direction_penalty
                && (edge.source_arrow != edge.target_arrow
                    || frame
                        .outer_direction_is_set(node, adjacent)
                        .map_err(owner_score_error)?)
            {
                let first = frame
                    .position(node)
                    .map_err(owner_score_error)?
                    .expect("runtime error: invalid memory address or nil pointer dereference");
                let mut used = if first.y < second.y {
                    if first.x < second.x {
                        Orientation::TopLeft
                    } else if second.x < first.x {
                        Orientation::TopRight
                    } else {
                        Orientation::Top
                    }
                } else if second.y < first.y {
                    if first.x < second.x {
                        Orientation::BottomLeft
                    } else if second.x < first.x {
                        Orientation::BottomRight
                    } else {
                        Orientation::Bottom
                    }
                } else if second.x < first.x {
                    Orientation::Right
                } else if first.x < second.x {
                    Orientation::Left
                } else {
                    Orientation::None
                };
                let from = if edge.source_arrow && !edge.target_arrow {
                    frame
                        .owner()
                        .owner_edge_endpoint(edge_id, true)
                        .map_err(owner_score_error)?
                } else {
                    from
                };
                if from == Some(node) {
                    used = used.opposite();
                }
                if cluster.is_some() || adjacent_cluster.is_some() {
                    used = match (prepared.direction, used) {
                        (Orientation::Left, Orientation::TopLeft | Orientation::BottomLeft) => {
                            Orientation::Left
                        }
                        (Orientation::Right, Orientation::TopRight | Orientation::BottomRight) => {
                            Orientation::Right
                        }
                        (Orientation::Top, Orientation::TopLeft | Orientation::TopRight) => {
                            Orientation::Top
                        }
                        (
                            Orientation::Bottom,
                            Orientation::BottomLeft | Orientation::BottomRight,
                        ) => Orientation::Bottom,
                        _ => used,
                    };
                }
                let mut delta =
                    f64::from(compass_delta(prepared.direction.compass(), used.compass())).abs();
                if edge.source_arrow == edge.target_arrow {
                    delta = 0.1 * delta
                        + 0.9
                            * compass_axis_delta(prepared.direction.compass(), used.compass())
                                .abs();
                }
                // Source multiplication order, with false-size baseCost=1.
                distance += prepared.direction_factor * delta * 1.0 * 0.25;
            }
            total += distance;
        }
        let frame = access.frame()?;
        let near_map = frame
            .owner()
            .owner_node_near_map(node)
            .map_err(owner_score_error)?;
        drop(frame);
        if !near_map.is_empty() {
            let mut minimum = f64::INFINITY;
            let cursor = near_map.live_cursor();
            let mut index = 0;
            for near in cursor {
                // As for the edges: only the first entry frame can observe a
                // state no earlier frame has validated.
                let frame = if index == 0 {
                    Some(access.frame()?)
                } else {
                    None
                };
                owner_sizeless_interval!(
                    context,
                    access,
                    frame,
                    "node-evaluate-near",
                    Some(node),
                    index
                );
                index += 1;
                let near =
                    near.expect("runtime error: invalid memory address or nil pointer dereference");
                if frame.position(near).map_err(owner_score_error)?.is_none() {
                    minimum = 0.0;
                    continue;
                }
                minimum = super::owner_sizeless_geometry::source_min(
                    minimum,
                    super::owner_sizeless_geometry::direct_distance(&frame, Some(node), Some(near)),
                );
            }
            total += minimum;
        }
        let frame = access.frame()?;
        let receiver = frame
            .graph_id(node)
            .map_err(owner_score_error)?
            .expect("runtime error: invalid memory address or nil pointer dereference");
        if let Some(nodes) = frame
            .owner()
            .owner_common_uncle_values(receiver, node)
            .map_err(owner_score_error)?
        {
            let axis = super::owner_sizeless_geometry::axis_score(&frame, &nodes);
            total += 1.0 * (1.0 - axis) * (nodes.len() as i64 - 1) as f64;
        }
        owner_sizeless_poll!(
            context,
            access,
            frame,
            "node-edge-length-final",
            Some(node),
            None
        );
        Ok(total)
    }
}

#[cfg(test)]
#[path = "owner_sizeless_operands_tests.rs"]
mod operand_controls;

#[cfg(test)]
mod owner_sizeless_scorer_tests {
    use super::*;
    use std::cell::Cell;

    struct PollContext {
        polls: Cell<usize>,
        cancel_at: Option<usize>,
    }

    impl PollContext {
        fn new(cancel_at: Option<usize>) -> Self {
            Self {
                polls: Cell::new(0),
                cancel_at,
            }
        }
    }

    impl limits::Context for PollContext {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<limits::ContextError> {
            let next = self.polls.get() + 1;
            self.polls.set(next);
            (self.cancel_at == Some(next)).then_some(limits::ContextError::Cancelled)
        }
    }

    fn fixture(degree: usize) -> (ArenaGraph, ArenaGraph, Vec<NodeId>) {
        let mut input = Graph::default();
        let size = Size {
            width: 100.0,
            height: 200.0,
        };
        let a = input.add_node(crate::Node::new("a", size));
        let b = input.add_node(crate::Node::new("b", size));
        for _ in 0..degree {
            let edge = input.add_edge(crate::Edge {
                source: a,
                target: b,
            });
            input.set_edge_arrows(
                edge,
                crate::EdgeArrows {
                    source: false,
                    target: true,
                },
            );
        }
        let mut owner = ArenaGraph::from_input(&input);
        owner.initialize_shared_topology();
        owner.set_position(a, Point { x: 0.0, y: 0.0 });
        owner.set_position(b, Point { x: 3.0, y: 4.0 });
        owner.directions.clear();
        let workspace = owner.clone();
        (owner, workspace, vec![a, b])
    }

    fn score(
        scorer: &mut OwnerSizelessEdgeLengthScorer,
        owner: &ArenaGraph,
        workspace: &ArenaGraph,
        mapping: &[NodeId],
        context: &PollContext,
    ) -> Result<f64, limits::Error> {
        let frame = OwnerScoreFrame::new(
            owner,
            owner.topology_arena_id().unwrap(),
            OwnerGraphId(0),
            mapping,
        )
        .unwrap();
        scorer.score(workspace, &frame, ScoreContext::new(context))
    }

    #[test]
    fn owner_sizeless_lazy_scorer_reads_live_geometry_across_stale_workspace_trials() {
        let (mut owner, workspace, mapping) = fixture(1);
        let mut scorer = OwnerSizelessEdgeLengthScorer::new(NodeId(0), false);
        let first = PollContext::new(None);
        assert_eq!(
            score(&mut scorer, &owner, &workspace, &mapping, &first)
                .unwrap()
                .to_bits(),
            (5.0_f64 + 3.0 / 20.0).to_bits()
        );
        assert_eq!(first.polls.get(), 7);
        assert_eq!(scorer.prepared.as_ref().unwrap().checks, 4);
        owner.set_position(mapping[0], Point { x: 1.0, y: 0.0 });
        owner.cell_size = 987.0;
        owner.nodes[0].rect.size.width = 987.0;
        let repeated = PollContext::new(None);
        assert_eq!(
            score(&mut scorer, &owner, &workspace, &mapping, &repeated)
                .unwrap()
                .to_bits(),
            (20.0_f64.sqrt() + 2.0 / 20.0).to_bits()
        );
        assert_eq!(repeated.polls.get(), 7);
        assert_eq!(workspace.nodes[0].position, Some(Point { x: 0.0, y: 0.0 }));
        assert_eq!(scorer.prepared.as_ref().unwrap().checks, 4);
    }

    #[test]
    fn owner_sizeless_nil_receiver_is_read_only_after_an_eligible_adjacent() {
        for degree in [0, 1] {
            let (mut owner, workspace, mapping) = fixture(degree);
            owner.clear_position(mapping[0]);
            owner.clear_position(mapping[1]);
            let mut scorer = OwnerSizelessEdgeLengthScorer::new(NodeId(0), true);
            let context = PollContext::new(None);
            assert_eq!(
                score(&mut scorer, &owner, &workspace, &mapping, &context).unwrap(),
                0.0
            );
            assert_eq!(context.polls.get(), if degree == 0 { 2 } else { 7 });
            assert!(workspace.nodes[0].position.is_some());
            if degree != 0 {
                owner.set_position(mapping[1], Point { x: 3.0, y: 4.0 });
                let canceled = PollContext::new(Some(1));
                assert_eq!(
                    score(&mut scorer, &owner, &workspace, &mapping, &canceled)
                        .unwrap_err()
                        .to_string(),
                    "EdgeLength: context canceled"
                );
                assert_eq!(canceled.polls.get(), 1);
                let context = PollContext::new(None);
                let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    score(&mut scorer, &owner, &workspace, &mapping, &context)
                }))
                .unwrap_err();
                assert_eq!(
                    panic
                        .downcast_ref::<&str>()
                        .copied()
                        .or_else(|| panic.downcast_ref::<String>().map(String::as_str)),
                    Some("runtime error: invalid memory address or nil pointer dereference")
                );
                assert_eq!(context.polls.get(), 6);
            }
        }
    }

    #[test]
    fn owner_sizeless_canceled_preparation_retries_and_canceled_replay_retains_setup() {
        let (owner, workspace, mapping) = fixture(1);
        let mut scorer = OwnerSizelessEdgeLengthScorer::new(NodeId(0), true);
        let context = PollContext::new(Some(3));
        assert_eq!(
            score(&mut scorer, &owner, &workspace, &mapping, &context)
                .unwrap_err()
                .to_string(),
            "EdgeLength: context canceled"
        );
        assert_eq!(context.polls.get(), 3);
        assert!(scorer.prepared.is_none());
        let retry = PollContext::new(None);
        assert_eq!(
            score(&mut scorer, &owner, &workspace, &mapping, &retry).unwrap(),
            5.15
        );
        assert_eq!(retry.polls.get(), 7);
        let canceled_replay = PollContext::new(Some(4));
        assert_eq!(
            score(&mut scorer, &owner, &workspace, &mapping, &canceled_replay)
                .unwrap_err()
                .to_string(),
            "EdgeLength: context canceled"
        );
        assert_eq!(canceled_replay.polls.get(), 4);
        assert!(scorer.prepared.is_some());
        let canceled_final = PollContext::new(Some(7));
        assert_eq!(
            score(&mut scorer, &owner, &workspace, &mapping, &canceled_final)
                .unwrap_err()
                .to_string(),
            "EdgeLength: context canceled"
        );
        assert_eq!(canceled_final.polls.get(), 7);
        assert!(scorer.prepared.is_some());
        let retry = PollContext::new(None);
        assert_eq!(
            score(&mut scorer, &owner, &workspace, &mapping, &retry).unwrap(),
            5.15
        );
        assert_eq!(retry.polls.get(), 7);
    }

    #[test]
    fn owner_sizeless_close_is_repeatable_and_context_precedes_closed_error() {
        let (owner, workspace, mapping) = fixture(1);
        let mut scorer = OwnerSizelessEdgeLengthScorer::new(NodeId(0), true);
        score(
            &mut scorer,
            &owner,
            &workspace,
            &mapping,
            &PollContext::new(None),
        )
        .unwrap();
        scorer.close();
        scorer.close();
        assert!(scorer.prepared.is_none());
        let context = PollContext::new(None);
        assert_eq!(
            score(&mut scorer, &owner, &workspace, &mapping, &context)
                .unwrap_err()
                .to_string(),
            "EdgeLength: scorer is closed"
        );
        assert_eq!(context.polls.get(), 1);
        let context = PollContext::new(Some(1));
        assert_eq!(
            score(&mut scorer, &owner, &workspace, &mapping, &context)
                .unwrap_err()
                .to_string(),
            "EdgeLength: context canceled"
        );
        assert_eq!(context.polls.get(), 1);
    }

    #[test]
    fn owner_sizeless_fixed_topology_preserves_none_presence_and_reads_near_uncle_terms() {
        let (mut owner, workspace, mapping) = fixture(1);
        let mut scorer = OwnerSizelessEdgeLengthScorer::new(NodeId(0), true);
        score(
            &mut scorer,
            &owner,
            &workspace,
            &mapping,
            &PollContext::new(None),
        )
        .unwrap();
        owner
            .set_owner_graph_direction_entry(OwnerGraphId(0), None, Some(Orientation::None))
            .unwrap();
        let context = PollContext::new(None);
        assert!(
            score(&mut scorer, &owner, &workspace, &mapping, &context)
                .unwrap_err()
                .to_string()
                .contains("topology changed")
        );
        assert_eq!(context.polls.get(), 1);
        for uncle in [false, true] {
            let (mut owner, workspace, mapping) = fixture(1);
            if uncle {
                owner
                    .common_uncle_siblings
                    .replace_header(SharedSliceMap::new());
                owner
                    .common_uncle_siblings
                    .insert(mapping[0], vec![mapping[1]]);
            } else {
                owner.nodes[0].nears.push(mapping[1]);
            }
            let mut scorer = OwnerSizelessEdgeLengthScorer::new(NodeId(0), false);
            let context = PollContext::new(None);
            let value = score(&mut scorer, &owner, &workspace, &mapping, &context).unwrap();
            assert_eq!(
                value.to_bits(),
                if uncle {
                    5.15_f64.to_bits()
                } else {
                    10.15_f64.to_bits()
                }
            );
            assert_eq!(context.polls.get(), if uncle { 7 } else { 8 });
            assert!(scorer.prepared.is_some());
        }
    }

    #[test]
    fn owner_sizeless_direction_uses_actual_arrow_table_and_false_size_cost() {
        for (source_arrow, target_arrow, root_direction, table, term) in [
            (false, true, None, false, 0.0),
            (false, true, Some(Orientation::Right), false, 0.375),
            (true, false, Some(Orientation::Right), false, 1.125),
            (false, false, Some(Orientation::Right), false, 0.375),
            (true, true, Some(Orientation::Right), false, 0.375),
            (false, false, None, false, 0.0),
            (false, true, None, true, 0.125),
        ] {
            let (mut owner, mut workspace, mapping) = fixture(1);
            owner
                .set_owner_graph_direction_entry(OwnerGraphId(0), None, root_direction)
                .unwrap();
            owner.nodes[0].shape = if table {
                ShapeKind::SqlTable
            } else {
                ShapeKind::Rectangle
            };
            owner.edge_records[0].source_arrow = source_arrow;
            owner.edge_records[0].target_arrow = target_arrow;
            workspace.edge_records[0].source_arrow = source_arrow;
            workspace.edge_records[0].target_arrow = target_arrow;
            owner.cell_size = 987.0;
            let mut scorer = OwnerSizelessEdgeLengthScorer::new(NodeId(0), true);
            let context = PollContext::new(None);
            assert_eq!(
                score(&mut scorer, &owner, &workspace, &mapping, &context)
                    .unwrap()
                    .to_bits(),
                (5.0_f64 + 3.0 / 20.0 + term).to_bits()
            );
            assert_eq!(context.polls.get(), 7);
        }
    }

    #[test]
    fn owner_sizeless_large_incidence_replays_source_sixty_four_item_polls() {
        let (owner, workspace, mapping) = fixture(65);
        let mut scorer = OwnerSizelessEdgeLengthScorer::new(NodeId(0), false);
        for _ in 0..2 {
            let context = PollContext::new(None);
            let value = score(&mut scorer, &owner, &workspace, &mapping, &context).unwrap();
            let mut expected = 0.0;
            for _ in 0..65 {
                expected += 5.0_f64 + 3.0 / 20.0;
            }
            assert_eq!(value.to_bits(), expected.to_bits());
            assert_eq!(context.polls.get(), 12);
            assert_eq!(scorer.prepared.as_ref().unwrap().checks, 8);
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct SymmetryNeighbor {
    identity: i64,
    owner: NodeId,
    projected: bool,
    container_tala_id: Option<i64>,
    position: Option<Point>,
    size: Size,
}

thread_local! {
    static SYMMETRY_NODE_SCRATCH: std::cell::RefCell<Vec<(Vec<i64>, Vec<SymmetryNeighbor>)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// symmetry.go's borrowSymmetryNodes: each recursive score checks out its own
/// buffers. Keep identities and neighbors in source order; retain no geometry
/// between calls, including when cancellation or a panic unwinds the score.
struct SymmetryNodeScratch {
    seen: Vec<i64>,
    adjacent: Vec<SymmetryNeighbor>,
}
impl SymmetryNodeScratch {
    fn take() -> Self {
        let (seen, adjacent) =
            SYMMETRY_NODE_SCRATCH.with(|pool| pool.borrow_mut().pop().unwrap_or_default());
        Self { seen, adjacent }
    }
}
impl Drop for SymmetryNodeScratch {
    fn drop(&mut self) {
        self.seen.clear();
        self.adjacent.clear();
        if self.seen.capacity() > MAX_SCORING_SCRATCH_ENTRIES
            || self.adjacent.capacity() > MAX_SCORING_SCRATCH_ENTRIES
        {
            return;
        }
        let _ = SYMMETRY_NODE_SCRATCH.try_with(|pool| {
            let mut pool = pool.borrow_mut();
            if pool.len() < MAX_SCORING_SCRATCH_BUFFERS {
                pool.push((
                    std::mem::take(&mut self.seen),
                    std::mem::take(&mut self.adjacent),
                ));
            }
        });
    }
}

/// placementcost/obstruction_bounds.go: a conservative rejection before the
/// original ancestry and segment predicates. Non-finite or very large trial
/// boxes retain the original path, as do touching and overlapping boxes.
#[derive(Clone, Copy, Default)]
struct SymmetryObstructionBounds {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
    usable: bool,
}

impl SymmetryObstructionBounds {
    fn from_box(position: Option<Point>, size: Size) -> Self {
        let Some(position) = position else {
            return Self::default();
        };
        let right = position.x + size.width;
        let bottom = position.y + size.height;
        let bound = f64::MAX / 8.0;
        Self {
            left: position.x.min(right),
            top: position.y.min(bottom),
            right: position.x.max(right),
            bottom: position.y.max(bottom),
            usable: [position.x, position.y, right, bottom]
                .into_iter()
                .all(|value| value >= -bound && value <= bound),
        }
    }

    fn including(self, other: Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
            usable: self.usable && other.usable,
        }
    }

    fn excludes(self, position: Option<Point>, size: Size) -> bool {
        if !self.usable {
            return false;
        }
        let Some(position) = position else {
            return false;
        };
        let right = position.x + size.width;
        let bottom = position.y + size.height;
        if ![position.x, position.y, right, bottom]
            .into_iter()
            .all(f64::is_finite)
        {
            return false;
        }
        (position.x < self.left && right < self.left)
            || (position.x > self.right && right > self.right)
            || (position.y < self.top && bottom < self.top)
            || (position.y > self.bottom && bottom > self.bottom)
    }
}

impl SymmetryNeighbor {
    // Go's identity-only and empty-neighbor paths never dereference TopLeft.
    // A nil TopLeft remains invalid at a meaningful geometry operation.
    fn positioned(self) -> Point {
        self.position
            .expect("TALA symmetry geometry requires a positioned node")
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct SizedRestoredEndpoints {
    node: Option<AbductedNodeRef>,
    adjacent: Option<AbductedNodeRef>,
    abduction: Option<(usize, bool)>,
}

struct SizedScoringPrepared {
    restored_endpoints: Option<ScoringScratchVec<SizedRestoredEndpoints>>,
    route_restored_endpoints: Option<Option<ScoringScratchVec<SizedRestoredEndpoints>>>,
    desired: Orientation,
    factor: f64,
    edge_label_counts: Vec<((i64, i64), usize)>,
    checks: usize,
    owner_obstructions: Vec<Option<SizedOwnerObstructions>>,
}

/// edge_length.go:875-901 captures ordered Containers headers, not geometry.
/// A geometry-only candidate sweep can retain the same headers until it ends.
struct SizedOwnerObstructions {
    arena: OwnerArenaId,
    view: OwnerGraphId,
    replacements: (NodeId, NodeId),
    sets: super::owner_score_frame::OwnerObstructionSets,
    filters: Vec<OwnerObstructionFilter>,
}

/// The ancestry, graph and grouping predicates in edge_length.go:914-946
/// depend only on topology. Record each stage at its original first-reader
/// boundary, after bounds and the item poll. Stable candidate sweeps change
/// geometry only; all other callers recompute.
#[derive(Clone, Copy, Default)]
enum OwnerObstructionFilter {
    #[default]
    Unknown,
    Ancestor,
    Other,
    Excluded,
    Eligible,
}
/// edge_length_scorer.go: lazy fixed-topology sweep with live evaluation.
pub(super) struct SizedEdgeLengthScorer {
    node: Option<NodeId>,
    direction_penalty: bool,
    owner_carrier_positions: bool,
    stable_owner_obstructions: bool,
    prepared: Option<SizedScoringPrepared>,
}
impl SizedEdgeLengthScorer {
    pub(super) fn new(node: NodeId, direction_penalty: bool) -> Self {
        Self {
            node: Some(node),
            direction_penalty,
            owner_carrier_positions: false,
            stable_owner_obstructions: false,
            prepared: None,
        }
    }
    pub(super) fn close(&mut self) {
        self.prepared = None;
        self.node = None;
        self.direction_penalty = false;
        self.owner_carrier_positions = false;
        self.stable_owner_obstructions = false;
    }
    /// Complete ordinary owner projections may score before carrier positions
    /// are published. Other adapters retain their existing geometry contract.
    pub(super) fn use_owner_carrier_positions(&mut self) {
        self.owner_carrier_positions = true;
    }
    /// The caller owns a geometry-only movement sweep; both work and score
    /// contexts must also leave its owning topology unchanged at every poll.
    pub(super) fn use_stable_owner_obstructions(&mut self) {
        self.stable_owner_obstructions = true;
    }
    pub(super) fn score_owner_routes(
        &mut self,
        workspace: &ArenaGraph,
        cache: Option<&[Vec<NodeId>]>,
        owner: &ArenaGraph,
        canonical: NodeId,
        frame: Option<&OwnerScoreFrame<'_>>,
        context: ScoreContext<'_>,
    ) -> Result<f64, limits::Error> {
        workspace.sized_edge_length_with_cache_mode_materialized_flow_mode(
            self.node.unwrap_or(NodeId(0)),
            self.direction_penalty,
            cache,
            false,
            false,
            false,
            workspace.materialized_ordinary_container_scoring(),
            None,
            Some((owner, canonical)),
            frame,
            context,
            Some(self),
        )
    }
}
impl Drop for SizedEdgeLengthScorer {
    fn drop(&mut self) {
        self.close();
    }
}

// Source edge_length.go obtains scratch before preparation and clears it on
// every return. Each checked-out vector owns its storage, so reentrant scoring
// never borrows a live outer buffer. No geometry or live record is cached.
const MAX_SCORING_SCRATCH_ENTRIES: usize = 4096;
const MAX_SCORING_SCRATCH_BUFFERS: usize = 4;
thread_local! {
    static USED_SCORING_SCRATCH: std::cell::RefCell<Vec<Vec<bool>>> = const { std::cell::RefCell::new(Vec::new()) };
    static ENDPOINT_SCORING_SCRATCH: std::cell::RefCell<Vec<Vec<SizedRestoredEndpoints>>> = const { std::cell::RefCell::new(Vec::new()) };
}
trait ScoringScratchElement: Copy + Default + 'static {
    fn pool() -> &'static std::thread::LocalKey<std::cell::RefCell<Vec<Vec<Self>>>>;
}
impl ScoringScratchElement for bool {
    fn pool() -> &'static std::thread::LocalKey<std::cell::RefCell<Vec<Vec<Self>>>> {
        &USED_SCORING_SCRATCH
    }
}
impl ScoringScratchElement for SizedRestoredEndpoints {
    fn pool() -> &'static std::thread::LocalKey<std::cell::RefCell<Vec<Vec<Self>>>> {
        &ENDPOINT_SCORING_SCRATCH
    }
}
#[derive(Debug)]
struct ScoringScratchVec<T: ScoringScratchElement>(Vec<T>);
impl<T: ScoringScratchElement> ScoringScratchVec<T> {
    fn take() -> Self {
        Self(T::pool().with(|pool| pool.borrow_mut().pop().unwrap_or_default()))
    }
    fn resize(&mut self, len: usize, value: T) {
        self.0.resize(len, value);
    }
    fn push(&mut self, value: T) {
        self.0.push(value);
    }
}
impl<T: ScoringScratchElement> std::ops::Deref for ScoringScratchVec<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.0
    }
}
impl<T: ScoringScratchElement> std::ops::DerefMut for ScoringScratchVec<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.0
    }
}
impl<T: ScoringScratchElement> Drop for ScoringScratchVec<T> {
    fn drop(&mut self) {
        // Clear old endpoint identities and used flags before retention. Unlike
        // Go's uncapped replacement buffers, native per-thread retention is
        // bounded; this changes allocation retention only, never scoring work.
        self.0.fill(T::default());
        self.0.clear();
        if self.0.capacity() > MAX_SCORING_SCRATCH_ENTRIES {
            return;
        }
        let _ = T::pool().try_with(|pool| {
            let mut pool = pool.borrow_mut();
            if pool.len() < MAX_SCORING_SCRATCH_BUFFERS {
                pool.push(std::mem::take(&mut self.0));
            }
        });
    }
}

#[derive(Clone, Copy)]
enum ProjectedObstructionSource<'a> {
    Edge,
    Ordered(&'a [ProjectedAdjacent], &'a OrderedObstructionInventory),
    Current,
}

impl ArenaGraph {
    /// The live Containers walk stores an active sequence vessel in its
    /// first member's slot (see `active_node_position`). edge_length.go
    /// 914/925 skips the vessel itself and any ancestor of a replacement
    /// endpoint, so compare that slot as the vessel it stands for.
    fn route_obstruction_identity(&self, obstruction: NodeId) -> NodeId {
        if let Some(index) = self.nodes[obstruction.0 as usize].sequence
            && self.sequences[index].members.first() == Some(&obstruction)
            && let Some(vessel) = self.topology_vessel_node(TopologyVessel::Sequence(index))
            && vessel != obstruction
            && self.node_order_contains(vessel)
        {
            return vessel;
        }
        obstruction
    }

    pub(super) fn projected_obstruction_box(
        &self,
        obstruction: &ProjectedObstructionRef,
    ) -> (Option<Point>, Size) {
        if let Some(source_arena) = obstruction.source_arena
            && self.topology_arena_id() == Some(source_arena)
            && let Some(node) = obstruction
                .source_node
                .and_then(|node| self.topology_node(node))
        {
            return (node.position, node.rect.size);
        }
        // A live root may acquire a position after the inventory was built.
        // Only exact source self identity permits reading this carrier box.
        if let AbductionNodeLocation::Local(owner) = obstruction.location
            && obstruction.source_node.is_some()
            && obstruction.source_node == obstruction.source_owner
        {
            return (self.position(owner), self.active_node_size(owner));
        }
        // Nullable source geometry is authoritative; a positioned carrier
        // cannot revive a genuine nil child pointer's TopLeft.
        if obstruction.position.is_none() {
            return (None, obstruction.size);
        }
        if let Some(projected) = obstruction.projection {
            let geometry = self.sized_projected_box(projected);
            return geometry
                .map(|(position, size)| (Some(position), size))
                .unwrap_or((None, obstruction.size));
        }
        (obstruction.position, obstruction.size)
    }

    pub(super) fn projected_obstruction_same_graph(
        &self,
        obstruction: &ProjectedObstructionRef,
        inventory: &OrderedObstructionInventory,
    ) -> bool {
        if let Some(source_arena) = obstruction.source_arena
            && inventory.source_arena == Some(source_arena)
            && self.topology_arena_id() == Some(source_arena)
            && let (Some(candidate), Some(replacement)) = (
                obstruction
                    .source_node
                    .and_then(|node| self.topology_node_owner(node)),
                inventory
                    .node_replacement
                    .and_then(|node| self.topology_node_owner(node)),
            )
        {
            return candidate == replacement;
        }
        obstruction.same_graph_as_replacement
    }

    fn projected_obstruction_replacement_ancestor(
        &self,
        obstruction: &ProjectedObstructionRef,
        inventory: &OrderedObstructionInventory,
    ) -> bool {
        if let Some(source_arena) = obstruction.source_arena
            && inventory.source_arena == Some(source_arena)
            && self.topology_arena_id() == Some(source_arena)
            && let Some(ancestor) = obstruction.source_node
        {
            return [inventory.node_replacement, inventory.adjacent_replacement]
                .into_iter()
                .flatten()
                .any(|endpoint| {
                    let mut current = self.topology_node(endpoint).and_then(|node| node.container);
                    while let Some(parent) = current {
                        if parent == ancestor {
                            return true;
                        }
                        current = self.topology_node(parent).and_then(|node| node.container);
                    }
                    false
                });
        }
        obstruction.is_replacement_ancestor
    }

    pub(super) fn has_sized_edge_abductions(&self) -> bool {
        self.sized_edge_abductions_present || !self.sized_edge_abductions.is_empty()
    }

    /// Resolve an abducted endpoint through the shared `Graph.Containers`
    /// pointers used by Go.  A projected edge keeps the original endpoint's
    /// owner and a historical offset, but nested containers can be moved by
    /// an enclosing temporary graph.  In that case the historical offset is
    /// no longer an absolute frame: derive the immediate child offset from
    /// the transaction snapshots and apply it to the parent's live projected
    /// position.  This preserves the pointer-relative geometry that Go's
    /// `moveNodeWithChildren` exposes without adding hidden nodes to
    /// `Graph.Nodes` (which would alter the optimizer's node count).
    pub(super) fn projected_external_position(&self, tala_id: i64) -> Option<Point> {
        let trace_position = crate::internal::trace_env_value("TALA_TRACE_EXTERNAL_POSITION")
            .is_some_and(|target| {
                target == "all"
                    || target
                        .split(',')
                        .any(|id| id.trim().parse::<i64>().ok() == Some(tala_id))
            });
        let active = self
            .nodes
            .iter()
            .find(|node| node.tala_id == tala_id && self.node_order.contains(&node.input_id))
            .and_then(|node| self.active_node_position(node.input_id));
        let direct = self
            .transaction_external_container_children
            .iter()
            .flat_map(|(parent, children)| {
                children
                    .iter()
                    .filter(move |node| node.tala_id == tala_id)
                    .map(move |node| (*parent, node.position))
            })
            .collect::<Vec<_>>();
        let aggregate = self
            .transaction_external_aggregate_children
            .iter()
            .flat_map(|(parent, children)| {
                children
                    .iter()
                    .filter(move |node| node.tala_id == tala_id)
                    .map(move |node| (*parent, node.position))
            })
            .collect::<Vec<_>>();
        let containers = self
            .transaction_external_containers
            .iter()
            .filter(|node| node.tala_id == tala_id)
            .map(|node| node.position)
            .collect::<Vec<_>>();
        let live = active
            .or_else(|| containers.iter().find_map(|position| *position))
            .or_else(|| direct.iter().find_map(|(_, position)| *position));
        let result = live.or_else(|| aggregate.iter().find_map(|(_, position)| *position));
        if trace_position {
            eprintln!(
                "EXTERNAL_POSITION_RUST id={} active={:?} direct={:?} aggregate={:?} containers={:?} result={:?}",
                tala_id, active, direct, aggregate, containers, result
            );
        }
        result
    }

    pub(super) fn projected_live_external_position(&self, tala_id: i64) -> Option<Point> {
        self.nodes
            .iter()
            .find(|node| node.tala_id == tala_id && self.node_order.contains(&node.input_id))
            .and_then(|node| self.active_node_position(node.input_id))
            .or_else(|| {
                self.transaction_external_containers
                    .iter()
                    .find(|node| node.tala_id == tala_id)
                    .and_then(|node| node.position)
            })
            .or_else(|| {
                self.transaction_external_container_children
                    .values()
                    .flatten()
                    .find(|node| node.tala_id == tala_id)
                    .and_then(|node| node.position)
            })
    }

    /// Resolve a retained child box in the coordinate frame used by the
    /// active temporary graph.  The Go implementation keeps the original
    /// `*Node` pointers in `Graph.Containers`; moving a parent therefore
    /// moves every descendant while preserving the source child/parent
    /// delta.  Rust's temporary graph deliberately omits those hidden nodes
    /// from `node_order`, so the transaction child maps can contain local
    /// positions instead of absolute positions.  Reconstruct the same
    /// pointer-relative walk from the stable source nodes and the currently
    /// active parent positions.
    pub(super) fn projected_source_position(
        &self,
        tala_id: i64,
        visiting: &mut BTreeSet<i64>,
    ) -> Option<Point> {
        if !visiting.insert(tala_id) {
            return None;
        }
        let source = self.nodes.iter().find(|node| node.tala_id == tala_id);
        let transaction_entry = self
            .projected_transaction_nodes
            .iter()
            .find(|entry| entry.node.tala_id == tala_id);
        if crate::internal::trace_env_value("TALA_TRACE_PROJECTION_CHAIN")
            .is_some_and(|target| target == "all" || target.parse::<i64>().ok() == Some(tala_id))
        {
            eprintln!(
                "PROJECTION_CHAIN_RUST enter id={} source={:?} tx={:?} sourcePos={:?} parentPos={:?} order={:?}",
                tala_id,
                source.map(|node| (
                    node.input_id.0,
                    node.position,
                    node.container.map(|parent| parent.0)
                )),
                transaction_entry.map(|entry| (
                    entry.node.input_id.0,
                    entry.node.position,
                    entry.container_tala_id,
                )),
                self.source_positions_by_tala.get(&tala_id),
                self.source_parent_tala_by_tala
                    .get(&tala_id)
                    .and_then(|parent| parent
                        .and_then(|parent| self.source_positions_by_tala.get(&parent))),
                self.node_order
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
        }
        let Some(source) = source.or_else(|| transaction_entry.map(|entry| &entry.node)) else {
            visiting.remove(&tala_id);
            return None;
        };
        if self.node_order.contains(&source.input_id)
            && self
                .nodes
                .get(source.input_id.0 as usize)
                .is_some_and(|node| node.tala_id == tala_id)
        {
            let position = self.active_node_position(source.input_id);
            visiting.remove(&tala_id);
            return position;
        }

        let parent_tala_id = self
            .source_parent_tala_by_tala
            .get(&tala_id)
            .copied()
            .flatten()
            .or_else(|| {
                source
                    .container
                    .and_then(|parent| self.nodes.get(parent.0 as usize))
                    .map(|parent| parent.tala_id)
            })
            .or_else(|| {
                self.projected_transaction_nodes
                    .iter()
                    .find(|entry| entry.node.tala_id == tala_id)
                    .and_then(|entry| entry.container_tala_id)
            });
        if let Some(parent_tala_id) = parent_tala_id
            && let Some(parent_position) = self.projected_source_position(parent_tala_id, visiting)
        {
            let relative = self
                .source_positions_by_tala
                .get(&tala_id)
                .copied()
                .zip(self.source_positions_by_tala.get(&parent_tala_id).copied())
                .map(|(child_position, parent_position)| Point {
                    x: child_position.x - parent_position.x,
                    y: child_position.y - parent_position.y,
                })
                .or_else(|| {
                    self.nodes
                        .iter()
                        .find(|node| node.tala_id == tala_id)
                        .and_then(|child| {
                            let parent = self
                                .nodes
                                .iter()
                                .find(|node| node.tala_id == parent_tala_id)?;
                            let child_position = child.position?;
                            let parent_position = parent.position?;
                            Some(Point {
                                x: child_position.x - parent_position.x,
                                y: child_position.y - parent_position.y,
                            })
                        })
                })
                .or_else(|| {
                    let child_position = transaction_entry.and_then(|entry| entry.node.position)?;
                    let parent_position = self
                        .projected_transaction_nodes
                        .iter()
                        .find(|entry| entry.node.tala_id == parent_tala_id)
                        .and_then(|entry| entry.node.position)?;
                    Some(Point {
                        x: child_position.x - parent_position.x,
                        y: child_position.y - parent_position.y,
                    })
                })
                .or_else(|| {
                    self.transaction_external_container_children
                        .get(&parent_tala_id)
                        .and_then(|children| {
                            let child = children.iter().find(|node| node.tala_id == tala_id)?;
                            child.position
                        })
                });
            if let Some(relative) = relative {
                if crate::internal::trace_env_value("TALA_TRACE_PROJECTION_CHAIN").is_some_and(
                    |target| target == "all" || target.parse::<i64>().ok() == Some(tala_id),
                ) {
                    eprintln!(
                        "PROJECTION_CHAIN_RUST resolve id={} parent={} parentPos={:?} relative={:?}",
                        tala_id, parent_tala_id, parent_position, relative
                    );
                }
                visiting.remove(&tala_id);
                return Some(Point {
                    x: parent_position.x + relative.x,
                    y: parent_position.y + relative.y,
                });
            }
        }

        // A source node without a resolvable parent can still be represented
        // by a transaction child map.  This fallback is intentionally last:
        // such entries are already absolute only for the outermost retained
        // carrier; descendants are handled by the recursive branch above.
        let fallback = self
            .transaction_external_container_children
            .values()
            .flatten()
            .find(|node| node.tala_id == tala_id)
            .and_then(|node| node.position)
            .or(source.position);
        visiting.remove(&tala_id);
        fallback
    }

    fn projected_live_source_position(
        &self,
        tala_id: i64,
        visiting: &mut BTreeSet<i64>,
    ) -> Option<Point> {
        if !visiting.insert(tala_id) {
            return None;
        }
        let source = self.nodes.iter().find(|node| node.tala_id == tala_id);
        if let Some(source) = source
            && self.node_order.contains(&source.input_id)
        {
            let position = self.active_node_position(source.input_id);
            visiting.remove(&tala_id);
            return position;
        }

        let parent_tala_id = self
            .source_parent_tala_by_tala
            .get(&tala_id)
            .copied()
            .flatten()
            .or_else(|| {
                source
                    .and_then(|node| node.container)
                    .and_then(|parent| self.nodes.get(parent.0 as usize))
                    .map(|parent| parent.tala_id)
            });
        if let Some(parent_tala_id) = parent_tala_id {
            let relative = self
                .source_positions_by_tala
                .get(&tala_id)
                .copied()
                .zip(self.source_positions_by_tala.get(&parent_tala_id).copied())
                .map(|(child, parent)| Point {
                    x: child.x - parent.x,
                    y: child.y - parent.y,
                })
                .or_else(|| {
                    let child = source?.position?;
                    let parent = self
                        .nodes
                        .iter()
                        .find(|node| node.tala_id == parent_tala_id)?
                        .position?;
                    Some(Point {
                        x: child.x - parent.x,
                        y: child.y - parent.y,
                    })
                });
            let parent_position = self
                .projected_live_external_position(parent_tala_id)
                .or_else(|| self.projected_live_source_position(parent_tala_id, visiting));
            if let Some(position) = relative
                .zip(parent_position)
                .map(|(relative, parent)| Point {
                    x: parent.x + relative.x,
                    y: parent.y + relative.y,
                })
            {
                visiting.remove(&tala_id);
                return Some(position);
            }
        }

        let position = self
            .projected_live_external_position(tala_id)
            .or_else(|| self.projected_external_position(tala_id))
            .or_else(|| source.and_then(|node| node.position));
        visiting.remove(&tala_id);
        position
    }

    fn retained_original_position(&self, tala_id: i64) -> Option<Option<Point>> {
        // Stop at a found node even when its real TopLeft is nil. A
        // positive-only lookup could resurrect an older aggregate copy.
        self.nodes
            .iter()
            .find(|node| node.tala_id == tala_id && self.node_order.contains(&node.input_id))
            .map(|node| self.active_node_position(node.input_id))
            .or_else(|| {
                self.transaction_external_containers
                    .iter()
                    .find(|node| node.tala_id == tala_id)
                    .map(|node| node.position)
            })
            .or_else(|| {
                self.transaction_external_container_children
                    .values()
                    .flatten()
                    .find(|node| node.tala_id == tala_id)
                    .map(|node| node.position)
            })
            .or_else(|| {
                self.transaction_external_aggregate_children
                    .values()
                    .flatten()
                    .find(|node| node.tala_id == tala_id)
                    .map(|node| node.position)
            })
    }

    fn abducted_node_projection(&self, original: AbductedNodeRef) -> Option<ProjectedAdjacent> {
        let local_owner = original.owner?;
        original.projection().or_else(|| {
            let bound_cluster_owner = self
                .active_cluster_index(local_owner)
                .and_then(|index| self.topology_vessel_node(TopologyVessel::Cluster(index)));
            let child = if (original.owner == original.source_owner
                || (bound_cluster_owner.is_some() && bound_cluster_owner == original.source_owner))
                && original.source_node.is_some_and(|node| {
                    self.nodes
                        .get(node.0 as usize)
                        .is_some_and(|node| node.tala_id == original.tala_id)
                }) {
                self.position(original.source_node.unwrap())?
            } else {
                self.retained_original_position(original.tala_id)??
            };
            let owner = self.position(local_owner)?;
            Some(ProjectedAdjacent {
                owner: local_owner,
                tala_id: original.tala_id,
                container_tala_id: original.container_tala_id,
                offset: Point {
                    x: child.x - owner.x,
                    y: child.y - owner.y,
                },
                size: original.size,
                cluster_member: original.cluster_member,
            })
        })
    }

    pub(super) fn projected_absolute_position(
        &self,
        projected: ProjectedAdjacent,
    ) -> Option<Point> {
        let owner = self.position(projected.owner)?;
        // A hand-authored or ordinary projection can name its owner as the
        // projected identity (the test and non-abducted edge form). In that
        // case the offset is the authoritative box displacement; resolving
        // the identity through the active arena would discard the offset.
        let owner_tala_id = self.nodes[projected.owner.0 as usize].tala_id;
        // A published canonical sequence vessel owns retained member pointers,
        // even though those members are absent from Graph.Nodes/Containers.
        // Their saved Container is provenance, not the frame of this vessel
        // relative offset. Read the actual live member point as Go does.
        if self.node_order_contains(projected.owner) {
            for (index, sequence) in self.sequences.iter().enumerate() {
                if self.topology_vessel_node(TopologyVessel::Sequence(index))
                    == Some(projected.owner)
                    && let Some(&member) = sequence
                        .members
                        .iter()
                        .find(|&&member| self.nodes[member.0 as usize].tala_id == projected.tala_id)
                {
                    return self.position(member);
                }
            }
        }
        if owner_tala_id == projected.tala_id
            // A retained ordinary descendant whose owner is its immediate
            // container is already expressed in that container's local
            // frame. Go's shared child pointer follows the container move,
            // so using the transaction snapshot/source-chain here can retain
            // a stale one-pixel phase from the pre-scope graph.
            || projected.container_tala_id == Some(owner_tala_id)
        {
            return Some(Point {
                x: owner.x + projected.offset.x,
                y: owner.y + projected.offset.y,
            });
        }
        // Sequence members are retained outside the ordinary Graph.Containers
        // slice while their vessel is active. Go keeps their node pointers in
        // Graph.Sequences, and moving the vessel translates those member boxes
        // with it. The aggregate-child snapshot is the stable-arena equivalent
        // of that live pointer. Prefer it before rebuilding the endpoint from
        // its outer container and a pre-abduction offset; that offset can be
        // identical for members of different sequences and collapse an edge
        // between them to zero length.
        if let Some(position) = self
            .transaction_external_aggregate_children
            .get(&owner_tala_id)
            .and_then(|members| {
                members
                    .iter()
                    .find(|member| member.tala_id == projected.tala_id)
                    .and_then(|member| member.position)
            })
        {
            return Some(position);
        }
        if let Some(container_tala_id) = projected.container_tala_id
            && container_tala_id != owner_tala_id
        {
            if let Some(position) = self.projected_live_external_position(projected.tala_id) {
                return Some(position);
            }
            if let Some(container_position) = self.projected_external_position(container_tala_id) {
                return Some(Point {
                    x: container_position.x + projected.offset.x,
                    y: container_position.y + projected.offset.y,
                });
            }
        }
        self.projected_external_position(projected.tala_id)
            .or(Some(Point {
                x: owner.x + projected.offset.x,
                y: owner.y + projected.offset.y,
            }))
    }

    /// Resolve the box of a retained cluster vessel in the current graph.
    /// The cached offset is only relative to the carrier frame where the
    /// abduction was captured; an enclosing container can subsequently move
    /// while the shared Go vessel and its members stay at their live positions.
    fn sized_projected_cluster_distance_box(
        &self,
        projected: ProjectedAdjacent,
    ) -> Option<(Point, Size)> {
        let retained = self
            .sized_cluster_distance_boxes
            .get(&(projected.owner, projected.tala_id))?;
        let live_cluster_position = self
            .clusters
            .iter()
            .enumerate()
            .find(|(_, cluster)| {
                cluster.vessel_tala_id == retained.vessel_tala_id && self.cluster_is_active(cluster)
            })
            .and_then(|(index, cluster)| {
                self.pending_cluster_vessel_positions
                    .get(&index)
                    .copied()
                    .or_else(|| {
                        cluster
                            .members
                            .first()
                            .and_then(|member| self.active_node_position(*member))
                    })
            });
        let live_projected_vessel_position = self
            .projected_node_position_by_tala(retained.vessel_tala_id)
            .or_else(|| {
                self.transaction_external_aggregate_children
                    .get(&retained.vessel_tala_id)
                    .and_then(|members| {
                        members
                            .iter()
                            .filter_map(|member| {
                                self.projected_live_source_position(
                                    member.tala_id,
                                    &mut BTreeSet::new(),
                                )
                            })
                            .reduce(|left, right| Point {
                                x: left.x.min(right.x),
                                y: left.y.min(right.y),
                            })
                    })
            });
        let position = live_cluster_position
            .or(live_projected_vessel_position)
            .or_else(|| {
                self.position(projected.owner).map(|owner| Point {
                    x: owner.x + retained.offset.x,
                    y: owner.y + retained.offset.y,
                })
            })?;
        Some((position, retained.size))
    }

    fn scoring_live_cluster_vessel_positions(
        &self,
        endpoints: Option<&[SizedRestoredEndpoints]>,
    ) -> BTreeMap<i64, Point> {
        let mut projected_members = BTreeMap::<i64, BTreeMap<i64, Point>>::new();
        for endpoint in endpoints.into_iter().flatten() {
            for projected in [endpoint.node, endpoint.adjacent].into_iter().flatten() {
                if !projected.cluster_member {
                    continue;
                }
                let Some(owner) = projected.owner else {
                    continue;
                };
                let Some(retained) = self
                    .sized_cluster_distance_boxes
                    .get(&(owner, projected.tala_id))
                else {
                    continue;
                };
                // Go scores a temporary cluster vessel at the live positions
                // of its original member pointers.  A projected member's
                // absolute position can instead be rebased through a nested
                // temporary container, which gives the vessel a different
                // frame even though the member snapshots still match Go's
                // pointer positions.
                let Some(position) =
                    self.projected_external_position(projected.tala_id)
                        .or_else(|| {
                            self.abducted_node_projection(projected)
                                .and_then(|projection| self.projected_absolute_position(projection))
                        })
                else {
                    continue;
                };
                projected_members
                    .entry(retained.vessel_tala_id)
                    .or_default()
                    .insert(projected.tala_id, position);
            }
        }

        projected_members
            .into_iter()
            .filter_map(|(vessel_tala_id, positions)| {
                let members = self
                    .transaction_external_aggregate_children
                    .get(&vessel_tala_id)?;
                let expected = members
                    .iter()
                    .map(|member| member.tala_id)
                    .collect::<BTreeSet<_>>();
                if positions.keys().copied().collect::<BTreeSet<_>>() != expected {
                    return None;
                }
                positions
                    .into_values()
                    .reduce(|left, right| Point {
                        x: left.x.min(right.x),
                        y: left.y.min(right.y),
                    })
                    .map(|position| (vessel_tala_id, position))
            })
            .collect()
    }

    pub(super) fn is_occupied(&self, point: Point, excluding: Option<NodeId>) -> bool {
        self.node_order.iter().copied().any(|node| {
            Some(node) != excluding
                && self.nodes[node.0 as usize]
                    .position
                    .is_some_and(|position| position == point)
        })
    }

    pub(super) fn adjacents(&self, node: NodeId) -> Vec<NodeId> {
        let arena_node = &self.nodes[node.0 as usize];
        let mut seen = BTreeSet::new();
        let mut result = Vec::new();
        for edge in arena_node.edges.read().iter().copied() {
            let adjacent = self.adjacent(node, edge);
            if adjacent != node && seen.insert(adjacent) {
                result.push(adjacent);
            }
        }
        // Node.getReachableNodes calls orderedNears after walking edges; the
        // recovered helper sorts the near map by TALA node ID before it
        // appends those neighbors. Materialize the actual map only at this
        // ordered source boundary; raw guarded eligibility uses a live cursor.
        let mut nears = arena_node.nears.keys();
        nears.sort_by_key(|near| self.nodes[near.0 as usize].tala_id);
        for near in nears {
            if seen.insert(near) {
                result.push(near);
            }
        }
        result
    }

    /// metrics.go substitutes an active Cluster vessel before Sequence.
    /// Activity is the actual Vessel.Graph pointer, independently of group
    /// Graph fields and graph-node membership. Projection precedes TopLeft.
    fn median_near_group_node(&self, near: NodeId) -> NodeId {
        use crate::internal::owner_groups::OwnerGroupKind;
        for kind in [OwnerGroupKind::Cluster, OwnerGroupKind::Sequence] {
            if let Some(vessel) = self
                .owner_node_group_vessel(near, kind)
                .expect("median Near group binding")
            {
                let vessel = vessel
                    .expect("runtime error: invalid memory address or nil pointer dereference");
                if self.topology_node_owner(vessel).is_some() {
                    return vessel;
                }
            }
        }
        near
    }

    /// metrics.go uses the moving Node.Graph's NodeToTree map after group
    /// projection, walks live Parent fields, then selects the root sentinel.
    fn median_near_node(&self, node: NodeId, near: NodeId) -> NodeId {
        let near = self.median_near_group_node(near);
        let graph = self
            .topology_node_owner(node)
            .expect("runtime error: invalid memory address or nil pointer dereference");
        let Some(tree) = self
            .owner_graph_node_to_tree_entry(graph, Some(near))
            .expect("median Near tree binding")
        else {
            return near;
        };
        let mut tree =
            tree.expect("runtime error: invalid memory address or nil pointer dereference");
        let root = loop {
            let fields = self.owner_tree(tree).expect("median Near Tree").fields();
            if let Some(parent) = fields.parent {
                tree = parent;
            } else {
                break fields;
            }
        };
        let edge = root
            .sentinel_edge
            .expect("runtime error: invalid memory address or nil pointer dereference");
        let from = self
            .owner_edge_endpoint(edge, false)
            .expect("median sentinel From");
        let sentinel = if from == root.node {
            self.owner_edge_endpoint(edge, true)
                .expect("median sentinel To")
        } else {
            from
        };
        sentinel.expect("runtime error: invalid memory address or nil pointer dereference")
    }

    fn median_ordered_nears(&self, node: NodeId) -> Vec<NodeId> {
        let mut nears = self.nodes[node.0 as usize].nears.keys();
        nears.sort_by_key(|near| self.nodes[near.0 as usize].tala_id);
        nears
    }

    fn median_fallback_nears(&self, node: NodeId) -> Vec<NodeId> {
        use crate::internal::owner_groups::OwnerGroupKind;
        let direct: Vec<_> = self
            .median_ordered_nears(node)
            .into_iter()
            .map(|near| self.median_near_node(node, near))
            .filter(|near| {
                self.position(*near).is_some()
                    && self.is_descendant_of_scope(*near, self.nodes[node.0 as usize].container)
            })
            .collect();
        if !direct.is_empty() {
            return direct;
        }
        let graph = self
            .topology_node_owner(node)
            .expect("runtime error: invalid memory address or nil pointer dereference");
        let mut nears = Vec::new();
        if self.owner_is_cluster_vessel(node) {
            let cluster = self
                .owner_graph_group_entry(graph, OwnerGroupKind::Cluster, Some(node))
                .expect("median Cluster map")
                .flatten()
                .expect("runtime error: invalid memory address or nil pointer dereference");
            for member in self
                .owner_group_fields(cluster)
                .expect("median Cluster")
                .nodes
                .iter_slots()
            {
                let member = member
                    .expect("runtime error: invalid memory address or nil pointer dereference");
                nears.extend(self.median_ordered_nears(member));
            }
        }
        if let Some(sequence) = self
            .owner_graph_group_entry(graph, OwnerGroupKind::Sequence, Some(node))
            .expect("median Sequence map")
            .flatten()
        {
            for member in self
                .owner_group_fields(sequence)
                .expect("median Sequence")
                .nodes
                .iter_slots()
            {
                let member = member
                    .expect("runtime error: invalid memory address or nil pointer dereference");
                nears.extend(self.median_ordered_nears(member));
            }
        }
        if let Some(roots) = self
            .owner_graph_tree_roots_entry(graph, Some(node))
            .expect("median Tree roots")
        {
            for index in 0..roots.len() {
                let mut queue = std::collections::VecDeque::from([roots
                    .get(index)
                    .expect("captured median Tree root slot")]);
                while let Some(tree) = queue.pop_front() {
                    let tree = tree
                        .expect("runtime error: invalid memory address or nil pointer dereference");
                    let fields = self.owner_tree(tree).expect("median Tree").fields();
                    let member = fields
                        .node
                        .expect("runtime error: invalid memory address or nil pointer dereference");
                    nears.extend(self.median_ordered_nears(member));
                    for index in 0..fields.children.len() {
                        queue.push_back(
                            fields
                                .children
                                .get(index)
                                .expect("captured median Tree child slot"),
                        );
                    }
                }
            }
        }
        // Collected member/tree Nears retain multiplicity and are projected
        // only after collection. Unlike direct Nears, source applies no
        // descendant-of-container filter at this final fallback stage.
        nears
            .into_iter()
            .map(|near| self.median_near_node(node, near))
            .filter(|near| self.position(*near).is_some())
            .collect()
    }

    // Direct translation of Nodes.median for sizeless placement.
    pub(super) fn median_to_neighbors(&self, node: NodeId) -> Point {
        // getAdjacents intentionally preserves parallel-edge duplicates and
        // self-loop adjacency. Near nodes are only the fallback when no edge
        // adjacent is currently positioned.
        let mut adjacent: Vec<_> = self.nodes[node.0 as usize]
            .edges
            .read()
            .iter()
            .copied()
            .map(|edge| self.adjacent(node, edge))
            .filter(|candidate| self.position(*candidate).is_some())
            .collect();
        if adjacent.is_empty() {
            adjacent.extend(self.median_fallback_nears(node));
        }
        if adjacent.is_empty() {
            return self.position(node).unwrap_or_default();
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_MEDIAN_DETAIL")
            && self.nodes[node.0 as usize].tala_id == 923_577_301
        {
            eprint!(
                "MEDIAN_SIZeless_ADJ_RUST node={} ",
                self.nodes[node.0 as usize].tala_id
            );
            for candidate in &adjacent {
                eprint!(
                    "{}:{:?}:{}:{} ",
                    self.nodes[candidate.0 as usize].tala_id,
                    self.position(*candidate),
                    self.nodes[candidate.0 as usize].rect.size.width,
                    self.nodes[candidate.0 as usize].rect.size.height
                );
            }
            eprintln!();
        }
        let median_axis = |x_axis: bool| {
            let mut ordered = adjacent.clone();
            ordered.sort_by(|a, b| {
                let a_position = self.position(*a).unwrap();
                let b_position = self.position(*b).unwrap();
                let a_value = if x_axis { a_position.x } else { a_position.y };
                let b_value = if x_axis { b_position.x } else { b_position.y };
                a_value.total_cmp(&b_value).then_with(|| {
                    self.nodes[a.0 as usize]
                        .tala_id
                        .cmp(&self.nodes[b.0 as usize].tala_id)
                })
            });
            let middle = ordered.len() / 2;
            let position = self.position(ordered[middle]).unwrap();
            let mut median = (if x_axis { position.x } else { position.y }) + 0.5;
            if ordered.len() % 2 == 0 {
                let previous = self.position(ordered[middle - 1]).unwrap();
                median = (median + if x_axis { previous.x } else { previous.y }) / 2.0;
            }
            median
        };
        Point {
            x: median_axis(true),
            y: median_axis(false),
        }
    }

    pub(super) fn sized_median_to_neighbors(&self, node: NodeId) -> Point {
        let active_edges = self.max_edge_length_incident_edges(node);
        let moving = &self.nodes[node.0 as usize];
        let trace_median_adjacents = crate::internal::trace_env_value(
            "D2RS_TRACE_SIZED_MEDIAN_NODE",
        )
        .is_some_and(|target| {
            (target == "all"
                || target == moving.external_id
                || target.parse::<i64>().ok() == Some(moving.tala_id))
                && super::sized::trace_sized_graph_scope(self)
                && (crate::internal::trace_env_value("TALA_TRACE_SIZED_PASS").is_none()
                    || crate::internal::trace_env_enabled("TALA_TRACE_SIZED_PASS_ACTIVE"))
        });
        let mut used_abductions = vec![false; self.sized_edge_abductions.len()];
        let mut adjacent: Vec<_> = active_edges
            .into_iter()
            .filter_map(|edge| {
                let candidate = self.active_adjacent(node, edge);
                // metrics.go rejects nil current TopLeft before consuming an
                // abduction or reading its original endpoint.
                let position = self.position(candidate)?;
                let projected = self
                    .sized_edge_abductions
                    .iter()
                    .enumerate()
                    .find_map(|(index, abduction)| {
                        if used_abductions[index] {
                            return None;
                        }
                        let projected = if abduction.current_from.and_then(|endpoint| endpoint.local_node()) == Some(node)
                            && abduction.current_to.and_then(|endpoint| endpoint.local_node()) == Some(candidate)
                        {
                            abduction.originally_to
                        } else if abduction.current_from.and_then(|endpoint| endpoint.local_node()) == Some(candidate)
                            && abduction.current_to.and_then(|endpoint| endpoint.local_node()) == Some(node)
                        {
                            abduction.originally_from
                        } else {
                            None
                        }?;
                        used_abductions[index] = true;
                        Some(projected)
                    })
                    .or_else(|| {
                        self.sized_edge_abductions
                            .is_empty()
                            .then(|| self.sized_adjacent_overrides.get(&(node, edge)).copied())
                            .flatten().map(AbductedNodeRef::from)
                    });
                if let Some(original) = projected {
                    let projected = self.abducted_node_projection(original)?;
                    // optimizerAdjacents substitutes the original endpoint
                    // pointer and median() reads that node's current TopLeft
                    // directly. Preserve the endpoint's recorded graph
                    // position here; projected_absolute_position is for
                    // geometry in the active container frame and can
                    // incorrectly rebase a retained endpoint through a
                    // different temporary container.
                    let position = self
                        .projected_external_position(projected.tala_id)
                        .or_else(|| self.projected_absolute_position(projected))?;
                    let center = Point {
                        x: position.x + projected.size.width * 0.5,
                        y: position.y + projected.size.height * 0.5,
                    };
                    if trace_median_adjacents {
                        let external_position =
                            self.projected_external_position(projected.tala_id);
                    let live_position =
                        self.projected_live_external_position(projected.tala_id);
                    let absolute_position = self.projected_absolute_position(projected);
                    let source_position = self
                        .projected_source_position(projected.tala_id, &mut BTreeSet::new());
                    eprintln!(
                            "MEDIAN_PROJECTED_RUST node={} child={} owner={} container={:?} owner_pos={:?} offset={},{} size={},{} external={:?} live={:?} absolute={:?} source={:?} cluster_member={} center={},{}",
                            self.nodes[node.0 as usize].tala_id,
                            projected.tala_id,
                            self.nodes[projected.owner.0 as usize].tala_id,
                            projected.container_tala_id,
                            self.position(projected.owner),
                            projected.offset.x,
                            projected.offset.y,
                            projected.size.width,
                            projected.size.height,
                            external_position,
                            live_position,
                            absolute_position,
                            source_position,
                            projected.cluster_member,
                            center.x,
                            center.y,
                        );
                    }
                    return Some((center.x, center.y, projected.tala_id, position, projected.size));
                }
                let size = self.active_node_size(candidate);
                if trace_median_adjacents {
                    eprintln!(
                        "MEDIAN_RAW_RUST node={} candidate_index={} raw_id={} active_id={} raw_pos={:?} active_pos={:?}",
                        self.nodes[node.0 as usize].tala_id,
                        candidate.0,
                        self.nodes[candidate.0 as usize].tala_id,
                        self.active_node_tala_id(candidate),
                        self.nodes[candidate.0 as usize].position,
                        self.position(candidate),
                    );
                }
                Some((
                    position.x + size.width * 0.5,
                    position.y + size.height * 0.5,
                    self.active_node_tala_id(candidate),
                    position,
                    size,
                ))
            })
            .collect();
        if adjacent.is_empty() {
            adjacent.extend(self.median_fallback_nears(node).into_iter().map(|near| {
                let position = self.position(near).expect("positioned median Near");
                let size = self.nodes[near.0 as usize].rect.size;
                (
                    position.x + size.width * 0.5,
                    position.y + size.height * 0.5,
                    self.nodes[near.0 as usize].tala_id,
                    position,
                    size,
                )
            }));
        }
        if adjacent.is_empty() {
            let position = self.position(node).unwrap_or_default();
            let size = self.nodes[node.0 as usize].rect.size;
            return Point {
                x: (position.x + size.width * 0.5) / self.cell_size,
                y: (position.y + size.height * 0.5) / self.cell_size,
            };
        }
        if trace_median_adjacents {
            eprint!(
                "MEDIAN_ADJ_RUST node={} ",
                self.nodes[node.0 as usize].tala_id
            );
            for (x, y, id, _, _) in &adjacent {
                eprint!("{}:{},{} ", id, x, y);
            }
            eprintln!();
        }
        // metrics.go divides by the original first neighbor's Graph.CellSize,
        // independently of either axis sort and of the moving node's graph.
        let cell_size = if let Some(first) = self
            .nodes
            .iter()
            .find(|neighbor| neighbor.tala_id == adjacent[0].2)
        {
            let graph = self
                .topology_node_owner(first.input_id)
                .expect("runtime error: invalid memory address or nil pointer dereference");
            self.owner_graph_cell_size(graph)
                .expect("median neighbor Graph")
        } else {
            // This historical numerical adapter can carry an external
            // projection without an owning-arena node/Graph handle. Its
            // scope-cell approximation remains unqualified; canonical
            // owner_median reads the retained original pointer directly.
            self.cell_size
        };
        let median_axis = |horizontal: bool| {
            let mut ordered = adjacent.clone();
            ordered.sort_by(|a, b| {
                let a_value = if horizontal { a.0 } else { a.1 };
                let b_value = if horizontal { b.0 } else { b.1 };
                a_value.total_cmp(&b_value).then_with(|| a.2.cmp(&b.2))
            });
            let center = |value: &(f64, f64, i64, Point, Size)| {
                if horizontal { value.0 } else { value.1 }
            };
            let middle = ordered.len() / 2;
            let mut value = center(&ordered[middle]);
            if ordered.len() % 2 == 0 {
                // metrics.go adds lower.TopLeft and lower.Size/2 to the
                // upper center separately. Precomputing the lower center
                // loses that rounding order when large coordinates cancel.
                let lower = &ordered[middle - 1];
                let position = if horizontal { lower.3.x } else { lower.3.y };
                let size = if horizontal {
                    lower.4.width
                } else {
                    lower.4.height
                };
                value = (value + position + size / 2.0) / 2.0;
            }
            value / cell_size
        };
        Point {
            x: median_axis(true),
            y: median_axis(false),
        }
    }

    pub(super) fn sized_protruding_children(&self, node: NodeId) -> Vec<ProjectedAdjacent> {
        if !self.sized_edge_abductions.is_empty() {
            let mut children = Vec::new();
            for abduction in &self.sized_edge_abductions {
                if abduction
                    .current_from
                    .and_then(|endpoint| endpoint.local_node())
                    == Some(node)
                    && let Some(original) = abduction.originally_from
                {
                    if let Some(projected) = self.abducted_node_projection(original) {
                        children.push(projected);
                    }
                }
                if abduction
                    .current_to
                    .and_then(|endpoint| endpoint.local_node())
                    == Some(node)
                    && let Some(original) = abduction.originally_to
                {
                    if let Some(projected) = self.abducted_node_projection(original) {
                        children.push(projected);
                    }
                }
            }
            return children;
        }
        self.sized_adjacent_overrides
            .values()
            .filter(|projected| projected.owner == node)
            .copied()
            .collect()
    }

    pub(super) fn sized_projected_children_median(
        &self,
        children: &[ProjectedAdjacent],
    ) -> Option<Point> {
        if children.is_empty() {
            return None;
        }
        let mut centers = children
            .iter()
            .filter_map(|child| {
                // Go's protruding child list contains original endpoint
                // pointers, and its median reads each pointer's live TopLeft
                // directly. Reprojecting a retained child through its
                // temporary container can shift it into a different frame.
                let position = self
                    .projected_external_position(child.tala_id)
                    .or_else(|| self.projected_absolute_position(*child))?;
                Some((
                    Point {
                        x: position.x + child.size.width * 0.5,
                        y: position.y + child.size.height * 0.5,
                    },
                    child.tala_id,
                ))
            })
            .collect::<Vec<_>>();
        if centers.is_empty() {
            return None;
        }
        let median_axis = |horizontal: bool, centers: &mut Vec<(Point, i64)>| {
            centers.sort_by(|left, right| {
                let left_axis = if horizontal { left.0.x } else { left.0.y };
                let right_axis = if horizontal { right.0.x } else { right.0.y };
                left_axis
                    .total_cmp(&right_axis)
                    .then_with(|| left.1.cmp(&right.1))
            });
            let middle = centers.len() / 2;
            let center = |point: (Point, i64)| {
                if horizontal { point.0.x } else { point.0.y }
            };
            let mut median = center(centers[middle]);
            if centers.len().is_multiple_of(2) {
                median = (median + center(centers[middle - 1])) * 0.5;
            }
            median / self.cell_size
        };
        let mut centers_by_y = centers.clone();
        Some(Point {
            x: median_axis(true, &mut centers),
            y: median_axis(false, &mut centers_by_y),
        })
    }

    pub(super) fn sized_distance(&self, a: NodeId, b: NodeId) -> f64 {
        let a_position = self.position(a).unwrap();
        let b_position = self.position(b).unwrap();
        let a_size = self.active_node_size(a);
        let b_size = self.active_node_size(b);
        let a_right = a_position.x + a_size.width;
        let a_bottom = a_position.y + a_size.height;
        let b_right = b_position.x + b_size.width;
        let b_bottom = b_position.y + b_size.height;
        let dx = if b_right < a_position.x {
            a_position.x - b_right
        } else if a_right < b_position.x {
            b_position.x - a_right
        } else {
            0.0
        };
        let dy = if b_bottom < a_position.y {
            a_position.y - b_bottom
        } else if a_bottom < b_position.y {
            b_position.y - a_bottom
        } else {
            0.0
        };
        let border_distance = if dx != 0.0 && dy != 0.0 {
            // TALA's geo.EuclideanDistance is `math.Sqrt(dx*dx+dy*dy)`,
            // rather than the platform hypot implementation. The two can
            // differ by one ULP at strict race ties.
            (dx * dx + dy * dy).sqrt()
        } else {
            dx + dy
        };
        let center_x = ((a_position.x + a_size.width / 2.0) - (b_position.x + b_size.width / 2.0))
            .abs()
            / (a_size.width + b_size.width);
        let center_y =
            ((a_position.y + a_size.height / 2.0) - (b_position.y + b_size.height / 2.0)).abs()
                / (a_size.height + b_size.height);
        // Keep the source expression as a division.  Go's placementDistance
        // computes this term with `/ 20`; multiplying by the decimal literal
        // `0.05` can round one or two ULPs differently on a candidate score.
        border_distance + center_x.min(center_y) / 20.0
    }

    // Recovered Node.distanceTo excludes distanceBetweenCenters. Near scoring
    // uses this border-only form rather than the ordinary edge distance.
    pub(super) fn sized_distance_to(&self, a: NodeId, b: NodeId) -> f64 {
        // Node.DistanceTo reads the live Graph node boxes. Stable arena
        // owners retain member dimensions while an active sequence/cluster
        // vessel carries the aggregate box, so use the active projection for
        // both positions and sizes.
        let a_position = self.active_node_position(a).unwrap();
        let b_position = self.active_node_position(b).unwrap();
        let a_size = self.active_node_size(a);
        let b_size = self.active_node_size(b);
        let dx = if b_position.x + b_size.width < a_position.x {
            a_position.x - (b_position.x + b_size.width)
        } else if a_position.x + a_size.width < b_position.x {
            b_position.x - (a_position.x + a_size.width)
        } else {
            0.0
        };
        let dy = if b_position.y + b_size.height < a_position.y {
            a_position.y - (b_position.y + b_size.height)
        } else if a_position.y + a_size.height < b_position.y {
            b_position.y - (a_position.y + a_size.height)
        } else {
            0.0
        };
        if dx != 0.0 && dy != 0.0 {
            (dx * dx + dy * dy).sqrt()
        } else {
            dx + dy
        }
    }

    pub(super) fn positioned_max_pair(&self) -> (i64, i64) {
        let mut max_length = 60.0;
        let mut pair = (0, 0);
        for node in self.node_order.iter().copied() {
            if self.active_node_position(node).is_none() {
                continue;
            }
            // Match Graph.maxEdgeLength's node-local incidence walk. These
            // pointers can include edges absent from Graph.Edges, so the
            // ordinary active-edge filter is intentionally not used here.
            for edge in self.max_edge_length_incident_edges(node) {
                let adjacent = self.active_adjacent(node, edge);
                let (Some(node_position), Some(adjacent_position)) = (
                    self.active_node_position(node),
                    self.active_node_position(adjacent),
                ) else {
                    continue;
                };
                let node_size = self.active_node_size(node);
                let adjacent_size = self.active_node_size(adjacent);
                let dx = if adjacent_position.x + adjacent_size.width < node_position.x {
                    node_position.x - (adjacent_position.x + adjacent_size.width)
                } else if node_position.x + node_size.width < adjacent_position.x {
                    adjacent_position.x - (node_position.x + node_size.width)
                } else {
                    0.0
                };
                let dy = if adjacent_position.y + adjacent_size.height < node_position.y {
                    node_position.y - (adjacent_position.y + adjacent_size.height)
                } else if node_position.y + node_size.height < adjacent_position.y {
                    adjacent_position.y - (node_position.y + node_size.height)
                } else {
                    0.0
                };
                let distance = if dx != 0.0 && dy != 0.0 {
                    (dx * dx + dy * dy).sqrt()
                } else {
                    dx + dy
                };
                if distance > max_length {
                    max_length = distance;
                    pair = (
                        self.nodes[node.0 as usize].tala_id,
                        self.nodes[adjacent.0 as usize].tala_id,
                    );
                }
            }
        }
        pair
    }

    pub(super) fn positioned_max_length(&self) -> f64 {
        let mut has_length = false;
        let mut max_length: f64 = 60.0;
        // Graph.maxEdgeLength walks the current Graph.Nodes slice. `node_order`
        // retains arena members that Go has removed from temporary aggregate
        // graphs, so use the active graph view here just as the scorer does.
        for node in self.graph_node_order_cow().iter().copied() {
            if self.active_node_position(node).is_none() {
                continue;
            }
            // Graph.maxEdgeLength follows Node.Edges even when the edge is
            // absent from the current Graph.Edges slice.
            for edge in self.max_edge_length_incident_edges(node) {
                let adjacent = self.active_adjacent(node, edge);
                let (Some(node_position), Some(adjacent_position)) = (
                    self.active_node_position(node),
                    self.active_node_position(adjacent),
                ) else {
                    continue;
                };
                has_length = true;
                let node_size = self.active_node_size(node);
                let adjacent_size = self.active_node_size(adjacent);
                let dx = if adjacent_position.x + adjacent_size.width < node_position.x {
                    node_position.x - (adjacent_position.x + adjacent_size.width)
                } else if node_position.x + node_size.width < adjacent_position.x {
                    adjacent_position.x - (node_position.x + node_size.width)
                } else {
                    0.0
                };
                let dy = if adjacent_position.y + adjacent_size.height < node_position.y {
                    node_position.y - (adjacent_position.y + adjacent_size.height)
                } else if node_position.y + node_size.height < adjacent_position.y {
                    adjacent_position.y - (node_position.y + node_size.height)
                } else {
                    0.0
                };
                let distance = if dx != 0.0 && dy != 0.0 {
                    (dx * dx + dy * dy).sqrt()
                } else {
                    dx + dy
                };
                if distance > max_length {
                    max_length = distance;
                }
            }
        }
        if has_length { max_length } else { 0.0 }
    }

    pub(super) fn non_center_port_cost_for_max_length(&self, max_length: f64) -> f64 {
        if self.edge_order.is_empty() {
            return 0.0;
        }
        let mut cost = (0.04287499999999999_f64 * self.edge_order.len() as f64) * max_length;
        let mut min_size = f64::INFINITY;
        for node in self.graph_node_order_cow().iter().copied() {
            if self.nodes[node.0 as usize].is_container {
                continue;
            }
            let size = self.active_node_size(node);
            min_size = routing::channel_solver::minimum(min_size, size.height);
            min_size = routing::channel_solver::minimum(min_size, size.width);
        }
        if min_size != f64::INFINITY {
            cost = routing::channel_solver::maximum(cost, min_size / 3.0);
        }
        cost
    }

    /// placeNodesOrthogonally materializes and halves only Graph.TurnCost.
    pub(super) fn initialize_turn_cost(&mut self) {
        self.turn_cost_value();
        self.cost_cache.halve_turn();
        if crate::internal::trace_env_enabled("TALA_TRACE_SCORING_INIT") {
            eprintln!(
                "SCORING_INIT_RUST turn={:.17} crossing={:.17} noncenter={:.17}",
                self.raw_turn_cost(),
                self.raw_crossing_cost(),
                self.raw_non_center_port_cost()
            );
        }
    }

    pub(super) fn initialize_combined_scoring_costs(&mut self) {
        self.refresh_node_order_membership();
        self.turn_cost_value();
    }

    /// Place rejoins scopes and resets only the main Graph's turn cache.
    pub(super) fn initialize_rejoined_scoring_costs(&mut self) {
        self.refresh_node_order_membership();
        self.reset_turn_cost();
    }

    pub(super) fn reset_alignment_container_cost(&mut self) {
        // AlignAxes does not clear the Graph's NonCenterPortCost cache.
        if crate::internal::trace_env_enabled("TALA_TRACE_CONTAINER_ALIGNMENT") {
            eprintln!(
                "ALIGN_CONTAINER_RESET_RUST root_hierarchy={} nodes={} graph_nodes={} cached={}",
                self.root_hierarchy,
                self.nodes.len(),
                self.graph_node_order().len(),
                self.raw_non_center_port_cost()
            );
        }
    }

    pub(super) fn raw_routing_costs(&self) -> (f64, f64, f64) {
        self.cost_cache.raw()
    }

    pub(super) fn restore_routing_costs(&self, values: (f64, f64, f64)) {
        self.cost_cache.restore_routing(values);
    }

    /// Source layoutgraph.SnapshotPlacementCosts; the enclosing placement
    /// operation owns work charging and decides explicitly when to restore.
    pub(super) fn snapshot_placement_costs(&self) -> PlacementCostSnapshot {
        self.cost_cache.snapshot_placement()
    }

    pub(super) fn raw_turn_cost(&self) -> f64 {
        self.raw_routing_costs().1
    }
    pub(super) fn raw_crossing_cost(&self) -> f64 {
        self.raw_routing_costs().0
    }
    pub(super) fn raw_non_center_port_cost(&self) -> f64 {
        self.raw_routing_costs().2
    }

    pub(super) fn turn_cost_value(&self) -> f64 {
        self.cost_cache.get_or_compute(RoutingCostField::Turn, || {
            (0.125 * self.edge_order.len() as f64) * self.positioned_max_length()
        })
    }

    #[cfg(feature = "diagnostic-traces")]
    fn trace_crossing_fill(&self, max_length: f64, cost: f64, caller: &std::panic::Location<'_>) {
        if !crate::internal::trace_env_enabled("D2RS_TRACE_CROSSING_FILL") {
            return;
        }
        if crate::internal::trace_env_enabled("D2RS_TRACE_CROSSING_CALLER") {
            eprintln!(
                "{}",
                serde_json::json!({"event":"crossing-caller", "caller":caller.to_string(),
                "cache":format!("{:p}", std::sync::Arc::as_ptr(&self.cost_cache)),
                "stack":std::backtrace::Backtrace::force_capture().to_string()})
            );
        }

        let geometry = |node| {
            let size = self.active_node_size(node);
            serde_json::json!({"id":self.active_node_tala_id(node).to_string(),
                "position":self.active_node_position(node), "width":size.width,"height":size.height})
        };
        let nodes = self
            .graph_node_order_cow()
            .iter()
            .copied()
            .map(|node| {
                let mut value = geometry(node);
                value["adjacent"] = serde_json::json!(
                    self.max_edge_length_incident_edges(node)
                        .into_iter()
                        .map(|edge| {
                            let other = self.active_adjacent(node, edge);
                            let mut row = geometry(other);
                            row["edge"] = serde_json::json!(
                                self.edge_records[edge.0 as usize].tala_id.to_string()
                            );
                            row["distance"] = serde_json::json!(
                                self.active_node_position(node)
                                    .is_some()
                                    .then(|| self.active_node_position(other))
                                    .flatten()
                                    .map(|_| self.sized_distance_to(node, other))
                            );
                            row
                        })
                        .collect::<Vec<_>>()
                );
                value
            })
            .collect::<Vec<_>>();
        eprintln!(
            "{}",
            serde_json::json!({"event":"crossing-fill","cost":cost,"maxLength":max_length,
            "nodes":nodes,"edges":self.edge_order.read().iter().map(|e|self.edge_records[e.0 as usize].tala_id.to_string()).collect::<Vec<_>>()})
        );
    }

    #[cfg_attr(feature = "diagnostic-traces", track_caller)]
    pub(super) fn crossing_cost_value(&self) -> f64 {
        #[cfg(feature = "diagnostic-traces")]
        let caller = std::panic::Location::caller();
        self.cost_cache
            .get_or_compute(RoutingCostField::Crossing, || {
                let max_length = self.positioned_max_length();
                let cost = (0.110592_f64 * self.edge_order.len() as f64) * max_length;
                #[cfg(feature = "diagnostic-traces")]
                self.trace_crossing_fill(max_length, cost, caller);
                cost
            })
    }

    pub(super) fn non_center_port_cost_value(&self) -> f64 {
        self.cost_cache
            .get_or_compute_optional(RoutingCostField::NonCenterPort, || {
                if self.edge_order.is_empty() {
                    None
                } else {
                    Some(self.non_center_port_cost_for_max_length(self.positioned_max_length()))
                }
            })
    }

    pub(super) fn set_turn_cost(&self, value: f64) {
        self.cost_cache.set(RoutingCostField::Turn, value);
    }
    pub(super) fn set_crossing_cost(&self, value: f64) {
        self.cost_cache.set(RoutingCostField::Crossing, value);
    }
    pub(super) fn set_non_center_port_cost(&self, value: f64) {
        self.cost_cache.set(RoutingCostField::NonCenterPort, value);
    }
    pub(super) fn reset_turn_cost(&self) {
        self.set_turn_cost(0.0);
    }

    /// layoutgraph.Clone copies the raw triple into a new Graph with an empty map.
    pub(super) fn detach_cost_owner(&mut self) {
        self.cost_cache = Arc::new(GraphCostCache::new(self.raw_routing_costs()));
        self.detach_owner_graph_score_states();
    }

    pub(super) fn fresh_cost_owner(&mut self) {
        self.cost_cache = Arc::new(GraphCostCache::new((0.0, 0.0, 0.0)));
    }

    pub(super) fn sized_orientation(&self, node: NodeId, other: NodeId) -> Orientation {
        // Node.getOrientation reads the current graph boxes. When a sequence
        // or cluster vessel is active, that box is the vessel geometry rather
        // than the first member's raw arena rectangle.
        self.sized_box_orientation(
            (
                self.active_node_position(node).unwrap(),
                self.active_node_size(node),
            ),
            (
                self.active_node_position(other).unwrap(),
                self.active_node_size(other),
            ),
        )
    }

    pub(super) fn sized_box_orientation(
        &self,
        (node_position, node_size): (Point, Size),
        (other_position, other_size): (Point, Size),
    ) -> Orientation {
        if node_position.y + node_size.height < other_position.y {
            if node_position.x + node_size.width < other_position.x {
                return Orientation::TopLeft;
            }
            if other_position.x + other_size.width < node_position.x {
                return Orientation::TopRight;
            }
            return Orientation::Top;
        }
        if other_position.y + other_size.height < node_position.y {
            if node_position.x + node_size.width < other_position.x {
                return Orientation::BottomLeft;
            }
            if other_position.x + other_size.width < node_position.x {
                return Orientation::BottomRight;
            }
            return Orientation::Bottom;
        }
        if other_position.x + other_size.width < node_position.x {
            return Orientation::Right;
        }
        if node_position.x + node_size.width < other_position.x {
            return Orientation::Left;
        }
        Orientation::None
    }

    pub(super) fn sized_box_distance_to_point(
        &self,
        (position, size): (Point, Size),
        point: Point,
    ) -> f64 {
        let right = position.x + size.width;
        let bottom = position.y + size.height;
        let dx = if point.x < position.x {
            position.x - point.x
        } else if right < point.x {
            point.x - right
        } else {
            0.0
        };
        let dy = if point.y < position.y {
            position.y - point.y
        } else if bottom < point.y {
            point.y - bottom
        } else {
            0.0
        };
        if dx != 0.0 && dy != 0.0 {
            (dx * dx + dy * dy).sqrt()
        } else {
            dx + dy
        }
    }

    fn scoring_turn_cost_with_owner_frame_checked(
        &self,
        node: NodeId,
        frame: Option<&OwnerScoreFrame<'_>>,
    ) -> Result<f64, limits::Error> {
        match frame {
            Some(frame) => frame.receiver_turn_cost(node).map_err(owner_score_error),
            None => Ok(self.turn_cost_value()),
        }
    }

    fn sized_edge_base_distance_between_with_owner_frame_checked(
        &self,
        node_box: (Point, Size),
        adjacent_box: (Point, Size),
        scoring_node: NodeId,
        frame: Option<&OwnerScoreFrame<'_>>,
    ) -> Result<f64, limits::Error> {
        let orientation = self.sized_box_orientation(node_box, adjacent_box);
        if !matches!(
            orientation,
            Orientation::TopLeft
                | Orientation::TopRight
                | Orientation::BottomLeft
                | Orientation::BottomRight
        ) {
            return Ok(Self::sized_box_distance(node_box, adjacent_box));
        }
        let (position, size) = node_box;
        let (adjacent_position, adjacent_size) = adjacent_box;
        let corner = Point {
            x: position.x + size.width * 0.5,
            y: adjacent_position.y + adjacent_size.height * 0.5,
        };
        Ok(self.sized_box_distance_to_point(node_box, corner)
            + self.sized_box_distance_to_point(adjacent_box, corner)
            + self.scoring_turn_cost_with_owner_frame_checked(scoring_node, frame)?)
    }

    pub(super) fn sized_box_distance(
        (a_position, a_size): (Point, Size),
        (b_position, b_size): (Point, Size),
    ) -> f64 {
        let a_right = a_position.x + a_size.width;
        let a_bottom = a_position.y + a_size.height;
        let b_right = b_position.x + b_size.width;
        let b_bottom = b_position.y + b_size.height;
        let dx = if b_right < a_position.x {
            a_position.x - b_right
        } else if a_right < b_position.x {
            b_position.x - a_right
        } else {
            0.0
        };
        let dy = if b_bottom < a_position.y {
            a_position.y - b_bottom
        } else if a_bottom < b_position.y {
            b_position.y - a_bottom
        } else {
            0.0
        };
        let border_distance = if dx != 0.0 && dy != 0.0 {
            (dx * dx + dy * dy).sqrt()
        } else {
            dx + dy
        };
        let center_x = ((a_position.x + a_size.width / 2.0) - (b_position.x + b_size.width / 2.0))
            .abs()
            / (a_size.width + b_size.width);
        let center_y =
            ((a_position.y + a_size.height / 2.0) - (b_position.y + b_size.height / 2.0)).abs()
                / (a_size.height + b_size.height);
        // Match geo placementDistance's `/20` operation order exactly.
        border_distance + center_x.min(center_y) / 20.0
    }

    /// Recovered Node.distanceTo(box, true).  The symmetry neighbor cleanup
    /// uses only the border gap; the separate center-distance term belongs to
    /// edge scoring and must not make a diagonal neighbor disappear at the
    /// 1200-cell cutoff.
    pub(super) fn sized_box_border_distance(
        (a_position, a_size): (Point, Size),
        (b_position, b_size): (Point, Size),
    ) -> f64 {
        let a_right = a_position.x + a_size.width;
        let a_bottom = a_position.y + a_size.height;
        let b_right = b_position.x + b_size.width;
        let b_bottom = b_position.y + b_size.height;
        let dx = if b_right < a_position.x {
            a_position.x - b_right
        } else if a_right < b_position.x {
            b_position.x - a_right
        } else {
            0.0
        };
        let dy = if b_bottom < a_position.y {
            a_position.y - b_bottom
        } else if a_bottom < b_position.y {
            b_position.y - a_bottom
        } else {
            0.0
        };
        if dx != 0.0 && dy != 0.0 {
            (dx * dx + dy * dy).sqrt()
        } else {
            dx + dy
        }
    }

    pub(super) fn sized_projected_box(
        &self,
        projected: ProjectedAdjacent,
    ) -> Option<(Point, Size)> {
        let trace_projected = crate::internal::trace_env_value("TALA_TRACE_PROJECTED_BOX_IDS")
            .map(|ids| {
                ids == "all"
                    || ids
                        .split(',')
                        .filter_map(|id| id.parse::<i64>().ok())
                        .any(|id| id == projected.tala_id)
            })
            .unwrap_or(false);
        let trace_projected = trace_projected
            && crate::internal::trace_env_value("TALA_TRACE_PROJECTED_BOX_POSITION").is_none_or(
                |target| {
                    let mut parts = target.split(',');
                    let Some(x) = parts.next().and_then(|value| value.parse::<f64>().ok()) else {
                        return false;
                    };
                    let Some(y) = parts.next().and_then(|value| value.parse::<f64>().ok()) else {
                        return false;
                    };
                    self.position(projected.owner)
                        .is_some_and(|position| position.x == x && position.y == y)
                },
            );
        if trace_projected {
            eprintln!(
                "PROJECTED_BOX_LOOKUP_RUST id={} owner={} ownerPos={:?} offset={:?} cluster_member={} clusters={:?} aggregateMaps={:?} directMaps={:?}",
                projected.tala_id,
                self.nodes[projected.owner.0 as usize].tala_id,
                self.position(projected.owner),
                projected.offset,
                projected.cluster_member,
                self.clusters
                    .iter()
                    .filter_map(|cluster| {
                        cluster
                            .members
                            .iter()
                            .any(|member| {
                                self.nodes[member.0 as usize].tala_id == projected.tala_id
                            })
                            .then_some((
                                cluster.vessel_tala_id,
                                self.cluster_is_active(cluster),
                                cluster.members.len(),
                            ))
                    })
                    .collect::<Vec<_>>(),
                self.transaction_external_aggregate_children
                    .iter()
                    .filter_map(|(vessel, members)| {
                        members
                            .iter()
                            .find(|member| member.tala_id == projected.tala_id)
                            .map(|member| (*vessel, member.position))
                    })
                    .collect::<Vec<_>>(),
                self.transaction_external_container_children
                    .iter()
                    .filter_map(|(parent, children)| {
                        children
                            .iter()
                            .find(|child| child.tala_id == projected.tala_id)
                            .map(|child| (*parent, child.position))
                    })
                    .collect::<Vec<_>>()
            );
        }
        // Cluster edge abductions retain the member identity but their
        // original offset is only a snapshot from before the vessel's latest
        // move/flip. OSS TALA keeps the member pointer live, so reconstruct
        // its current arranged box from the active vessel on every trial.
        if projected.cluster_member
            && let Some((cluster_index, member)) = self
                .clusters
                .iter()
                .enumerate()
                .filter(|(_, cluster)| self.cluster_is_active(cluster))
                .find_map(|(cluster_index, cluster)| {
                    cluster
                        .members
                        .iter()
                        .copied()
                        .find(|member| self.nodes[member.0 as usize].tala_id == projected.tala_id)
                        .map(|member| (cluster_index, member))
                })
            && let Some(position) = self
                .transaction_external_aggregate_children
                .get(&self.clusters[cluster_index].vessel_tala_id)
                .and_then(|members| {
                    members
                        .iter()
                        .find(|candidate| candidate.tala_id == projected.tala_id)
                        .and_then(|candidate| candidate.position)
                })
                .or_else(|| self.position(member))
                // `projected_external_position` resolves an active carrier
                // through the vessel view. For the first stable cluster
                // member that is not the member pointer's live position:
                // NodePlacement can leave it crossed until the next
                // SyncClusters call, and OSS TALA's edge abduction scores
                // that live member pointer. Prefer the stable member box
                // before falling back to the carrier projection.
                .or_else(|| self.projected_external_position(projected.tala_id))
        {
            // The induced Go graph keeps the original member pointer as the
            // replacement endpoint. Its aggregate-child snapshot is the live
            // pointer position; the stable arena can expose that member
            // rebased through a different nested-container frame. If the
            // snapshot is unavailable, use the live external/active position
            // rather than recomputing an ideal rounded position from vessel.
            let size = self.nodes[member.0 as usize].rect.size;
            if trace_projected {
                eprintln!(
                    "PROJECTED_BOX_RUST id={} owner={} source=live-cluster member={} cluster={} memberPos={:?} size={:?}",
                    projected.tala_id,
                    self.nodes[projected.owner.0 as usize].tala_id,
                    self.nodes[member.0 as usize].tala_id,
                    cluster_index,
                    position,
                    size,
                );
            }
            return Some((position, size));
        }
        // An induced aggregate can outlive the ClusterState that originally
        // described its members. In that case the retained Go child pointer
        // is still available in the transaction aggregate snapshot, while
        // `projected_absolute_position` would rebase the member through a
        // different nested-container frame. Resolve that pointer by identity
        // before falling back to the source-chain reconstruction.
        if projected.cluster_member
            && let Some((position, size)) = self
                .transaction_external_aggregate_children
                .values()
                .flatten()
                .find(|member| member.tala_id == projected.tala_id)
                .and_then(|member| member.position.map(|position| (position, member.rect.size)))
        {
            if trace_projected {
                eprintln!(
                    "PROJECTED_BOX_RUST id={} owner={} source=aggregate-pointer pos={:?} size={:?}",
                    projected.tala_id,
                    self.nodes[projected.owner.0 as usize].tala_id,
                    position,
                    size,
                );
            }
            return Some((position, size));
        }
        // Ordinary retained descendants follow their source container chain
        // through the active temporary graph.  Resolve this before consulting
        // transaction child maps: those maps intentionally contain a mixture
        // of local and absolute frames, while the source chain has one
        // unambiguous parent-relative interpretation.
        if projected.container_tala_id.is_some()
            && let Some(position) = self.projected_absolute_position(projected)
        {
            if trace_projected {
                eprintln!(
                    "PROJECTED_BOX_RUST id={} owner={} source=source-chain pos={:?} size={:?}",
                    projected.tala_id,
                    self.nodes[projected.owner.0 as usize].tala_id,
                    position,
                    projected.size,
                );
            }
            return Some((position, projected.size));
        }
        // Go's induced graph retains the original child pointer.  A trial
        // move therefore changes the child box immediately, and every score
        // in that same trial observes the new absolute position.  Rust keeps
        // the induced child as an external snapshot, so the cached owner
        // offset is not sufficient while an optimizer candidate is being
        // evaluated.  Prefer the live snapshot position when this projected
        // identity is present; the owner/offset form remains the fallback for
        // vessel geometry that has no retained child snapshot.
        let direct_container_children = projected.container_tala_id.and_then(|container_tala_id| {
            self.transaction_external_container_children
                .get(&container_tala_id)
        });
        if let Some(container_tala_id) = projected.container_tala_id
            && let Some(owner_position) = self.position(projected.owner)
            && let Some(parent) = self
                .transaction_external_container_children
                .values()
                .flatten()
                .find(|child| child.tala_id == container_tala_id)
            && let Some(parent_position) = parent.position
            && let Some(children) = direct_container_children
            && let Some(child) = children
                .iter()
                .find(|child| child.tala_id == projected.tala_id)
            && let Some(child_position) = child.position
            && container_tala_id != self.nodes[projected.owner.0 as usize].tala_id
            // Descendant edges retain the same edge pointer in the copied
            // parent and child nodes.  Those shared pointers are the cases
            // where Go's induced graph moves the nested child with its
            // immediate container frame; ordinary nested children already
            // carry an owner-relative snapshot.
            && child.edges.read().iter().any(|edge| parent.edges.contains(edge))
            && (child_position.x - owner_position.x - projected.offset.x).abs() < 1e-9
            && (child_position.y - owner_position.y - projected.offset.y).abs() < 1e-9
        {
            // `projected.offset` is reconciled in the temporary owner's
            // frame. When the retained parent is itself nested below that
            // owner, remove the parent-owner displacement before applying
            // the offset to the live parent. Go's shared child pointer makes
            // this conversion implicitly; adding the unreduced offset here
            // counts the ancestor frame twice. The environment escape is
            // diagnostic-only while seed witnesses are being isolated.
            if crate::internal::trace_env_enabled("TALA_TRACE_NESTED_DECISION") {
                let old_position = Point {
                    x: parent_position.x + projected.offset.x,
                    y: parent_position.y + projected.offset.y,
                };
                let fixed_position = Point {
                    x: parent_position.x + projected.offset.x
                        - (parent_position.x - owner_position.x),
                    y: parent_position.y + projected.offset.y
                        - (parent_position.y - owner_position.y),
                };
                eprintln!(
                    "NESTED_DECISION_RUST id={} owner={} container={} child={:?} ownerPos={:?} parentPos={:?} offset={:?} old={:?} fixed={:?}",
                    projected.tala_id,
                    self.nodes[projected.owner.0 as usize].tala_id,
                    container_tala_id,
                    child_position,
                    owner_position,
                    parent_position,
                    projected.offset,
                    old_position,
                    fixed_position,
                );
            }
            let local_offset = Point {
                x: projected.offset.x - (parent_position.x - owner_position.x),
                y: projected.offset.y - (parent_position.y - owner_position.y),
            };
            let position = Point {
                x: parent_position.x + local_offset.x,
                y: parent_position.y + local_offset.y,
            };
            if (position.x - child_position.x).abs() < 1e-9
                && (position.y - child_position.y).abs() < 1e-9
            {
                // Already synchronized through the immediate parent.
            } else {
                if crate::internal::trace_env_enabled("TALA_TRACE_NESTED_CORRECTIONS") {
                    eprintln!(
                        "NESTED_CORRECTION_RUST id={} owner={} container={} child={:?} ownerPos={:?} parentPos={:?} offset={:?} result={:?}",
                        projected.tala_id,
                        self.nodes[projected.owner.0 as usize].tala_id,
                        container_tala_id,
                        child_position,
                        owner_position,
                        parent_position,
                        projected.offset,
                        position,
                    );
                }
                if trace_projected {
                    eprintln!(
                        "PROJECTED_BOX_RUST id={} owner={} source=nested-parent parent={} parentPos={:?} offset={:?} resultPos={:?} size={:?}",
                        projected.tala_id,
                        self.nodes[projected.owner.0 as usize].tala_id,
                        container_tala_id,
                        parent_position,
                        projected.offset,
                        position,
                        projected.size,
                    );
                }
                return Some((position, projected.size));
            }
        }
        if let Some(children) = direct_container_children
            && let Some(child) = children
                .iter()
                .find(|child| child.tala_id == projected.tala_id)
            && let Some(position) = child.position
        {
            if trace_projected {
                eprintln!(
                    "PROJECTED_BOX_RUST id={} owner={} source=external-container childPos={:?} ownerPos={:?} offset={:?} size={:?}",
                    projected.tala_id,
                    self.nodes[projected.owner.0 as usize].tala_id,
                    position,
                    self.position(projected.owner),
                    projected.offset,
                    projected.size,
                );
            }
            return Some((position, projected.size));
        }
        if direct_container_children.is_none() {
            for children in self.transaction_external_container_children.values() {
                if let Some(child) = children
                    .iter()
                    .find(|child| child.tala_id == projected.tala_id)
                    && let Some(position) = child.position
                {
                    if trace_projected {
                        eprintln!(
                            "PROJECTED_BOX_RUST id={} owner={} source=external-container childPos={:?} ownerPos={:?} offset={:?} size={:?}",
                            projected.tala_id,
                            self.nodes[projected.owner.0 as usize].tala_id,
                            position,
                            self.position(projected.owner),
                            projected.offset,
                            projected.size,
                        );
                    }
                    return Some((position, projected.size));
                }
            }
        }
        let direct_aggregate_children = self
            .transaction_external_aggregate_children
            .get(&self.active_node_tala_id(projected.owner));
        if let Some(children) = direct_aggregate_children
            && let Some(child) = children
                .iter()
                .find(|child| child.tala_id == projected.tala_id)
            && let Some(position) = child.position
        {
            if trace_projected {
                eprintln!(
                    "PROJECTED_BOX_RUST id={} owner={} source=external-aggregate childPos={:?} ownerPos={:?} offset={:?} size={:?}",
                    projected.tala_id,
                    self.nodes[projected.owner.0 as usize].tala_id,
                    position,
                    self.position(projected.owner),
                    projected.offset,
                    projected.size,
                );
            }
            return Some((position, projected.size));
        }
        if direct_aggregate_children.is_none() {
            for children in self.transaction_external_aggregate_children.values() {
                if let Some(child) = children
                    .iter()
                    .find(|child| child.tala_id == projected.tala_id)
                    && let Some(position) = child.position
                {
                    if trace_projected {
                        eprintln!(
                            "PROJECTED_BOX_RUST id={} owner={} source=external-aggregate childPos={:?} ownerPos={:?} offset={:?} size={:?}",
                            projected.tala_id,
                            self.nodes[projected.owner.0 as usize].tala_id,
                            position,
                            self.position(projected.owner),
                            projected.offset,
                            projected.size,
                        );
                    }
                    return Some((position, projected.size));
                }
            }
        }
        // The stable carrier retains the first member's input position, while
        // TALA's active cluster owner is a separate vessel with its own
        // pending top-left. Project member boxes from that vessel frame.
        let owner = if self.active_cluster_index(projected.owner).is_some() {
            self.active_node_position(projected.owner)?
        } else {
            self.position(projected.owner)?
        };
        let result = (
            Point {
                x: owner.x + projected.offset.x,
                y: owner.y + projected.offset.y,
            },
            projected.size,
        );
        if trace_projected {
            eprintln!(
                "PROJECTED_BOX_LOOKUP_RUST id={} owner={} ownerPos={:?} offset={:?} directMaps={:?}",
                projected.tala_id,
                self.nodes[projected.owner.0 as usize].tala_id,
                self.position(projected.owner),
                projected.offset,
                self.transaction_external_container_children
                    .iter()
                    .filter_map(|(parent, children)| {
                        children
                            .iter()
                            .find(|child| child.tala_id == projected.tala_id)
                            .map(|child| (*parent, child.position))
                    })
                    .collect::<Vec<_>>()
            );
        }
        if trace_projected {
            eprintln!(
                "PROJECTED_BOX_RUST id={} owner={} source=owner-offset ownerPos={:?} offset={:?} resultPos={:?} size={:?}",
                projected.tala_id,
                self.nodes[projected.owner.0 as usize].tala_id,
                owner,
                projected.offset,
                result.0,
                result.1,
            );
        }
        Some(result)
    }

    /// Projects an original stable-arena endpoint through the active
    /// AddSequences/AddClusters vessel that currently owns it.
    ///
    /// TALA physically reconnects every external member edge to the temporary
    /// vessel. Unbound numerical scopes retain stable input endpoints and
    /// represent that lifecycle as a scoring projection until cleanup. Bound
    /// scopes instead retain the actual original endpoint records.
    pub(super) fn active_aggregate_endpoint_projection(
        &self,
        endpoint: NodeId,
    ) -> Option<ProjectedAdjacent> {
        self.active_aggregate_endpoint_projection_with_sequence_abductions(endpoint, false)
    }

    /// Reconstructs the endpoint replacements visible to a particular TALA
    /// scoring call. Most aggregate-stage callers retain cluster abductions
    /// only: a direct sequence edge therefore measures from the full vessel.
    /// `Graph.AlignAxes` additionally passes every sequence EdgeAbduction to
    /// `getGlobalEdgeLength`, restoring that edge's original step geometry.
    pub(super) fn active_aggregate_endpoint_projection_with_sequence_abductions(
        &self,
        endpoint: NodeId,
        restore_sequence_endpoint: bool,
    ) -> Option<ProjectedAdjacent> {
        if let Some(cluster_index) = self.active_cluster_index(endpoint) {
            let cluster = &self.clusters[cluster_index];
            if restore_sequence_endpoint {
                // Supplying the sequence-abduction slice is materially
                // different from passing nil: Node.edgeLength only gathers
                // cluster abductions in the nil case. After Cluster.AbductEdges
                // reconnects an edge to this vessel, a sequence abduction's
                // CurrentFrom/CurrentTo no longer matches either endpoint.
                // Alignment scoring therefore observes the full current
                // cluster vessel, not its retained member geometry.
                let owner = cluster.members[0];
                return Some(ProjectedAdjacent {
                    owner,
                    tala_id: cluster.vessel_tala_id,
                    container_tala_id: None,
                    offset: Point::default(),
                    size: self.cluster_vessel_size(cluster_index),
                    cluster_member: false,
                });
            }
            // Cluster.AbductEdges reconnects the edge to the vessel, but its
            // EdgeAbduction retains the endpoint that existed immediately
            // before that operation. Node.edgeLength therefore routes from
            // that member's arranged box, not from the full vessel bounds.
            let member = self.active_sequence_owner(endpoint);
            let (offset, size) = self.cluster_member_geometry(cluster_index, member)?;
            let tala_id = self
                .active_sequence_index(endpoint)
                .map(|sequence| self.sequences[sequence].vessel_tala_id)
                .unwrap_or(self.nodes[member.0 as usize].tala_id);
            return Some(ProjectedAdjacent {
                owner: cluster.members[0],
                tala_id,
                container_tala_id: None,
                offset,
                size,
                cluster_member: true,
            });
        }
        self.active_sequence_index(endpoint).map(|sequence_index| {
            let sequence = &self.sequences[sequence_index];
            let (offset, size, tala_id, container_tala_id) = if restore_sequence_endpoint {
                let (offset, size) = self
                    .sequence_member_geometry(sequence_index, endpoint)
                    .expect("active sequence member geometry");
                (offset, size, self.nodes[endpoint.0 as usize].tala_id, None)
            } else {
                (
                    Point::default(),
                    self.sequence_vessel_size(sequence_index),
                    sequence.vessel_tala_id,
                    sequence
                        .container
                        .map(|container| self.nodes[container.0 as usize].tala_id),
                )
            };
            ProjectedAdjacent {
                owner: sequence.members[0],
                tala_id,
                container_tala_id,
                offset,
                size,
                cluster_member: false,
            }
        })
    }

    /// Applies the full CurrentFrom/CurrentTo match used by Node.edgeLength.
    /// A sequence abduction cannot restore just one endpoint after a later
    /// cluster abduction has reconnected either current endpoint.
    pub(super) fn active_aggregate_edge_endpoint_projection(
        &self,
        endpoint: NodeId,
        other_endpoint: NodeId,
        restore_sequence_endpoints: bool,
    ) -> Option<ProjectedAdjacent> {
        // With a nil EdgeAbductions slice, OSS TALA reconstructs cluster
        // replacements but leaves sequence-vessel endpoints as the current
        // vessel. Sequence abductions are only restored by AlignAxes, which
        // explicitly supplies that slice.
        if !restore_sequence_endpoints && self.active_sequence_index(endpoint).is_some() {
            return None;
        }
        if restore_sequence_endpoints && self.active_cluster_index(endpoint).is_some() {
            return self
                .active_aggregate_endpoint_projection_with_sequence_abductions(endpoint, true);
        }
        let sequence_pair_still_current = self.active_cluster_index(endpoint).is_none()
            && self.active_cluster_index(other_endpoint).is_none();
        self.active_aggregate_endpoint_projection_with_sequence_abductions(
            endpoint,
            restore_sequence_endpoints && sequence_pair_still_current,
        )
    }

    /// edge_length.go:971–1130 reads both actual Cluster pointers separately.
    /// Vessel geometry and external abductions belong to that Cluster, not to
    /// an induced carrier's retained projection.
    fn sized_owner_cluster_arrangement_penalty_checked(
        &self,
        frame: &OwnerScoreFrame<'_>,
        replacement: NodeId,
        other: NodeId,
        edge_direction: Orientation,
        turn_cost: f64,
    ) -> Result<Option<[f64; 3]>, limits::Error> {
        let Some(group) = frame
            .owner()
            .owner_node_group_id(replacement, super::owner_groups::OwnerGroupKind::Cluster)
            .map_err(owner_score_error)?
        else {
            return Ok(None);
        };
        let policy = frame
            .owner()
            .owner_cluster_policy(group)
            .map_err(owner_score_error)?;
        let fields = frame
            .owner()
            .owner_group_fields(group)
            .map_err(owner_score_error)?;
        let vessel = fields
            .vessel
            .ok_or_else(|| owner_score_error("cluster has nil Vessel"))?;
        let live_box = |node| -> Result<_, limits::Error> {
            let size = frame.node(node).map_err(owner_score_error)?.rect.size;
            Ok(frame
                .position(node)
                .map_err(owner_score_error)?
                .map(|position| (position, size)))
        };
        let vessel_box = live_box(vessel)?;
        let other_box = live_box(other)?;
        // Node.Orientation returns NONE for either nil TopLeft; Center is
        // read only in branches whose cardinal orientation proves positions.
        let orientation = |left: Option<(Point, Size)>, right: Option<(Point, Size)>| {
            left.zip(right).map_or(Orientation::None, |(left, right)| {
                self.sized_box_orientation(left, right)
            })
        };
        let orientation_penalty = match policy.arrangement.as_str() {
            "Row" if matches!(edge_direction, Orientation::Left | Orientation::Right) => {
                turn_cost * 2.0
            }
            "Column" if matches!(edge_direction, Orientation::Top | Orientation::Bottom) => {
                turn_cost * 2.0
            }
            _ => 0.0,
        };
        let this_direction = orientation(vessel_box, other_box);
        let mut side_penalty = 0.0;
        if let Some([first, second]) = frame
            .cluster_external_connected_nodes(group)
            .map_err(owner_score_error)?
        {
            let external = if first == other { second } else { first };
            let other_direction = orientation(vessel_box, live_box(external)?);
            let side = |direction| match policy.arrangement.as_str() {
                "Row" => match direction {
                    Orientation::TopLeft | Orientation::Top | Orientation::TopRight => -1,
                    Orientation::BottomLeft | Orientation::Bottom | Orientation::BottomRight => 1,
                    _ => 0,
                },
                "Column" => match direction {
                    Orientation::TopLeft | Orientation::Left | Orientation::BottomLeft => -1,
                    Orientation::TopRight | Orientation::Right | Orientation::BottomRight => 1,
                    _ => 0,
                },
                _ => 0,
            };
            if side(other_direction) != 0 && side(other_direction) == side(this_direction) {
                side_penalty = turn_cost;
            }
        }
        let center_cost = match policy.arrangement.as_str() {
            "Row" if matches!(this_direction, Orientation::Top | Orientation::Bottom) => {
                let vessel_box = vessel_box.expect("cardinal orientation requires TopLeft");
                let other_box = other_box.expect("cardinal orientation requires TopLeft");
                ((vessel_box.0.x + vessel_box.1.width * 0.5)
                    - (other_box.0.x + other_box.1.width * 0.5))
                    .abs()
            }
            "Column" if matches!(this_direction, Orientation::Left | Orientation::Right) => {
                let vessel_box = vessel_box.expect("cardinal orientation requires TopLeft");
                let other_box = other_box.expect("cardinal orientation requires TopLeft");
                ((vessel_box.0.y + vessel_box.1.height * 0.5)
                    - (other_box.0.y + other_box.1.height * 0.5))
                    .abs()
            }
            _ => turn_cost,
        };
        Ok(Some([orientation_penalty, side_penalty, center_cost]))
    }

    fn sized_cluster_arrangement_penalty(
        &self,
        projected: ProjectedAdjacent,
        other_box: (Point, Size),
        edge_direction: Orientation,
        current_other_tala_id: i64,
        live_vessel_positions: &BTreeMap<i64, Point>,
    ) -> [f64; 3] {
        let retained = self
            .sized_cluster_distance_boxes
            .get(&(projected.owner, projected.tala_id));
        let arrangement = retained
            .map(|cluster| cluster.arrangement)
            .unwrap_or_else(|| {
                self.nodes[projected.owner.0 as usize]
                    .scoring_cluster_arrangement
                    .or_else(|| {
                        self.clusters
                            .iter()
                            .find(|cluster| {
                                cluster.members.first() == Some(&projected.owner)
                                    && self.cluster_is_active(cluster)
                            })
                            .map(|cluster| cluster.arrangement)
                    })
                    .expect("cluster projection owner")
            });
        let owner_position = self
            .active_node_position(projected.owner)
            .expect("cluster vessel position");
        let vessel_box = retained.map_or(
            (owner_position, self.active_node_size(projected.owner)),
            |cluster| {
                (
                    live_vessel_positions
                        .get(&cluster.vessel_tala_id)
                        .copied()
                        .unwrap_or(Point {
                            x: owner_position.x + cluster.offset.x,
                            y: owner_position.y + cluster.offset.y,
                        }),
                    cluster.size,
                )
            },
        );
        let orientation_penalty = match arrangement {
            ClusterArrangement::Row
                if matches!(edge_direction, Orientation::Left | Orientation::Right) =>
            {
                self.turn_cost_value() * 2.0
            }
            ClusterArrangement::Column
                if matches!(edge_direction, Orientation::Top | Orientation::Bottom) =>
            {
                self.turn_cost_value() * 2.0
            }
            _ => 0.0,
        };
        let mut side_penalty = 0.0;

        let this_direction = self.sized_box_orientation(vessel_box, other_box);
        let external_nodes = if let Some(retained) = retained {
            retained
                .external_connected
                .iter()
                .filter_map(|external| {
                    let owner = self.position(external.owner)?;
                    Some((
                        external.tala_id,
                        (
                            Point {
                                x: owner.x + external.offset.x,
                                y: owner.y + external.offset.y,
                            },
                            external.size,
                        ),
                    ))
                })
                .collect::<Vec<_>>()
        } else {
            // The main graph has no materialized SizedEdgeAbduction record;
            // Go's Cluster.getExternalConnectedNodes instead walks the
            // cluster's live edge-abduction inventory. Reconstruct that
            // inventory from the active cluster membership and stable edges,
            // collapsing an external aggregate to its current vessel.
            if let Some(cluster_index) = self.active_cluster_index(projected.owner) {
                let members = self.clusters[cluster_index]
                    .members
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>();
                let vessel = self.topology_vessel_node(TopologyVessel::Cluster(cluster_index));
                let mut external_nodes = BTreeMap::new();
                for edge in self.member_edges() {
                    // CurrentFrom/CurrentTo become the actual Vessel after
                    // AddCluster reconnects the original member endpoints.
                    // Keep the same external inventory across that boundary.
                    let from_in = members.contains(&edge.from) || vessel == Some(edge.from);
                    let to_in = members.contains(&edge.to) || vessel == Some(edge.to);
                    if from_in == to_in {
                        continue;
                    }
                    let external = if from_in { edge.to } else { edge.from };
                    let external = self.active_aggregate_owner(external);
                    if let Some(position) = self.position(external) {
                        external_nodes
                            .entry(self.nodes[external.0 as usize].tala_id)
                            .or_insert((position, self.active_node_size(external)));
                    }
                }
                external_nodes.into_iter().collect::<Vec<_>>()
            } else {
                self.sized_adjacent_overrides
                    .iter()
                    .filter_map(|(&(external, _), candidate)| {
                        if !candidate.cluster_member || candidate.owner != projected.owner {
                            return None;
                        }
                        Some((
                            self.nodes[external.0 as usize].tala_id,
                            (
                                self.position(external)?,
                                self.nodes[external.0 as usize].rect.size,
                            ),
                        ))
                    })
                    .collect::<BTreeMap<_, _>>()
                    .into_iter()
                    .collect::<Vec<_>>()
            }
        };
        if external_nodes.len() == 2 {
            let mut other_external = external_nodes[0];
            if other_external.0 == current_other_tala_id {
                other_external = external_nodes[1];
            }
            let (_, (other_position, other_size)) = other_external;
            let other_direction =
                self.sized_box_orientation(vessel_box, (other_position, other_size));
            let same_arrangement_side = match arrangement {
                ClusterArrangement::Row => {
                    let vertical_side = |direction| match direction {
                        Orientation::TopLeft | Orientation::TopRight | Orientation::Top => -1,
                        Orientation::BottomLeft
                        | Orientation::BottomRight
                        | Orientation::Bottom => 1,
                        _ => 0,
                    };
                    vertical_side(other_direction) != 0
                        && vertical_side(other_direction) == vertical_side(this_direction)
                }
                ClusterArrangement::Column => {
                    let horizontal_side = |direction| match direction {
                        Orientation::TopLeft | Orientation::BottomLeft | Orientation::Left => -1,
                        Orientation::TopRight | Orientation::BottomRight | Orientation::Right => 1,
                        _ => 0,
                    };
                    horizontal_side(other_direction) != 0
                        && horizontal_side(other_direction) == horizontal_side(this_direction)
                }
            };
            if same_arrangement_side {
                side_penalty = self.turn_cost_value();
            }
        }

        let vessel_center = Point {
            x: vessel_box.0.x + vessel_box.1.width * 0.5,
            y: vessel_box.0.y + vessel_box.1.height * 0.5,
        };
        let other_center = Point {
            x: other_box.0.x + other_box.1.width * 0.5,
            y: other_box.0.y + other_box.1.height * 0.5,
        };
        let trace_cluster_penalty =
            crate::internal::trace_env_enabled("TALA_TRACE_CLUSTER_PENALTY")
                && crate::internal::trace_env_value("TALA_TRACE_CLUSTER_PENALTY_TARGET")
                    .is_none_or(|target| {
                        target == "all"
                            || target
                                .split(',')
                                .any(|id| id.parse::<i64>().ok() == Some(projected.tala_id))
                    });
        let center_cost = match arrangement {
            ClusterArrangement::Row
                if matches!(this_direction, Orientation::Top | Orientation::Bottom) =>
            {
                (vessel_center.x - other_center.x).abs()
            }
            ClusterArrangement::Column
                if matches!(this_direction, Orientation::Left | Orientation::Right) =>
            {
                (vessel_center.y - other_center.y).abs()
            }
            _ => self.turn_cost_value(),
        };
        if trace_cluster_penalty {
            eprintln!(
                "CLUSTER_PENALTY_RUST projected={} owner={} ownerPos={:?} arrangement={:?} vessel={:?} other={:?} this={:?} orientation={} side={} center={} turn={} externalCount={}",
                projected.tala_id,
                self.nodes[projected.owner.0 as usize].tala_id,
                self.position(projected.owner),
                arrangement,
                vessel_box,
                other_box,
                this_direction,
                orientation_penalty,
                side_penalty,
                center_cost,
                self.raw_turn_cost(),
                external_nodes.len(),
            );
            eprintln!(
                "CLUSTER_EXTERNALS_RUST projected={} current={} externals={:?}",
                projected.tala_id, current_other_tala_id, external_nodes
            );
        }
        let result = orientation_penalty + side_penalty + center_cost;
        if trace_cluster_penalty {
            eprintln!(
                "CLUSTER_PENALTY_RUST_RESULT projected={} value={} bits=0x{:016x}",
                projected.tala_id,
                result,
                result.to_bits(),
            );
        }
        [orientation_penalty, side_penalty, center_cost]
    }

    pub(super) fn sized_edge_boxes(
        &self,
        node: NodeId,
        adjacent: NodeId,
        edge: EdgeId,
    ) -> Option<((Point, Size), (Point, Size))> {
        let mut node_box = (self.position(node)?, self.nodes[node.0 as usize].rect.size);
        let mut adjacent_box = (
            self.position(adjacent)?,
            self.nodes[adjacent.0 as usize].rect.size,
        );
        if let Some(projected_node) = self.sized_adjacent_overrides.get(&(adjacent, edge)) {
            node_box = self.sized_projected_box(*projected_node)?;
        }
        if let Some(projected_adjacent) = self.sized_adjacent_overrides.get(&(node, edge)) {
            adjacent_box = self.sized_projected_box(*projected_adjacent)?;
        }
        Some((node_box, adjacent_box))
    }

    fn symmetry_segment_intersects_node(&self, start: Point, end: Point, node: NodeId) -> bool {
        // Active cluster/sequence members stand in for Go's temporary vessel
        // pointer during symmetry scoring. Use the vessel position together
        // with the vessel size; combining the retained member position with
        // the vessel size creates an obstruction box Go never sees.
        let position = self.active_node_position(node).unwrap();
        let size = self.active_node_size(node);
        Self::segment_intersects_box(start, end, position, size)
    }

    /// Translation of TALA's shared `segmentIntersectsBox` geometry helper.
    pub(super) fn segment_intersects_box(
        start: Point,
        end: Point,
        position: Point,
        size: Size,
    ) -> bool {
        let mut left = position.x;
        let mut right = position.x + size.width;
        if left > right {
            std::mem::swap(&mut left, &mut right);
        }
        let mut top = position.y;
        let mut bottom = position.y + size.height;
        if top > bottom {
            std::mem::swap(&mut top, &mut bottom);
        }

        // Match the v0.9.0 Go helper's explicit NaN guard.  Ordered
        // comparisons alone would otherwise let a NaN boundary through.
        if left.is_nan() || right.is_nan() || top.is_nan() || bottom.is_nan() {
            return false;
        }

        if (start.x < left && end.x < left)
            || (start.x > right && end.x > right)
            || (start.y < top && end.y < top)
            || (start.y > bottom && end.y > bottom)
        {
            return false;
        }

        let contains = |point: Point| {
            left <= point.x && point.x <= right && top <= point.y && point.y <= bottom
        };
        if contains(start) || contains(end) {
            return true;
        }

        let mut t_enter: f64 = 0.0;
        let mut t_exit: f64 = 1.0;
        let mut clip_axis = |axis_start: f64, delta: f64, min_coord: f64, max_coord: f64| -> bool {
            if delta == 0.0 {
                return min_coord <= axis_start && axis_start <= max_coord;
            }
            let mut t1 = (min_coord - axis_start) / delta;
            let mut t2 = (max_coord - axis_start) / delta;
            if t1 > t2 {
                std::mem::swap(&mut t1, &mut t2);
            }
            t_enter = t_enter.max(t1);
            t_exit = t_exit.min(t2);
            t_enter <= t_exit
        };

        if !clip_axis(start.x, end.x - start.x, left, right)
            || !clip_axis(start.y, end.y - start.y, top, bottom)
        {
            return false;
        }
        t_enter < t_exit
    }

    pub(super) fn edge_route_penalty(
        &self,
        node: NodeId,
        adjacent: NodeId,
        edge: EdgeId,
        _sorted_obstructions: Option<(&[NodeId], f64)>,
        projected_endpoints: Option<ProjectedEndpoints>,
        cluster_endpoint: bool,
    ) -> f64 {
        self.edge_route_penalty_cached(
            node,
            adjacent,
            edge,
            None,
            projected_endpoints,
            ProjectedObstructionSource::Edge,
            cluster_endpoint,
            false,
        )
    }

    fn edge_route_penalty_cached(
        &self,
        node: NodeId,
        adjacent: NodeId,
        edge: EdgeId,
        cached_obstructions: Option<&[NodeId]>,
        projected_endpoints: Option<ProjectedEndpoints>,
        projected_obstruction_source: ProjectedObstructionSource<'_>,
        cluster_endpoint: bool,
        tree_children_restored: bool,
    ) -> f64 {
        self.edge_route_penalty_cached_checked(
            node,
            adjacent,
            edge,
            cached_obstructions,
            projected_endpoints,
            projected_obstruction_source,
            cluster_endpoint,
            tree_children_restored,
            ScoreContext::none(),
        )
        .expect("unchecked scoring context cannot cancel")
    }

    /// Retain Graph.Containers slice boundaries as well as the existing
    /// endpoint-to-shared-ancestor visit order. Cancellation resets its inner
    /// index at each source slice, including an empty slice.
    fn scoring_obstruction_sets(
        &self,
        node: NodeId,
        adjacent: NodeId,
        tree_children_restored: bool,
    ) -> Vec<Vec<NodeId>> {
        let chain = |endpoint: NodeId| {
            let mut scopes = Vec::new();
            let mut scope = self.nodes[endpoint.0 as usize].container;
            loop {
                scopes.push(scope);
                let Some(parent) = scope else { break };
                scope = self.nodes[parent.0 as usize].container;
            }
            scopes
        };
        let node_chain = chain(node);
        let adjacent_chain = chain(adjacent);
        let ancestor = node_chain
            .iter()
            .copied()
            .find(|scope| adjacent_chain.contains(scope))
            .flatten();
        let mut seen = BTreeSet::new();
        let mut sets = Vec::new();
        for endpoint in [node, adjacent] {
            let mut scope = self.nodes[endpoint.0 as usize].container;
            loop {
                if !seen.insert(scope) {
                    break;
                }
                sets.push(self.obstruction_container_node_order_with_materialized(
                    scope,
                    tree_children_restored,
                ));
                let Some(parent) = scope else { break };
                scope = self.nodes[parent.0 as usize].container;
                if scope == ancestor {
                    break;
                }
            }
        }
        sets
    }

    fn edge_route_penalty_cached_checked(
        &self,
        node: NodeId,
        adjacent: NodeId,
        edge: EdgeId,
        cached_obstructions: Option<&[NodeId]>,
        projected_endpoints: Option<ProjectedEndpoints>,
        projected_obstruction_source: ProjectedObstructionSource<'_>,
        cluster_endpoint: bool,
        tree_children_restored: bool,
        context: ScoreContext<'_>,
    ) -> Result<f64, limits::Error> {
        self.edge_route_penalty_with_owner_frame_checked(
            node,
            adjacent,
            edge,
            cached_obstructions,
            projected_endpoints,
            projected_obstruction_source,
            cluster_endpoint,
            tree_children_restored,
            None,
            None,
            context,
        )
    }

    fn edge_route_penalty_with_owner_frame_checked(
        &self,
        node: NodeId,
        adjacent: NodeId,
        edge: EdgeId,
        cached_obstructions: Option<&[NodeId]>,
        projected_endpoints: Option<ProjectedEndpoints>,
        projected_obstruction_source: ProjectedObstructionSource<'_>,
        cluster_endpoint: bool,
        tree_children_restored: bool,
        owner_operands: Option<(&OwnerScoreFrame<'_>, NodeId, NodeId)>,
        cached_owner_obstructions: Option<&mut Option<SizedOwnerObstructions>>,
        context: ScoreContext<'_>,
    ) -> Result<f64, limits::Error> {
        let (
            (node_position, node_size, node_tala_id),
            (adjacent_position, adjacent_size, adjacent_tala_id),
        ) = projected_endpoints.unwrap_or_else(|| {
            (
                (
                    self.position(node).unwrap(),
                    self.nodes[node.0 as usize].rect.size,
                    self.nodes[node.0 as usize].tala_id,
                ),
                (
                    self.position(adjacent).unwrap(),
                    self.nodes[adjacent.0 as usize].rect.size,
                    self.nodes[adjacent.0 as usize].tala_id,
                ),
            )
        });
        let orientation = self.sized_box_orientation(
            (node_position, node_size),
            (adjacent_position, adjacent_size),
        );
        if orientation == Orientation::None {
            return Ok(0.0);
        }
        let node_center = Point {
            x: node_position.x + node_size.width * 0.5,
            y: node_position.y + node_size.height * 0.5,
        };
        let adjacent_center = Point {
            x: adjacent_position.x + adjacent_size.width * 0.5,
            y: adjacent_position.y + adjacent_size.height * 0.5,
        };

        let (mut start, mut end, mut diagonal) = match orientation {
            Orientation::Top => {
                let middle = ((node_position.x + node_size.width)
                    .min(adjacent_position.x + adjacent_size.width)
                    + node_position.x.max(adjacent_position.x))
                    * 0.5;
                (
                    Point {
                        x: middle,
                        y: node_position.y + node_size.height,
                    },
                    Point {
                        x: middle,
                        y: adjacent_position.y,
                    },
                    false,
                )
            }
            Orientation::Bottom => {
                let middle = ((node_position.x + node_size.width)
                    .min(adjacent_position.x + adjacent_size.width)
                    + node_position.x.max(adjacent_position.x))
                    * 0.5;
                (
                    Point {
                        x: middle,
                        y: node_position.y,
                    },
                    Point {
                        x: middle,
                        y: adjacent_position.y + adjacent_size.height,
                    },
                    false,
                )
            }
            Orientation::Left => {
                let middle = ((node_position.y + node_size.height)
                    .min(adjacent_position.y + adjacent_size.height)
                    + node_position.y.max(adjacent_position.y))
                    * 0.5;
                (
                    Point {
                        x: node_position.x + node_size.width,
                        y: middle,
                    },
                    Point {
                        x: adjacent_position.x,
                        y: middle,
                    },
                    false,
                )
            }
            Orientation::Right => {
                let middle = ((node_position.y + node_size.height)
                    .min(adjacent_position.y + adjacent_size.height)
                    + node_position.y.max(adjacent_position.y))
                    * 0.5;
                (
                    Point {
                        x: node_position.x,
                        y: middle,
                    },
                    Point {
                        x: adjacent_position.x + adjacent_size.width,
                        y: middle,
                    },
                    false,
                )
            }
            Orientation::TopLeft
            | Orientation::TopRight
            | Orientation::BottomLeft
            | Orientation::BottomRight => (node_center, adjacent_center, true),
            Orientation::None => unreachable!(),
        };
        if cluster_endpoint {
            (start, end) = match orientation {
                Orientation::TopLeft | Orientation::TopRight | Orientation::Top => (
                    Point {
                        x: node_center.x,
                        y: node_position.y + node_size.height,
                    },
                    Point {
                        x: adjacent_center.x,
                        y: adjacent_position.y,
                    },
                ),
                Orientation::BottomLeft | Orientation::BottomRight | Orientation::Bottom => (
                    Point {
                        x: node_center.x,
                        y: node_position.y,
                    },
                    Point {
                        x: adjacent_center.x,
                        y: adjacent_position.y + adjacent_size.height,
                    },
                ),
                Orientation::Left => (
                    Point {
                        x: node_position.x + node_size.width,
                        y: node_center.y,
                    },
                    Point {
                        x: adjacent_position.x,
                        y: adjacent_center.y,
                    },
                ),
                Orientation::Right => (
                    Point {
                        x: node_position.x,
                        y: node_center.y,
                    },
                    Point {
                        x: adjacent_position.x + adjacent_size.width,
                        y: adjacent_center.y,
                    },
                ),
                Orientation::None => unreachable!(),
            };
            diagonal = false;
        }
        // Direct translation of calculateHorizontalMidpoint and
        // calculateVerticalMidpoint: sufficiently offset parallel sides admit
        // either of two alternate one-turn routes when the direct segment is
        // obstructed.
        let semi_diagonal = !cluster_endpoint
            && match orientation {
                Orientation::Top | Orientation::Bottom => {
                    (node_position.x - adjacent_position.x).abs() > 40.0
                        || ((node_position.x + node_size.width)
                            - (adjacent_position.x + adjacent_size.width))
                            .abs()
                            > 40.0
                }
                Orientation::Left | Orientation::Right => {
                    (node_position.y - adjacent_position.y).abs() > 40.0
                        || ((node_position.y + node_size.height)
                            - (adjacent_position.y + adjacent_size.height))
                            .abs()
                            > 40.0
                }
                _ => false,
            };
        let mut corner_a_blocked = false;
        let mut corner_b_blocked = false;
        let mut direct_blocked = false;
        // Go keeps this route-level flag separate from the two alternate
        // corridor flags.  The latter may be pre-blocked by the side
        // alignment of a semi-diagonal edge; that does not mean both
        // alternate routes have been proven blocked.  In particular, TALA
        // charges one turn when only the pre-blocked corridor is unavailable.
        let mut alternate_route_blocked = !semi_diagonal;
        let (mut alternate_a_blocked, mut alternate_b_blocked) = match orientation {
            Orientation::Top | Orientation::Bottom
                if (node_position.x - adjacent_position.x).abs() <= 40.0 =>
            {
                (false, true)
            }
            Orientation::Top | Orientation::Bottom
                if ((node_position.x + node_size.width)
                    - (adjacent_position.x + adjacent_size.width))
                    .abs()
                    <= 40.0 =>
            {
                (true, false)
            }
            Orientation::Left | Orientation::Right
                if (node_position.y - adjacent_position.y).abs() <= 40.0 =>
            {
                (true, false)
            }
            Orientation::Left | Orientation::Right
                if ((node_position.y + node_size.height)
                    - (adjacent_position.y + adjacent_size.height))
                    .abs()
                    <= 40.0 =>
            {
                (false, true)
            }
            _ => (false, false),
        };
        let route_left = start.x.min(end.x);
        let route_right = start.x.max(end.x);
        let route_top = start.y.min(end.y);
        let route_bottom = start.y.max(end.y);
        // Go's scoringNodeBounds filter runs before the ancestry and segment
        // predicates.  It is easy to miss because the segment helper itself
        // has a similar early-out for non-semi-diagonal routes, but Go also
        // applies the filter to semi-diagonal projected obstructions.  Keep
        // the same finite-value escape hatch and paired comparisons here so
        // projected child boxes cannot spuriously block a route merely by
        // being present in the temporary obstruction inventory.
        let bound_limit = f64::MAX / 8.0;
        let bounds_left = node_position.x.min(adjacent_position.x);
        let bounds_top = node_position.y.min(adjacent_position.y);
        let bounds_right =
            (node_position.x + node_size.width).max(adjacent_position.x + adjacent_size.width);
        let bounds_bottom =
            (node_position.y + node_size.height).max(adjacent_position.y + adjacent_size.height);
        let bounds_usable = [
            node_position.x,
            node_position.y,
            node_position.x + node_size.width,
            node_position.y + node_size.height,
            adjacent_position.x,
            adjacent_position.y,
            adjacent_position.x + adjacent_size.width,
            adjacent_position.y + adjacent_size.height,
        ]
        .into_iter()
        .all(|value| value >= -bound_limit && value <= bound_limit);
        let obstruction_excluded = |obstruction_position: Point, obstruction_size: Size| {
            if !bounds_usable {
                return false;
            }
            let right = obstruction_position.x + obstruction_size.width;
            let bottom = obstruction_position.y + obstruction_size.height;
            // All four are finite exactly when this sum of self-differences
            // is zero: a finite value gives +0.0, an infinity or NaN gives NaN.
            // It stays in floating-point registers, unlike four is_finite
            // bit tests.
            #[allow(clippy::eq_op)]
            let finite = (obstruction_position.x - obstruction_position.x)
                + (obstruction_position.y - obstruction_position.y)
                + (right - right)
                + (bottom - bottom)
                == 0.0;
            if !finite {
                return false;
            }
            (obstruction_position.x < bounds_left && right < bounds_left)
                || (obstruction_position.x > bounds_right && right > bounds_right)
                || (obstruction_position.y < bounds_top && bottom < bounds_top)
                || (obstruction_position.y > bounds_bottom && bottom > bounds_bottom)
        };
        let alternate_route_hits = |obstruction_position: Point,
                                    obstruction_size: Size|
         -> (bool, bool) {
            let intersects = |a: Point, b: Point| {
                Self::segment_intersects_box(a, b, obstruction_position, obstruction_size)
            };
            let mut a_position = node_position;
            let mut a_size = node_size;
            let mut a_center = node_center;
            let mut b_position = adjacent_position;
            let mut b_size = adjacent_size;
            let mut b_center = adjacent_center;
            if matches!(orientation, Orientation::Bottom | Orientation::Right) {
                std::mem::swap(&mut a_position, &mut b_position);
                std::mem::swap(&mut a_size, &mut b_size);
                std::mem::swap(&mut a_center, &mut b_center);
            }
            match orientation {
                Orientation::Top | Orientation::Bottom => {
                    let floor = a_position.x.max(b_position.x);
                    let ceiling = (a_position.x + a_size.width).min(b_position.x + b_size.width);
                    let minimum = a_position.x.min(b_position.x);
                    let lower = minimum + (floor - minimum).abs() * 0.5;
                    let lower_blocked = if a_position.x < b_position.x {
                        intersects(
                            Point {
                                x: lower,
                                y: a_center.y,
                            },
                            Point {
                                x: lower,
                                y: b_center.y,
                            },
                        ) || intersects(
                            Point {
                                x: lower,
                                y: b_center.y,
                            },
                            b_center,
                        )
                    } else if a_position.x > b_position.x {
                        intersects(
                            a_center,
                            Point {
                                x: lower,
                                y: a_center.y,
                            },
                        ) || intersects(
                            Point {
                                x: lower,
                                y: b_center.y,
                            },
                            Point {
                                x: lower,
                                y: a_center.y,
                            },
                        )
                    } else {
                        false
                    };
                    let maximum = (a_position.x + a_size.width).max(b_position.x + b_size.width);
                    let upper = maximum - (ceiling - maximum).abs() * 0.5;
                    let upper_blocked = if a_position.x + a_size.width > b_position.x + b_size.width
                    {
                        intersects(
                            Point {
                                x: upper,
                                y: a_center.y,
                            },
                            Point {
                                x: upper,
                                y: b_center.y,
                            },
                        ) || intersects(
                            b_center,
                            Point {
                                x: upper,
                                y: b_center.y,
                            },
                        )
                    } else if a_position.x + a_size.width < b_position.x + b_size.width {
                        intersects(
                            a_center,
                            Point {
                                x: upper,
                                y: a_center.y,
                            },
                        ) || intersects(
                            Point {
                                x: upper,
                                y: b_center.y,
                            },
                            Point {
                                x: upper,
                                y: a_center.y,
                            },
                        )
                    } else {
                        false
                    };
                    (upper_blocked, lower_blocked)
                }
                Orientation::Left | Orientation::Right => {
                    let floor = a_position.y.max(b_position.y);
                    let ceiling = (a_position.y + a_size.height).min(b_position.y + b_size.height);
                    let minimum = a_position.y.min(b_position.y);
                    let lower = minimum + (floor - minimum).abs() * 0.5;
                    let lower_blocked = if a_position.y < b_position.y {
                        intersects(
                            Point {
                                x: a_center.x,
                                y: lower,
                            },
                            Point {
                                x: b_center.x,
                                y: lower,
                            },
                        ) || intersects(
                            b_center,
                            Point {
                                x: b_center.x,
                                y: lower,
                            },
                        )
                    } else if a_position.y > b_position.y {
                        intersects(
                            a_center,
                            Point {
                                x: a_center.x,
                                y: lower,
                            },
                        ) || intersects(
                            Point {
                                x: b_center.x,
                                y: lower,
                            },
                            Point {
                                x: a_center.x,
                                y: lower,
                            },
                        )
                    } else {
                        false
                    };
                    let maximum = (a_position.y + a_size.height).max(b_position.y + b_size.height);
                    let upper = maximum - (ceiling - maximum).abs() * 0.5;
                    let upper_blocked =
                        if a_position.y + a_size.height > b_position.y + b_size.height {
                            intersects(
                                Point {
                                    x: a_center.x,
                                    y: upper,
                                },
                                Point {
                                    x: b_center.x,
                                    y: upper,
                                },
                            ) || intersects(
                                b_center,
                                Point {
                                    x: b_center.x,
                                    y: upper,
                                },
                            )
                        } else if a_position.y + a_size.height < b_position.y + b_size.height {
                            intersects(
                                a_center,
                                Point {
                                    x: a_center.x,
                                    y: upper,
                                },
                            ) || intersects(
                                Point {
                                    x: b_center.x,
                                    y: upper,
                                },
                                Point {
                                    x: a_center.x,
                                    y: upper,
                                },
                            )
                        } else {
                            false
                        };
                    (lower_blocked, upper_blocked)
                }
                _ => (false, false),
            }
        };
        macro_rules! inspect_obstruction_box {
            ($obstruction_position:expr, $obstruction_size:expr) => {{
                let obstruction_position = $obstruction_position;
                let obstruction_size = $obstruction_size;
                if !semi_diagonal
                    && (obstruction_position.x > route_right
                        || obstruction_position.x + obstruction_size.width < route_left
                        || obstruction_position.y > route_bottom
                        || obstruction_position.y + obstruction_size.height < route_top)
                {
                    (false, false, false, false, false)
                } else if diagonal {
                    let corner_a = Point {
                        x: start.x,
                        y: end.y,
                    };
                    let corner_b = Point {
                        x: end.x,
                        y: start.y,
                    };
                    (
                        !corner_a_blocked
                            && (Self::segment_intersects_box(
                                start,
                                corner_a,
                                obstruction_position,
                                obstruction_size,
                            ) || Self::segment_intersects_box(
                                end,
                                corner_a,
                                obstruction_position,
                                obstruction_size,
                            )),
                        !corner_b_blocked
                            && (Self::segment_intersects_box(
                                start,
                                corner_b,
                                obstruction_position,
                                obstruction_size,
                            ) || Self::segment_intersects_box(
                                end,
                                corner_b,
                                obstruction_position,
                                obstruction_size,
                            )),
                        false,
                        false,
                        false,
                    )
                } else {
                    // edge_length.go:935-940 only probes the alternate after
                    // the best route is blocked, and only for semi-diagonals.
                    // Intersections are pure; keep all item polls outside
                    // these guards in the same order as the source.
                    let direct = !direct_blocked
                        && Self::segment_intersects_box(
                            start,
                            end,
                            obstruction_position,
                            obstruction_size,
                        );
                    let (alternate_a, alternate_b) = if semi_diagonal
                        && (direct_blocked || direct)
                        && !alternate_route_blocked
                    {
                        alternate_route_hits(obstruction_position, obstruction_size)
                    } else {
                        (false, false)
                    };
                    (false, false, direct, alternate_a, alternate_b)
                }
            }};
        }
        macro_rules! inspect_obstruction {
            ($obstruction:expr) => {{
                let obstruction = $obstruction;
                let obstruction_identity = self.route_obstruction_identity(obstruction);
                if cached_obstructions.is_none()
                    && (obstruction_identity == node
                        || obstruction_identity == adjacent
                        || self.is_descendant_of(node, obstruction_identity)
                        || self.is_descendant_of(adjacent, obstruction_identity))
                {
                    (false, false, false, false, false)
                } else {
                    if let Some(obstruction_position) = self.active_node_position(obstruction) {
                        inspect_obstruction_box!(
                            obstruction_position,
                            self.active_node_size(obstruction)
                        )
                    } else {
                        (false, false, false, false, false)
                    }
                }
            }};
        }
        let full_obstruction_inventory = match projected_obstruction_source {
            ProjectedObstructionSource::Edge => {
                self.sized_projected_obstruction_sets.get(&(node, edge))
            }
            ProjectedObstructionSource::Ordered(_, inventory) => Some(inventory),
            ProjectedObstructionSource::Current => None,
        }
        .filter(|inventory| owner_operands.is_none() && inventory.sets_present);
        let projected_obstructions_source = match projected_obstruction_source {
            ProjectedObstructionSource::Edge => self
                .sized_projected_obstructions
                .get(&(node, edge))
                .map(|projections| projections.as_slice()),
            ProjectedObstructionSource::Ordered(obstructions, _) => Some(obstructions),
            ProjectedObstructionSource::Current => None,
        }
        .filter(|_| owner_operands.is_none());
        // The release route probe consumes Graph.Containers in order.  A
        // projected inventory can be copied through an induced arena in a
        // different sibling order, even though each projected box is right.
        // Restore the order within each retained container before running the
        // stateful alternate-route predicate.
        let mut ordered_projected_obstructions = None;
        // The stable sort leaves a group whose keys already ascend unchanged,
        // so copy the inventory only when some group is out of order.
        let container_order_key = |projected: &ProjectedAdjacent| {
            projected.container_tala_id.map(|container_tala_id| {
                self.transaction_external_container_children
                    .get(&container_tala_id)
                    .map(|children| {
                        children
                            .iter()
                            .position(|child| child.tala_id == projected.tala_id)
                            .unwrap_or(usize::MAX)
                    })
            })
        };
        let needs_container_order = |obstructions: &[ProjectedAdjacent]| {
            obstructions.windows(2).any(|pair| {
                pair[0].container_tala_id.is_some()
                    && pair[0].container_tala_id == pair[1].container_tala_id
                    && matches!(
                        (container_order_key(&pair[0]), container_order_key(&pair[1])),
                        (Some(Some(left)), Some(Some(right))) if left > right
                    )
            })
        };
        if let Some(obstructions) = projected_obstructions_source
            .filter(|_| full_obstruction_inventory.is_none())
            .filter(|obstructions| needs_container_order(obstructions))
        {
            let mut ordered = obstructions.to_vec();
            let mut group_start = 0;
            while group_start < ordered.len() {
                let Some(container_tala_id) = ordered[group_start].container_tala_id else {
                    group_start += 1;
                    continue;
                };
                let mut group_end = group_start + 1;
                while group_end < ordered.len()
                    && ordered[group_end].container_tala_id == Some(container_tala_id)
                {
                    group_end += 1;
                }
                if let Some(children) = self
                    .transaction_external_container_children
                    .get(&container_tala_id)
                {
                    ordered[group_start..group_end].sort_by_key(|projected| {
                        children
                            .iter()
                            .position(|child| child.tala_id == projected.tala_id)
                            .unwrap_or(usize::MAX)
                    });
                }
                group_start = group_end;
            }
            ordered_projected_obstructions = Some(ordered);
        }
        let projected_obstructions = ordered_projected_obstructions
            .as_deref()
            .or(projected_obstructions_source);
        let trace_route_obstruction_inventory =
            crate::internal::trace_env_enabled("TALA_TRACE_ROUTE_OBSTRUCTION_INVENTORY")
                && super::sized::trace_sized_graph_scope(self)
                && (crate::internal::trace_env_value("TALA_TRACE_SIZED_PASS").is_none()
                    || crate::internal::trace_env_enabled("TALA_TRACE_SIZED_PASS_ACTIVE"))
                && crate::internal::trace_env_value("TALA_TRACE_ROUTE_OBSTRUCTIONS_NODE")
                    .and_then(|value| value.parse::<i64>().ok())
                    == Some(self.nodes[node.0 as usize].tala_id)
                && crate::internal::trace_env_value("TALA_TRACE_ROUTE_OBSTRUCTIONS_ADJACENT")
                    .is_none_or(|target| {
                        target == "all"
                            || target
                                .parse::<i64>()
                                .ok()
                                .is_some_and(|id| id == self.nodes[adjacent.0 as usize].tala_id)
                    })
                && crate::internal::trace_env_value("TALA_TRACE_ROUTE_OBSTRUCTIONS_POSITION")
                    .is_none_or(|target| {
                        let mut parts = target.split(',');
                        let Some(x) = parts.next().and_then(|value| value.parse::<f64>().ok())
                        else {
                            return false;
                        };
                        let Some(y) = parts.next().and_then(|value| value.parse::<f64>().ok())
                        else {
                            return false;
                        };
                        self.position(node)
                            .is_some_and(|position| position.x == x && position.y == y)
                    });
        if trace_route_obstruction_inventory {
            let current = self
                .edge_obstruction_nodes(node, adjacent)
                .into_iter()
                .map(|obstruction| self.nodes[obstruction.0 as usize].tala_id)
                .collect::<Vec<_>>();
            let projected = projected_obstructions
                .map(|obstructions| {
                    obstructions
                        .iter()
                        .map(|obstruction| {
                            let live_owner_box = (!obstruction.cluster_member
                                && self.nodes[obstruction.owner.0 as usize].tala_id
                                    == obstruction.tala_id)
                                .then(|| {
                                    Some((
                                        self.position(obstruction.owner)?,
                                        self.active_node_size(obstruction.owner),
                                    ))
                                })
                                .flatten();
                            let resolved_box =
                                live_owner_box.or_else(|| self.sized_projected_box(*obstruction));
                            (
                                obstruction.tala_id,
                                self.nodes[obstruction.owner.0 as usize].tala_id,
                                obstruction.container_tala_id,
                                obstruction.offset,
                                obstruction.size,
                                resolved_box,
                                resolved_box.is_some_and(|(position, size)| {
                                    obstruction_excluded(position, size)
                                }),
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            eprintln!(
                "ROUTE_OBSTRUCTION_INVENTORY_RUST node={} adjacent={} edge={} source={} current={:?} projected={:?}",
                self.nodes[node.0 as usize].tala_id,
                self.nodes[adjacent.0 as usize].tala_id,
                edge.0,
                match (
                    projected_obstruction_source,
                    projected_obstructions.is_some()
                ) {
                    (ProjectedObstructionSource::Edge, true) => "projected-edge",
                    (ProjectedObstructionSource::Ordered(..), true) => "projected-ordered",
                    (_, true) => "projected",
                    _ => "current",
                },
                current,
                projected,
            );
        }
        if let Some((frame, node_replacement, adjacent_replacement)) = owner_operands {
            // Source875/891 copy real map headers before source903 polls.
            // Preserve the header's len/cap/offset and read each live slot only
            // after source907, so later map replacement cannot rebind scratch.
            let fresh_sets;
            let (sets, mut filters) = if let Some(slot) = cached_owner_obstructions {
                let replacements = (node_replacement, adjacent_replacement);
                if slot.as_ref().is_none_or(|cached| {
                    cached.arena != frame.source_arena()
                        || cached.view != frame.mapping_view()
                        || cached.replacements != replacements
                }) {
                    let sets = frame
                        .capture_obstructions(node, node_replacement, adjacent_replacement)
                        .map_err(owner_score_error)?;
                    let filters = vec![
                        OwnerObstructionFilter::Unknown;
                        sets.iter().map(|set| set.len()).sum()
                    ];
                    *slot = Some(SizedOwnerObstructions {
                        arena: frame.source_arena(),
                        view: frame.mapping_view(),
                        replacements,
                        sets,
                        filters,
                    });
                }
                let cached = slot.as_mut().expect("captured obstruction headers");
                (&cached.sets, Some(cached.filters.as_mut_slice()))
            } else {
                fresh_sets = frame
                    .capture_obstructions(node, node_replacement, adjacent_replacement)
                    .map_err(owner_score_error)?;
                (&fresh_sets, None)
            };
            #[cfg(feature = "diagnostic-traces")]
            context.observe_owner_obstruction_inventory(
                frame,
                node,
                node_replacement,
                adjacent_replacement,
                sets,
            );
            // Both replacement ancestry walks depend only on live vessel
            // ancestry, which can change only at a poll. Record each once
            // after an item poll, when first needed, and replay it for the
            // following items.
            let mut node_chain = super::owner_score_frame::OwnerAncestryChain::unrecorded();
            let mut adjacent_chain = super::owner_score_frame::OwnerAncestryChain::unrecorded();
            let mut replacement_graph_cache = None;
            let mut filter_offset = 0;
            'owner_sets: for (set_index, set) in sets.iter().enumerate() {
                let set_filter_offset = filter_offset;
                filter_offset += set.len();
                #[cfg(feature = "diagnostic-traces")]
                context.observe_obstruction_set(set_index);
                score_interval!(context, self, "node-obstruction-set", Some(node), set_index)?;
                for (index, slot) in set.slot_readers().readers().enumerate() {
                    score_interval!(context, self, "node-obstruction", Some(node), index)?;
                    if index % 64 == 0 {
                        node_chain.invalidate();
                        adjacent_chain.invalidate();
                        // The replacement's graph is read from the same live
                        // topology as the chains; refresh it on their schedule.
                        replacement_graph_cache = None;
                    }
                    let obstruction = slot.get();
                    let geometry = obstruction
                        .map(|node| frame.position_and_size(node))
                        .transpose()
                        .map_err(owner_score_error)?;
                    let position = geometry.and_then(|(position, _)| position);
                    let size = geometry.map(|(_, size)| size);
                    if let (Some(position), Some(size)) = (position, size) {
                        if obstruction_excluded(position, size) {
                            continue;
                        }
                    }
                    // Source914: nil is a universal ancestor, so a genuine nil
                    // child slot is polled and skipped before its Box is read.
                    let mut filter = filters
                        .as_deref_mut()
                        .map(|filters| &mut filters[set_filter_offset + index]);
                    let ancestor = match filter.as_deref() {
                        Some(OwnerObstructionFilter::Ancestor) => true,
                        Some(
                            OwnerObstructionFilter::Other
                            | OwnerObstructionFilter::Excluded
                            | OwnerObstructionFilter::Eligible,
                        ) => false,
                        _ => {
                            frame
                                .chain_is_descendant_of(
                                    frame.record_ancestry_chain(
                                        &mut node_chain,
                                        Some(node_replacement),
                                    ),
                                    obstruction,
                                )
                                .map_err(owner_score_error)?
                                || frame
                                    .chain_is_descendant_of(
                                        frame.record_ancestry_chain(
                                            &mut adjacent_chain,
                                            Some(adjacent_replacement),
                                        ),
                                        obstruction,
                                    )
                                    .map_err(owner_score_error)?
                        }
                    };
                    if ancestor {
                        if let Some(filter) = filter {
                            *filter = OwnerObstructionFilter::Ancestor;
                        }
                        continue;
                    }
                    if let Some(filter) = filter.as_deref_mut() {
                        if matches!(filter, OwnerObstructionFilter::Unknown) {
                            *filter = OwnerObstructionFilter::Other;
                        }
                    }
                    let (Some(obstruction), Some(position), Some(size)) =
                        (obstruction, position, size)
                    else {
                        continue;
                    };
                    let eligible = match filter.as_deref() {
                        Some(OwnerObstructionFilter::Excluded) => false,
                        Some(OwnerObstructionFilter::Eligible) => true,
                        _ => {
                            let obstruction_graph =
                                frame.graph(obstruction).map_err(owner_score_error)?;
                            let replacement_graph = match replacement_graph_cache {
                                Some(graph) => graph,
                                None => {
                                    let graph =
                                        frame.graph(node_replacement).map_err(owner_score_error)?;
                                    replacement_graph_cache = Some(graph);
                                    graph
                                }
                            };
                            let same_graph = match (obstruction_graph, replacement_graph) {
                                (None, None) => true,
                                (Some(left), Some(right)) => std::ptr::eq(left, right),
                                _ => false,
                            };
                            if !same_graph {
                                if let Some(filter) = filter {
                                    *filter = OwnerObstructionFilter::Excluded;
                                }
                                continue;
                            }
                            // All three handles were validated above (the obstruction's
                            // Box read and both replacement ancestry walks), and
                            // distinct canonical ids are distinct nodes.
                            if obstruction == node_replacement
                                || obstruction == adjacent_replacement
                            {
                                if let Some(filter) = filter {
                                    *filter = OwnerObstructionFilter::Excluded;
                                }
                                continue;
                            }
                            if frame
                                .is_endpoint_cluster_obstruction(
                                    obstruction,
                                    node_replacement,
                                    adjacent_replacement,
                                )
                                .map_err(owner_score_error)?
                            {
                                if let Some(filter) = filter {
                                    *filter = OwnerObstructionFilter::Excluded;
                                }
                                continue;
                            }
                            // checkBest551 and checkAlternate604 additionally reject
                            // descendants of either actual replacement endpoint.
                            if frame
                                .is_descendant_of(Some(obstruction), Some(node_replacement))
                                .map_err(owner_score_error)?
                                || frame
                                    .is_descendant_of(Some(obstruction), Some(adjacent_replacement))
                                    .map_err(owner_score_error)?
                            {
                                if let Some(filter) = filter {
                                    *filter = OwnerObstructionFilter::Excluded;
                                }
                                continue;
                            }
                            true
                        }
                    };
                    if !eligible {
                        continue;
                    }
                    if let Some(filter) = filter {
                        *filter = OwnerObstructionFilter::Eligible;
                    }
                    let (corner_a, corner_b, direct, alternate_a, alternate_b) =
                        inspect_obstruction_box!(position, size);
                    corner_a_blocked |= corner_a;
                    corner_b_blocked |= corner_b;
                    direct_blocked |= direct;
                    if direct_blocked && semi_diagonal {
                        alternate_a_blocked |= alternate_a;
                        alternate_b_blocked |= alternate_b;
                        alternate_route_blocked = alternate_a_blocked && alternate_b_blocked;
                    }
                    if (diagonal && corner_a_blocked && corner_b_blocked)
                        || (!diagonal && direct_blocked && alternate_route_blocked)
                    {
                        break 'owner_sets;
                    }
                }
            }
        }
        if owner_operands.is_none()
            && projected_obstructions.is_none()
            && full_obstruction_inventory.is_none()
        {
            let obstruction_sets = if let Some(obstructions) = cached_obstructions {
                vec![obstructions.to_vec()]
            } else {
                self.scoring_obstruction_sets(node, adjacent, tree_children_restored)
            };
            'sets: for (set_index, obstructions) in obstruction_sets.iter().enumerate() {
                #[cfg(feature = "diagnostic-traces")]
                context.observe_obstruction_set(set_index);
                score_interval!(context, self, "node-obstruction-set", Some(node), set_index)?;
                for (obstruction_index, obstruction) in obstructions.iter().copied().enumerate() {
                    score_interval!(
                        context,
                        self,
                        "node-obstruction",
                        Some(node),
                        obstruction_index
                    )?;
                    if self
                        .active_node_position(obstruction)
                        .is_some_and(|position| {
                            obstruction_excluded(position, self.active_node_size(obstruction))
                        })
                    {
                        continue;
                    }
                    let (corner_a, corner_b, direct, alternate_a, alternate_b) =
                        inspect_obstruction!(obstruction);
                    if super::sized::trace_sized_graph_scope(self)
                        && (crate::internal::trace_env_value("TALA_TRACE_SIZED_PASS").is_none()
                            || crate::internal::trace_env_enabled("TALA_TRACE_SIZED_PASS_ACTIVE"))
                        && crate::internal::trace_env_value("TALA_TRACE_ROUTE_OBSTRUCTIONS_NODE")
                            .and_then(|value| value.parse::<i64>().ok())
                            == Some(self.nodes[node.0 as usize].tala_id)
                        && crate::internal::trace_env_value(
                            "TALA_TRACE_ROUTE_OBSTRUCTIONS_POSITION",
                        )
                        .is_none_or(|target| {
                            let mut parts = target.split(',');
                            let Some(x) = parts.next().and_then(|value| value.parse::<f64>().ok())
                            else {
                                return false;
                            };
                            let Some(y) = parts.next().and_then(|value| value.parse::<f64>().ok())
                            else {
                                return false;
                            };
                            self.position(node)
                                .is_some_and(|position| position.x == x && position.y == y)
                        })
                    {
                        let position = self.position(node).unwrap();
                        let obstruction_node = &self.nodes[obstruction.0 as usize];
                        eprintln!(
                            "ROUTE_OBSTRUCTION_RUST node={} pos={},{} edge={}>{} replacement={}>{} obstruction={} projected=false obstructionPos={:?} obstructionSize={:?} route={},{}>{},{} diagonal={} semi={} cornerA={corner_a} cornerB={corner_b} direct={direct} alternateA={alternate_a} alternateB={alternate_b}",
                            self.nodes[node.0 as usize].tala_id,
                            position.x,
                            position.y,
                            self.nodes[self.edge_records[edge.0 as usize].from.0 as usize].tala_id,
                            self.nodes[self.edge_records[edge.0 as usize].to.0 as usize].tala_id,
                            node_tala_id,
                            adjacent_tala_id,
                            obstruction_node.tala_id,
                            self.active_node_position(obstruction),
                            self.active_node_size(obstruction),
                            start.x,
                            start.y,
                            end.x,
                            end.y,
                            diagonal,
                            semi_diagonal,
                        );
                    }
                    corner_a_blocked |= corner_a;
                    corner_b_blocked |= corner_b;
                    direct_blocked |= direct;
                    if direct_blocked && semi_diagonal {
                        alternate_a_blocked |= alternate_a;
                        alternate_b_blocked |= alternate_b;
                        alternate_route_blocked = alternate_a_blocked && alternate_b_blocked;
                    }
                    if (diagonal && corner_a_blocked && corner_b_blocked)
                        || (!diagonal && direct_blocked && alternate_route_blocked)
                    {
                        break 'sets;
                    }
                }
            }
        }
        if let Some(inventory) = full_obstruction_inventory {
            #[cfg(feature = "diagnostic-traces")]
            if trace_route_obstruction_inventory {
                eprintln!(
                    "ROUTE_FULL_OBSTRUCTION_INVENTORY_RUST node={} adjacent={} edge={} inventory={inventory:?}",
                    self.nodes[node.0 as usize].tala_id,
                    self.nodes[adjacent.0 as usize].tala_id,
                    edge.0
                );
            }
            #[cfg(feature = "diagnostic-traces")]
            context.observe_obstruction_inventory(self, node, inventory);
            'full_sets: for (set_index, set) in inventory.sets.iter().enumerate() {
                #[cfg(feature = "diagnostic-traces")]
                context.observe_obstruction_set(set_index);
                score_interval!(context, self, "node-obstruction-set", Some(node), set_index)?;
                for (index, obstruction) in set.items.iter().flatten().enumerate() {
                    score_interval!(context, self, "node-obstruction", Some(node), index)?;
                    let (position, size) = self.projected_obstruction_box(obstruction);
                    if position.is_some_and(|position| obstruction_excluded(position, size)) {
                        continue;
                    }
                    if self.projected_obstruction_replacement_ancestor(obstruction, inventory) {
                        continue;
                    }
                    let Some(position) = position else {
                        continue;
                    };
                    if !self.projected_obstruction_same_graph(obstruction, inventory) {
                        continue;
                    }
                    if obstruction.is_endpoint {
                        continue;
                    }
                    if obstruction.is_cluster_vessel && obstruction.is_endpoint_cluster_vessel {
                        continue;
                    }
                    let (corner_a, corner_b, direct, alternate_a, alternate_b) =
                        inspect_obstruction_box!(position, size);
                    corner_a_blocked |= corner_a;
                    corner_b_blocked |= corner_b;
                    direct_blocked |= direct;
                    if direct_blocked && semi_diagonal {
                        alternate_a_blocked |= alternate_a;
                        alternate_b_blocked |= alternate_b;
                        alternate_route_blocked = alternate_a_blocked && alternate_b_blocked;
                    }
                    if (diagonal && corner_a_blocked && corner_b_blocked)
                        || (!diagonal && direct_blocked && alternate_route_blocked)
                    {
                        break 'full_sets;
                    }
                }
            }
        }
        let mut projected_set = 0;
        let mut projected_index = 0;
        let mut previous_scope = None;
        for projected in projected_obstructions
            .filter(|_| full_obstruction_inventory.is_none())
            .into_iter()
            .flatten()
            .copied()
        {
            if projected_index == 0 || previous_scope != Some(projected.container_tala_id) {
                if projected_index != 0 {
                    projected_set += 1;
                }
                projected_index = 0;
                previous_scope = Some(projected.container_tala_id);
                #[cfg(feature = "diagnostic-traces")]
                context.observe_obstruction_set(projected_set);
                score_interval!(
                    context,
                    self,
                    "node-obstruction-set",
                    Some(node),
                    projected_set
                )?;
            }
            score_interval!(
                context,
                self,
                "node-obstruction",
                Some(node),
                projected_index
            )?;
            projected_index += 1;
            if projected.tala_id == node_tala_id || projected.tala_id == adjacent_tala_id {
                continue;
            }
            // A projected obstruction whose identity is its live owner is a
            // carrier node in the temporary graph.  Go's induced graph keeps
            // that node pointer live while a sibling candidate moves, so its
            // current box is authoritative; the stored offset belongs to the
            // source-to-owner projection and would apply the ancestor frame a
            // second time.  Retained members and aggregate vessels continue
            // through the full projection resolver below.
            let live_owner_box = (!projected.cluster_member
                && self.nodes[projected.owner.0 as usize].tala_id == projected.tala_id)
                .then(|| {
                    Some((
                        self.position(projected.owner)?,
                        self.active_node_size(projected.owner),
                    ))
                })
                .flatten();
            let Some((obstruction_position, obstruction_size)) =
                live_owner_box.or_else(|| self.sized_projected_box(projected))
            else {
                continue;
            };
            if obstruction_excluded(obstruction_position, obstruction_size) {
                continue;
            }
            let (corner_a, corner_b, direct, alternate_a, alternate_b) =
                inspect_obstruction_box!(obstruction_position, obstruction_size);
            if super::sized::trace_sized_graph_scope(self)
                && (crate::internal::trace_env_value("TALA_TRACE_SIZED_PASS").is_none()
                    || crate::internal::trace_env_enabled("TALA_TRACE_SIZED_PASS_ACTIVE"))
                && crate::internal::trace_env_value("TALA_TRACE_ROUTE_OBSTRUCTIONS_NODE")
                    .and_then(|value| value.parse::<i64>().ok())
                    == Some(self.nodes[node.0 as usize].tala_id)
                && crate::internal::trace_env_value("TALA_TRACE_ROUTE_OBSTRUCTIONS_POSITION")
                    .is_none_or(|target| {
                        let mut parts = target.split(',');
                        let Some(x) = parts.next().and_then(|value| value.parse::<f64>().ok())
                        else {
                            return false;
                        };
                        let Some(y) = parts.next().and_then(|value| value.parse::<f64>().ok())
                        else {
                            return false;
                        };
                        self.position(node)
                            .is_some_and(|position| position.x == x && position.y == y)
                    })
            {
                let position = self.position(node).unwrap();
                eprintln!(
                    "ROUTE_OBSTRUCTION_RUST node={} pos={},{} edge={}>{} replacement={}>{} obstruction={} owner={} clusterMember={} projected=true obstructionPos={},{} obstructionSize={},{} actualPos={:?} actualSize={:?} route={},{}>{},{} diagonal={} semi={} cornerA={corner_a} cornerB={corner_b} direct={direct} alternateA={alternate_a} alternateB={alternate_b}",
                    self.nodes[node.0 as usize].tala_id,
                    position.x,
                    position.y,
                    self.nodes[self.edge_records[edge.0 as usize].from.0 as usize].tala_id,
                    self.nodes[self.edge_records[edge.0 as usize].to.0 as usize].tala_id,
                    node_tala_id,
                    adjacent_tala_id,
                    projected.tala_id,
                    self.nodes[projected.owner.0 as usize].tala_id,
                    projected.cluster_member,
                    obstruction_position.x,
                    obstruction_position.y,
                    obstruction_size.width,
                    obstruction_size.height,
                    self.position(projected.owner),
                    self.active_node_size(projected.owner),
                    start.x,
                    start.y,
                    end.x,
                    end.y,
                    diagonal,
                    semi_diagonal
                );
            }
            corner_a_blocked |= corner_a;
            corner_b_blocked |= corner_b;
            direct_blocked |= direct;
            if direct_blocked && semi_diagonal {
                alternate_a_blocked |= alternate_a;
                alternate_b_blocked |= alternate_b;
                alternate_route_blocked = alternate_a_blocked && alternate_b_blocked;
            }
            if (diagonal && corner_a_blocked && corner_b_blocked)
                || (!diagonal && direct_blocked && alternate_route_blocked)
            {
                break;
            }
        }

        Ok(if diagonal && corner_a_blocked && corner_b_blocked {
            self.scoring_turn_cost_with_owner_frame_checked(
                node,
                owner_operands.map(|(frame, _, _)| frame),
            )?
        } else if !diagonal && direct_blocked {
            if semi_diagonal && !alternate_route_blocked {
                self.scoring_turn_cost_with_owner_frame_checked(
                    node,
                    owner_operands.map(|(frame, _, _)| frame),
                )?
            } else {
                self.scoring_turn_cost_with_owner_frame_checked(
                    node,
                    owner_operands.map(|(frame, _, _)| frame),
                )? * 2.0
            }
        } else {
            0.0
        })
    }

    // Represented subset of recovered Node.getDeltaTo. Edge MinWidth and
    // MinHeight include the main and both arrowhead labels as recovered in
    // d2transpiler.go. Directional margins are enabled on the recovered
    // CombineSubgraphs surface below.
    pub(super) fn spacing_delta(&self, node: NodeId, other: NodeId, proposed: Point) -> f64 {
        let active_other = self.active_aggregate_owner(other);
        // Ordinary nodes retain the recovered Node.Edges membership for the
        // whole BinPack transaction.  Avoid rebuilding and sorting a Vec on
        // every overlap candidate; aggregate vessels still take the dynamic
        // active_edge_ids path because AbductEdges changes their endpoint
        // inventory.  The reduction is source-equivalent: only connectivity
        // and the maximum edge dimensions are consumed here, never order.
        let (connected, max_edge_width, max_edge_height) =
            self.spacing_edge_metrics(node, active_other);
        let first = &self.nodes[node.0 as usize];
        let second = &self.nodes[other.0 as usize];
        let table = first.shape == ShapeKind::SqlTable || second.shape == ShapeKind::SqlTable;
        // node.go deltaToGuarded keeps the incident scan above, then returns
        // an isotropic gap before reading geometry when loops and margins
        // cannot affect it. Compaction's numerical view has the same branch.
        if first.loop_offsets.as_ref().is_none_or(|map| map.is_empty())
            && second
                .loop_offsets
                .as_ref()
                .is_none_or(|map| map.is_empty())
            && first.layout_margins == Insets::uniform(0.0)
            && second.layout_margins == Insets::uniform(0.0)
        {
            let (horizontal, vertical) =
                Self::spacing_axis_deltas(table, (connected, max_edge_width, max_edge_height));
            if horizontal == vertical {
                return horizontal;
            }
        }
        self.spacing_delta_for_geometry(
            proposed,
            self.active_node_size(node),
            self.active_node_position(other).unwrap(),
            self.active_node_size(other),
            table,
            first.layout_margins,
            second.layout_margins,
            (connected, max_edge_width, max_edge_height),
        )
    }

    #[inline]
    fn spacing_axis_deltas(
        table: bool,
        (connected, max_edge_width, max_edge_height): (bool, f64, f64),
    ) -> (f64, f64) {
        let mut horizontal: f64 = if connected { 60.0 } else { 20.0 };
        let vertical: f64 = if connected { 60.0 } else { 20.0 };
        if table {
            horizontal = 120.0;
        }
        (
            horizontal.max(max_edge_width),
            vertical.max(max_edge_height),
        )
    }

    pub(super) fn spacing_delta_for_geometry(
        &self,
        proposed: Point,
        node_size: Size,
        other_position: Point,
        other_size: Size,
        table: bool,
        node_margins: Insets,
        other_margins: Insets,
        (connected, max_edge_width, max_edge_height): (bool, f64, f64),
    ) -> f64 {
        let (mut horizontal, mut vertical) =
            Self::spacing_axis_deltas(table, (connected, max_edge_width, max_edge_height));
        let orientation =
            self.sized_box_orientation((proposed, node_size), (other_position, other_size));
        // Recovered Node.getDeltaTo indexes the first node's label margin by
        // the opposite orientation and the second node's margin by the direct
        // orientation. Each source float is converted to int before the pair
        // is summed.
        let directional_margin = |margins: Insets, side: Orientation| match side {
            Orientation::TopLeft => (margins.left.trunc(), margins.top.trunc()),
            Orientation::Top => (0.0, margins.top.trunc()),
            Orientation::TopRight => (margins.right.trunc(), margins.top.trunc()),
            Orientation::Right => (margins.right.trunc(), 0.0),
            Orientation::BottomRight => (margins.right.trunc(), margins.bottom.trunc()),
            Orientation::Bottom => (0.0, margins.bottom.trunc()),
            Orientation::BottomLeft => (margins.left.trunc(), margins.bottom.trunc()),
            Orientation::Left => (margins.left.trunc(), 0.0),
            Orientation::None => (0.0, 0.0),
        };
        let (node_margin_width, node_margin_height) =
            directional_margin(node_margins, orientation.opposite());
        let (other_margin_width, other_margin_height) =
            directional_margin(other_margins, orientation);
        horizontal = horizontal.max(node_margin_width + other_margin_width);
        vertical = vertical.max(node_margin_height + other_margin_height);
        match orientation {
            Orientation::Top | Orientation::Bottom => vertical,
            Orientation::Right | Orientation::Left => horizontal,
            _ => horizontal.min(vertical),
        }
    }

    fn spacing_edge_metrics(&self, node: NodeId, active_other: NodeId) -> (bool, f64, f64) {
        let mut connected = false;
        let mut max_edge_width = f64::NEG_INFINITY;
        let mut max_edge_height = f64::NEG_INFINITY;
        // Keep the immutable arena header outside the visitor, while reading
        // each live incident slot and its current endpoints when reached.
        let edge_records = self.edge_records.as_slice();
        let aggregate =
            self.active_cluster_index(node).is_some() || self.active_sequence_index(node).is_some();
        if aggregate {
            for edge_id in self.active_edge_ids(node) {
                let edge = &edge_records[edge_id.0 as usize];
                if self.active_adjacent_for_edge(node, edge) != active_other {
                    continue;
                }
                connected = true;
                max_edge_width = max_edge_width.max(edge.min_width);
                max_edge_height = max_edge_height.max(edge.min_height);
            }
        } else {
            self.nodes[node.0 as usize].edges.with_header(|edges| {
                edges.for_each_value(|&edge_id| {
                    // node.go deltaToGuarded retains e through adjacent and
                    // the dimension reduction. Borrow the arena record once.
                    let edge = &edge_records[edge_id.0 as usize];
                    if self.active_adjacent_for_edge(node, edge) != active_other {
                        return;
                    }
                    connected = true;
                    max_edge_width = max_edge_width.max(edge.min_width);
                    max_edge_height = max_edge_height.max(edge.min_height);
                });
            });
        }
        (connected, max_edge_width, max_edge_height)
    }

    /// `CombineSubgraphs` compares nodes from disconnected components, so its
    /// recovered `getDeltaTo` base is 20 on both axes (120 horizontally for
    /// tables) plus the two facing directional margins.
    pub(super) fn combine_subgraph_spacing_delta(
        &self,
        node: NodeId,
        other: NodeId,
        proposed: Point,
    ) -> f64 {
        let node_size = self.nodes[node.0 as usize].rect.size;
        let other_position = self.position(other).unwrap();
        let other_size = self.nodes[other.0 as usize].rect.size;
        let orientation =
            self.sized_box_orientation((proposed, node_size), (other_position, other_size));
        let mut horizontal: f64 =
            if matches!(self.nodes[node.0 as usize].shape, ShapeKind::SqlTable)
                || matches!(self.nodes[other.0 as usize].shape, ShapeKind::SqlTable)
            {
                120.0
            } else {
                20.0
            };
        let mut vertical: f64 = 20.0;
        let directional_margin = |margins: Insets, side: Orientation| match side {
            Orientation::TopLeft => (margins.left, margins.top),
            Orientation::Top => (0.0, margins.top),
            Orientation::TopRight => (margins.right, margins.top),
            Orientation::Right => (margins.right, 0.0),
            Orientation::BottomRight => (margins.right, margins.bottom),
            Orientation::Bottom => (0.0, margins.bottom),
            Orientation::BottomLeft => (margins.left, margins.bottom),
            Orientation::Left => (margins.left, 0.0),
            Orientation::None => (0.0, 0.0),
        };
        let (node_margin_width, node_margin_height) = directional_margin(
            self.nodes[node.0 as usize].layout_margins,
            orientation.opposite(),
        );
        let (other_margin_width, other_margin_height) =
            directional_margin(self.nodes[other.0 as usize].layout_margins, orientation);
        horizontal = horizontal.max(node_margin_width + other_margin_width);
        vertical = vertical.max(node_margin_height + other_margin_height);
        match orientation {
            Orientation::Top | Orientation::Bottom => vertical,
            Orientation::Right | Orientation::Left => horizontal,
            _ => horizontal.min(vertical),
        }
    }

    pub(super) fn spacing_delta_with_loops(
        &self,
        node: NodeId,
        other: NodeId,
        proposed: Point,
    ) -> f64 {
        let delta = self.spacing_delta(node, other, proposed);
        // node.go deltaToGuarded reads loop offsets directly. An empty map
        // cannot contribute a gap, so resolve aggregate membership only for
        // a nonempty member map that a temporary vessel must suppress.
        let node_loop_extents = self.nodes[node.0 as usize]
            .loop_offsets
            .as_ref()
            .filter(|map| !map.is_empty())
            .filter(|_| !self.active_node_is_aggregate(node));
        let other_loop_extents = self.nodes[other.0 as usize]
            .loop_offsets
            .as_ref()
            .filter(|map| !map.is_empty())
            .filter(|_| !self.active_node_is_aggregate(other));
        if node_loop_extents.is_none() && other_loop_extents.is_none() {
            return delta;
        }
        Self::spacing_delta_with_loop_geometry(
            delta,
            proposed,
            self.active_node_size(node),
            self.active_node_position(other).unwrap(),
            self.active_node_size(other),
            node_loop_extents,
            other_loop_extents,
        )
    }

    pub(super) fn spacing_delta_with_loop_geometry(
        mut delta: f64,
        proposed: Point,
        node_size: Size,
        other_position: Point,
        other_size: Size,
        node_loop_extents: Option<&LoopOffsetsMap>,
        other_loop_extents: Option<&LoopOffsetsMap>,
    ) -> f64 {
        let has_loop_extent =
            |extents: Option<&LoopOffsetsMap>| extents.is_some_and(|values| !values.is_empty());
        if !has_loop_extent(node_loop_extents) && !has_loop_extent(other_loop_extents) {
            return delta;
        }
        let orientation = if proposed.y + node_size.height < other_position.y {
            if proposed.x + node_size.width < other_position.x {
                Orientation::TopLeft
            } else if other_position.x + other_size.width < proposed.x {
                Orientation::TopRight
            } else {
                Orientation::Top
            }
        } else if other_position.y + other_size.height < proposed.y {
            if proposed.x + node_size.width < other_position.x {
                Orientation::BottomLeft
            } else if other_position.x + other_size.width < proposed.x {
                Orientation::BottomRight
            } else {
                Orientation::Bottom
            }
        } else if other_position.x + other_size.width < proposed.x {
            Orientation::Right
        } else if proposed.x + node_size.width < other_position.x {
            Orientation::Left
        } else {
            Orientation::None
        };
        delta = delta.max(loop_spacing_gap(
            node_loop_extents,
            other_loop_extents,
            orientation,
        ));
        delta
    }

    pub(super) fn sized_edge_length(&self, node: NodeId, direction_penalty: bool) -> f64 {
        self.sized_edge_length_with_obstructions(node, direction_penalty, None)
    }

    pub(super) fn sized_edge_length_with_obstructions(
        &self,
        node: NodeId,
        direction_penalty: bool,
        _sorted_obstructions: Option<(&[NodeId], f64)>,
    ) -> f64 {
        self.sized_edge_length_with_cache(node, direction_penalty, None)
    }

    pub(super) fn sized_edge_length_with_cache(
        &self,
        node: NodeId,
        direction_penalty: bool,
        obstruction_cache: Option<&[Vec<NodeId>]>,
    ) -> f64 {
        self.sized_edge_length_with_cache_mode(
            node,
            direction_penalty,
            obstruction_cache,
            false,
            false,
            false,
        )
    }

    /// Replays the first phase of recovered `Node.edgeLength`.
    ///
    /// TALA walks `node.Edges` and, for each edge, consumes the first unused
    /// `EdgeAbduction` whose *current endpoint pair* matches. It deliberately
    /// does not compare `EdgeAbduction.Edge`, so this cannot be represented by
    /// a `(node, edge)` lookup when parallel edges share a carrier pair.
    fn sized_restored_endpoints(
        &self,
        node: NodeId,
        active_edges: &[EdgeId],
        restore_sequence_endpoints: bool,
    ) -> Option<ScoringScratchVec<SizedRestoredEndpoints>> {
        self.sized_restored_endpoints_checked(
            node,
            active_edges,
            restore_sequence_endpoints,
            ScoreContext::none(),
        )
        .expect("unchecked scoring context cannot cancel")
    }

    fn sized_restored_endpoints_checked(
        &self,
        node: NodeId,
        active_edges: &[EdgeId],
        restore_sequence_endpoints: bool,
        context: ScoreContext<'_>,
    ) -> Result<Option<ScoringScratchVec<SizedRestoredEndpoints>>, limits::Error> {
        self.sized_restored_endpoints_with_owner_frame_checked(
            node,
            active_edges,
            restore_sequence_endpoints,
            None,
            context,
        )
    }

    fn sized_restored_endpoints_with_owner_frame_checked(
        &self,
        node: NodeId,
        active_edges: &[EdgeId],
        restore_sequence_endpoints: bool,
        owner_frame: Option<&OwnerScoreFrame<'_>>,
        context: ScoreContext<'_>,
    ) -> Result<Option<ScoringScratchVec<SizedRestoredEndpoints>>, limits::Error> {
        // Source prepareNodeEdgeLength gathers Cluster.EdgeAbductions only
        // when the explicit slice is nil, sorting the vessel pointers by ID.
        // Retain record identities: each evaluation reads their live fields.
        let canonical = |local| {
            self.active_cluster_index(local)
                .and_then(|index| self.topology_vessel_node(TopologyVessel::Cluster(index)))
                .or_else(|| {
                    self.active_sequence_index(local).and_then(|index| {
                        self.topology_vessel_node(TopologyVessel::Sequence(index))
                    })
                })
                .unwrap_or(local)
        };
        let mut cluster_records = None;
        if owner_frame.is_none()
            && !self.suppress_aggregate_projection
            && !self.sized_edge_abductions_present
            && self.sized_edge_abductions.is_empty()
            && let Some(receiver) = self.topology_node_owner(canonical(node))
        {
            let kind = super::owner_groups::OwnerGroupKind::Cluster;
            let map = self
                .owner_graph_group_map(receiver, kind)
                .map_err(owner_score_error)?;
            let mut vessels = self.owner_group_map_keys(&map).map_err(owner_score_error)?;
            super::go_sort::sort_by(&mut vessels, |a, b| {
                self.nodes[a.expect("nil cluster vessel").0 as usize].tala_id
                    < self.nodes[b.expect("nil cluster vessel").0 as usize].tala_id
            });
            if !vessels.is_empty() {
                let mut records = Vec::new();
                for (index, vessel) in vessels.into_iter().enumerate() {
                    score_interval!(
                        context,
                        self,
                        "node-prepare-cluster-abductions",
                        Some(node),
                        index
                    )?;
                    let group = self
                        .owner_graph_group_entry(receiver, kind, vessel)
                        .map_err(owner_score_error)?
                        .flatten()
                        .expect("runtime error: invalid memory address or nil pointer dereference");
                    if let Some(header) = &self
                        .read_owner_cluster_policy(group)
                        .map_err(owner_score_error)?
                        .edge_abductions
                    {
                        // Read each live slot through the borrowed header;
                        // nothing between those reads can call back.
                        records.extend(header.borrowed_slots());
                    }
                }
                cluster_records = Some(records);
            }
        }
        let mut matched = ScoringScratchVec::<bool>::take();
        matched.resize(
            cluster_records
                .as_ref()
                .map_or(self.sized_edge_abductions.len(), Vec::len),
            false,
        );
        // These are the three real scratch-slice boundaries in upstream
        // prepareNodeEdgeLength. Rust stores both replacement sides together;
        // their independent source cancellation boundaries remain distinct.
        for _index in (0..matched.len()).step_by(64) {
            score_poll!(
                context,
                self,
                "node-prepare-used-abductions",
                Some(node),
                Some(_index)
            )?;
        }
        for _index in (0..active_edges.len()).step_by(64) {
            score_poll!(
                context,
                self,
                "node-prepare-node-replacements",
                Some(node),
                Some(_index)
            )?;
        }
        for _index in (0..active_edges.len()).step_by(64) {
            score_poll!(
                context,
                self,
                "node-prepare-adjacent-replacements",
                Some(node),
                Some(_index)
            )?;
        }
        let no_aggregate_projection = self.suppress_aggregate_projection
            || (self.node_order_membership_valid
                && self.sized_adjacent_overrides.is_empty()
                && !self.active_sequence_flags.iter().any(|active| *active)
                && !self.active_cluster_flags.iter().any(|active| *active));
        let replacements_needed = cluster_records.is_some()
            || !self.sized_edge_abductions.is_empty()
            || !no_aggregate_projection;
        let mut replacements =
            replacements_needed.then(ScoringScratchVec::<SizedRestoredEndpoints>::take);
        for (edge_index, edge_id) in active_edges.iter().copied().enumerate() {
            score_interval!(context, self, "node-prepare-edge", Some(node), edge_index)?;
            let restored = (|| -> Result<SizedRestoredEndpoints, limits::Error> {
                let adjacent = self.active_adjacent(node, edge_id);
                let canonical_pair = owner_frame
                    .map(|frame| {
                        Ok::<_, limits::Error>((
                            frame.canonical_node(node).map_err(owner_score_error)?,
                            frame.canonical_node(adjacent).map_err(owner_score_error)?,
                        ))
                    })
                    .transpose()?;
                for (index, abduction) in self.sized_edge_abductions.iter().enumerate() {
                    score_interval!(context, self, "node-prepare-abduction", Some(node), index)?;
                    if matched[index] {
                        continue;
                    }
                    if restore_sequence_endpoints && !abduction.sequence_abduction {
                        continue;
                    }
                    if let Some((canonical_node, canonical_adjacent)) = canonical_pair {
                        let frame = owner_frame.expect("canonical pair retains frame");
                        let slice = frame
                            .abductions()
                            .expect("owner API validated ordered records");
                        let actual = frame
                            .owner()
                            .owner_edge_abduction(slice.records[index])
                            .expect("frame retains a validated live owner record");
                        let reversed = if actual.current_from == Some(canonical_node)
                            && actual.current_to == Some(canonical_adjacent)
                        {
                            false
                        } else if actual.current_from == Some(canonical_adjacent)
                            && actual.current_to == Some(canonical_node)
                        {
                            true
                        } else {
                            continue;
                        };
                        matched[index] = true;
                        return Ok(SizedRestoredEndpoints {
                            node: if reversed {
                                abduction.originally_to
                            } else {
                                abduction.originally_from
                            },
                            adjacent: if reversed {
                                abduction.originally_from
                            } else {
                                abduction.originally_to
                            },
                            abduction: Some((index, reversed)),
                        });
                    }
                    if abduction
                        .current_from
                        .and_then(|endpoint| endpoint.local_node())
                        == Some(node)
                        && abduction
                            .current_to
                            .and_then(|endpoint| endpoint.local_node())
                            == Some(adjacent)
                    {
                        matched[index] = true;
                        return Ok(SizedRestoredEndpoints {
                            node: abduction.originally_from,
                            adjacent: abduction.originally_to,
                            abduction: Some((index, false)),
                        });
                    }
                    if abduction
                        .current_from
                        .and_then(|endpoint| endpoint.local_node())
                        == Some(adjacent)
                        && abduction
                            .current_to
                            .and_then(|endpoint| endpoint.local_node())
                            == Some(node)
                    {
                        matched[index] = true;
                        return Ok(SizedRestoredEndpoints {
                            node: abduction.originally_to,
                            adjacent: abduction.originally_from,
                            abduction: Some((index, true)),
                        });
                    }
                }

                if !self.sized_edge_abductions.is_empty() {
                    return Ok(SizedRestoredEndpoints {
                        node: None,
                        adjacent: None,
                        abduction: None,
                    });
                }

                if self.sized_edge_abductions_present {
                    // A nonnil empty explicit slice suppresses nil-only
                    // cluster gathering, just like a nonempty explicit slice.
                    return Ok(SizedRestoredEndpoints {
                        node: None,
                        adjacent: None,
                        abduction: None,
                    });
                }
                if let Some(records) = &cluster_records {
                    let current_node = canonical(node);
                    let current_adjacent = canonical(adjacent);
                    for (index, &id) in records.iter().enumerate() {
                        score_interval!(
                            context,
                            self,
                            "node-prepare-abduction",
                            Some(node),
                            index
                        )?;
                        if matched[index] {
                            continue;
                        }
                        let record = self
                            .owner_edge_abduction(id.expect("nil cluster abduction"))
                            .expect("live cluster abduction");
                        let reversed = if record.current_from == Some(current_node)
                            && record.current_to == Some(current_adjacent)
                        {
                            false
                        } else if record.current_from == Some(current_adjacent)
                            && record.current_to == Some(current_node)
                        {
                            true
                        } else {
                            continue;
                        };
                        matched[index] = true;
                        let original = |source: Option<NodeId>, owner, source_owner| {
                            source.map(|source| AbductedNodeRef {
                                source_node: Some(source),
                                source_owner: Some(source_owner),
                                owner: Some(owner),
                                owner_tala_id: Some(self.nodes[source_owner.0 as usize].tala_id),
                                tala_id: self.nodes[source.0 as usize].tala_id,
                                container_tala_id: self
                                    .active_node_container(source)
                                    .map(|parent| self.nodes[parent.0 as usize].tala_id),
                                offset: None,
                                size: self.nodes[source.0 as usize].rect.size,
                                cluster_member: self.active_cluster_index(source).is_some(),
                            })
                        };
                        return Ok(SizedRestoredEndpoints {
                            node: original(
                                if reversed {
                                    record.originally_to
                                } else {
                                    record.originally_from
                                },
                                node,
                                current_node,
                            ),
                            adjacent: original(
                                if reversed {
                                    record.originally_from
                                } else {
                                    record.originally_to
                                },
                                adjacent,
                                current_adjacent,
                            ),
                            abduction: None,
                        });
                    }
                    return Ok(SizedRestoredEndpoints {
                        node: None,
                        adjacent: None,
                        abduction: None,
                    });
                }

                // Aggregate stages outside a materialized placement scope
                // still use the stable-arena projection. No ordered placement
                // abduction matched this incident edge, so preserve that
                // independently recovered behavior.
                if no_aggregate_projection {
                    return Ok(SizedRestoredEndpoints {
                        node: None,
                        adjacent: None,
                        abduction: None,
                    });
                }
                let (original_node, original_adjacent) =
                    self.active_edge_original_endpoints(node, edge_id);
                Ok(SizedRestoredEndpoints {
                    node: self
                        .sized_adjacent_overrides
                        .get(&(adjacent, edge_id))
                        .copied()
                        .or_else(|| {
                            self.active_aggregate_edge_endpoint_projection(
                                original_node,
                                original_adjacent,
                                restore_sequence_endpoints,
                            )
                        })
                        .map(AbductedNodeRef::from),
                    adjacent: self
                        .sized_adjacent_overrides
                        .get(&(node, edge_id))
                        .copied()
                        .or_else(|| {
                            self.active_aggregate_edge_endpoint_projection(
                                original_adjacent,
                                original_node,
                                restore_sequence_endpoints,
                            )
                        })
                        .map(AbductedNodeRef::from),
                    abduction: None,
                })
            })()?;
            if let Some(replacements) = replacements.as_mut() {
                replacements.push(restored);
            }
        }
        Ok(replacements)
    }

    fn sized_edge_length_direction(
        &self,
        node: NodeId,
        active_edges: &[EdgeId],
        restored_endpoints: Option<&[SizedRestoredEndpoints]>,
    ) -> (Orientation, f64, bool) {
        self.sized_edge_length_direction_checked(
            node,
            active_edges,
            restored_endpoints,
            true,
            ScoreContext::none(),
        )
        .expect("unchecked scoring context cannot cancel")
    }

    fn sized_edge_length_direction_checked(
        &self,
        node: NodeId,
        active_edges: &[EdgeId],
        restored_endpoints: Option<&[SizedRestoredEndpoints]>,
        direction_penalty: bool,
        context: ScoreContext<'_>,
    ) -> Result<(Orientation, f64, bool), limits::Error> {
        self.sized_edge_length_direction_with_identity_checked(
            node,
            active_edges,
            restored_endpoints,
            direction_penalty,
            context,
            None,
        )
    }

    fn sized_edge_length_direction_with_identity_checked(
        &self,
        node: NodeId,
        active_edges: &[EdgeId],
        restored_endpoints: Option<&[SizedRestoredEndpoints]>,
        direction_penalty: bool,
        context: ScoreContext<'_>,
        identity_frame: Option<&OwnerScoreFrame<'_>>,
    ) -> Result<(Orientation, f64, bool), limits::Error> {
        let direction_is_unset = self.container_direction_is_unset(node);
        // edge_length.go240–243 initializes the actual container direction,
        // but inference and its factor exist only when PenalizeDirection is true.
        if direction_is_unset && !direction_penalty {
            return Ok((Orientation::None, 0.0, true));
        }

        if !direction_is_unset {
            let container = self.nodes[node.0 as usize].container;
            let container_tala_id = self.nodes[node.0 as usize].scoring_container_parent;
            // CopyEntitiesFrom retains the owner's explicit direction map.
            // A projected node's stable parent can differ from its local
            // container (often None), so consult that identity first.
            let direction = self
                .scoring_directions_by_tala
                .get(&container_tala_id)
                .copied()
                .or_else(|| self.directions.get(&container).copied())
                .or_else(|| self.directions_by_tala.get(&container_tala_id).copied())
                .unwrap_or(Direction::Right);
            return Ok((
                layout_orientation(direction),
                if direction_penalty { 6.0 } else { 0.0 },
                false,
            ));
        }

        // node.go initializes an unset container direction to geo.NONE and
        // leaves directionFactor at zero.  A directed edge may still enter
        // the final direction branch, but with no requested direction its
        // recovered cost remains zero.  Do not substitute a compass default
        // here: that charges a spurious direction penalty during transpose
        // trials (and changes which rotation wins).
        let (mut direction, mut factor);
        if self.nodes[node.0 as usize].shape == ShapeKind::SqlTable {
            direction = Orientation::Right;
            factor = 0.5;
        } else {
            let mut label_counts = BTreeMap::<NodeId, usize>::new();
            let mut outgoing_label_counts = BTreeMap::<NodeId, usize>::new();
            let mut multi_label_node = None;
            for (edge_index, edge_id) in active_edges.iter().copied().enumerate() {
                if direction_penalty {
                    score_interval!(
                        context,
                        self,
                        "node-prepare-direction-edge",
                        Some(node),
                        edge_index
                    )?;
                }
                // Recovered node.go:3117..3126 counts a label only when both
                // endpoint replacements are still the current carrier
                // pointers. An abducted labeled edge must not manufacture the
                // strong multi-label direction preference.
                let restored = restored_endpoints
                    .and_then(|endpoints| endpoints.get(edge_index))
                    .copied()
                    .unwrap_or_default();
                let adjacent = self.active_adjacent(node, edge_id);
                let differs = |original: Option<AbductedNodeRef>,
                               local: NodeId|
                 -> Result<bool, limits::Error> {
                    let Some(original) = original else {
                        return Ok(false);
                    };
                    let Some(frame) = identity_frame else {
                        return Ok(true);
                    };
                    // edge_length.go270–276 compares pointers. A retained
                    // original can be exactly the carrier, even when nonnil.
                    // Canonical NodeIds preserve that identity independently
                    // of authored IDs and temporary dense numbering.
                    Ok(original.source_node
                        != Some(frame.canonical_node(local).map_err(owner_score_error)?))
                };
                if differs(restored.node, node)? || differs(restored.adjacent, adjacent)? {
                    continue;
                }
                let edge = &self.edge_records[edge_id.0 as usize];
                if edge.label.is_none() {
                    continue;
                }
                let adjacent = self.active_adjacent(node, edge_id);
                if adjacent == node {
                    continue;
                }
                let count = label_counts.entry(adjacent).or_default();
                *count += 1;
                // Match DirectedEndpoints for directed edges, while the
                // two undirected forms keep the declaration's From.
                let semantic_from = if edge.source_arrow && !edge.target_arrow {
                    edge.to
                } else {
                    edge.from
                };
                if self.active_aggregate_owner(semantic_from) == node {
                    *outgoing_label_counts.entry(adjacent).or_default() += 1;
                }
                if *count > 2 {
                    multi_label_node = Some(adjacent);
                }
            }
            if let Some(multi_label_node) = multi_label_node {
                let label_count = label_counts[&multi_label_node];
                direction = if outgoing_label_counts
                    .get(&multi_label_node)
                    .copied()
                    .unwrap_or(0)
                    > label_count / 2
                {
                    Orientation::Left
                } else {
                    Orientation::Right
                };
                factor = 10.0;
            } else {
                // Recovered node.go:3149-3151 supplies a neutral diagonal
                // preference even when the unset container has no repeated
                // labels.  This is still consumed for directed edges: the
                // direction term is gated by the edge (or outer container),
                // not by `GetContainerDirection` being non-NONE.  Leaving
                // this as NONE/zero drops the 0.3 direction cost and changes
                // sized-optimizer choices in otherwise ordinary scopes.
                direction = Orientation::BottomRight;
                factor = 0.3;
            }
        }

        if let Some(arrangement) = self
            .active_cluster_index(node)
            .filter(|cluster| self.clusters[*cluster].members.first() == Some(&node))
            .map(|cluster| self.clusters[cluster].arrangement)
            .or(self.nodes[node.0 as usize].scoring_cluster_arrangement)
        {
            direction = match arrangement {
                ClusterArrangement::Row => Orientation::Bottom,
                ClusterArrangement::Column => Orientation::Right,
            };
            factor = 0.2;
        }

        Ok((direction, factor, true))
    }

    /// edge_length.go241–319 prepares direction from the actual scoring
    /// pointer and its incident/replacement pointers. Dense direction maps
    /// are projections and do not supply operands for a borrowed owner frame.
    fn sized_edge_length_owner_direction_checked(
        &self,
        node: NodeId,
        active_edges: &[EdgeId],
        restored_endpoints: Option<&[SizedRestoredEndpoints]>,
        direction_penalty: bool,
        frame: &OwnerScoreFrame<'_>,
        context: ScoreContext<'_>,
    ) -> Result<(Orientation, f64, bool), limits::Error> {
        let canonical = frame.canonical_node(node).map_err(owner_score_error)?;
        let actual = frame.node(canonical).map_err(owner_score_error)?;
        let direction = frame
            .container_direction(canonical)
            .map_err(owner_score_error)?;
        if !direction_penalty {
            return Ok((direction, 0.0, direction == Orientation::None));
        }
        if direction != Orientation::None {
            return Ok((direction, 6.0, false));
        }
        let finish =
            |mut direction, mut factor| -> Result<(Orientation, f64, bool), limits::Error> {
                if frame.owner().owner_is_cluster_vessel(canonical) {
                    let receiver = frame
                        .graph_id(canonical)
                        .map_err(owner_score_error)?
                        .expect("runtime error: invalid memory address or nil pointer dereference");
                    let group = frame
                        .owner()
                        .owner_graph_group_entry(
                            receiver,
                            super::owner_groups::OwnerGroupKind::Cluster,
                            Some(canonical),
                        )
                        .map_err(owner_score_error)?
                        .flatten()
                        .expect("runtime error: invalid memory address or nil pointer dereference");
                    let policy = frame
                        .owner()
                        .owner_cluster_policy(group)
                        .map_err(owner_score_error)?;
                    direction = match policy.arrangement.as_str() {
                        "Row" => Orientation::Bottom,
                        "Column" => Orientation::Right,
                        _ => direction,
                    };
                    factor = 0.2;
                }
                Ok((direction, factor, true))
            };
        if actual.shape == ShapeKind::SqlTable {
            return finish(Orientation::Right, 0.5);
        }

        let mut label_counts = BTreeMap::<NodeId, usize>::new();
        let mut outgoing_label_counts = BTreeMap::<NodeId, usize>::new();
        let mut multi_label_node = None;
        for (edge_index, local_edge) in active_edges.iter().copied().enumerate() {
            // Source265 polls every 64 incident pointers, before either
            // replacement identity check and before looking for a label.
            score_interval!(
                context,
                self,
                "node-prepare-direction-edge",
                Some(node),
                edge_index
            )?;
            let edge = frame
                .edge(
                    frame
                        .canonical_edge(local_edge)
                        .map_err(owner_score_error)?,
                )
                .map_err(owner_score_error)?;
            let adjacent = if edge.from == canonical {
                edge.to
            } else {
                edge.from
            };
            let restored = restored_endpoints
                .and_then(|endpoints| endpoints.get(edge_index))
                .copied()
                .unwrap_or_default();
            // A nonnil original pointer can equal the current pointer. The
            // source tests pointer equality, rather than original presence.
            let replacement = restored
                .node
                .map(|original| {
                    original.source_node.ok_or_else(|| {
                        owner_score_error("direction replacement has no canonical binding")
                    })
                })
                .transpose()?
                .unwrap_or(canonical);
            let adjacent_replacement = restored
                .adjacent
                .map(|original| {
                    original.source_node.ok_or_else(|| {
                        owner_score_error("direction replacement has no canonical binding")
                    })
                })
                .transpose()?
                .unwrap_or(adjacent);
            frame.node(replacement).map_err(owner_score_error)?;
            frame
                .node(adjacent_replacement)
                .map_err(owner_score_error)?;
            if replacement != canonical || adjacent_replacement != adjacent {
                continue;
            }
            if edge.label.is_none() || adjacent_replacement == replacement {
                continue;
            }
            let count = label_counts.entry(adjacent_replacement).or_default();
            *count += 1;
            let semantic_from = if edge.source_arrow && !edge.target_arrow {
                edge.to
            } else {
                edge.from
            };
            if semantic_from == canonical {
                *outgoing_label_counts
                    .entry(adjacent_replacement)
                    .or_default() += 1;
            }
            if *count > 2 {
                multi_label_node = Some(adjacent_replacement);
            }
        }
        if let Some(adjacent) = multi_label_node {
            let direction = if outgoing_label_counts.get(&adjacent).copied().unwrap_or(0)
                > label_counts[&adjacent] / 2
            {
                Orientation::Left
            } else {
                Orientation::Right
            };
            finish(direction, 10.0)
        } else {
            finish(Orientation::BottomRight, 0.3)
        }
    }

    /// Original node pointers survive temporary graph extraction. Their
    /// ancestry may be retained outside the current Graph.Nodes slice.
    pub(super) fn retained_scoring_nodes(&self) -> impl Iterator<Item = &ArenaNode> {
        self.nodes
            .iter()
            .chain(self.transaction_external_containers.iter())
            .chain(
                self.transaction_external_container_children
                    .values()
                    .flatten(),
            )
            .chain(
                self.transaction_external_aggregate_children
                    .values()
                    .flatten(),
            )
            .chain(
                self.projected_transaction_nodes
                    .iter()
                    .map(|candidate| &candidate.node),
            )
    }

    /// `Node.edgeLength` checks the direction of the selected outer
    /// replacement node, not the direction of the temporary carrier being
    /// optimized. Abducted endpoints are retained as projections from the
    /// owning graph; their container key only has a direction when that key
    /// is present in this temporary graph's `Directions` map. In particular,
    /// a replacement imported from a sibling scope has `GetDirection ==
    /// geo.NONE` even when the current carrier has a requested direction.
    pub(super) fn projected_outer_direction_is_set(
        &self,
        projected_node: Option<ProjectedAdjacent>,
        projected_adjacent: Option<ProjectedAdjacent>,
        node: NodeId,
        adjacent: NodeId,
    ) -> bool {
        // Upstream edge_length.go selects one `outerNode` before checking its
        // direction.  It does not ask whether either replacement has a
        // direction: when replacement containers differ, the deeper
        // replacement is treated as the inner node and the other endpoint is
        // the outer one.  The previous Rust shortcut used `any(...)`, which
        // charged a direction cost for a directed carrier even when TALA's
        // selected outer replacement was an undirected projection.
        let node_container = projected_node
            .map(|projected| projected.container_tala_id)
            .unwrap_or(self.nodes[node.0 as usize].scoring_container_parent);
        let adjacent_container = projected_adjacent
            .map(|projected| projected.container_tala_id)
            .unwrap_or(self.nodes[adjacent.0 as usize].scoring_container_parent);
        let node_depth = projected_node
            .and_then(|projected| {
                self.retained_scoring_nodes()
                    .find(|candidate| candidate.tala_id == projected.tala_id)
                    .map(|candidate| candidate.scoring_container_ancestors.len())
            })
            .unwrap_or_else(|| {
                self.nodes[node.0 as usize]
                    .scoring_container_ancestors
                    .len()
            });
        let adjacent_depth = projected_adjacent
            .and_then(|projected| {
                self.retained_scoring_nodes()
                    .find(|candidate| candidate.tala_id == projected.tala_id)
                    .map(|candidate| candidate.scoring_container_ancestors.len())
            })
            .unwrap_or_else(|| {
                self.nodes[adjacent.0 as usize]
                    .scoring_container_ancestors
                    .len()
            });
        let outer_container = if node_container != adjacent_container && node_depth > adjacent_depth
        {
            adjacent_container
        } else {
            node_container
        };
        let Some(container_tala_id) = outer_container else {
            return self.scoring_directions.contains_key(&None)
                || self.scoring_directions_by_tala.contains_key(&None);
        };
        // CopyEntitiesFrom shares Graph.Directions with each temporary graph.
        // Its pointer keys remain valid even when the keyed container is not
        // itself a member of the temporary Graph.Nodes slice. The stable-ID
        // equivalent is therefore authoritative without requiring a local
        // arena node for that container.
        if self
            .scoring_directions_by_tala
            .contains_key(&Some(container_tala_id))
        {
            return true;
        }
        let Some(container) = self
            .nodes
            .iter()
            .find(|candidate| candidate.tala_id == container_tala_id)
            .map(|candidate| candidate.input_id)
        else {
            return false;
        };
        self.scoring_directions.contains_key(&Some(container))
    }

    pub(super) fn sized_edge_length_with_cache_mode(
        &self,
        node: NodeId,
        direction_penalty: bool,
        obstruction_cache: Option<&[Vec<NodeId>]>,
        restore_sequence_endpoints: bool,
        alignment_cluster_distance_full: bool,
        tree_children_restored: bool,
    ) -> f64 {
        self.sized_edge_length_with_cache_mode_materialized(
            node,
            direction_penalty,
            obstruction_cache,
            restore_sequence_endpoints,
            alignment_cluster_distance_full,
            tree_children_restored,
            self.materialized_ordinary_container_scoring(),
        )
    }

    pub(super) fn sized_edge_length_with_cache_mode_materialized(
        &self,
        node: NodeId,
        direction_penalty: bool,
        obstruction_cache: Option<&[Vec<NodeId>]>,
        restore_sequence_endpoints: bool,
        alignment_cluster_distance_full: bool,
        tree_children_restored: bool,
        materialized_containers: bool,
    ) -> f64 {
        self.sized_edge_length_with_cache_mode_materialized_checked(
            node,
            direction_penalty,
            obstruction_cache,
            restore_sequence_endpoints,
            alignment_cluster_distance_full,
            tree_children_restored,
            materialized_containers,
            ScoreContext::none(),
        )
        .expect("unchecked scoring context cannot cancel")
    }

    pub(super) fn sized_edge_length_with_cache_mode_materialized_checked(
        &self,
        node: NodeId,
        direction_penalty: bool,
        obstruction_cache: Option<&[Vec<NodeId>]>,
        restore_sequence_endpoints: bool,
        alignment_cluster_distance_full: bool,
        tree_children_restored: bool,
        materialized_containers: bool,
        context: ScoreContext<'_>,
    ) -> Result<f64, limits::Error> {
        self.sized_edge_length_with_cache_mode_materialized_flow_mode(
            node,
            direction_penalty,
            obstruction_cache,
            restore_sequence_endpoints,
            alignment_cluster_distance_full,
            tree_children_restored,
            materialized_containers,
            None,
            None,
            None,
            context,
            None,
        )
    }

    /// edge_length.go1124–1181 walks the actual adjacent replacement's
    /// Node.Edges header, polling before the table filter and charging each
    /// crossing directly to the running score using node.Graph's cost.
    fn sized_owner_table_crossing_cost_into_checked(
        &self,
        node: NodeId,
        edge_id: EdgeId,
        node_replacement: NodeId,
        adjacent_replacement: NodeId,
        node_box: (Point, Size),
        adjacent_box: (Point, Size),
        frame: &OwnerScoreFrame<'_>,
        total: &mut f64,
        context: ScoreContext<'_>,
    ) -> Result<(), limits::Error> {
        let edge = frame
            .edge(frame.canonical_edge(edge_id).map_err(owner_score_error)?)
            .map_err(owner_score_error)?;
        if !edge.is_between_table_columns() {
            return Ok(());
        }
        let column = if edge.from == node_replacement {
            edge.target_table_column.expect("target table column")
        } else {
            edge.source_table_column.expect("source table column")
        };
        let direction = self.sized_box_orientation(node_box, adjacent_box);
        // Go range retains the header and copies this iteration's pointer
        // before the body poll. Later backing slots remain live. Keep nil
        // dereferencing after cancellation, exactly like e2's first method.
        for (index, other_slot) in frame
            .node(adjacent_replacement)
            .map_err(owner_score_error)?
            .edges
            .header()
            .iter_slots()
            .enumerate()
        {
            score_interval!(
                context,
                self,
                "node-edge.table-neighbors",
                Some(node),
                index
            )?;
            let other_edge = frame
                .edge(other_slot.expect("nil owner pointer slot"))
                .map_err(owner_score_error)?;
            if !other_edge.is_between_table_columns() {
                continue;
            }
            let other = if other_edge.from == adjacent_replacement {
                other_edge.to
            } else {
                other_edge.from
            };
            let other_column = if other_edge.from == other {
                other_edge.target_table_column.expect("target table column")
            } else {
                other_edge.source_table_column.expect("source table column")
            };
            let other_box = (
                frame
                    .position(other)
                    .map_err(owner_score_error)?
                    .expect("TALA table crossing requires live endpoint geometry"),
                frame.node(other).map_err(owner_score_error)?.rect.size,
            );
            let other_direction = self.sized_box_orientation(other_box, adjacent_box);
            let same_side = match direction {
                Orientation::TopRight | Orientation::Right | Orientation::BottomRight => matches!(
                    other_direction,
                    Orientation::TopRight | Orientation::Right | Orientation::BottomRight
                ),
                Orientation::TopLeft | Orientation::Left | Orientation::BottomLeft => matches!(
                    other_direction,
                    Orientation::TopLeft | Orientation::Left | Orientation::BottomLeft
                ),
                _ => false,
            };
            if same_side {
                let crossing = match self.sized_box_orientation(node_box, other_box) {
                    Orientation::TopRight | Orientation::Top | Orientation::TopLeft => {
                        other_column < column
                    }
                    Orientation::BottomRight | Orientation::Bottom | Orientation::BottomLeft => {
                        other_column > column
                    }
                    _ => false,
                };
                if crossing {
                    *total += frame
                        .receiver_crossing_cost(node)
                        .map_err(owner_score_error)?;
                }
            }
        }
        Ok(())
    }

    /// Binds ordinary endpoint and obstruction operands to the canonical
    /// owner inside the existing NodeEdgeLength kernel. Geometry, labels and
    /// table ports come from live canonical nodes/edges; retained originals
    /// and foreign obstructions stay live through the frame. Terms without owner bindings are rejected here
    /// until the broader scoring session supplies their actual operands.
    pub(super) fn sized_edge_length_with_owner_frame_checked(
        &self,
        node: NodeId,
        direction_penalty: bool,
        restore_sequence_endpoints: bool,
        alignment_cluster_distance_full: bool,
        tree_children_restored: bool,
        materialized_containers: bool,
        frame: &OwnerScoreFrame<'_>,
        context: ScoreContext<'_>,
    ) -> Result<f64, limits::Error> {
        if restore_sequence_endpoints || alignment_cluster_distance_full {
            return Err(owner_score_error(
                "grouping terms require owner scoring bindings",
            ));
        }
        if frame.abductions().is_none() {
            return Err(owner_score_error(
                "direct owner scoring requires an explicitly bound ordered abduction slice",
            ));
        }
        // edge_length.go147–162 collects all Graph.Clusters on a nil
        // argument, including groups outside the local dense membership.
        // That canonical collection is not yet bound by this direct entry.
        if frame.abductions().is_some_and(|slice| !slice.present)
            && !frame.owner().clusters.is_empty()
        {
            return Err(owner_score_error(
                "nil automatic-abduction collection requires owner cluster bindings",
            ));
        }
        if self.nodes.len() != frame.local_to_canonical().len() {
            return Err(owner_score_error("workspace mapping length differs"));
        }
        for (local, canonical) in frame.local_to_canonical().iter().copied().enumerate() {
            if self.nodes[local].tala_id
                != frame.node(canonical).map_err(owner_score_error)?.tala_id
            {
                return Err(owner_score_error(
                    "workspace mapping identifies a different node",
                ));
            }
        }
        let mapped_edge_ids = frame.local_to_canonical_edges();
        if self.edge_records.len() != mapped_edge_ids.len() {
            return Err(owner_score_error("workspace edge mapping length differs"));
        }
        for (local, canonical) in mapped_edge_ids.iter().copied().enumerate() {
            let actual = &frame.owner().edge_records[canonical.0 as usize];
            let workspace = &self.edge_records[local];
            if workspace.tala_id != actual.tala_id
                || workspace.is_invisible() != actual.is_invisible()
                || workspace.source_arrow != actual.source_arrow
                || workspace.target_arrow != actual.target_arrow
                || frame
                    .canonical_node(workspace.from)
                    .map_err(owner_score_error)?
                    != actual.from
                || frame
                    .canonical_node(workspace.to)
                    .map_err(owner_score_error)?
                    != actual.to
            {
                return Err(owner_score_error(
                    "workspace edge differs from its canonical owner edge",
                ));
            }
        }
        for (local, canonical) in frame.local_to_canonical().iter().copied().enumerate() {
            let local = NodeId(local as u32);
            let actual = frame.node(canonical).map_err(owner_score_error)?;
            if actual.cluster.is_some()
                || actual.sequence.is_some()
                || frame.owner().topology_node_vessel(canonical).is_some()
                || self.active_node_is_aggregate(local)
            {
                return Err(owner_score_error(
                    "grouped carriers require owner vessel scoring",
                ));
            }
            // Source evaluateNodeEdgeLength reads live replacement Boxes.
            // This full-owner entry binds those identities explicitly; dense
            // workspace geometry is not an operand. Keep topology validation
            // here while allowing geometry to change on the owning nodes.
            let mapped_edges = self
                .scoring_edge_ids(local)
                .iter()
                .map(|edge| {
                    mapped_edge_ids
                        .get(edge.0 as usize)
                        .copied()
                        .ok_or_else(|| owner_score_error("incident edge has no canonical binding"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            if mapped_edges != actual.edges {
                return Err(owner_score_error(
                    "incident edge order differs from actual owner pointers",
                ));
            }
            for &edge in &actual.edges.read() {
                let actual_edge = frame.edge(edge).map_err(owner_score_error)?;
                let expected_adjacent = if actual_edge.from == canonical {
                    actual_edge.to
                } else {
                    actual_edge.from
                };
                let local_edge = EdgeId(
                    mapped_edge_ids
                        .iter()
                        .position(|candidate| *candidate == edge)
                        .expect("validated incident edge mapping") as u32,
                );
                if frame
                    .canonical_node(self.active_adjacent(local, local_edge))
                    .map_err(owner_score_error)?
                    != expected_adjacent
                {
                    return Err(owner_score_error(
                        "incident adjacency differs from actual owner pointers",
                    ));
                }
            }
        }
        if let Some(slice) = frame.abductions() {
            if slice.records.len() != self.sized_edge_abductions.len() {
                return Err(owner_score_error("ordered owner abduction count differs"));
            }
            for (&record, workspace) in slice.records.iter().zip(&self.sized_edge_abductions) {
                let actual = frame
                    .owner()
                    .owner_edge_abduction(record)
                    .expect("frame retains a live ordered abduction record");
                if Some(workspace.source_edge) != actual.source_edge
                    || workspace.current_from.and_then(|node| node.source_node)
                        != actual.current_from
                    || workspace.current_to.and_then(|node| node.source_node) != actual.current_to
                    || workspace.originally_from.and_then(|node| node.source_node)
                        != actual.originally_from
                    || workspace.originally_to.and_then(|node| node.source_node)
                        != actual.originally_to
                    || workspace.current_from.is_some() != actual.current_from.is_some()
                    || workspace.current_to.is_some() != actual.current_to.is_some()
                    || workspace.originally_from.is_some() != actual.originally_from.is_some()
                    || workspace.originally_to.is_some() != actual.originally_to.is_some()
                {
                    return Err(owner_score_error(
                        "ordered abduction descriptor differs from actual owner pointers",
                    ));
                }
            }
        } else if !self.sized_edge_abductions.is_empty() {
            return Err(owner_score_error("ordered owner abductions are not bound"));
        }
        self.sized_edge_length_with_cache_mode_materialized_flow_mode(
            node,
            direction_penalty,
            None,
            restore_sequence_endpoints,
            alignment_cluster_distance_full,
            tree_children_restored,
            materialized_containers,
            Some(frame),
            None,
            None,
            context,
            None,
        )
    }

    /// Scores an alignment baseline. This now computes exactly what
    /// `sized_edge_length_with_cache_mode_materialized` computes.
    pub(super) fn sized_edge_length_with_cache_mode_materialized_alignment_baseline(
        &self,
        node: NodeId,
        direction_penalty: bool,
        obstruction_cache: Option<&[Vec<NodeId>]>,
        restore_sequence_endpoints: bool,
        alignment_cluster_distance_full: bool,
        tree_children_restored: bool,
        materialized_containers: bool,
    ) -> f64 {
        self.sized_edge_length_with_cache_mode_materialized_flow_mode(
            node,
            direction_penalty,
            obstruction_cache,
            restore_sequence_endpoints,
            alignment_cluster_distance_full,
            tree_children_restored,
            materialized_containers,
            None,
            None,
            None,
            ScoreContext::none(),
            None,
        )
        .expect("unchecked scoring context cannot cancel")
    }

    pub(super) fn sized_edge_length_with_owner_nears(
        &self,
        node: NodeId,
        direction_penalty: bool,
        obstruction_cache: Option<&[Vec<NodeId>]>,
        owner: &ArenaGraph,
        canonical: NodeId,
    ) -> f64 {
        self.sized_edge_length_with_owner_routes(
            node,
            direction_penalty,
            obstruction_cache,
            owner,
            canonical,
            None,
        )
        .expect("unchecked scoring context cannot cancel")
    }

    /// Source route probes read the current owner's actual replacement
    /// pointers and Containers headers, even when distance terms are still
    /// evaluated in the dense workspace.
    pub(super) fn sized_edge_length_with_owner_routes(
        &self,
        node: NodeId,
        direction_penalty: bool,
        obstruction_cache: Option<&[Vec<NodeId>]>,
        owner: &ArenaGraph,
        canonical: NodeId,
        route_owner_frame: Option<&OwnerScoreFrame<'_>>,
    ) -> Result<f64, limits::Error> {
        self.sized_edge_length_with_owner_routes_checked(
            node,
            direction_penalty,
            obstruction_cache,
            owner,
            canonical,
            route_owner_frame,
            ScoreContext::none(),
        )
    }

    pub(super) fn sized_edge_length_with_owner_routes_checked(
        &self,
        node: NodeId,
        direction_penalty: bool,
        obstruction_cache: Option<&[Vec<NodeId>]>,
        owner: &ArenaGraph,
        canonical: NodeId,
        route_owner_frame: Option<&OwnerScoreFrame<'_>>,
        context: ScoreContext<'_>,
    ) -> Result<f64, limits::Error> {
        self.sized_edge_length_with_cache_mode_materialized_flow_mode(
            node,
            direction_penalty,
            obstruction_cache,
            false,
            false,
            false,
            self.materialized_ordinary_container_scoring(),
            None,
            Some((owner, canonical)),
            route_owner_frame,
            context,
            None,
        )
    }

    fn sized_edge_length_with_cache_mode_materialized_flow_mode(
        &self,
        node: NodeId,
        direction_penalty: bool,
        obstruction_cache: Option<&[Vec<NodeId>]>,
        restore_sequence_endpoints: bool,
        alignment_cluster_distance_full: bool,
        tree_children_restored: bool,
        materialized_containers: bool,
        owner_frame: Option<&OwnerScoreFrame<'_>>,
        near_owner: Option<(&ArenaGraph, NodeId)>,
        route_owner_frame: Option<&OwnerScoreFrame<'_>>,
        context: ScoreContext<'_>,
        sweep: Option<&mut SizedEdgeLengthScorer>,
    ) -> Result<f64, limits::Error> {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(NodeScore);
        score_poll!(context, self, "node-edge-length-entry", Some(node), None)?;
        if sweep.as_ref().is_some_and(|scorer| scorer.node.is_none()) {
            return Err(limits::Error::Invalid(
                "EdgeLength: scorer is closed".into(),
            ));
        }
        let receiver_frame = owner_frame.or(route_owner_frame);
        let owner_carrier_positions = sweep
            .as_ref()
            .is_some_and(|scorer| scorer.owner_carrier_positions);
        let active_edges = self.scoring_edge_ids(node);
        let trace_sized_detail_position = crate::internal::trace_env_value(
            "TALA_TRACE_SIZED_DETAIL_POSITION",
        )
        .is_none_or(|target| {
            let mut parts = target.split(',');
            let Some(x) = parts.next().and_then(|value| value.parse::<f64>().ok()) else {
                return false;
            };
            let Some(y) = parts.next().and_then(|value| value.parse::<f64>().ok()) else {
                return false;
            };
            self.position(node)
                .is_some_and(|position| position.x == x && position.y == y)
        });
        let trace_sized_detail = crate::internal::trace_env_enabled("TALA_TRACE_SIZED_DETAIL")
            && (crate::internal::trace_env_value("TALA_TRACE_SIZED_PASS").is_none()
                || crate::internal::trace_env_enabled("TALA_TRACE_SIZED_PASS_ACTIVE"))
            && super::sized::trace_sized_graph_scope(self)
            && trace_sized_detail_position
            && (crate::internal::trace_env_value("TALA_TRACE_SIZED_DETAIL_NODE").is_some_and(
                |target| {
                    target == "all"
                        || target.split(',').any(|id| {
                            id.parse::<i64>().ok() == Some(self.nodes[node.0 as usize].tala_id)
                        })
                },
            ) || (self.nodes[node.0 as usize].tala_id == 527074092
                && self
                    .position(node)
                    .is_some_and(|position| position.y >= 1000.0)));
        if trace_sized_detail {
            eprintln!(
                "SIZED_DETAIL_RUST node={} external={} pos={:?} turnCost={} crossingCost={} restoreSequence={} fullCluster={} abductions={} projectedKeys={} cluster={:?} sequence={:?} edges={:?}",
                self.nodes[node.0 as usize].tala_id,
                self.nodes[node.0 as usize].external_id,
                self.position(node),
                self.raw_turn_cost(),
                self.raw_crossing_cost(),
                restore_sequence_endpoints,
                alignment_cluster_distance_full,
                self.sized_edge_abductions.len(),
                self.sized_projected_obstructions.len(),
                self.active_cluster_index(node),
                self.active_sequence_index(node),
                active_edges
                    .iter()
                    .map(|edge| {
                        let edge = &self.edge_records[edge.0 as usize];
                        (
                            edge.from.0,
                            self.nodes[edge.from.0 as usize].tala_id,
                            edge.to.0,
                            self.nodes[edge.to.0 as usize].tala_id,
                        )
                    })
                    .collect::<Vec<_>>()
            );
            if crate::internal::trace_env_enabled("TALA_TRACE_PROJECTION") {
                let local_ids = self
                    .nodes
                    .iter()
                    .map(|node| node.tala_id)
                    .collect::<Vec<_>>();
                let projections = self
                    .sized_projected_obstructions
                    .values()
                    .flatten()
                    .map(|projected| {
                        (
                            projected.tala_id,
                            self.nodes[projected.owner.0 as usize].tala_id,
                        )
                    })
                    .collect::<Vec<_>>();
                eprintln!(
                    "PROJECTION_RUST node={} local={:?} projections={:?}",
                    self.nodes[node.0 as usize].tala_id, local_ids, projections
                );
            }
        }
        let owner_replacements = |adjacent: NodeId,
                                  restored: SizedRestoredEndpoints|
         -> Result<Option<(NodeId, NodeId)>, limits::Error> {
            let Some(frame) = owner_frame else {
                return Ok(None);
            };
            let resolve = |local: NodeId, original: Option<AbductedNodeRef>| {
                let canonical = if let Some(original) = original {
                    original.source_node.ok_or_else(|| {
                        owner_score_error("restored endpoint has no canonical node binding")
                    })?
                } else {
                    frame.canonical_node(local).map_err(owner_score_error)?
                };
                let actual = frame.node(canonical).map_err(owner_score_error)?;
                if original.is_some_and(|original| original.tala_id != actual.tala_id) {
                    return Err(owner_score_error(
                        "restored endpoint binding identifies a different node",
                    ));
                }
                if actual.cluster.is_some()
                    || actual.sequence.is_some()
                    || frame.owner().topology_node_vessel(canonical).is_some()
                {
                    return Err(owner_score_error(
                        "endpoint grouping requires owner vessel scoring",
                    ));
                }
                Ok(canonical)
            };
            Ok(Some((
                resolve(node, restored.node)?,
                resolve(adjacent, restored.adjacent)?,
            )))
        };
        let mut local_prepared = None;
        let reusable = sweep.is_some();
        let cache_owner_obstructions = context.stable_owner()
            && sweep
                .as_ref()
                .is_some_and(|scorer| scorer.stable_owner_obstructions);
        let slot = match sweep {
            Some(scorer) => &mut scorer.prepared,
            None => &mut local_prepared,
        };
        if let Some(prepared) = slot.as_ref() {
            for _index in 0..prepared.checks {
                score_poll!(
                    context,
                    self,
                    "scorer-replay-preparation",
                    Some(node),
                    Some(_index)
                )?;
            }
        } else {
            #[cfg(feature = "diagnostic-traces")]
            crate::internal::call_counts::count!(NodeScorePrepare);
            let preparation_polls = std::cell::Cell::new(0);
            let preparation_context = if reusable {
                context.counted(&preparation_polls)
            } else {
                context
            };
            let restored_endpoints = self.sized_restored_endpoints_with_owner_frame_checked(
                node,
                &active_edges,
                restore_sequence_endpoints,
                owner_frame,
                preparation_context,
            )?;
            // This second projection only supplies canonical route operands for the
            // adapter. prepareNodeEdgeLength has one replacement scan, charged
            // above; repeating its polls here would add a second source preparation.
            let route_restored_endpoints = route_owner_frame
                .map(|frame| {
                    self.sized_restored_endpoints_with_owner_frame_checked(
                        node,
                        &active_edges,
                        restore_sequence_endpoints,
                        Some(frame),
                        ScoreContext::none(),
                    )
                })
                .transpose()?;
            let restored_endpoint = |edge_index| {
                restored_endpoints
                    .as_deref()
                    .and_then(|endpoints| endpoints.get(edge_index))
                    .copied()
                    .unwrap_or_default()
            };
            let (desired, factor, _use_recovered_unset_scoring) =
                if let Some(frame) = receiver_frame {
                    self.sized_edge_length_owner_direction_checked(
                        node,
                        &active_edges,
                        restored_endpoints.as_deref(),
                        direction_penalty,
                        frame,
                        preparation_context,
                    )?
                } else {
                    self.sized_edge_length_direction_with_identity_checked(
                        node,
                        &active_edges,
                        restored_endpoints.as_deref(),
                        direction_penalty,
                        preparation_context,
                        route_owner_frame,
                    )?
                };
            // Node degree is small, while BTreeMap allocates one node per unique
            // replacement pair on every candidate score. A compact insertion-
            // ordered counter has the same lookup semantics and no allocation at
            // all for the common no-label path.
            let mut edge_label_counts = Vec::<((i64, i64), usize)>::new();
            for (edge_index, edge_id) in active_edges.iter().copied().enumerate() {
                score_interval!(
                    preparation_context,
                    self,
                    "node-prepare-label-edge",
                    Some(node),
                    edge_index
                )?;
                let edge = &self.edge_records[edge_id.0 as usize];
                // Full-owner preparation counts actual Label pointers. The
                // route-only adapter keeps its workspace metadata contract.
                let label_edge = if let Some(frame) = owner_frame {
                    frame
                        .edge(frame.canonical_edge(edge_id).map_err(owner_score_error)?)
                        .map_err(owner_score_error)?
                } else {
                    edge
                };
                if label_edge.label.is_none() {
                    continue;
                }
                let adjacent = self.active_adjacent(node, edge_id);
                let restored = restored_endpoint(edge_index);
                let owner_pair = owner_replacements(adjacent, restored)?;
                let node_replacement_id = restored
                    .node
                    .map(|projected| projected.tala_id)
                    .unwrap_or(self.nodes[node.0 as usize].tala_id);
                let adjacent_replacement_id = restored
                    .adjacent
                    .map(|projected| projected.tala_id)
                    .unwrap_or(self.nodes[adjacent.0 as usize].tala_id);
                let (node_replacement_id, adjacent_replacement_id) =
                    if let Some((left, right)) = owner_pair {
                        let frame = owner_frame.expect("owner pair retains frame");
                        (
                            frame.node(left).map_err(owner_score_error)?.tala_id,
                            frame.node(right).map_err(owner_score_error)?.tala_id,
                        )
                    } else {
                        (node_replacement_id, adjacent_replacement_id)
                    };
                let key = (node_replacement_id, adjacent_replacement_id);
                if let Some((_, count)) = edge_label_counts
                    .iter_mut()
                    .find(|(candidate, _)| *candidate == key)
                {
                    *count += 1;
                } else {
                    edge_label_counts.push((key, 1));
                }
            }

            *slot = Some(SizedScoringPrepared {
                restored_endpoints,
                route_restored_endpoints,
                desired,
                factor,
                edge_label_counts,
                checks: preparation_polls.get(),
                owner_obstructions: if cache_owner_obstructions {
                    (0..active_edges.len()).map(|_| None).collect()
                } else {
                    Vec::new()
                },
            });
        }
        let prepared = slot.as_mut().expect("successful sized scorer preparation");
        let restored_endpoints = &prepared.restored_endpoints;
        let route_restored_endpoints = &prepared.route_restored_endpoints;
        let desired = prepared.desired;
        let factor = prepared.factor;
        let edge_label_counts = &prepared.edge_label_counts;
        let owner_obstructions = &mut prepared.owner_obstructions;
        let scoring_live_cluster_vessel_positions = if owner_frame.is_none() {
            self.scoring_live_cluster_vessel_positions(restored_endpoints.as_deref())
        } else {
            BTreeMap::new()
        };
        let restored_endpoint = |edge_index| {
            restored_endpoints
                .as_deref()
                .and_then(|endpoints| endpoints.get(edge_index))
                .copied()
                .unwrap_or_default()
        };
        let owner_scoring_container = owner_frame
            .map(|frame| frame.effective_container(Some(frame.canonical_node(node)?)))
            .transpose()
            .map_err(owner_score_error)?;
        let mut total = 0.0;
        // evaluateNodeEdgeLength checks the current adjacent TopLeft before
        // dereferencing any retained replacement geometry (edge_length.go361).
        for (edge_index, edge_id) in active_edges.iter().copied().enumerate() {
            score_interval!(context, self, "node-evaluate-edge", Some(node), edge_index)?;
            let adjacent = self.active_adjacent(node, edge_id);
            let restored = restored_endpoint(edge_index);
            if let Some(frame) = owner_frame.or(route_owner_frame) {
                let carrier = frame.canonical_node(adjacent).map_err(owner_score_error)?;
                if frame
                    .position(carrier)
                    .map_err(owner_score_error)?
                    .is_none()
                {
                    continue;
                }
            } else if self.active_node_position(adjacent).is_none() {
                continue;
            }
            let owner_pair = owner_replacements(adjacent, restored)?;
            let route_owner_pair = if let Some(frame) = route_owner_frame {
                let route_restored = route_restored_endpoints
                    .as_ref()
                    .and_then(Option::as_ref)
                    .and_then(|endpoints| endpoints.get(edge_index))
                    .copied()
                    .unwrap_or_default();
                let resolve = |local, original: Option<AbductedNodeRef>| {
                    original.map_or_else(
                        || frame.canonical_node(local).map_err(owner_score_error),
                        |original| {
                            original.source_node.ok_or_else(|| {
                                owner_score_error("route endpoint has no owner binding")
                            })
                        },
                    )
                };
                Some((
                    frame,
                    resolve(node, route_restored.node)?,
                    resolve(adjacent, route_restored.adjacent)?,
                ))
            } else {
                owner_pair.map(|(left, right)| {
                    (owner_frame.expect("owner pair retains frame"), left, right)
                })
            };
            let project_original =
                |original: Option<AbductedNodeRef>,
                 local: NodeId|
                 -> Result<Option<ProjectedAdjacent>, limits::Error> {
                    let Some(original) = original.filter(|_| owner_frame.is_none()) else {
                        return Ok(None);
                    };
                    if let Some(frame) = route_owner_frame {
                        // A retained original can be the current pointer itself.
                        // Source evaluation reads that node's live Box; descriptor
                        // size/offset data must not replace its current geometry.
                        if original.source_node
                            == Some(frame.canonical_node(local).map_err(owner_score_error)?)
                        {
                            return Ok(None);
                        }
                    }
                    Ok(Some(self.abducted_node_projection(original).expect(
                        "TALA edge scoring requires original endpoint geometry",
                    )))
                };
            // An owner frame scores live owner boxes: project_original's
            // first test returns None for both endpoints.
            let (projected_node, projected_adjacent) = if owner_frame.is_some() {
                (None, None)
            } else {
                // A nil original returns None before any read; skip the call
                // and its by-value operand for the common unabducted edge.
                (
                    if restored.node.is_some() {
                        project_original(restored.node, node)?
                    } else {
                        None
                    },
                    if restored.adjacent.is_some() {
                        project_original(restored.adjacent, adjacent)?
                    } else {
                        None
                    },
                )
            };
            let live_owner_box = |second: bool| -> Result<Option<(Point, Size)>, limits::Error> {
                let Some((frame, left, right)) = route_owner_pair else {
                    return Ok(None);
                };
                let source = if second { right } else { left };
                // prepareNodeEdgeLength retains both current and restored
                // pointers. Read the owner after context callbacks even when
                // no original-endpoint replacement was needed.
                Ok(Some((
                    frame
                        .position(source)
                        .map_err(owner_score_error)?
                        .expect("TALA edge scoring requires original endpoint geometry"),
                    frame.node(source).map_err(owner_score_error)?.rect.size,
                )))
            };
            let live_node_box = live_owner_box(false)?;
            let live_adjacent_box = live_owner_box(true)?;
            let node_is_own_aggregate_vessel = projected_node.is_some_and(|projected| {
                projected.cluster_member
                    && projected.owner == node
                    && self.active_node_is_aggregate(node)
                    && self.active_aggregate_owner(node) == node
                    && (self.swap_scoring_vessel_boxes
                        || crate::internal::trace_env_enabled("TALA_TRACE_FORCE_VESSEL_BOXES"))
            });
            // The scored node stands in for TALA's temporary cluster vessel.
            // Even when the stable carrier is itself an ordinary container,
            // the recovered endpoint replacement still has Cluster set, so
            // route and arrangement penalties must be charged exactly as they
            // are for the Go vessel. The carrier's container flag is an arena
            // representation detail and must not suppress those terms.
            let node_is_container_aggregate_vessel = false;
            let node_box = if let Some((replacement, _)) = owner_pair {
                let frame = owner_frame.expect("owner pair retains frame");
                (
                    frame
                        .position(replacement)
                        .map_err(owner_score_error)?
                        .expect("TALA edge scoring requires original endpoint geometry"),
                    frame
                        .node(replacement)
                        .map_err(owner_score_error)?
                        .rect
                        .size,
                )
            } else if node_is_own_aggregate_vessel {
                // The temporary Go graph replaces all cluster members with a
                // single vessel pointer.  Keep the projected marker for its
                // arrangement penalty, but use the vessel's full box for the
                // distance and direction terms when that pointer is the
                // scored node itself.
                let Some(position) = self.active_node_position(node) else {
                    continue;
                };
                (position, self.active_node_size(node))
            } else if let Some(live_box) = live_node_box {
                live_box
            } else if let Some(projected) = projected_node {
                let Some(projected_box) = self.sized_projected_box(projected) else {
                    continue;
                };
                projected_box
            } else {
                let Some(position) = self.active_node_position(node) else {
                    continue;
                };
                (position, self.active_node_size(node))
            };
            let adjacent_box = if let Some((_, replacement)) = owner_pair {
                let frame = owner_frame.expect("owner pair retains frame");
                (
                    frame
                        .position(replacement)
                        .map_err(owner_score_error)?
                        .expect("TALA edge scoring requires original endpoint geometry"),
                    frame
                        .node(replacement)
                        .map_err(owner_score_error)?
                        .rect
                        .size,
                )
            } else if let Some(live_box) = live_adjacent_box {
                live_box
            } else if let Some(projected) = projected_adjacent {
                let Some(projected_box) = self.sized_projected_box(projected) else {
                    continue;
                };
                projected_box
            } else {
                let Some(position) = self.active_node_position(adjacent) else {
                    continue;
                };
                (position, self.active_node_size(adjacent))
            };
            let distance_box = |projected: ProjectedAdjacent| {
                let raw_owner = self.position(projected.owner).unwrap();
                if alignment_cluster_distance_full {
                    (
                        self.active_node_position(projected.owner).unwrap(),
                        self.active_node_size(projected.owner),
                    )
                } else if let Some(cluster_box) =
                    self.sized_projected_cluster_distance_box(projected)
                {
                    let position = scoring_live_cluster_vessel_positions
                        .get(
                            &self.sized_cluster_distance_boxes
                                [&(projected.owner, projected.tala_id)]
                                .vessel_tala_id,
                        )
                        .copied()
                        .unwrap_or(cluster_box.0);
                    (position, cluster_box.1)
                } else {
                    // During SwapOptimize the sized projection cache is
                    // intentionally absent. Node.edgeLength still measures
                    // a retained cluster member from the active vessel, not
                    // from the stable member carrier's input position.
                    let owner = if self.active_cluster_index(projected.owner).is_some() {
                        self.active_node_position(projected.owner).unwrap()
                    } else {
                        raw_owner
                    };
                    (owner, self.active_node_size(projected.owner))
                }
            };
            // A cluster member endpoint is scored through the temporary
            // vessel on both sides of an edge. The stable arena keeps the
            // first member as the carrier, so retain its projected marker
            // even when it is also the scored node; `distance_box` resolves
            // that marker to the vessel geometry. Dropping this projection
            // makes diagonal edges pay the ordinary member corner/turn cost
            // and diverges from OSS TALA's `placementDistance(vessel, ...)`.
            // edge_length.go:505–515 reads the retained replacement's actual
            // Cluster pointer, then the live Vessel box. Projection metadata
            // cannot represent a group assigned independently of that cache.
            let canonical_clusters = route_owner_pair
                .map(|(frame, left, right)| {
                    let kind = super::owner_groups::OwnerGroupKind::Cluster;
                    Ok::<_, limits::Error>((
                        frame
                            .owner()
                            .owner_node_group_vessel(left, kind)
                            .map_err(owner_score_error)?,
                        frame
                            .owner()
                            .owner_node_group_vessel(right, kind)
                            .map_err(owner_score_error)?,
                    ))
                })
                .transpose()?;
            let cluster_endpoint = canonical_clusters.map_or_else(
                || {
                    (projected_node.is_some_and(|projected| projected.cluster_member)
                        && !node_is_container_aggregate_vessel)
                        || projected_adjacent.is_some_and(|projected| projected.cluster_member)
                },
                |(left, right)| left.is_some() || right.is_some(),
            );
            let live_vessel_box = |vessel: Option<NodeId>| -> Result<_, limits::Error> {
                let frame = route_owner_pair.expect("canonical cluster retains frame").0;
                let vessel = vessel
                    .expect("runtime error: invalid memory address or nil pointer dereference");
                Ok((
                    frame
                        .position(vessel)
                        .map_err(owner_score_error)?
                        .expect("runtime error: invalid memory address or nil pointer dereference"),
                    frame.node(vessel).map_err(owner_score_error)?.rect.size,
                ))
            };
            let (node_distance_box, adjacent_distance_box) =
                if let Some((left, right)) = canonical_clusters {
                    // Source gives the first endpoint's cluster priority. It
                    // does not dereference the second Vessel in that branch.
                    let left_box = left.map(&live_vessel_box).transpose()?;
                    let right_box = if left.is_none() {
                        right.map(&live_vessel_box).transpose()?
                    } else {
                        None
                    };
                    (left_box, right_box)
                } else {
                    (
                        projected_node
                            .filter(|projected| projected.cluster_member)
                            .map(&distance_box),
                        projected_adjacent
                            .filter(|projected| projected.cluster_member)
                            .map(&distance_box),
                    )
                };
            if crate::internal::trace_env_enabled("TALA_TRACE_SIZED_DETAIL")
                && self.nodes[node.0 as usize].tala_id == 2_749_337_804
                && self
                    .position(node)
                    .is_some_and(|position| position.x == -495.0 && position.y == 99.0)
            {
                eprintln!(
                    "SIZED_DISTANCE_BOX_RUST node={:?} adjacent={:?} nodeDistance={:?} adjacentDistance={:?} retained={:?}",
                    node_distance_box,
                    projected_adjacent.map(|projected| projected.tala_id),
                    node_distance_box,
                    adjacent_distance_box,
                    projected_adjacent.and_then(|projected| self
                        .sized_cluster_distance_boxes
                        .get(&(projected.owner, projected.tala_id))),
                );
            }
            let edge = &self.edge_records[edge_id.0 as usize];
            let scoring_edge = if let Some(frame) = owner_frame {
                frame
                    .edge(frame.canonical_edge(edge_id).map_err(owner_score_error)?)
                    .map_err(owner_score_error)?
            } else {
                edge
            };
            let edge_from_is_node = self.active_aggregate_owner(edge.from) == node;
            let (source_box, target_box) = if edge_from_is_node {
                (node_box, adjacent_box)
            } else {
                (adjacent_box, node_box)
            };
            let mut distance = if scoring_edge.has_table_column() {
                if let Some(frame) = owner_frame {
                    let (left, right) = owner_pair.expect("full-owner replacements");
                    let endpoints = if edge_from_is_node {
                        (left, right)
                    } else {
                        (right, left)
                    };
                    frame
                        .owner()
                        .table_column_distance_for_boxes_with_owner_nodes(
                            frame.canonical_edge(edge_id).map_err(owner_score_error)?,
                            source_box,
                            target_box,
                            frame.receiver_cell_size(node).map_err(owner_score_error)?,
                            Some(endpoints),
                        )
                } else {
                    self.table_column_distance_for_boxes(edge_id, source_box, target_box)
                }
            } else if let Some(projected_box) = node_distance_box {
                Self::sized_box_distance(projected_box, adjacent_box)
            } else if let Some(projected_box) = adjacent_distance_box {
                Self::sized_box_distance(node_box, projected_box)
            } else {
                self.sized_edge_base_distance_between_with_owner_frame_checked(
                    node_box,
                    adjacent_box,
                    node,
                    receiver_frame,
                )?
            };
            if trace_sized_detail {
                eprintln!(
                    "SIZED_DETAIL_RUST edge={} adjacent={} nodeBox={:?} adjacentBox={:?} projected={:?}/{:?} distance={}",
                    edge_id.0,
                    self.nodes[adjacent.0 as usize].tala_id,
                    node_box,
                    adjacent_box,
                    projected_node,
                    projected_adjacent,
                    distance
                );
            }
            // `Node.edgeLength` keeps the replacement node's live box for
            // orientation even when distance/routing use a cluster vessel.
            // The induced Go graph points the edge at the original member;
            // only its placement distance is measured from the vessel. Keep
            // those two geometries separate so member-specific diagonal
            // directions (and their compass penalties) survive scoring.
            let node_direction_box = live_node_box
                .or_else(|| {
                    projected_node.and_then(|projected| self.sized_projected_box(projected))
                })
                .unwrap_or(node_box);
            let adjacent_direction_box = live_adjacent_box
                .or_else(|| {
                    projected_adjacent.and_then(|projected| self.sized_projected_box(projected))
                })
                .unwrap_or(adjacent_box);
            let edge_direction =
                self.sized_box_orientation(node_direction_box, adjacent_direction_box);
            // Recovered Node.edgeLength skips a sized edge whose replacement
            // boxes have no strict compass separation (getOrientation()==NONE)
            // before applying distance or direction penalties.  In
            // particular, touching boxes must contribute no edge term during
            // transpose trials; scoring them as an overlapping/neutral edge
            // changes which rotation is accepted.
            if edge_direction == Orientation::None {
                continue;
            }
            let node_replacement_id = projected_node
                .map(|projected| projected.tala_id)
                .unwrap_or(self.nodes[node.0 as usize].tala_id);
            let adjacent_replacement_id = projected_adjacent
                .map(|projected| projected.tala_id)
                .unwrap_or(self.nodes[adjacent.0 as usize].tala_id);
            let (node_replacement_id, adjacent_replacement_id) =
                if let Some((left, right)) = owner_pair {
                    let frame = owner_frame.expect("owner pair retains frame");
                    (
                        frame.node(left).map_err(owner_score_error)?.tala_id,
                        frame.node(right).map_err(owner_score_error)?.tala_id,
                    )
                } else {
                    (node_replacement_id, adjacent_replacement_id)
                };
            let (node_ref, adjacent_ref) = if let Some(frame) = owner_frame {
                (
                    frame
                        .node(frame.canonical_node(node).map_err(owner_score_error)?)
                        .map_err(owner_score_error)?,
                    frame
                        .node(frame.canonical_node(adjacent).map_err(owner_score_error)?)
                        .map_err(owner_score_error)?,
                )
            } else {
                (
                    &self.nodes[node.0 as usize],
                    &self.nodes[adjacent.0 as usize],
                )
            };
            let edge_is_directed = self.edge_is_directed(edge_id);
            let mut alignment_penalty = 0.0;
            let align_carriers = if let Some(frame) = owner_frame {
                node_ref.is_container
                    && adjacent_ref.is_container
                    && owner_scoring_container.expect("owner container was captured")
                        == frame
                            .effective_container(Some(
                                frame.canonical_node(adjacent).map_err(owner_score_error)?,
                            ))
                            .map_err(owner_score_error)?
            } else {
                !self.active_node_is_aggregate(node)
                    && !self.active_node_is_aggregate(adjacent)
                    && (node_ref.scoring_is_container
                        || (materialized_containers && node_ref.is_container))
                    && (adjacent_ref.scoring_is_container
                        || (materialized_containers && adjacent_ref.is_container))
                    && node_ref.scoring_container_parent == adjacent_ref.scoring_container_parent
            };
            // AddCluster's vessel is a plain node even when its stable arena
            // carrier is a container member. Container alignment reads the
            // live endpoints, not their retained member decorations.
            if align_carriers {
                // node.go's alignment term is deliberately carrier-owned:
                // edge abductions replace endpoints for distance and route
                // scoring, but these comparisons still read `node.Box` and
                // `adjacentNode.Box`.
                // Candidate movement mutates the owner first. Carrier sizes
                // and the alignment eligibility above are fixed for a search;
                // its live positions need no workspace publication.
                let alignment_frame = route_owner_frame.filter(|_| owner_carrier_positions);
                let node_position = if let Some(frame) = alignment_frame {
                    frame.local_position(node).map_err(owner_score_error)?
                } else {
                    node_ref.position
                }
                .expect("positioned scoring carrier");
                let node_size = node_ref.rect.size;
                let adjacent_position = if let Some(frame) = alignment_frame {
                    frame.local_position(adjacent).map_err(owner_score_error)?
                } else {
                    adjacent_ref.position
                }
                .expect("positioned adjacent carrier");
                let adjacent_size = adjacent_ref.rect.size;
                if node_size.width.min(adjacent_size.width)
                    > node_size.width.max(adjacent_size.width) * 0.75
                    && node_position.x != adjacent_position.x
                    && node_position.x + node_size.width
                        != adjacent_position.x + adjacent_size.width
                    && (node_position.x + node_size.width) * 0.5
                        != (adjacent_position.x + adjacent_size.width) * 0.5
                {
                    alignment_penalty =
                        self.scoring_turn_cost_with_owner_frame_checked(node, receiver_frame)?;
                }
                if node_size.height.min(adjacent_size.height)
                    > node_size.height.max(adjacent_size.height) * 0.75
                    && node_position.y != adjacent_position.y
                    && node_position.y + node_size.height
                        != adjacent_position.y + adjacent_size.height
                    && (node_position.y + node_size.height) * 0.5
                        != (adjacent_position.y + adjacent_size.height) * 0.5
                {
                    // Recovered node.go:3364 overwrites rather than adds when
                    // both axes are misaligned.
                    alignment_penalty =
                        self.scoring_turn_cost_with_owner_frame_checked(node, receiver_frame)?;
                }
                distance += alignment_penalty;
            }
            let route_penalty = if scoring_edge.is_between_table_columns() {
                0.0
            } else {
                self.edge_route_penalty_with_owner_frame_checked(
                    node,
                    adjacent,
                    edge_id,
                    obstruction_cache
                        .and_then(|cache| cache.get(edge_id.0 as usize))
                        .map(Vec::as_slice),
                    Some((
                        (node_box.0, node_box.1, node_replacement_id),
                        (adjacent_box.0, adjacent_box.1, adjacent_replacement_id),
                    )),
                    if let Some((index, reversed)) = restored_endpoint(edge_index).abduction {
                        let abduction = &self.sized_edge_abductions[index];
                        let (recorded_obstructions, recorded_sets) = if reversed {
                            (
                                &abduction.obstructions_to_from,
                                &abduction.obstruction_sets_to_from,
                            )
                        } else {
                            (
                                &abduction.obstructions_from_to,
                                &abduction.obstruction_sets_from_to,
                            )
                        };
                        // placement/alignment.go passes Sequence.EdgeAbductions
                        // to EdgeLength on the live graph: edge_length.go
                        // 869-899 walks node.Graph.Containers from the
                        // replacement endpoints at score time.  Those records
                        // carry no inventory recorded by a temporary placement
                        // scope, so an empty projected slice must not stand in
                        // for that live walk (it would test no obstruction).
                        if recorded_obstructions.is_empty() && !recorded_sets.sets_present {
                            ProjectedObstructionSource::Current
                        } else if reversed {
                            ProjectedObstructionSource::Ordered(
                                abduction.obstructions_to_from.as_slice(),
                                &abduction.obstruction_sets_to_from,
                            )
                        } else {
                            ProjectedObstructionSource::Ordered(
                                abduction.obstructions_from_to.as_slice(),
                                &abduction.obstruction_sets_from_to,
                            )
                        }
                    } else {
                        // AddCluster/AddSequence records the projected
                        // obstruction inventory for the entire temporary
                        // graph. A graph with no endpoint abductions has no
                        // temporary aggregate carrier and must use the live
                        // Graph.Containers walk (this is material for nested
                        // ordinary containers such as container_escape).
                        if !self.sized_edge_abductions.is_empty() {
                            ProjectedObstructionSource::Edge
                        } else {
                            ProjectedObstructionSource::Current
                        }
                    },
                    cluster_endpoint,
                    tree_children_restored,
                    route_owner_pair,
                    if cache_owner_obstructions {
                        owner_obstructions.get_mut(edge_index)
                    } else {
                        None
                    },
                    context,
                )?
            };
            distance += route_penalty;
            // Go reads badClusterArrangementPenalty within this sized,
            // non-column score even when neither endpoint is a cluster.
            // Keep this actual first-reader boundary after route penalties.
            if !scoring_edge.is_between_table_columns() {
                self.scoring_turn_cost_with_owner_frame_checked(node, receiver_frame)?;
            }
            let mut cluster_penalty = 0.0;
            if !scoring_edge.is_between_table_columns() {
                if let Some((frame, left, right)) = route_owner_pair {
                    // Same first-reader boundary and current Graph cost as Source.
                    let turn_cost = frame.receiver_turn_cost(node).map_err(owner_score_error)?;
                    for (replacement, other) in [(left, right), (right, left)] {
                        if let Some(parts) = self.sized_owner_cluster_arrangement_penalty_checked(
                            frame,
                            replacement,
                            other,
                            edge_direction,
                            turn_cost,
                        )? {
                            cluster_penalty += parts[0] + parts[1] + parts[2];
                            distance += parts[0];
                            distance += parts[1];
                            distance += parts[2];
                        }
                    }
                } else {
                    if let Some(projected) =
                        projected_node.filter(|projected| projected.cluster_member)
                    {
                        if !node_is_container_aggregate_vessel {
                            let parts = self.sized_cluster_arrangement_penalty(
                                projected,
                                adjacent_box,
                                edge_direction,
                                projected_adjacent
                                    .map(|projected| projected.tala_id)
                                    .unwrap_or(self.nodes[adjacent.0 as usize].tala_id),
                                &scoring_live_cluster_vessel_positions,
                            );
                            // Keep Go's left-to-right accumulation order.  Summing
                            // the three terms first changes the final IEEE-754 bits
                            // for some candidates (even when the mathematical score
                            // is unchanged).
                            cluster_penalty = parts[0] + parts[1] + parts[2];
                            distance += parts[0];
                            distance += parts[1];
                            distance += parts[2];
                        }
                    } else if let Some(projected) =
                        projected_adjacent.filter(|projected| projected.cluster_member)
                    {
                        let parts = self.sized_cluster_arrangement_penalty(
                            projected,
                            node_box,
                            edge_direction,
                            projected_node
                                .map(|projected| projected.tala_id)
                                .unwrap_or(self.nodes[node.0 as usize].tala_id),
                            &scoring_live_cluster_vessel_positions,
                        );
                        cluster_penalty = parts[0] + parts[1] + parts[2];
                        distance += parts[0];
                        distance += parts[1];
                        distance += parts[2];
                    }
                }
            }
            let projected_table_neighbors =
                restored_endpoint(edge_index)
                    .abduction
                    .and_then(|(index, reversed)| {
                        let abduction = &self.sized_edge_abductions[index];
                        if reversed {
                            projected_adjacent
                                .is_some()
                                .then_some(abduction.originally_from_table_neighbors.as_slice())
                        } else {
                            projected_adjacent
                                .is_some()
                                .then_some(abduction.originally_to_table_neighbors.as_slice())
                        }
                    });
            if let Some(frame) = owner_frame {
                let (left, right) = owner_pair.expect("full-owner replacements");
                self.sized_owner_table_crossing_cost_into_checked(
                    node,
                    edge_id,
                    left,
                    right,
                    node_box,
                    adjacent_box,
                    frame,
                    &mut distance,
                    context,
                )?;
            } else {
                self.table_column_order_crossing_cost_into_checked(
                    node,
                    adjacent,
                    edge_id,
                    node_box,
                    adjacent_box,
                    projected_node,
                    projected_adjacent,
                    projected_table_neighbors,
                    &mut distance,
                    context,
                )?;
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_EDGE_DETAIL") {
                let node_target = crate::internal::trace_env_value("TALA_TRACE_EDGE_DETAIL_NODE")
                    .is_none_or(|target| {
                        target == "all"
                            || target == self.nodes[node.0 as usize].external_id
                            || target
                                .parse::<i64>()
                                .ok()
                                .is_some_and(|id| id == self.nodes[node.0 as usize].tala_id)
                    });
                let position_target = crate::internal::trace_env_value(
                    "TALA_TRACE_EDGE_DETAIL_POSITION",
                )
                .is_none_or(|target| {
                    let mut parts = target.split(',');
                    let Some(x) = parts.next().and_then(|value| value.parse::<f64>().ok()) else {
                        return false;
                    };
                    let Some(y) = parts.next().and_then(|value| value.parse::<f64>().ok()) else {
                        return false;
                    };
                    self.position(node)
                        .is_some_and(|position| position.x == x && position.y == y)
                });
                if node_target && position_target {
                    eprintln!(
                        "EDGE_DETAIL_RUST node={} adjacent={} replacement={:?}/{:?} edge_dir={:?} distance={:.17} projected={:?}/{:?} boxes={:?}/{:?} base={:.17} route={:.17} cluster={:.17} alignment={:.17}",
                        self.nodes[node.0 as usize].tala_id,
                        self.nodes[adjacent.0 as usize].tala_id,
                        projected_node.map(|projected| projected.tala_id),
                        projected_adjacent.map(|projected| projected.tala_id),
                        edge_direction,
                        distance,
                        projected_node,
                        projected_adjacent,
                        node_box,
                        adjacent_box,
                        distance - route_penalty - cluster_penalty - alignment_penalty,
                        route_penalty,
                        cluster_penalty,
                        alignment_penalty,
                    );
                    if crate::internal::trace_env_enabled("TALA_TRACE_EDGE_DETAIL_COMPONENTS") {
                        let cluster_view = |projected: Option<ProjectedAdjacent>| {
                            projected.map(|projected| {
                                let cluster_index = self.active_cluster_index(projected.owner);
                                let cluster = cluster_index.map(|index| {
                                    let cluster = &self.clusters[index];
                                    (
                                        cluster.vessel_tala_id,
                                        cluster
                                            .members
                                            .iter()
                                            .map(|member| self.nodes[member.0 as usize].tala_id)
                                            .collect::<Vec<_>>(),
                                        self.cluster_vessel_size(index),
                                        self.pending_cluster_vessel_positions.get(&index).copied(),
                                    )
                                });
                                (
                                    projected.tala_id,
                                    self.nodes[projected.owner.0 as usize].tala_id,
                                    self.active_node_tala_id(projected.owner),
                                    self.active_node_position(projected.owner),
                                    self.active_node_size(projected.owner),
                                    cluster,
                                    self.sized_cluster_distance_boxes
                                        .get(&(projected.owner, projected.tala_id))
                                        .map(|distance| {
                                            (
                                                distance.vessel_tala_id,
                                                distance.offset,
                                                distance.size,
                                                self.transaction_external_aggregate_children
                                                    .get(&distance.vessel_tala_id)
                                                    .map(|members| {
                                                        members
                                                            .iter()
                                                            .map(|member| {
                                                                (member.tala_id, member.position)
                                                            })
                                                            .collect::<Vec<_>>()
                                                    }),
                                                self.clusters
                                                    .iter()
                                                    .enumerate()
                                                    .find(|(_, cluster)| {
                                                        cluster.vessel_tala_id
                                                            == distance.vessel_tala_id
                                                    })
                                                    .map(|(index, cluster)| {
                                                        (
                                                            index,
                                                            self.cluster_is_active(cluster),
                                                            self.pending_cluster_vessel_positions
                                                                .get(&index)
                                                                .copied(),
                                                            cluster.members.first().map(|first| {
                                                                (
                                                                    self.nodes[first.0 as usize]
                                                                        .tala_id,
                                                                    self.active_node_position(
                                                                        *first,
                                                                    ),
                                                                    self.active_node_size(*first),
                                                                )
                                                            }),
                                                            cluster
                                                                .members
                                                                .iter()
                                                                .map(|member| {
                                                                    (
                                                                        self.nodes
                                                                            [member.0 as usize]
                                                                            .tala_id,
                                                                        self.position(*member),
                                                                    )
                                                                })
                                                                .collect::<Vec<_>>(),
                                                        )
                                                    }),
                                            )
                                        }),
                                    distance_box(projected),
                                )
                            })
                        };
                        let cluster_distance_before_alignment =
                            distance - route_penalty - cluster_penalty - alignment_penalty;
                        eprintln!(
                            "EDGE_BOX_RUST node={} adjacent={} edge={} node_projection={:?} adjacent_projection={:?} initial_distance={:.17} initial_bits=0x{:016x} turn_cost={:.17}",
                            self.nodes[node.0 as usize].tala_id,
                            self.nodes[adjacent.0 as usize].tala_id,
                            edge_id.0,
                            cluster_view(projected_node),
                            cluster_view(projected_adjacent),
                            cluster_distance_before_alignment,
                            cluster_distance_before_alignment.to_bits(),
                            self.raw_turn_cost(),
                        );
                    }
                }
            }
            if trace_sized_detail {
                eprintln!(
                    "SIZED_DETAIL_RUST_COMPONENT edge={} base={} base_bits=0x{:016x} alignment={} route={} cluster={} after={} after_bits=0x{:016x}",
                    edge_id.0,
                    distance - route_penalty - cluster_penalty - alignment_penalty,
                    (distance - route_penalty - cluster_penalty - alignment_penalty).to_bits(),
                    alignment_penalty,
                    route_penalty,
                    cluster_penalty,
                    distance,
                    distance.to_bits()
                );
            }
            let outer_direction_is_set = if let Some((frame, left, right)) = route_owner_pair {
                frame
                    .outer_direction_is_set(left, right)
                    .map_err(owner_score_error)?
            } else {
                self.projected_outer_direction_is_set(
                    projected_node,
                    projected_adjacent,
                    node,
                    adjacent,
                )
            };
            if direction_penalty
                && (owner_frame.is_some() || adjacent != node)
                // TALA gates the direction term on the edge itself or on
                // the selected outer replacement's container direction. A
                // direction on the temporary carrier is not sufficient:
                // edgeLength reads GetContainerDirection from outerNode.
                && (edge_is_directed || outer_direction_is_set)
            {
                let mut used = edge_direction;
                // Edge endpoints remain in declaration order, while a source
                // arrow reverses the semantic direction exposed by
                // Edge.DirectedEndpoints. Use that semantic source when
                // deciding whether this node's measured orientation must be
                // reversed, matching the upstream TALA edge-length score.
                let reverse_direction = if let Some(frame) = owner_frame {
                    let actual = frame
                        .edge(frame.canonical_edge(edge_id).map_err(owner_score_error)?)
                        .map_err(owner_score_error)?;
                    let semantic_from = if actual.source_arrow && !actual.target_arrow {
                        actual.to
                    } else {
                        actual.from
                    };
                    semantic_from == frame.canonical_node(node).map_err(owner_score_error)?
                } else {
                    let semantic_from = if edge_is_directed && edge.source_arrow {
                        edge.to
                    } else {
                        edge.from
                    };
                    (edge_from_is_node && self.active_node_is_aggregate(node))
                        || semantic_from == node
                };
                // Node.edgeLength compares the semantic edge endpoint with
                // the live `node` pointer.  A projected child edge may be
                // owned by the temporary parent vessel, but that pointer
                // identity does not make it equal to the vessel for this
                // direction reversal check.
                if reverse_direction {
                    used = used.opposite();
                }
                if cluster_endpoint {
                    used = match (desired, used) {
                        (Orientation::Left, Orientation::TopLeft | Orientation::BottomLeft) => {
                            Orientation::Left
                        }
                        (Orientation::Right, Orientation::TopRight | Orientation::BottomRight) => {
                            Orientation::Right
                        }
                        (Orientation::Top, Orientation::TopLeft | Orientation::TopRight) => {
                            Orientation::Top
                        }
                        (
                            Orientation::Bottom,
                            Orientation::BottomLeft | Orientation::BottomRight,
                        ) => Orientation::Bottom,
                        _ => used,
                    };
                }
                let compass = desired.compass();
                let used = used.compass();
                let mut delta = f64::from(compass_delta(compass, used));
                if !edge_is_directed {
                    delta = delta * 0.1 + compass_axis_delta(compass, used) * 0.9;
                }
                let cell_size = if let Some(frame) = receiver_frame {
                    frame.receiver_cell_size(node).map_err(owner_score_error)?
                } else {
                    self.cell_size
                };
                distance += delta * factor * cell_size * 0.25;
                if crate::internal::trace_env_enabled("TALA_TRACE_EDGE_DIRECTION_DETAIL")
                    && self.nodes[node.0 as usize].external_id == "table9"
                {
                    eprintln!(
                        "EDGE_DIRECTION_RUST edge={} adjacent={} desired={:?} used={:?} delta={} factor={} cell={} term={} total={}",
                        edge_id.0,
                        self.nodes[adjacent.0 as usize].external_id,
                        desired,
                        used,
                        delta,
                        factor,
                        cell_size,
                        delta * factor * cell_size * 0.25,
                        distance
                    );
                }
            }
            // edge.go HasLargeArrowheadLabel tests UTF-8 byte length on
            // the actual owner edge, independently of carrier label copies.
            let label_edge = if let Some(frame) = owner_frame {
                frame
                    .edge(frame.canonical_edge(edge_id).map_err(owner_score_error)?)
                    .map_err(owner_score_error)?
            } else {
                edge
            };
            let has_large_arrowhead_label = label_edge
                .source_arrowhead_label
                .as_ref()
                .is_some_and(|label| label.text.len() > 3)
                || label_edge
                    .target_arrowhead_label
                    .as_ref()
                    .is_some_and(|label| label.text.len() > 3);
            if has_large_arrowhead_label {
                if !edge_direction.is_horizontal() {
                    distance += self
                        .scoring_turn_cost_with_owner_frame_checked(node, receiver_frame)?
                        * 10.0;
                }
            } else if label_edge.label.is_some() && (!direction_penalty || !desired.is_vertical()) {
                let count = edge_label_counts
                    .iter()
                    .find_map(|(key, count)| {
                        (*key == (node_replacement_id, adjacent_replacement_id)).then_some(*count)
                    })
                    .unwrap_or(0);
                if count > 2 && !edge_direction.is_horizontal() {
                    distance += count as f64
                        * self.scoring_turn_cost_with_owner_frame_checked(node, receiver_frame)?;
                }
            }
            if trace_sized_detail {
                eprintln!(
                    "SIZED_DETAIL_RUST_EDGE_FINAL edge={} distance={} distance_bits=0x{:016x} direction={:?} desired={:?} factor={} directed={} outerDirection={}",
                    edge_id.0,
                    distance,
                    distance.to_bits(),
                    edge_direction,
                    desired,
                    factor,
                    edge_is_directed,
                    outer_direction_is_set,
                );
            }
            total += distance;
            if crate::internal::trace_env_enabled("TALA_TRACE_EDGE_DIRECTION_DETAIL")
                && self.nodes[node.0 as usize].external_id == "table9"
            {
                eprintln!(
                    "EDGE_TOTAL_RUST edge={} adjacent={} distance={} distance_bits=0x{:016x} total={} total_bits=0x{:016x}",
                    edge_id.0,
                    self.nodes[adjacent.0 as usize].external_id,
                    distance,
                    distance.to_bits(),
                    total,
                    total.to_bits(),
                );
            }
        }
        if trace_sized_detail {
            eprintln!(
                "SIZED_DETAIL_RUST_STEP node={} step=edge total={} bits=0x{:016x}",
                self.nodes[node.0 as usize].tala_id,
                total,
                total.to_bits(),
            );
        }
        // Source reads the scoring node's own Nears field. A canonical
        // allocated vessel does not inherit its retained members' maps.
        let bound_near_vessel = self
            .active_cluster_index(node)
            .filter(|index| self.clusters[*index].members.first() == Some(&node))
            .and_then(|index| self.topology_vessel_node(TopologyVessel::Cluster(index)))
            .or_else(|| {
                self.active_sequence_index(node)
                    .filter(|index| self.sequences[*index].members.first() == Some(&node))
                    .and_then(|index| self.topology_vessel_node(TopologyVessel::Sequence(index)))
            });
        let near_count;
        if let Some(frame) = owner_frame {
            let canonical = frame.canonical_node(node).map_err(owner_score_error)?;
            let near_map = frame
                .owner()
                .owner_node_near_map(canonical)
                .map_err(owner_score_error)?;
            near_count = near_map.len();
            if !near_map.is_empty() {
                let mut minimum = f64::INFINITY;
                let cursor = near_map.live_cursor();
                let mut index = 0;
                for near in cursor {
                    score_interval!(context, self, "node-near", Some(node), index)?;
                    index += 1;
                    let near = near
                        .expect("runtime error: invalid memory address or nil pointer dereference");
                    if frame.position(near).map_err(owner_score_error)?.is_none() {
                        minimum = 0.0;
                        continue;
                    }
                    minimum = super::owner_sizeless_geometry::source_min(
                        minimum,
                        super::owner_sizeless_geometry::sized_distance(frame, canonical, near),
                    );
                }
                total += minimum;
            }
        } else if let Some((owner, canonical)) = near_owner {
            // edge_length.go1276–1292 reads the actual shared Near map and
            // actual TopLeft pointers, including nodes outside this workspace.
            let near_map = owner
                .owner_node_near_map(canonical)
                .map_err(owner_score_error)?;
            near_count = near_map.len();
            if !near_map.is_empty() {
                let mut minimum = f64::INFINITY;
                let cursor = near_map.live_cursor();
                let mut index = 0;
                for near in cursor {
                    score_interval!(context, self, "node-near", Some(node), index)?;
                    index += 1;
                    let near = near
                        .expect("runtime error: invalid memory address or nil pointer dereference");
                    if owner.position(near).is_none() {
                        minimum = 0.0;
                        continue;
                    }
                    // A Near key denotes the actual pointer, even when it is
                    // a retained cluster/sequence member. Candidate placement
                    // projections must not replace either source Box here.
                    let distance = if let Some(frame) = route_owner_frame {
                        super::owner_sizeless_geometry::sized_distance(frame, canonical, near)
                    } else {
                        let first = owner.topology_node(canonical).ok_or_else(|| {
                            owner_score_error(OwnerTopologyError::UnknownNode(canonical))
                        })?;
                        let second = owner.topology_node(near).ok_or_else(|| {
                            owner_score_error(OwnerTopologyError::UnknownNode(near))
                        })?;
                        super::owner_sizeless_geometry::sized_box_distance(
                            (
                                owner.position(canonical).expect("positioned scoring node"),
                                first.rect.size,
                            ),
                            (
                                owner.position(near).expect("positioned Near target"),
                                second.rect.size,
                            ),
                        )
                    };
                    minimum = super::owner_sizeless_geometry::source_min(minimum, distance);
                }
                total += minimum;
            }
        } else if let Some(vessel) = bound_near_vessel.or_else(|| {
            // Ordinary canonical nodes also retain their exact Near keys.
            // A member key is not replaced by its containing vessel.
            self.topology_node_owner(node).map(|_| node)
        }) {
            let near_map = self
                .owner_node_near_map(vessel)
                .map_err(owner_score_error)?;
            near_count = near_map.len();
            if !near_map.is_empty() {
                let scoring_box = (
                    self.active_node_position(node).expect("positioned vessel"),
                    self.active_node_size(node),
                );
                let mut minimum = f64::INFINITY;
                let cursor = near_map.live_cursor();
                let mut index = 0;
                for near in cursor {
                    score_interval!(context, self, "node-near", Some(node), index)?;
                    index += 1;
                    let near = near
                        .expect("runtime error: invalid memory address or nil pointer dereference");
                    // Only an exact allocated vessel pointer denotes another
                    // numerical carrier. Original member keys stay original.
                    let near_carrier = self
                        .active_cluster_index(near)
                        .filter(|cluster| {
                            self.topology_vessel_node(TopologyVessel::Cluster(*cluster))
                                == Some(near)
                        })
                        .map(|cluster| self.clusters[cluster].members[0]);
                    let position = near_carrier.map_or_else(
                        || self.position(near),
                        |carrier| self.active_node_position(carrier),
                    );
                    let Some(position) = position else {
                        minimum = 0.0;
                        continue;
                    };
                    let size = near_carrier
                        .map_or(self.nodes[near.0 as usize].rect.size, |carrier| {
                            self.active_node_size(carrier)
                        });
                    minimum = minimum.min(Self::sized_box_border_distance(
                        scoring_box,
                        (position, size),
                    ));
                }
                total += minimum;
            }
        } else {
            let mut nears = self.nodes[node.0 as usize].nears.keys();
            if let Some(members) = self.active_aggregate_leaf_members(node) {
                for member in members {
                    nears.extend(self.nodes[member.0 as usize].nears.keys());
                }
            }
            nears = nears
                .into_iter()
                .map(|near| self.active_aggregate_owner(near))
                .filter(|near| *near != node)
                .collect();
            nears.sort_by_key(|near| self.nodes[near.0 as usize].tala_id);
            nears.dedup();
            near_count = nears.len();
            if !nears.is_empty() {
                let mut minimum = f64::INFINITY;
                for (near_index, near) in nears.into_iter().enumerate() {
                    score_interval!(context, self, "node-near", Some(node), near_index)?;
                    if trace_sized_detail {
                        eprintln!(
                            "SIZED_NEAR_RUST node={} near={} nodePos={:?} nearPos={:?} nodeSize={:?} nearSize={:?} distance={}",
                            self.nodes[node.0 as usize].tala_id,
                            self.nodes[near.0 as usize].tala_id,
                            self.position(node),
                            self.position(near),
                            self.active_node_size(node),
                            self.active_node_size(near),
                            self.sized_distance_to(node, near)
                        );
                    }
                    if self.position(near).is_none() {
                        minimum = 0.0;
                        continue;
                    }
                    minimum = minimum.min(self.sized_distance_to(node, near));
                }
                total += minimum;
                if trace_sized_detail {
                    eprintln!(
                        "SIZED_DETAIL_RUST_STEP node={} step=near minimum={} min_bits=0x{:016x} total={} total_bits=0x{:016x}",
                        self.nodes[node.0 as usize].tala_id,
                        minimum,
                        minimum.to_bits(),
                        total,
                        total.to_bits(),
                    );
                }
            }
        }
        // Routed candidates retain an owning frame too. Herd fields and
        // sibling headers belong to that actual node/Graph, not the carrier.
        let herd_distance = if let Some(frame) = receiver_frame {
            let canonical = frame.canonical_node(node).map_err(owner_score_error)?;
            if let Some((orientation, value)) = frame
                .owner()
                .owner_node_herd_geometry(canonical)
                .map_err(owner_score_error)?
            {
                let position = || -> Result<Point, limits::Error> {
                    Ok(frame
                        .position(canonical)
                        .map_err(owner_score_error)?
                        .expect("runtime error: invalid memory address or nil pointer dereference"))
                };
                let size = frame.node(canonical).map_err(owner_score_error)?.rect.size;
                Some(match orientation {
                    6 => value - (position()?.y + size.height),
                    4 => position()?.y - value,
                    7 => position()?.x - value,
                    5 => value - (position()?.x + size.width),
                    _ => 0.0,
                })
            } else {
                None
            }
        } else {
            self.herd_fence_distance(node)
        };
        if let Some(distance) = herd_distance {
            // Use source subtraction and the scored node's actual Graph
            // cell size, independently of the numerical workspace receiver.
            if distance >= 0.0 {
                total += distance;
            } else {
                let cell_size = if let Some(frame) = receiver_frame {
                    frame.receiver_cell_size(node).map_err(owner_score_error)?
                } else {
                    self.cell_size
                };
                total -= distance - cell_size;
            }
        }
        if trace_sized_detail {
            eprintln!(
                "SIZED_DETAIL_RUST_STEP node={} step=herd value={} bits=0x{:016x}",
                self.nodes[node.0 as usize].tala_id,
                total,
                total.to_bits(),
            );
        }
        // Go 1.27 emits a fused multiply-add for the source expression
        // `totalDistance += cost * (1-axisScore) * float64(len-1)` on arm64.
        // Keep the multiplication order while fusing the final multiply with
        // the accumulator; materializing `common_uncle_penalty` first rounds
        // one ULP differently on non-integral axis scores.
        if let Some(frame) = receiver_frame {
            let canonical = frame.canonical_node(node).map_err(owner_score_error)?;
            let receiver = frame.receiver_for_local(node).map_err(owner_score_error)?;
            if let Some(nodes) = frame
                .owner()
                .owner_common_uncle_values(receiver, canonical)
                .map_err(owner_score_error)?
            {
                let axis = super::owner_sizeless_geometry::axis_score(frame, &nodes);
                let cost = frame.receiver_cell_size(node).map_err(owner_score_error)?;
                total = (cost * (1.0 - axis)).mul_add((nodes.len() as i64 - 1) as f64, total);
            }
        } else if let Some(siblings) = self.common_uncle_siblings.get(&node) {
            let cost = if let Some(frame) = receiver_frame {
                frame.receiver_cell_size(node).map_err(owner_score_error)?
            } else {
                self.cell_size
            };
            let axis_score = self.axis_score(&siblings);
            total = (cost * (1.0 - axis_score)).mul_add((siblings.len() as i64 - 1) as f64, total);
        }
        if trace_sized_detail {
            eprintln!(
                "SIZED_DETAIL_RUST_STEP node={} step=uncle value={} bits=0x{:016x}",
                self.nodes[node.0 as usize].tala_id,
                total,
                total.to_bits(),
            );
        }
        // evaluateNodeEdgeLength's final direct Err is before flow continuity.
        score_poll!(context, self, "node-edge-length-final", Some(node), None)?;
        let flow_continuity = if let Some(frame) = owner_frame {
            self.flow_continuity_with_owner_frame_checked(
                node,
                restored_endpoints.as_deref(),
                frame,
            )?
        } else {
            self.flow_continuity_cost(
                node,
                &active_edges,
                restored_endpoints.as_deref(),
                route_owner_frame.filter(|_| owner_carrier_positions),
                route_restored_endpoints
                    .as_ref()
                    .and_then(|values| values.as_deref()),
            )?
        };
        total += flow_continuity;
        if trace_sized_detail {
            eprintln!(
                "SIZED_DETAIL_RUST_TOTAL node={} total={} total_bits=0x{:016x} herd={} herd_bits=0x{:016x} commonUncle={} common_bits=0x{:016x} flow={} nears={}",
                self.nodes[node.0 as usize].tala_id,
                total,
                total.to_bits(),
                self.herd_penalty(node),
                self.herd_penalty(node).to_bits(),
                self.common_uncle_penalty(node, true),
                self.common_uncle_penalty(node, true).to_bits(),
                flow_continuity,
                near_count,
            );
            eprintln!(
                "SIZED_DETAIL_RUST_UNCLE node={} siblings={:?}",
                self.nodes[node.0 as usize].tala_id,
                self.common_uncle_siblings
                    .get(&node)
                    .map(|siblings| siblings
                        .iter()
                        .map(|sibling| self.nodes[sibling.0 as usize].tala_id)
                        .collect::<Vec<_>>()),
            );
            if let Some(siblings) = self.common_uncle_siblings.get(&node) {
                eprintln!(
                    "SIZED_DETAIL_RUST_UNCLE_DETAIL node={} axis={} axis_bits=0x{:016x} positions={:?}",
                    self.nodes[node.0 as usize].tala_id,
                    self.axis_score(&siblings),
                    self.axis_score(&siblings).to_bits(),
                    siblings
                        .iter()
                        .map(|sibling| (
                            self.nodes[sibling.0 as usize].tala_id,
                            self.position(*sibling)
                        ))
                        .collect::<Vec<_>>(),
                );
            }
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_EDGE_TOTAL_RUST") {
            eprintln!(
                "EDGE_TOTAL_RUST node={} pos={:?} total={:.17} edges={}",
                self.nodes[node.0 as usize].tala_id,
                self.active_node_position(node),
                total,
                self.active_edge_count(node),
            );
        }
        Ok(total)
    }

    /// flow_continuity.go19–102 uses actual node/edge pointers, raw Container
    /// fields and the live Graph.Containers length. A historical container
    /// flag or an equal Tala ID must not replace either of those predicates.
    fn flow_continuity_with_owner_frame_checked(
        &self,
        local: NodeId,
        restored: Option<&[SizedRestoredEndpoints]>,
        frame: &OwnerScoreFrame<'_>,
    ) -> Result<f64, limits::Error> {
        let canonical = frame.canonical_node(local).map_err(owner_score_error)?;
        let node = frame.node(canonical).map_err(owner_score_error)?;
        let receiver = frame.receiver_for_local(local).map_err(owner_score_error)?;
        let containers = frame
            .owner()
            .owner_container_map(receiver)
            .expect("validated live receiver has its actual container map");
        if node.position.is_none()
            || node.edges.len() < 2
            || node.edges.len() > 8
            || node.cluster.is_some()
            || node.sequence.is_some()
            || frame.owner().has_herd_assignment(canonical)
            || containers
                .get(Some(canonical))
                .is_some_and(|header| header.len() != 0)
        {
            return Ok(0.0);
        }
        const INCOMING: u8 = 1;
        const OUTGOING: u8 = 2;
        struct Ray {
            node: NodeId,
            x: f64,
            y: f64,
            directions: u8,
        }
        let mut rays = Vec::<Ray>::with_capacity(8);
        let position = node.position.expect("flow receiver is positioned");
        let center = Point {
            x: position.x + node.rect.size.width / 2.0,
            y: position.y + node.rect.size.height / 2.0,
        };
        for (index, &edge_id) in node.edges.read().iter().enumerate() {
            let edge = frame.edge(edge_id).map_err(owner_score_error)?;
            let replacements = restored
                .and_then(|values| values.get(index))
                .copied()
                .unwrap_or_default();
            let replacement = replacements
                .node
                .and_then(|reference| reference.source_node)
                .unwrap_or(canonical);
            if edge.is_invisible()
                || edge.from == edge.to
                || edge.has_table_column()
                || edge.source_arrow == edge.target_arrow
                || replacement != canonical
            {
                continue;
            }
            let current_adjacent = if edge.from == canonical {
                edge.to
            } else {
                edge.from
            };
            let adjacent = replacements
                .adjacent
                .and_then(|reference| reference.source_node)
                .unwrap_or(current_adjacent);
            let adjacent_node = frame.node(adjacent).map_err(owner_score_error)?;
            let Some(adjacent_position) = adjacent_node.position else {
                continue;
            };
            if adjacent_node.container != node.container {
                continue;
            }
            let incoming = if edge.source_arrow {
                edge.to != canonical
            } else {
                edge.to == canonical
            };
            let direction = if incoming { INCOMING } else { OUTGOING };
            if let Some(ray) = rays.iter_mut().find(|ray| ray.node == adjacent) {
                ray.directions |= direction;
                continue;
            }
            let x = adjacent_position.x + adjacent_node.rect.size.width / 2.0 - center.x;
            let y = adjacent_position.y + adjacent_node.rect.size.height / 2.0 - center.y;
            let length = crate::internal::routing::channel_points::go_hypot(x, y);
            if length == 0.0 {
                continue;
            }
            rays.push(Ray {
                node: adjacent,
                x: x / length,
                y: y / length,
                directions: direction,
            });
        }
        let mut spine = f64::INFINITY;
        let mut branch_sum = 0.0;
        let mut branches = 0usize;
        for first in 0..rays.len() {
            for second in first + 1..rays.len() {
                let dot = rays[first]
                    .y
                    .mul_add(rays[second].y, rays[first].x * rays[second].x);
                let dot = routing::channel_solver::maximum(
                    -1.0,
                    routing::channel_solver::minimum(1.0, dot),
                );
                let a = rays[first].directions;
                let b = rays[second].directions;
                if a & INCOMING != 0 && b & OUTGOING != 0 || a & OUTGOING != 0 && b & INCOMING != 0
                {
                    spine = routing::channel_solver::minimum(spine, 1.0 + dot);
                }
                if a & b != 0 {
                    branch_sum += routing::channel_solver::maximum(0.0, 2.0 * dot - 1.0);
                    branches += 1;
                }
            }
        }
        let mut cost = 0.0;
        if spine != f64::INFINITY {
            cost += spine;
        }
        if branches > 0 {
            cost += branch_sum / branches as f64;
        }
        Ok(frame.receiver_turn_cost(local).map_err(owner_score_error)? * cost)
    }

    fn flow_continuity_is_inert(&self, node: NodeId, edge_count: usize) -> bool {
        let node_ref = &self.nodes[node.0 as usize];
        self.position(node).is_none()
            || !(2..=8).contains(&edge_count)
            || node_ref.cluster.is_some()
            // AddSequence replaces its members with a vessel whose Sequence
            // pointer is nil in the recovered Go graph. The stable arena
            // keeps the owner member's historical pointer, so permit flow
            // scoring for an active sequence vessel while continuing to skip
            // a detached ordinary sequence member.
            || (node_ref.sequence.is_some() && !self.active_node_is_aggregate(node))
            || self.has_herd_assignment(node)
            // A placement scope may contain a container carrier without
            // materializing its descendants in the temporary graph. OSS
            // TALA still sees those descendants through Graph.Containers and
            // therefore excludes the carrier from flow continuity scoring.
            || node_ref.is_container
            || self
                .containers
                .get(&Some(node))
                .is_some_and(|children| !children.is_empty())
    }

    /// Current OSS TALA's experimental flow-continuity placement preference.
    ///
    /// An incoming and outgoing ray through a node should form a recognizable
    /// continuation, while same-role branches should leave enough angular
    /// room. The completed-layout score is unchanged; this only participates
    /// in sized candidate placement and therefore must use the live candidate
    /// geometry on every call.
    fn flow_continuity_cost(
        &self,
        node: NodeId,
        active_edges: &[EdgeId],
        restored_endpoints: Option<&[SizedRestoredEndpoints]>,
        carrier_frame: Option<&OwnerScoreFrame<'_>>,
        owner_replacements: Option<&[SizedRestoredEndpoints]>,
    ) -> Result<f64, limits::Error> {
        // The numerical carrier and the actual vessel are distinct nodes.
        // Source flow_continuity.go reads the latter's raw fields and pointer
        // identities; active membership alone cannot erase a member backlink.
        if let Some(index) = self.active_cluster_index(node)
            && self.clusters[index].members.first() == Some(&node)
            && let Some(vessel) = self.topology_vessel_node(TopologyVessel::Cluster(index))
            && self.topology_node_owner(vessel).is_some()
        {
            let mapping = &self
                .owner_graph_view(OwnerGraphId(0))
                .expect("original owning graph")
                .local_to_canonical;
            let local = NodeId(
                mapping
                    .iter()
                    .position(|&n| n == vessel)
                    .expect("bound vessel in owning mapping") as u32,
            );
            let frame = OwnerScoreFrame::new(
                self,
                self.topology_arena_id().expect("owning arena"),
                OwnerGraphId(0),
                mapping,
            )
            .expect("original owning flow frame");
            return Ok(self
                .flow_continuity_with_owner_frame_checked(local, restored_endpoints, &frame)
                .expect("bound vessel flow operands"));
        }
        let node_ref = &self.nodes[node.0 as usize];
        let trace_flow_node = crate::internal::trace_env_value("TALA_TRACE_FLOW_NODE");
        let trace_flow = crate::internal::trace_env_enabled("TALA_TRACE_FLOW_DETAIL")
            && trace_flow_node.as_deref().is_none_or(|target| {
                target == "all"
                    || target.parse::<i64>().ok() == Some(self.nodes[node.0 as usize].tala_id)
            });
        if trace_flow {
            eprintln!(
                "FLOW_GUARD_RUST node={} pos={} edges={} cluster={} sequence={} herd={} isContainer={} children={}",
                self.nodes[node.0 as usize].tala_id,
                self.position(node).is_some(),
                active_edges.len(),
                node_ref.cluster.is_some(),
                node_ref.sequence.is_some(),
                node_ref.herd_assignment.is_some(),
                node_ref.is_container,
                self.containers
                    .get(&Some(node))
                    .map_or(0, |children| children.len()),
            );
        }
        if self.flow_continuity_is_inert(node, active_edges.len()) {
            return Ok(0.0);
        }

        const INCOMING: u8 = 1;
        const OUTGOING: u8 = 2;
        #[derive(Clone, Copy)]
        struct Ray {
            identity: i64,
            x: f64,
            y: f64,
            directions: u8,
        }

        // The adapter retains replacement eligibility, edge order, dimensions
        // and receiver costs. Only live coordinates move onto the owner view:
        // retained group-member Container decorations are still interpreted
        // by the projection below, including when no group is materialized.
        let position = |local| {
            carrier_frame.map_or_else(
                || Ok(self.active_node_position(local)),
                |frame| frame.local_position(local).map_err(owner_score_error),
            )
        };
        let node_position = position(node)?.unwrap();
        let node_size = self.active_node_size(node);
        let node_center = Point {
            x: node_position.x + node_size.width / 2.0,
            y: node_position.y + node_size.height / 2.0,
        };
        // Induced placement scopes omit the owning container from their
        // local topology, but Go retains that pointer for flow eligibility.
        let node_container = node_ref.scoring_container_parent.or_else(|| {
            self.active_node_container(node)
                .map(|container| self.nodes[container.0 as usize].tala_id)
        });
        let mut rays: Vec<Ray> = Vec::with_capacity(active_edges.len());

        for (edge_index, edge_id) in active_edges.iter().copied().enumerate() {
            let edge = &self.edge_records[edge_id.0 as usize];
            if edge.is_invisible()
                || edge.from == edge.to
                || edge.has_table_column()
                || edge.source_arrow == edge.target_arrow
            {
                continue;
            }
            let restored = restored_endpoints
                .and_then(|endpoints| endpoints.get(edge_index))
                .copied()
                .unwrap_or_default();
            if trace_flow {
                eprintln!(
                    "FLOW_RESTORED_RUST edge={} adjacent={} nodeContainer={:?} restoredNode={:?} restoredAdjacent={:?} adjacentContainer={:?}",
                    edge_id.0,
                    self.nodes[self.active_adjacent(node, edge_id).0 as usize].tala_id,
                    node_container,
                    restored.node.and_then(|projected| projected
                        .owner
                        .map(|owner| self.nodes[owner.0 as usize].tala_id)
                        .or(projected.owner_tala_id)),
                    restored.adjacent.map(|projected| projected.tala_id),
                    restored
                        .adjacent
                        .and_then(|projected| projected.container_tala_id),
                );
            }
            // OSS flowContinuityCost ignores an edge whose receiver endpoint
            // was replaced by an abducted original node (s.nRepl[i] != node).
            if restored.node.is_some() {
                continue;
            }
            let adjacent = self.active_adjacent(node, edge_id);
            let adjacent_box = if let Some(projected) = restored.adjacent {
                // Flow continuity reads the replacement node pointer itself,
                // just like Go's flowContinuityCost. A retained cluster
                // member can be one pixel on either side of its ideal
                // half-cell center after repeated SyncGeometry calls; using
                // the projected vessel frame here would erase that state.
                let Some(mut projected_box) = self
                    .abducted_node_projection(projected)
                    .and_then(|projection| self.sized_projected_box(projection))
                else {
                    continue;
                };
                if projected.cluster_member
                    && let Some(member) = self.clusters.iter().find_map(|cluster| {
                        cluster.members.iter().copied().find(|member| {
                            self.nodes[member.0 as usize].tala_id == projected.tala_id
                        })
                    })
                    && let Some(current) = self.position(member)
                {
                    projected_box.0 = current;
                }
                if let Some(frame) = carrier_frame {
                    let source = owner_replacements
                        .and_then(|values| values.get(edge_index))
                        .and_then(|value| value.adjacent)
                        .and_then(|reference| reference.source_node)
                        .ok_or_else(|| owner_score_error("flow endpoint has no owner binding"))?;
                    let Some(current) = frame.position(source).map_err(owner_score_error)? else {
                        continue;
                    };
                    projected_box.0 = current;
                }
                projected_box
            } else {
                let Some(position) = position(adjacent)? else {
                    continue;
                };
                (position, self.active_node_size(adjacent))
            };
            if trace_flow {
                eprintln!(
                    "FLOW_BOX_RUST edge={} projected={:?} box={:?} currentMember={:?}",
                    edge_id.0,
                    restored.adjacent.map(|projected| projected.tala_id),
                    adjacent_box,
                    restored
                        .adjacent
                        .and_then(|projected| {
                            self.clusters.iter().find_map(|cluster| {
                                cluster.members.iter().copied().find(|member| {
                                    self.nodes[member.0 as usize].tala_id == projected.tala_id
                                })
                            })
                        })
                        .and_then(|member| self.position(member)),
                );
            }
            // OSS flow continuity compares the replacement endpoint's own
            // `Container` field. AddCluster and AddSequence clear that field
            // on their member nodes, even though the stable Rust arena keeps
            // the member's original parent for other projections.
            let adjacent_container = if let Some(projected) = restored.adjacent {
                let is_grouped_member = projected.cluster_member
                    || self.sequences.iter().any(|sequence| {
                        sequence.members.iter().any(|member| {
                            self.nodes[member.0 as usize].tala_id == projected.tala_id
                        })
                    });
                if is_grouped_member {
                    None
                } else {
                    projected.container_tala_id
                }
            } else {
                self.nodes[adjacent.0 as usize]
                    .scoring_container_parent
                    .or_else(|| {
                        self.active_node_container(adjacent)
                            .map(|container| self.nodes[container.0 as usize].tala_id)
                    })
            };
            if adjacent_container != node_container {
                continue;
            }
            if trace_flow {
                eprintln!(
                    "FLOW_DETAIL_RUST adjacent={} nodeContainer={:?} adjacentContainer={:?} sourceArrow={} targetArrow={} restored={:?}",
                    self.nodes[adjacent.0 as usize].tala_id,
                    node_container,
                    adjacent_container,
                    edge.source_arrow,
                    edge.target_arrow,
                    restored,
                );
            }
            let adjacent_center = Point {
                x: adjacent_box.0.x + adjacent_box.1.width / 2.0,
                y: adjacent_box.0.y + adjacent_box.1.height / 2.0,
            };
            let dx = adjacent_center.x - node_center.x;
            let dy = adjacent_center.y - node_center.y;
            // Go 1.27 on the pinned ARM64 oracle uses the portable
            // correctly-rounded math path for Hypot/Sqrt. Keep this call
            // source-shaped instead of relying on Rust's platform sqrt.
            let length = crate::internal::routing::channel_points::go_hypot(dx, dy);
            if length == 0.0 {
                continue;
            }
            let incoming = if self.active_aggregate_owner(edge.to) == node {
                !edge.source_arrow
            } else {
                edge.source_arrow
            };
            let directions = if incoming { INCOMING } else { OUTGOING };
            let identity = restored
                .adjacent
                .map(|projected| projected.tala_id)
                .unwrap_or(self.nodes[adjacent.0 as usize].tala_id);
            if let Some(existing) = rays.iter_mut().find(|ray| ray.identity == identity) {
                existing.directions |= directions;
            } else {
                rays.push(Ray {
                    identity,
                    x: dx / length,
                    y: dy / length,
                    directions,
                });
            }
        }

        let mut spine = f64::INFINITY;
        let mut branch_sum: f64 = 0.0;
        let mut branches = 0usize;
        for first in 0..rays.len() {
            for second in first + 1..rays.len() {
                // The pinned Go 1.27 oracle evaluates the dot product through
                // a fused multiply-add on ARM64. Preserve that operation order
                // so the score retains the same IEEE-754 bits.
                // Go's arm64 lowering multiplies the x components first and
                // then fuses the y product into that partial sum. FMA is
                // sensitive to which product is fused, so preserve this
                // operand order rather than using the algebraically symmetric
                // x-product FMA.
                let dot = rays[first]
                    .y
                    .mul_add(rays[second].y, rays[first].x * rays[second].x)
                    .clamp(-1.0, 1.0);
                let first_directions = rays[first].directions;
                let second_directions = rays[second].directions;
                if (first_directions & INCOMING != 0 && second_directions & OUTGOING != 0)
                    || (first_directions & OUTGOING != 0 && second_directions & INCOMING != 0)
                {
                    spine = spine.min(1.0 + dot);
                }
                if first_directions & second_directions != 0 {
                    branch_sum += (2.0 * dot - 1.0).max(0.0);
                    branches += 1;
                }
            }
        }

        let mut cost = if spine.is_finite() { spine } else { 0.0 };
        if branches > 0 {
            cost += branch_sum / branches as f64;
        }
        let total = self.turn_cost_value() * cost;
        if trace_flow {
            eprintln!(
                "FLOW_RAYS_RUST node={} pos={:?} count={} spine={} spine_bits=0x{:016x} branchSum={} branches={} turn={} turn_bits=0x{:016x} cost={} cost_bits=0x{:016x}",
                self.nodes[node.0 as usize].tala_id,
                self.active_node_position(node),
                rays.len(),
                spine,
                spine.to_bits(),
                branch_sum,
                branches,
                self.raw_turn_cost(),
                self.raw_turn_cost().to_bits(),
                total,
                total.to_bits(),
            );
            for (index, ray) in rays.iter().enumerate() {
                eprintln!(
                    "FLOW_RAY_RUST node={} index={} identity={} x={} x_bits=0x{:016x} y={} y_bits=0x{:016x} directions={}",
                    self.nodes[node.0 as usize].tala_id,
                    index,
                    ray.identity,
                    ray.x,
                    ray.x.to_bits(),
                    ray.y,
                    ray.y.to_bits(),
                    ray.directions,
                );
            }
        }
        Ok(total)
    }

    // Recovered `obstructed` guard used by computeSymmetryScore. A mirrored
    // pair does not earn symmetry credit when another sibling lies on either
    // ray from the center, except across ancestor/descendant boundaries.
    pub(super) fn symmetry_pair_obstructed(&self, center: NodeId, a: NodeId, b: NodeId) -> bool {
        self.symmetry_pair_obstructed_checked(center, a, b, ScoreContext::none(), &mut Vec::new())
            .expect("unchecked scoring context cannot cancel")
    }

    /// `container_orders` keeps the sibling orders already read during one
    /// center's pair scan. They are reused only under a context whose polls
    /// cannot mutate the owner, where the order cannot change in between.
    fn symmetry_pair_obstructed_checked(
        &self,
        center: NodeId,
        a: NodeId,
        b: NodeId,
        context: ScoreContext<'_>,
        container_orders: &mut Vec<(Option<NodeId>, Vec<NodeId>)>,
    ) -> Result<bool, limits::Error> {
        let reuse_orders = context.stable_owner();
        let center_scope = self.nodes[center.0 as usize].container;
        let mut sibling_scopes = vec![center_scope];
        let a_scope = self.nodes[a.0 as usize].container;
        if a_scope != center_scope {
            sibling_scopes.push(a_scope);
        }
        let geometry = |node| {
            let position = self
                .active_node_position(node)
                .expect("positioned active node");
            let size = self.active_node_size(node);
            (
                Point {
                    x: position.x + size.width * 0.5,
                    y: position.y + size.height * 0.5,
                },
                SymmetryObstructionBounds::from_box(Some(position), size),
            )
        };
        let (center_point, bounds) = geometry(center);
        let (a_point, a_bounds) = geometry(a);
        let (b_point, b_bounds) = geometry(b);
        let bounds = bounds.including(a_bounds).including(b_bounds);
        // The three endpoints' ownership chains do not change during the
        // scan: walk each once, on first use, rather than once per sibling.
        let endpoint_chains = std::cell::OnceCell::new();
        for scope in sibling_scopes {
            let fresh_order;
            let order: &[NodeId] = if reuse_orders {
                match container_orders
                    .iter()
                    .position(|(cached, _)| *cached == scope)
                {
                    Some(index) => &container_orders[index].1,
                    None => {
                        container_orders.push((scope, self.container_node_order(scope)));
                        &container_orders.last().expect("just pushed").1
                    }
                }
            } else {
                fresh_order = self.container_node_order(scope);
                &fresh_order
            };
            for (sibling_index, sibling) in order.iter().copied().enumerate() {
                score_interval!(
                    context,
                    self,
                    "symmetry-obstruction",
                    Some(center),
                    sibling_index
                )?;
                if sibling == center
                    || sibling == a
                    || sibling == b
                    || self.position(sibling).is_none()
                {
                    continue;
                }
                if bounds.excludes(
                    self.active_node_position(sibling),
                    self.active_node_size(sibling),
                ) {
                    continue;
                }
                if endpoint_chains
                    .get_or_init(|| {
                        [
                            self.scope_ancestor_chain(center),
                            self.scope_ancestor_chain(a),
                            self.scope_ancestor_chain(b),
                        ]
                    })
                    .iter()
                    .any(|chain| chain.contains(&sibling))
                    || self.is_descendant_of_any_scope(sibling, &[center, a, b])
                {
                    continue;
                }
                if self.symmetry_segment_intersects_node(center_point, a_point, sibling)
                    || self.symmetry_segment_intersects_node(center_point, b_point, sibling)
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// `computeSymmetryScore` receives the original pointer-keyed sibling
    /// slices, even when the current placement graph has collapsed those
    /// children onto a container vessel.  In that case the projected owner
    /// is the vessel, not the original node's container; using the owner's
    /// `container` would inspect the parent scope and miss the real sibling
    /// obstruction.  Reuse the retained external-child snapshots keyed by
    /// the original `container_tala_id`, which is the Rust equivalent of
    /// `Graph.Containers[node.Container]`.
    ///
    /// `retained` caches the neighbor's identity matches among the retained
    /// scoring nodes. Those are plain graph fields, unchanged by polls, so one
    /// gathering serves every sibling of the obstruction query.
    fn symmetry_neighbor_is_descendant_of<'s>(
        &'s self,
        neighbor: SymmetryNeighbor,
        ancestor_tala_id: i64,
        retained: &std::cell::OnceCell<Vec<&'s ArenaNode>>,
    ) -> bool {
        if neighbor.identity == ancestor_tala_id {
            return false;
        }
        if neighbor.container_tala_id == Some(ancestor_tala_id) {
            return true;
        }
        let matches_ancestor = |candidate: &&ArenaNode| {
            candidate.scoring_container_parent == Some(ancestor_tala_id)
                || candidate
                    .scoring_container_ancestors
                    .contains(&ancestor_tala_id)
        };
        retained
            .get_or_init(|| {
                self.retained_scoring_nodes()
                    .filter(|candidate| candidate.tala_id == neighbor.identity)
                    .collect()
            })
            .iter()
            .any(matches_ancestor)
    }

    fn symmetry_neighbor_has_ancestor_relation<'s>(
        &'s self,
        neighbor: SymmetryNeighbor,
        sibling: &ArenaNode,
        retained: &std::cell::OnceCell<Vec<&'s ArenaNode>>,
    ) -> bool {
        self.symmetry_neighbor_is_descendant_of(neighbor, sibling.tala_id, retained)
            || (sibling.tala_id != neighbor.identity
                && (sibling.scoring_container_parent == Some(neighbor.identity)
                    || sibling
                        .scoring_container_ancestors
                        .contains(&neighbor.identity)))
    }

    fn symmetry_pair_obstructed_neighbors_checked(
        &self,
        center: SymmetryNeighbor,
        a: SymmetryNeighbor,
        b: SymmetryNeighbor,
        context: ScoreContext<'_>,
    ) -> Result<bool, limits::Error> {
        let trace_pair_target = crate::internal::trace_env_value("TALA_TRACE_SYMMETRY_PAIR")
            .is_some_and(|targets| {
                targets.split(',').any(|target| {
                    target == "all" || target.parse::<i64>().ok() == Some(center.identity)
                })
            });
        let center_point = Point {
            x: center.positioned().x + center.size.width * 0.5,
            y: center.positioned().y + center.size.height * 0.5,
        };
        let a_point = Point {
            x: a.positioned().x + a.size.width * 0.5,
            y: a.positioned().y + a.size.height * 0.5,
        };
        let b_point = Point {
            x: b.positioned().x + b.size.width * 0.5,
            y: b.positioned().y + b.size.height * 0.5,
        };
        let bounds = SymmetryObstructionBounds::from_box(center.position, center.size)
            .including(SymmetryObstructionBounds::from_box(a.position, a.size))
            .including(SymmetryObstructionBounds::from_box(b.position, b.size));
        let (center_retained, a_retained, b_retained) = (
            std::cell::OnceCell::new(),
            std::cell::OnceCell::new(),
            std::cell::OnceCell::new(),
        );
        let mut scopes = vec![center.container_tala_id, a.container_tala_id];
        scopes.dedup();
        for scope in scopes {
            if let Some(scope_tala_id) = scope
                && let Some(children) = self
                    .transaction_external_container_children
                    .get(&scope_tala_id)
            {
                for (sibling_index, sibling) in children.iter().enumerate() {
                    score_interval!(
                        context,
                        self,
                        "symmetry-obstruction",
                        Some(center.owner),
                        sibling_index
                    )?;
                    if sibling.tala_id == center.identity
                        || sibling.tala_id == a.identity
                        || sibling.tala_id == b.identity
                    {
                        continue;
                    }
                    // External child snapshots may deliberately retain a nil
                    // position while the live temporary graph owns the
                    // current geometry. Go's pointer record still observes
                    // that live TopLeft during symmetry scoring, so resolve
                    // the stable identity back to the active arena before
                    // discarding an otherwise valid obstruction.
                    let position = sibling.position.or_else(|| {
                        self.nodes
                            .iter()
                            .find(|candidate| candidate.tala_id == sibling.tala_id)
                            .and_then(|candidate| candidate.position)
                    });
                    let Some(position) = position else {
                        continue;
                    };
                    let sibling_size = if sibling.position.is_some() {
                        sibling.rect.size
                    } else {
                        self.nodes
                            .iter()
                            .enumerate()
                            .find(|(_, candidate)| candidate.tala_id == sibling.tala_id)
                            .map(|(index, _)| self.active_node_size(NodeId(index as u32)))
                            .unwrap_or(sibling.rect.size)
                    };
                    if bounds.excludes(Some(position), sibling_size) {
                        continue;
                    }
                    // symmetry.go rejects separated boxes before all six
                    // ancestry directions. A projected owner's identity is
                    // only its carrier; keep the retained hierarchy checks.
                    if self.symmetry_neighbor_has_ancestor_relation(
                        center,
                        sibling,
                        &center_retained,
                    ) || self.symmetry_neighbor_has_ancestor_relation(a, sibling, &a_retained)
                        || self.symmetry_neighbor_has_ancestor_relation(b, sibling, &b_retained)
                    {
                        continue;
                    }
                    let hit_a =
                        Self::segment_intersects_box(center_point, a_point, position, sibling_size);
                    let hit_b =
                        Self::segment_intersects_box(center_point, b_point, position, sibling_size);
                    if hit_a || hit_b {
                        if trace_pair_target {
                            eprintln!(
                                "SYMMETRY_BLOCKER_RUST center={} scope={} sibling={} pos={},{} size={},{} ray_a={} ray_b={}",
                                center.identity,
                                scope_tala_id,
                                sibling.tala_id,
                                position.x,
                                position.y,
                                sibling_size.width,
                                sibling_size.height,
                                hit_a,
                                hit_b,
                            );
                        }
                        return Ok(true);
                    }
                }
                continue;
            }
            // A non-projected scope still has its ordinary arena inventory;
            // retain the existing descendant-aware implementation as the
            // fallback for scopes that do not have an external snapshot.
            let scope_node = scope.and_then(|scope_tala_id| {
                self.nodes
                    .iter()
                    .position(|node| node.tala_id == scope_tala_id)
                    .map(|index| NodeId(index as u32))
            });
            if let Some(scope_node) = scope_node {
                for (sibling_index, sibling) in self
                    .container_node_order(Some(scope_node))
                    .into_iter()
                    .enumerate()
                {
                    score_interval!(
                        context,
                        self,
                        "symmetry-obstruction",
                        Some(center.owner),
                        sibling_index
                    )?;
                    // symmetry.obstructed excludes the participant pointers.
                    // This live sibling slot can represent a cluster/sequence
                    // vessel, whose identity differs from its retained member.
                    let sibling_identity = self.active_node_tala_id(sibling);
                    if sibling_identity == center.identity
                        || sibling_identity == a.identity
                        || sibling_identity == b.identity
                    {
                        continue;
                    }
                    let sibling_ref = &self.nodes[sibling.0 as usize];
                    if bounds.excludes(self.position(sibling), self.active_node_size(sibling)) {
                        continue;
                    }
                    if self.symmetry_neighbor_has_ancestor_relation(
                        center,
                        sibling_ref,
                        &center_retained,
                    ) || self.symmetry_neighbor_has_ancestor_relation(
                        a,
                        sibling_ref,
                        &a_retained,
                    ) || self.symmetry_neighbor_has_ancestor_relation(
                        b,
                        sibling_ref,
                        &b_retained,
                    ) {
                        continue;
                    }
                    if Self::segment_intersects_box(
                        center_point,
                        a_point,
                        self.position(sibling).unwrap(),
                        self.active_node_size(sibling),
                    ) || Self::segment_intersects_box(
                        center_point,
                        b_point,
                        self.position(sibling).unwrap(),
                        self.active_node_size(sibling),
                    ) {
                        if trace_pair_target {
                            let position = self.position(sibling).unwrap();
                            let size = self.active_node_size(sibling);
                            eprintln!(
                                "SYMMETRY_BLOCKER_RUST center={} scope={} sibling={} pos={},{} size={},{}",
                                center.identity,
                                scope.unwrap_or(0),
                                self.nodes[sibling.0 as usize].tala_id,
                                position.x,
                                position.y,
                                size.width,
                                size.height,
                            );
                        }
                        return Ok(true);
                    }
                }
            } else {
                for (sibling_index, sibling) in
                    self.container_node_order(None).into_iter().enumerate()
                {
                    score_interval!(
                        context,
                        self,
                        "symmetry-obstruction",
                        Some(center.owner),
                        sibling_index
                    )?;
                    if sibling == center.owner || sibling == a.owner || sibling == b.owner {
                        continue;
                    }
                    let sibling_ref = &self.nodes[sibling.0 as usize];
                    if bounds.excludes(self.position(sibling), self.active_node_size(sibling)) {
                        continue;
                    }
                    if self.symmetry_neighbor_has_ancestor_relation(
                        center,
                        sibling_ref,
                        &center_retained,
                    ) || self.symmetry_neighbor_has_ancestor_relation(
                        a,
                        sibling_ref,
                        &a_retained,
                    ) || self.symmetry_neighbor_has_ancestor_relation(
                        b,
                        sibling_ref,
                        &b_retained,
                    ) {
                        continue;
                    }
                    if Self::segment_intersects_box(
                        center_point,
                        a_point,
                        self.position(sibling).unwrap(),
                        self.active_node_size(sibling),
                    ) || Self::segment_intersects_box(
                        center_point,
                        b_point,
                        self.position(sibling).unwrap(),
                        self.active_node_size(sibling),
                    ) {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }

    pub(super) fn symmetry_pair_score(
        &self,
        center: NodeId,
        neighbors: &[NodeId],
    ) -> (f64, BTreeSet<NodeId>) {
        let neighbors = neighbors
            .iter()
            .copied()
            .map(|owner| self.direct_symmetry_neighbor(owner))
            .collect::<Vec<_>>();
        let (score, matched) = self
            .symmetry_pair_score_from_center_checked(
                self.direct_symmetry_neighbor(center),
                &neighbors,
                ScoreContext::none(),
            )
            .expect("unchecked scoring context cannot cancel");
        (
            score,
            neighbors
                .into_iter()
                .zip(matched.iter().copied())
                .filter_map(|(neighbor, is_matched)| is_matched.then_some(neighbor.owner))
                .collect::<BTreeSet<_>>(),
        )
    }

    fn direct_symmetry_neighbor(&self, owner: NodeId) -> SymmetryNeighbor {
        SymmetryNeighbor {
            identity: self.nodes[owner.0 as usize].tala_id,
            owner,
            projected: false,
            // Temporary placement graphs clear the arena `container` pointer
            // while retaining the source hierarchy in
            // `scoring_container_parent`. Go's Node.Container still points
            // at the enclosing vessel during optimizer scoring, so use the
            // recovered parent as the equivalent sibling scope.
            container_tala_id: self.nodes[owner.0 as usize]
                .container
                .map(|container| self.nodes[container.0 as usize].tala_id)
                .or(self.nodes[owner.0 as usize].scoring_container_parent),
            // Temporary TALA graphs expose aggregate vessels at their live
            // vessel position while retaining stable arena members.  Use the
            // active position here so symmetry sees the same synthetic box
            // that Go's Node.getSymmetry sees.
            position: self.active_node_position(owner),
            size: self.active_node_size(owner),
        }
    }

    fn projected_symmetry_neighbor(&self, projected: ProjectedAdjacent) -> SymmetryNeighbor {
        let owner = self.active_node_position(projected.owner);
        // AddSequence and AddCluster clear each member's Node.Container and
        // attach ancestry through the temporary vessel instead. Keep that
        // direct-pointer relationship for computeSymmetryScore's sibling
        // inventory; the stable source parent remains available separately
        // through the member's scoring ancestry metadata.
        let container_tala_id = if self
            .active_aggregate_vessels_by_member
            .contains_key(&projected.tala_id)
        {
            None
        } else {
            projected.container_tala_id
        };
        SymmetryNeighbor {
            identity: projected.tala_id,
            owner: projected.owner,
            projected: true,
            container_tala_id,
            position: match owner {
                Some(owner) => Some(Point {
                    x: owner.x + projected.offset.x,
                    y: owner.y + projected.offset.y,
                }),
                // Go symmetry.go:169/329 uses the retained child's actual
                // TopLeft even when its current carrier has no position.
                // Read that value; a nil carrier is not a zero origin.
                None => self.retained_original_position(projected.tala_id).flatten(),
            },
            size: projected.size,
        }
    }

    fn abducted_symmetry_neighbor(&self, original: AbductedNodeRef) -> Option<SymmetryNeighbor> {
        let owner = original.owner?;
        if let Some(projected) = self.abducted_node_projection(original) {
            return Some(self.projected_symmetry_neighbor(projected));
        }
        Some(SymmetryNeighbor {
            identity: original.tala_id,
            owner,
            projected: true,
            container_tala_id: if self
                .active_aggregate_vessels_by_member
                .contains_key(&original.tala_id)
            {
                None
            } else {
                original.container_tala_id
            },
            // Position reconciliation cannot form an owner-relative offset
            // while the owner is nil. The original child's actual position
            // can still exist independently of that missing projection.
            position: if self.active_node_position(owner).is_none() {
                self.retained_original_position(original.tala_id).flatten()
            } else {
                None
            },
            size: original.size,
        })
    }

    fn abducted_symmetry_neighbor_with_active_vessel(
        &self,
        original: AbductedNodeRef,
    ) -> Option<SymmetryNeighbor> {
        let owner = original.owner?;
        if let Some(projected) = self.abducted_node_projection(original) {
            Some(self.projected_symmetry_neighbor_with_active_vessel(projected))
        } else {
            self.active_aggregate_vessel_neighbor(original.tala_id, owner)
                .or_else(|| self.abducted_symmetry_neighbor(original))
        }
    }

    fn active_aggregate_vessel_neighbor(
        &self,
        identity: i64,
        fallback_owner: NodeId,
    ) -> Option<SymmetryNeighbor> {
        // A temporary Go cluster/sequence vessel remains a child of the
        // owning container.  The Rust snapshot is keyed by that container
        // but the copied vessel itself has no arena parent, so recover the
        // pointer-keyed scope from the external child inventory before
        // comparing symmetry neighbors.
        let snapshot_container = |vessel_tala_id: i64| {
            self.transaction_external_container_children
                .iter()
                .find_map(|(&container_tala_id, children)| {
                    children
                        .iter()
                        .any(|child| child.tala_id == vessel_tala_id)
                        .then_some(container_tala_id)
                })
        };
        if let Some(cluster_index) = self.active_cluster_index(fallback_owner) {
            let cluster = &self.clusters[cluster_index];
            if cluster
                .members
                .iter()
                .any(|member| self.nodes[member.0 as usize].tala_id == identity)
            {
                let vessel_owner = *cluster.members.first()?;
                return Some(SymmetryNeighbor {
                    identity: cluster.vessel_tala_id,
                    owner: vessel_owner,
                    projected: true,
                    container_tala_id: snapshot_container(cluster.vessel_tala_id).or_else(|| {
                        self.nodes[vessel_owner.0 as usize]
                            .container
                            .map(|container| self.nodes[container.0 as usize].tala_id)
                            .or(self.nodes[vessel_owner.0 as usize].scoring_container_parent)
                    }),
                    position: self.active_node_position(vessel_owner),
                    size: self.cluster_vessel_size(cluster_index),
                });
            }
        }
        // Cluster/sequence activation owns this identity mapping for the
        // whole temporary-graph lifetime. A transaction can invalidate the
        // node-order membership cache without deactivating the vessel; Go's
        // member.Cluster/Sequence pointer still resolves to that vessel in
        // this phase (placementcost/symmetry.go, nodeSymmetry). Consult the
        // stable mapping first, then use the captured external inventory for
        // scopes whose aggregate lives outside this arena.
        let vessel_tala_id = self
            .active_aggregate_vessels_by_member
            .get(&identity)
            .copied()
            .or_else(|| {
                self.transaction_external_aggregate_children
                    .iter()
                    .find_map(|(&vessel_tala_id, members)| {
                        members
                            .iter()
                            .any(|member| member.tala_id == identity)
                            .then_some(vessel_tala_id)
                    })
            })?;
        let local_vessel = if self.node_order_membership_valid {
            self.active_aggregate_vessel_nodes
                .get(&vessel_tala_id)
                .copied()
                .map(|node| (node.0 as usize, &self.nodes[node.0 as usize]))
        } else {
            self.nodes.iter().enumerate().find(|(_, node)| {
                node.tala_id == vessel_tala_id
                    && node.scoring_is_aggregate_vessel
                    && node.position.is_some()
            })
        };
        let external_vessel = || {
            self.transaction_external_containers
                .iter()
                .chain(
                    self.transaction_external_container_children
                        .values()
                        .flatten(),
                )
                .find(|node| node.tala_id == vessel_tala_id && node.position.is_some())
        };
        let (vessel_owner, vessel_position, vessel_size, vessel_container) =
            if let Some((index, vessel)) = local_vessel {
                let vessel_tala_id = vessel.tala_id;
                (
                    NodeId(index as u32),
                    vessel.position.unwrap(),
                    self.active_node_size(NodeId(index as u32)),
                    vessel
                        .container
                        .map(|container| self.nodes[container.0 as usize].tala_id)
                        .or_else(|| snapshot_container(vessel_tala_id)),
                )
            } else if let Some(vessel) = external_vessel() {
                (
                    fallback_owner,
                    vessel.position.unwrap(),
                    vessel.rect.size,
                    snapshot_container(vessel.tala_id).or_else(|| {
                        vessel.container.and_then(|container| {
                            self.nodes
                                .get(container.0 as usize)
                                .map(|container| container.tala_id)
                        })
                    }),
                )
            } else {
                // During AlignAxes' pre-trial baseline the temporary cluster
                // vessel can be represented only by its stable member
                // backlink: it is no longer in Graph.Nodes, and no external
                // snapshot has been installed yet. Go still follows the
                // member's live Cluster.Vessel pointer here.
                let cluster_index = self.active_cluster_index(fallback_owner)?;
                let cluster = &self.clusters[cluster_index];
                if cluster.vessel_tala_id != vessel_tala_id {
                    return None;
                }
                let vessel_owner = *cluster.members.first()?;
                (
                    vessel_owner,
                    self.active_node_position(vessel_owner)?,
                    self.cluster_vessel_size(cluster_index),
                    snapshot_container(vessel_tala_id),
                )
            };
        Some(SymmetryNeighbor {
            identity: vessel_tala_id,
            owner: vessel_owner,
            projected: true,
            container_tala_id: vessel_container,
            position: Some(vessel_position),
            size: vessel_size,
        })
    }

    /// Recovered `Node.getSymmetry` restores an original endpoint from the
    /// matching edge abduction, then immediately replaces that endpoint with
    /// its live `Cluster.Vessel`. Edge-length scoring still uses the member
    /// box, but symmetry therefore deduplicates and measures the one retained
    /// vessel box.
    fn projected_symmetry_neighbor_with_active_vessel(
        &self,
        projected: ProjectedAdjacent,
    ) -> SymmetryNeighbor {
        // A cluster that is active in the current placement scope has a real
        // temporary vessel in this graph, while its stable members remain in
        // the shared aggregate-child inventory.  The projected record is not
        // always marked `cluster_member`: a direct current endpoint can reach
        // the same retired member through an ordinary edge.  TALA's
        // getSymmetry replaces either member with the vessel before
        // deduplication, so consult the live aggregate inventory for every
        // projected identity before falling back to member geometry.
        if let Some(vessel) =
            self.active_aggregate_vessel_neighbor(projected.tala_id, projected.owner)
        {
            return vessel;
        }
        let Some(vessel) = projected
            .cluster_member
            .then(|| {
                self.sized_cluster_distance_boxes
                    .get(&(projected.owner, projected.tala_id))
            })
            .flatten()
        else {
            return self.projected_symmetry_neighbor(projected);
        };
        let owner = self.active_node_position(projected.owner).unwrap();
        SymmetryNeighbor {
            identity: vessel.vessel_tala_id,
            owner: projected.owner,
            projected: true,
            container_tala_id: projected.container_tala_id,
            position: Some(Point {
                x: owner.x + vessel.offset.x,
                y: owner.y + vessel.offset.y,
            }),
            size: vessel.size,
        }
    }

    /// Restores a retained child's adjacency from the full ordered
    /// EdgeAbductions slice, after the surviving Node.Edges walk. Upstream
    /// symmetry.go:289-322 polls before testing each record, including records
    /// that cannot match this receiver and records consumed by earlier walks.
    fn recover_symmetry_abduction_neighbors_checked(
        &self,
        center: SymmetryNeighbor,
        seen: &mut Vec<i64>,
        adjacent: &mut Vec<SymmetryNeighbor>,
        context: ScoreContext<'_>,
    ) -> Result<(), limits::Error> {
        let recovered_endpoint =
            |original: Option<AbductedNodeRef>, current: Option<AbductionCurrentNodeRef>| {
                let current = current?.local_node()?;
                // Production scope metadata identifies the current vessel.
                // Membership in an active aggregate alone does not make the
                // original member pointer a cluster/sequence vessel in Go.
                let keeps_current = self.nodes[current.0 as usize].scoring_is_aggregate_vessel;
                match original {
                    Some(original) if !keeps_current => self.abducted_symmetry_neighbor(original),
                    _ => Some(self.direct_symmetry_neighbor(current)),
                }
            };
        for (index, abduction) in self.sized_edge_abductions.iter().enumerate() {
            score_interval!(
                context,
                self,
                "symmetry-abduction-recovery",
                Some(center.owner),
                index
            )?;
            let mut connected = None;
            if abduction.originally_from.is_some_and(|original| {
                original.owner.is_some() && original.tala_id == center.identity
            }) {
                connected = recovered_endpoint(abduction.originally_to, abduction.current_to);
            }
            // These are independent tests in Go. If both original endpoint
            // pointers identify the receiver, the second assignment wins.
            if abduction.originally_to.is_some_and(|original| {
                original.owner.is_some() && original.tala_id == center.identity
            }) {
                connected = recovered_endpoint(abduction.originally_from, abduction.current_from);
            }
            if let Some(candidate) = connected
                && !seen.contains(&candidate.identity)
            {
                seen.push(candidate.identity);
                adjacent.push(candidate);
            }
        }
        Ok(())
    }

    /// `Node.getSymmetry` is also invoked on original child endpoints that
    /// are absent from a temporary placement graph's `Nodes` slice. The
    /// EdgeAbduction records still retain their identity and geometry. TALA's
    /// later `OriginallyFrom`/`OriginallyTo` scan restores the ordinary
    /// opposite endpoint, but deliberately keeps a current cluster or sequence
    /// vessel instead of restoring one of its retired members.
    fn projected_original_symmetry(&self, center: SymmetryNeighbor, check_neighbors: bool) -> f64 {
        self.projected_original_symmetry_checked(center, check_neighbors, ScoreContext::none())
            .expect("unchecked scoring context cannot cancel")
    }

    fn projected_original_symmetry_checked(
        &self,
        center: SymmetryNeighbor,
        check_neighbors: bool,
        context: ScoreContext<'_>,
    ) -> Result<f64, limits::Error> {
        score_poll!(context, self, "symmetry-entry", Some(center.owner), None)?;
        let mut scratch = SymmetryNodeScratch::take();
        let (seen, adjacent) = (&mut scratch.seen, &mut scratch.adjacent);
        for (index, projected) in self
            .sized_collapsed_symmetry_neighbors
            .get(&center.identity)
            .into_iter()
            .flatten()
            .copied()
            .enumerate()
        {
            score_interval!(
                context,
                self,
                "symmetry-projected-collapsed-adjacent",
                Some(center.owner),
                index
            )?;
            let candidate = self.projected_symmetry_neighbor_with_active_vessel(projected);
            if !seen.contains(&candidate.identity) {
                seen.push(candidate.identity);
                adjacent.push(candidate);
            }
        }
        self.recover_symmetry_abduction_neighbors_checked(center, seen, adjacent, context)?;
        self.filter_symmetry_neighbors_checked(center, adjacent, context)?;
        let (mut score, matched) =
            self.symmetry_pair_score_from_center_checked(center, adjacent, context)?;
        let trace_symmetry_detail =
            crate::internal::trace_env_enabled("TALA_TRACE_SYMMETRY_DETAIL")
                && crate::internal::trace_env_value("TALA_TRACE_SYMMETRY_NODE").is_some_and(
                    |targets| {
                        targets
                            .split(',')
                            .any(|target| target.parse::<i64>().ok() == Some(center.identity))
                    },
                );
        if trace_symmetry_detail {
            eprintln!(
                "SYMMETRY_DETAIL_RUST projected center={} pos={:?} size={},{} adjacent={} direct={} matched={:?} check_neighbors={}",
                center.identity,
                center.position,
                center.size.width,
                center.size.height,
                adjacent.len(),
                score,
                matched,
                check_neighbors
            );
            for candidate in adjacent.iter() {
                eprintln!(
                    "SYMMETRY_DETAIL_RUST projected-candidate center={} id={} owner={} pos={},{} size={},{} container={:?}",
                    center.identity,
                    candidate.identity,
                    candidate.owner.0,
                    candidate.positioned().x,
                    candidate.positioned().y,
                    candidate.size.width,
                    candidate.size.height,
                    candidate.container_tala_id,
                );
            }
        }
        if check_neighbors {
            for (neighbor_index, candidate) in adjacent.iter().copied().enumerate() {
                score_interval!(
                    context,
                    self,
                    "symmetry-neighbor-recursion",
                    Some(center.owner),
                    neighbor_index
                )?;
                if matched[neighbor_index] {
                    continue;
                }
                let child_score = if candidate.projected {
                    self.projected_original_symmetry_checked(candidate, false, context)?
                } else {
                    self.sized_symmetry_checked(candidate.owner, false, context)?
                };
                if trace_symmetry_detail {
                    eprintln!(
                        "SYMMETRY_DETAIL_RUST recurse center={} candidate={} projected={} score={child_score}",
                        center.identity, candidate.identity, candidate.projected
                    );
                }
                score += child_score;
            }
        }
        if !adjacent.is_empty() {
            score_poll!(context, self, "symmetry-final", Some(center.owner), None)?;
        }
        let result = if adjacent.is_empty() {
            0.0
        } else {
            score / adjacent.len() as f64
        };
        if crate::internal::trace_env_value("TALA_TRACE_SYMMETRY_NODE").is_some_and(|targets| {
            targets
                .split(',')
                .any(|target| target.parse::<i64>().ok() == Some(center.identity))
        }) {
            eprint!(
                "SYMMETRY_PROJECTED_RUST center={} adjacent=",
                center.identity
            );
            for candidate in adjacent.iter() {
                eprint!("{},", candidate.identity);
            }
            eprintln!(" raw={score} max={} result={result}", adjacent.len());
        }
        Ok(result)
    }

    fn symmetry_neighbor_is_mirrored(
        &self,
        first: SymmetryNeighbor,
        second: SymmetryNeighbor,
        x_axis: bool,
        axis: f64,
    ) -> bool {
        let (a_position, a_size) = (first.positioned(), first.size);
        let (b_position, b_size) = (second.positioned(), second.size);
        if x_axis {
            if a_position.x == b_position.x {
                return false;
            }
            let residual = if a_position.x > b_position.x {
                if !(b_position.x + b_size.width < axis && axis < a_position.x) {
                    return false;
                }
                ((a_position.x - axis) - (axis - (b_position.x + b_size.width))).abs()
            } else {
                if !(a_position.x + a_size.width < axis && axis < b_position.x) {
                    return false;
                }
                ((b_position.x - axis) - (axis - (a_position.x + a_size.width))).abs()
            };
            residual <= self.cell_size
                && ((a_position.y + a_size.height * 0.5) - (b_position.y + b_size.height * 0.5))
                    .abs()
                    <= self.cell_size
        } else {
            if a_position.y == b_position.y {
                return false;
            }
            let residual = if a_position.y > b_position.y {
                if !(b_position.y + b_size.height < axis && axis < a_position.y) {
                    return false;
                }
                ((a_position.y - axis) - (axis - (b_position.y + b_size.height))).abs()
            } else {
                if !(a_position.y + a_size.height < axis && axis < b_position.y) {
                    return false;
                }
                ((b_position.y - axis) - (axis - (a_position.y + a_size.height))).abs()
            };
            residual <= self.cell_size
                && ((a_position.x + a_size.width * 0.5) - (b_position.x + b_size.width * 0.5)).abs()
                    <= self.cell_size
        }
    }

    fn filter_symmetry_neighbors_checked(
        &self,
        center: SymmetryNeighbor,
        neighbors: &mut Vec<SymmetryNeighbor>,
        context: ScoreContext<'_>,
    ) -> Result<(), limits::Error> {
        // nodeSymmetry removes distant neighbors in place and rechecks the
        // same index after removal. Its actual index drives the 64 stride.
        let mut index = 0;
        while index < neighbors.len() {
            score_interval!(
                context,
                self,
                "symmetry-distance-filter",
                Some(center.owner),
                index
            )?;
            let candidate = neighbors[index];
            if Self::sized_box_border_distance(
                (center.positioned(), center.size),
                (candidate.positioned(), candidate.size),
            ) > 1200.0
            {
                neighbors.remove(index);
            } else {
                index += 1;
            }
        }
        Ok(())
    }

    fn symmetry_pair_score_from_center(
        &self,
        center: SymmetryNeighbor,
        neighbors: &[SymmetryNeighbor],
    ) -> (f64, Vec<i64>) {
        let (score, matched) = self
            .symmetry_pair_score_from_center_checked(center, neighbors, ScoreContext::none())
            .expect("unchecked scoring context cannot cancel");
        (
            score,
            neighbors
                .iter()
                .zip(matched.iter())
                .filter_map(|(n, &m)| m.then_some(n.identity))
                .collect(),
        )
    }

    fn symmetry_pair_score_from_center_checked(
        &self,
        center: SymmetryNeighbor,
        neighbors: &[SymmetryNeighbor],
        context: ScoreContext<'_>,
    ) -> Result<(f64, ScoringScratchVec<bool>), limits::Error> {
        score_poll!(
            context,
            self,
            "symmetry-pair-entry",
            Some(center.owner),
            None
        )?;
        let mut matched = ScoringScratchVec::<bool>::take();
        matched.resize(neighbors.len(), false);
        if neighbors.len() < 2 {
            return Ok((0.0, matched));
        }
        let center_size = center.size;
        let mut score = 0.0;
        // Sibling orders read by this scan's obstruction checks.
        let mut container_orders = Vec::new();
        for (index, first) in neighbors.iter().copied().enumerate() {
            score_interval!(
                context,
                self,
                "symmetry-pair-first",
                Some(center.owner),
                index
            )?;
            if matched[index] {
                continue;
            }
            let first_area = first.size.width * first.size.height;
            let mut best: Option<(SymmetryNeighbor, f64)> = None;
            for (second_index, second) in neighbors.iter().copied().skip(index + 1).enumerate() {
                score_interval!(
                    context,
                    self,
                    "symmetry-pair-second",
                    Some(center.owner),
                    second_index
                )?;
                if matched[index + 1 + second_index] {
                    continue;
                }
                // TALA's computeSymmetryScore only compares neighbors that
                // belong to the same container. Nodes in different hierarchy
                // scopes cannot form a mirrored pair around this center.
                if first.container_tala_id != second.container_tala_id {
                    continue;
                }
                let second_area = second.size.width * second.size.height;
                if first_area > second_area * 2.0 || second_area > first_area * 2.0 {
                    continue;
                }
                let mut pair_score = None;
                for x_axis in [true, false] {
                    // Source symmetry.go:463 dereferences TopLeft after the
                    // first/second polls and eligibility filters.
                    let center_position = center.positioned();
                    let axis = if x_axis {
                        center_position.x + center_size.width * 0.5
                    } else {
                        center_position.y + center_size.height * 0.5
                    };
                    if self.symmetry_neighbor_is_mirrored(first, second, x_axis, axis) {
                        let first_overlaps = if x_axis {
                            center_position.y <= first.positioned().y + first.size.height
                                && first.positioned().y <= center_position.y + center_size.height
                        } else {
                            center_position.x <= first.positioned().x + first.size.width
                                && first.positioned().x <= center_position.x + center_size.width
                        };
                        let second_overlaps = if x_axis {
                            center_position.y <= second.positioned().y + second.size.height
                                && second.positioned().y <= center_position.y + center_size.height
                        } else {
                            center_position.x <= second.positioned().x + second.size.width
                                && second.positioned().x <= center_position.x + center_size.width
                        };
                        pair_score = Some(if first_overlaps && second_overlaps {
                            2.0
                        } else {
                            0.5
                        });
                        break;
                    }
                }
                if let Some(pair_score) = pair_score {
                    // Go passes the original center and neighbor pointers to
                    // obstructed. If any Rust participant is projected, its
                    // owner is only a carrier; retain the original identity,
                    // geometry, and container sibling inventory instead.
                    let obstructed = if center.projected || first.projected || second.projected {
                        self.symmetry_pair_obstructed_neighbors_checked(
                            center, first, second, context,
                        )?
                    } else {
                        self.symmetry_pair_obstructed_checked(
                            center.owner,
                            first.owner,
                            second.owner,
                            context,
                            &mut container_orders,
                        )?
                    };
                    if crate::internal::trace_env_value("TALA_TRACE_SYMMETRY_PAIR").is_some_and(
                        |targets| {
                            targets.split(',').any(|target| {
                                target == "all"
                                    || target.parse::<i64>().ok() == Some(center.identity)
                            })
                        },
                    ) {
                        eprintln!(
                            "SYMMETRY_PAIR_RUST center={} first={} second={} centerProjected={} firstProjected={} secondProjected={} centerPos={},{} firstContainer={:?} secondContainer={:?} own={} firstPos={},{} secondPos={},{} ms={} blocked={}",
                            center.identity,
                            first.identity,
                            second.identity,
                            center.projected,
                            first.projected,
                            second.projected,
                            center.positioned().x,
                            center.positioned().y,
                            first.container_tala_id,
                            second.container_tala_id,
                            center.owner.0,
                            first.positioned().x,
                            first.positioned().y,
                            second.positioned().x,
                            second.positioned().y,
                            pair_score,
                            obstructed,
                        );
                    }
                    if obstructed {
                        continue;
                    }
                    if best.is_none_or(|(_, current)| pair_score > current) {
                        best = Some((second, pair_score));
                        if pair_score == 2.0 {
                            break;
                        }
                    }
                }
            }
            if let Some((second, pair_score)) = best {
                // Preserve identity-set semantics when a helper receives duplicates.
                for (flag, neighbor) in matched.iter_mut().zip(neighbors) {
                    if neighbor.identity == first.identity || neighbor.identity == second.identity {
                        *flag = true;
                    }
                }
                score += pair_score;
            }
        }
        score_poll!(
            context,
            self,
            "symmetry-pair-final",
            Some(center.owner),
            None
        )?;
        Ok((score, matched))
    }

    pub(super) fn sized_symmetry(&self, node: NodeId, check_neighbors: bool) -> f64 {
        self.sized_symmetry_checked(node, check_neighbors, ScoreContext::none())
            .expect("unchecked scoring context cannot cancel")
    }

    pub(super) fn sized_symmetry_checked(
        &self,
        node: NodeId,
        check_neighbors: bool,
        context: ScoreContext<'_>,
    ) -> Result<f64, limits::Error> {
        score_poll!(context, self, "symmetry-entry", Some(node), None)?;
        let mut scratch = SymmetryNodeScratch::take();
        let (seen, adjacent) = (&mut scratch.seen, &mut scratch.adjacent);
        let mut score = 0.0;
        let mut max_score = 0_usize;
        let mut used_edge_abductions = vec![false; self.sized_edge_abductions.len()];
        // Aggregate membership is stable throughout one symmetry score. The
        // recovered abduction scan revisits the same endpoint pair for every
        // incident edge; materialize that predicate once per abduction rather
        // than resolving both active owners in the nested loop.
        let abduction_has_aggregate_endpoint = self
            .sized_edge_abductions
            .iter()
            .map(|abduction| {
                [abduction.current_from, abduction.current_to]
                    .into_iter()
                    .flatten()
                    .any(|endpoint| self.abduction_current_is_aggregate(endpoint))
            })
            .collect::<Vec<_>>();

        // Recovered Node.getSymmetry first handles a container's abducted
        // child endpoints as symmetry centers in their own right. Those
        // original children are not present in the temporary Graph.Nodes
        // slice, but the paired endpoint projections retain their complete
        // identity, box, and original-edge relationships.
        if self.nodes[node.0 as usize].scoring_is_container {
            // With a nil EdgeAbductions slice, recovered Node.getSymmetry
            // scans each concrete container child and includes it when any
            // original child edge reaches outside this container. These
            // zero-scoring children are still material: each increments
            // maxScore and therefore normalizes the container's direct
            // symmetry reward.
            if !self.has_sized_edge_abductions() {
                let mut child_nodes = Vec::new();
                for (child_index, child) in self
                    .containers
                    .get(&Some(node))
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    score_interval!(
                        context,
                        self,
                        "symmetry-container-child",
                        Some(node),
                        child_index
                    )?;
                    let mut reaches_outside = false;
                    for (edge_index, edge_id) in self.nodes[child.0 as usize]
                        .edges
                        .read()
                        .iter()
                        .copied()
                        .enumerate()
                    {
                        score_interval!(
                            context,
                            self,
                            "symmetry-container-child-edge",
                            Some(node),
                            edge_index
                        )?;
                        let edge = &self.edge_records[edge_id.0 as usize];
                        let adjacent = if edge.from == child {
                            edge.to
                        } else {
                            edge.from
                        };
                        if self.nodes[adjacent.0 as usize]
                            .container
                            .is_none_or(|container| !self.is_descendant_of(container, node))
                        {
                            reaches_outside = true;
                            break;
                        }
                    }
                    // Cluster/sequence members have already been replaced by
                    // their live vessel in Graph.Containers. Stable arena
                    // members must collapse to that same active child before
                    // the seen/maxScore accounting.
                    let active_child = self.active_aggregate_owner(child);
                    let identity = self.active_node_tala_id(active_child);
                    if !reaches_outside || seen.contains(&identity) {
                        continue;
                    }
                    seen.push(identity);
                    child_nodes.push(active_child);
                }
                for (child_index, active_child) in child_nodes.into_iter().enumerate() {
                    score_interval!(
                        context,
                        self,
                        "symmetry-child-recursion",
                        Some(node),
                        child_index
                    )?;
                    let identity = self.active_node_tala_id(active_child);
                    let child_score =
                        self.sized_symmetry_checked(active_child, check_neighbors, context)?;
                    score += child_score;
                    if crate::internal::trace_env_value("TALA_TRACE_SYMMETRY_NODE").is_some_and(
                        |targets| {
                            targets.split(',').any(|target| {
                                target.parse::<i64>().ok()
                                    == Some(self.nodes[node.0 as usize].tala_id)
                            })
                        },
                    ) {
                        eprintln!(
                            "SYMMETRY_CHILD_RUST node={} child={} score={child_score}",
                            self.nodes[node.0 as usize].tala_id, identity
                        );
                    }
                    max_score += 1;
                }
            }
            let mut original_children = Vec::new();
            if self.has_sized_edge_abductions() {
                for (index, abduction) in self.sized_edge_abductions.iter().enumerate() {
                    score_interval!(
                        context,
                        self,
                        "symmetry-container-abduction",
                        Some(node),
                        index
                    )?;
                    if abduction
                        .current_from
                        .and_then(|endpoint| endpoint.local_node())
                        == Some(node)
                        && let Some(original) = abduction.originally_from
                    {
                        used_edge_abductions[index] = true;
                        original_children.push(original);
                    }
                    if abduction
                        .current_to
                        .and_then(|endpoint| endpoint.local_node())
                        == Some(node)
                        && let Some(original) = abduction.originally_to
                    {
                        used_edge_abductions[index] = true;
                        original_children.push(original);
                    }
                }
            } else {
                original_children.extend(
                    self.max_edge_length_incident_edges(node)
                        .into_iter()
                        .filter_map(|edge| {
                            let owner = self.active_adjacent(node, edge);
                            self.sized_adjacent_overrides
                                .get(&(owner, edge))
                                .copied()
                                .map(AbductedNodeRef::from)
                        }),
                );
            }
            for (child_index, original_child) in original_children.into_iter().enumerate() {
                let Some(original_child) = self.abducted_symmetry_neighbor(original_child) else {
                    continue;
                };
                if seen.contains(&original_child.identity) {
                    continue;
                }
                seen.push(original_child.identity);
                score_interval!(
                    context,
                    self,
                    "symmetry-child-recursion",
                    Some(node),
                    child_index
                )?;
                let child_score = self.projected_original_symmetry_checked(
                    original_child,
                    check_neighbors,
                    context,
                )?;
                score += child_score;
                if crate::internal::trace_env_value("TALA_TRACE_SYMMETRY_NODE").is_some_and(
                    |targets| {
                        targets.split(',').any(|target| {
                            target.parse::<i64>().ok() == Some(self.nodes[node.0 as usize].tala_id)
                        })
                    },
                ) {
                    eprintln!(
                        "SYMMETRY_CHILD_RUST node={} child={} score={child_score}",
                        self.nodes[node.0 as usize].tala_id, original_child.identity
                    );
                }
                max_score += 1;
            }
        }

        for (edge_index, edge) in self
            .max_edge_length_incident_edges(node)
            .into_iter()
            .enumerate()
        {
            score_interval!(
                context,
                self,
                "symmetry-adjacent-edge",
                Some(node),
                edge_index
            )?;
            let owner = self.active_adjacent(node, edge);
            let arena_edge = &self.edge_records[edge.0 as usize];
            let current_from = self.active_aggregate_owner(arena_edge.from);
            let current_to = self.active_aggregate_owner(arena_edge.to);
            let mut exact_reverse_abduction_consumed = false;
            if self.nodes[node.0 as usize].scoring_is_container && self.has_sized_edge_abductions()
            {
                let mut connected_to_child = false;
                for (index, abduction) in self.sized_edge_abductions.iter().enumerate() {
                    score_interval!(
                        context,
                        self,
                        "symmetry-connected-child-abduction",
                        Some(node),
                        index
                    )?;
                    if abduction
                        .current_from
                        .and_then(|endpoint| endpoint.local_node())
                        == Some(node)
                        && abduction
                            .current_to
                            .and_then(|endpoint| endpoint.local_node())
                            == Some(owner)
                        && abduction.originally_from.is_some()
                    {
                        connected_to_child = true;
                        break;
                    }
                    if abduction
                        .current_from
                        .and_then(|endpoint| endpoint.local_node())
                        == Some(owner)
                        && abduction
                            .current_to
                            .and_then(|endpoint| endpoint.local_node())
                            == Some(node)
                    {
                        used_edge_abductions[index] = true;
                        if abduction.originally_to.is_some() {
                            connected_to_child = true;
                            break;
                        }
                    }
                }
                if connected_to_child {
                    continue;
                }
            } else if self.nodes[node.0 as usize].scoring_is_container {
                if current_from == node
                    && self.sized_adjacent_overrides.contains_key(&(owner, edge))
                {
                    // Forward abduction from a child of this container:
                    // the child pre-scan above owns the recursive score and
                    // this current carrier edge is skipped.
                    continue;
                }
                if current_to == node && self.sized_adjacent_overrides.contains_key(&(node, edge)) {
                    // The reverse container scan marks the abduction used
                    // before testing its nil OriginallyTo field. Preserve that
                    // consumption so the later direct-neighbor loop cannot
                    // restore OriginallyFrom onto this edge.
                    exact_reverse_abduction_consumed = true;
                }
            }
            // getSymmetry deliberately skips EdgeAbductions whose current
            // endpoint is a cluster or sequence vessel, then replaces the
            // adjacent original with that active vessel. Multiple member
            // edges therefore deduplicate as one neighbor here even though
            // edgeLength restores each member's projected geometry.
            let aggregate_endpoint = [current_from, current_to].into_iter().any(|endpoint| {
                self.active_node_is_aggregate(endpoint)
                    || self.nodes[endpoint.0 as usize].scoring_is_aggregate_vessel
            });
            let mut projected_candidate = None;
            if !aggregate_endpoint && self.has_sized_edge_abductions() {
                for (index, abduction) in self.sized_edge_abductions.iter().enumerate() {
                    score_interval!(
                        context,
                        self,
                        "symmetry-adjacent-abduction",
                        Some(node),
                        index
                    )?;
                    if used_edge_abductions[index] {
                        continue;
                    }
                    if abduction_has_aggregate_endpoint[index] {
                        continue;
                    }
                    if abduction
                        .current_from
                        .and_then(|endpoint| endpoint.local_node())
                        == Some(node)
                        && abduction
                            .current_to
                            .and_then(|endpoint| endpoint.local_node())
                            == Some(owner)
                    {
                        used_edge_abductions[index] = true;
                        projected_candidate = abduction.originally_to;
                        break;
                    }
                    if abduction
                        .current_from
                        .and_then(|endpoint| endpoint.local_node())
                        == Some(owner)
                        && abduction
                            .current_to
                            .and_then(|endpoint| endpoint.local_node())
                            == Some(node)
                    {
                        used_edge_abductions[index] = true;
                        projected_candidate = abduction.originally_from;
                        break;
                    }
                }
            } else if !self.has_sized_edge_abductions()
                && !aggregate_endpoint
                && !exact_reverse_abduction_consumed
            {
                projected_candidate = self
                    .sized_adjacent_overrides
                    .get(&(node, edge))
                    .copied()
                    .map(AbductedNodeRef::from);
            }
            let candidate = projected_candidate
                .and_then(|original| self.abducted_symmetry_neighbor_with_active_vessel(original))
                .unwrap_or_else(|| self.direct_symmetry_neighbor(owner));
            let candidate = self
                .active_aggregate_vessel_neighbor(candidate.identity, candidate.owner)
                .unwrap_or(candidate);
            if !seen.contains(&candidate.identity) {
                seen.push(candidate.identity);
                adjacent.push(candidate);
            }
        }
        let center = self.direct_symmetry_neighbor(node);
        self.recover_symmetry_abduction_neighbors_checked(center, seen, adjacent, context)?;
        self.filter_symmetry_neighbors_checked(center, adjacent, context)?;
        let (direct_score, matched) =
            self.symmetry_pair_score_from_center_checked(center, adjacent, context)?;
        let trace_sized_detail = crate::internal::trace_env_enabled("TALA_TRACE_SYMMETRY_DETAIL")
            && crate::internal::trace_env_value("TALA_TRACE_SYMMETRY_NODE").is_some_and(
                |targets| {
                    targets.split(',').any(|target| {
                        target.parse::<i64>().ok() == Some(self.nodes[node.0 as usize].tala_id)
                    })
                },
            );
        if trace_sized_detail {
            eprintln!(
                "SYMMETRY_DETAIL_RUST sized node={} adjacent={} direct={} matched={:?} check_neighbors={}",
                self.nodes[node.0 as usize].tala_id,
                adjacent.len(),
                direct_score,
                matched,
                check_neighbors
            );
            for candidate in adjacent.iter() {
                eprintln!(
                    "SYMMETRY_DETAIL_RUST sized-candidate node={} id={} owner={} projected={} pos={},{} size={},{} container={:?}",
                    self.nodes[node.0 as usize].tala_id,
                    candidate.identity,
                    candidate.owner.0,
                    candidate.projected,
                    candidate.positioned().x,
                    candidate.positioned().y,
                    candidate.size.width,
                    candidate.size.height,
                    candidate.container_tala_id,
                );
            }
        }
        score += direct_score;
        max_score += adjacent.len();
        if check_neighbors {
            for (neighbor_index, candidate) in adjacent.iter().copied().enumerate() {
                score_interval!(
                    context,
                    self,
                    "symmetry-neighbor-recursion",
                    Some(node),
                    neighbor_index
                )?;
                // getSymmetry recurses through the restored original endpoint,
                // not through the CurrentFrom/CurrentTo carrier. Scope graphs
                // do not materialize that original node as an arena node, so
                // reproduce its recursive call from the retained abduction
                // geometry instead of substituting the current carrier.
                if matched[neighbor_index] {
                    continue;
                }
                let child_score = if candidate.projected {
                    self.projected_original_symmetry_checked(candidate, false, context)?
                } else {
                    self.sized_symmetry_checked(candidate.owner, false, context)?
                };
                if trace_sized_detail {
                    eprintln!(
                        "SYMMETRY_DETAIL_RUST sized-recurse node={} candidate={} projected={} score={child_score}",
                        self.nodes[node.0 as usize].tala_id,
                        candidate.identity,
                        candidate.projected
                    );
                }
                score += child_score;
            }
        }
        if max_score != 0 {
            score_poll!(context, self, "symmetry-final", Some(node), None)?;
        }
        let result = if max_score == 0 {
            0.0
        } else {
            score / max_score as f64
        };
        if crate::internal::trace_env_value("TALA_TRACE_SYMMETRY_NODE").is_some_and(|targets| {
            targets.split(',').any(|target| {
                target.parse::<i64>().ok() == Some(self.nodes[node.0 as usize].tala_id)
            })
        }) {
            eprint!(
                "SYMMETRY_RUST node={} pos={},{} adjacent=",
                self.nodes[node.0 as usize].tala_id,
                self.position(node).unwrap().x,
                self.position(node).unwrap().y
            );
            for candidate in adjacent.iter() {
                eprint!("{},", candidate.identity);
            }
            eprintln!(" raw={score} max={max_score} result={result}");
        }
        Ok(result)
    }

    // This naming follows the recovered function exactly. It describes the
    // other point's compass relation as used by edgeLength.
}

#[cfg(test)]
mod ordered_obstruction_contract_tests {
    use super::*;
    use std::cell::Cell;

    struct PollContext {
        polls: Cell<usize>,
        cancel_at: Option<usize>,
    }

    impl limits::Context for PollContext {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<limits::ContextError> {
            let poll = self.polls.get() + 1;
            self.polls.set(poll);
            self.cancel_at
                .filter(|&at| poll >= at)
                .map(|_| limits::ContextError::Cancelled)
        }
    }

    fn pointer(
        source_node: Option<NodeId>,
        source_owner: Option<NodeId>,
        location: AbductionNodeLocation,
    ) -> ProjectedObstructionRef {
        ProjectedObstructionRef {
            source_arena: None,
            source_node,
            source_owner,
            location,
            tala_id: 99,
            source_container: None,
            container_tala_id: None,
            position: None,
            size: Size {
                width: 10.0,
                height: 10.0,
            },
            projection: None,
            graph_owner: None,
            same_graph_as_replacement: false,
            is_endpoint: false,
            is_replacement_ancestor: false,
            is_cluster_vessel: false,
            is_endpoint_cluster_vessel: false,
        }
    }

    fn route_graph() -> (ArenaGraph, NodeId, NodeId, EdgeId) {
        let mut input = Graph::default();
        let a = input.add_node(crate::Node::new(
            "A",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let b = input.add_node(crate::Node::new(
            "B",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let edge = input.add_edge(crate::Edge {
            source: a,
            target: b,
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(a, Point { x: 0.0, y: 0.0 });
        graph.set_position(b, Point { x: 200.0, y: 0.0 });
        (graph, a, b, edge)
    }

    fn route(
        graph: &ArenaGraph,
        a: NodeId,
        b: NodeId,
        edge: EdgeId,
        context: &PollContext,
    ) -> Result<f64, limits::Error> {
        graph.edge_route_penalty_cached_checked(
            a,
            b,
            edge,
            None,
            None,
            ProjectedObstructionSource::Edge,
            false,
            false,
            ScoreContext::new(context),
        )
    }

    #[test]
    fn obstruction_snapshot_detaches_only_when_projection_bits_change() {
        let mut item = pointer(
            Some(NodeId(3)),
            Some(NodeId(3)),
            AbductionNodeLocation::Foreign,
        );
        let nan = f64::from_bits(0x7ff8_0000_0000_0042);
        item.projection = Some(ProjectedAdjacent {
            owner: NodeId(0),
            tala_id: 3,
            container_tala_id: None,
            offset: Point { x: -0.0, y: nan },
            size: Size {
                width: 10.0,
                height: 20.0,
            },
            cluster_member: false,
        });
        let mut live = OrderedObstructionInventory {
            sets: vec![
                ProjectedObstructionSet {
                    source_container: None,
                    container_tala_id: None,
                    items: None,
                },
                ProjectedObstructionSet {
                    source_container: None,
                    container_tala_id: None,
                    items: Some(Vec::new()),
                },
                ProjectedObstructionSet {
                    source_container: None,
                    container_tala_id: None,
                    items: Some(vec![item]),
                },
            ]
            .into(),
            ..Default::default()
        };
        let saved = live.clone();
        let mut visits = 0;
        live.update_projections(|projection| {
            visits += 1;
            projection.offset.y = nan;
        });
        assert_eq!(visits, 1);
        assert!(std::sync::Arc::ptr_eq(&live.sets, &saved.sets));
        live.update_projections(|projection| projection.offset.x = 0.0);
        assert!(!std::sync::Arc::ptr_eq(&live.sets, &saved.sets));
        let projection = |inventory: &OrderedObstructionInventory| {
            inventory.sets[2].items.as_ref().unwrap()[0]
                .projection
                .unwrap()
        };
        assert_eq!(projection(&saved).offset.x.to_bits(), (-0.0_f64).to_bits());
        assert_eq!(projection(&live).offset.x.to_bits(), 0.0_f64.to_bits());
        assert_eq!(projection(&live).offset.y.to_bits(), nan.to_bits());
        assert!(live.sets[0].items.is_none());
        assert!(live.sets[1].items.as_ref().unwrap().is_empty());
        live.clone_from(&saved);
        assert!(std::sync::Arc::ptr_eq(&live.sets, &saved.sets));
        drop(saved);
        let backing = std::sync::Arc::as_ptr(&live.sets);
        live.update_projections(|projection| projection.size.width = 30.0);
        assert_eq!(std::sync::Arc::as_ptr(&live.sets), backing);
        assert_eq!(projection(&live).size.width, 30.0);
    }

    #[test]
    fn nil_and_allocated_empty_sets_poll_before_predicates_at_zero_and_sixty_four() {
        let (mut graph, a, b, edge) = route_graph();
        let sets = (0..65)
            .map(|index| ProjectedObstructionSet {
                source_container: Some(NodeId(index)),
                container_tala_id: Some(index as i64),
                items: (index % 2 == 1).then(Vec::new),
            })
            .collect();
        graph.sized_projected_obstruction_sets.insert(
            (a, edge),
            OrderedObstructionInventory {
                sets_present: true,
                sets,
                ..OrderedObstructionInventory::default()
            },
        );
        let context = PollContext {
            polls: Cell::new(0),
            cancel_at: None,
        };
        assert!(route(&graph, a, b, edge, &context).unwrap().is_finite());
        assert_eq!(context.polls.get(), 2);
        for cancel_at in [1, 2] {
            let context = PollContext {
                polls: Cell::new(0),
                cancel_at: Some(cancel_at),
            };
            assert_eq!(
                route(&graph, a, b, edge, &context).unwrap_err().to_string(),
                "EdgeLength: context canceled"
            );
            assert_eq!(context.polls.get(), cancel_at);
        }
        let inventory = &graph.sized_projected_obstruction_sets[&(a, edge)];
        assert!(inventory.sets[0].items.is_none());
        assert!(inventory.sets[1].items.as_ref().unwrap().is_empty());
    }

    #[test]
    fn skipped_foreign_nil_endpoint_and_ancestor_items_still_poll_before_zero_and_sixty_four() {
        let (mut graph, a, b, edge) = route_graph();
        let items = (0..65)
            .map(|index| {
                let mut item = pointer(
                    (index != 0).then_some(NodeId(index)),
                    Some(NodeId(index)),
                    AbductionNodeLocation::Foreign,
                );
                item.is_endpoint = index % 2 == 0;
                item.is_replacement_ancestor = index % 2 == 1;
                item
            })
            .collect();
        graph.sized_projected_obstruction_sets.insert(
            (a, edge),
            OrderedObstructionInventory {
                sets_present: true,
                sets: vec![
                    ProjectedObstructionSet {
                        source_container: Some(a),
                        container_tala_id: None,
                        items: None,
                    },
                    ProjectedObstructionSet {
                        source_container: None,
                        container_tala_id: None,
                        items: Some(items),
                    },
                ]
                .into(),
                ..OrderedObstructionInventory::default()
            },
        );
        let context = PollContext {
            polls: Cell::new(0),
            cancel_at: None,
        };
        assert!(route(&graph, a, b, edge, &context).unwrap().is_finite());
        assert_eq!(context.polls.get(), 3);
        for cancel_at in [2, 3] {
            let context = PollContext {
                polls: Cell::new(0),
                cancel_at: Some(cancel_at),
            };
            assert_eq!(
                route(&graph, a, b, edge, &context).unwrap_err().to_string(),
                "EdgeLength: context canceled"
            );
            assert_eq!(context.polls.get(), cancel_at);
        }
        assert!(
            graph.sized_projected_obstruction_sets[&(a, edge)].sets[1]
                .items
                .as_ref()
                .unwrap()[0]
                .source_node
                .is_none()
        );
    }

    #[test]
    fn foreign_pointer_keeps_owning_namespace_after_repeated_dense_induction_and_initialize() {
        let mut input = Graph::default();
        let foreign = input.add_node(crate::Node::new(
            "D",
            Size {
                width: 60.0,
                height: 60.0,
            },
        ));
        let a = input.add_node(crate::Node::new(
            "A",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let b = input.add_node(crate::Node::new(
            "B",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let edge = input.add_edge(crate::Edge {
            source: a,
            target: b,
        });
        let mut owner = ArenaGraph::from_input(&input);
        let foreign_position = Point { x: 300.0, y: 80.0 };
        owner.set_position(foreign, foreign_position);
        owner.set_position(a, Point { x: 0.0, y: 0.0 });
        owner.set_position(b, Point { x: 200.0, y: 0.0 });
        owner.initialize_shared_topology();
        let source_arena = owner.topology_arena_id();
        let mut item = pointer(
            Some(foreign),
            Some(foreign),
            AbductionNodeLocation::Local(foreign),
        );
        item.source_arena = source_arena;
        item.position = Some(foreign_position);
        item.graph_owner = owner.topology_node_owner(foreign);
        item.same_graph_as_replacement = true;
        owner.sized_projected_obstruction_sets.insert(
            (a, edge),
            OrderedObstructionInventory {
                source_arena,
                sets_present: true,
                node_replacement: Some(a),
                adjacent_replacement: Some(b),
                sets: vec![ProjectedObstructionSet {
                    source_container: None,
                    container_tala_id: None,
                    items: Some(vec![item]),
                }]
                .into(),
                projection_index: Default::default(),
            },
        );
        let (mut component, _) = owner.induced_placement_subgraph(&[a, b]);
        for _ in 0..2 {
            component.initialize_shared_topology();
            assert_ne!(component.topology_arena_id(), source_arena);
            let inventory = &component.sized_projected_obstruction_sets[&(NodeId(0), EdgeId(0))];
            let item = &inventory.sets[0].items.as_ref().unwrap()[0];
            assert_eq!(item.source_node, Some(NodeId(0)));
            assert_eq!(item.source_arena, source_arena);
            assert_eq!(item.location, AbductionNodeLocation::Foreign);
            assert_eq!(
                component.projected_obstruction_box(item).0,
                Some(foreign_position)
            );
            assert!(!component.projected_obstruction_same_graph(item, inventory));
            (component, _) = component.induced_placement_subgraph(&[NodeId(0), NodeId(1)]);
        }
    }

    #[test]
    fn live_self_geometry_can_become_positioned_while_nil_child_geometry_stays_nil() {
        let (mut graph, carrier, _, _) = route_graph();
        graph.nodes[carrier.0 as usize].position = None;
        let direct = pointer(
            Some(NodeId(20)),
            Some(NodeId(20)),
            AbductionNodeLocation::Local(carrier),
        );
        let mut child = pointer(
            Some(NodeId(21)),
            Some(NodeId(20)),
            AbductionNodeLocation::Local(carrier),
        );
        child.projection = Some(ProjectedAdjacent {
            owner: carrier,
            tala_id: 99,
            container_tala_id: None,
            offset: Point { x: 10.0, y: 10.0 },
            size: child.size,
            cluster_member: false,
        });
        assert!(graph.projected_obstruction_box(&direct).0.is_none());
        let placed = Point { x: 80.0, y: 80.0 };
        graph.set_position(carrier, placed);
        assert_eq!(graph.projected_obstruction_box(&direct).0, Some(placed));
        assert!(graph.projected_obstruction_box(&child).0.is_none());
        graph.nodes[carrier.0 as usize].position = None;
        assert!(graph.projected_obstruction_box(&direct).0.is_none());
    }
}

#[cfg(test)]
mod projected_symmetry_tests {
    use super::*;
    use crate::Edge;
    use crate::internal::model::ProjectedTransactionNode;
    use std::cell::Cell;

    #[test]
    fn symmetry_obstruction_bounds_preserve_touching_and_nonfinite_boxes() {
        // Pinned placementcost/obstruction_bounds_test.go:
        // TestObstructionBoundsStrictSeparation and TestObstructionBoundsFallback.
        let size = |width, height| Size { width, height };
        let bounds = SymmetryObstructionBounds::from_box(
            Some(Point { x: 30.0, y: 40.0 }),
            size(-20.0, -20.0),
        );
        for (x, y, width, height, excluded) in [
            (0.0, 25.0, 9.0, 5.0, true),
            (31.0, 25.0, 9.0, 5.0, true),
            (15.0, 10.0, 5.0, 9.0, true),
            (15.0, 41.0, 5.0, 9.0, true),
            (9.0, 25.0, -5.0, 5.0, true),
            (15.0, 19.0, 5.0, -5.0, true),
            (0.0, 25.0, 10.0, 5.0, false),
            (30.0, 25.0, 9.0, 5.0, false),
            (15.0, 10.0, 5.0, 10.0, false),
            (15.0, 40.0, 5.0, 10.0, false),
            (0.0, 0.0, 100.0, 100.0, false),
            (20.0, 30.0, 0.0, 0.0, false),
            (31.0, 30.0, 0.0, 0.0, true),
        ] {
            assert_eq!(
                bounds.excludes(Some(Point { x, y }), size(width, height)),
                excluded,
                "box {x},{y} {width}x{height}",
            );
        }
        assert!(!bounds.excludes(None, size(1.0, 1.0)));
        for position in [
            None,
            Some(Point {
                x: f64::NAN,
                y: 0.0,
            }),
            Some(Point {
                x: 0.0,
                y: f64::INFINITY,
            }),
            Some(Point {
                x: f64::MAX,
                y: 0.0,
            }),
        ] {
            let fallback = SymmetryObstructionBounds::from_box(position, size(1.0, 1.0));
            assert!(!fallback.usable);
            assert!(!fallback.including(bounds).excludes(
                Some(Point {
                    x: -100.0,
                    y: -100.0
                }),
                size(1.0, 1.0),
            ));
            if position.is_none_or(|point| !point.x.is_finite() || !point.y.is_finite()) {
                assert!(!bounds.excludes(position, size(1.0, 1.0)));
            }
        }
    }

    #[test]
    fn cluster_side_penalty_keeps_external_nodes_after_vessel_reconnection() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", 20.0, 20.0));
        let second = input.add_node(node("second", 20.0, 20.0));
        let a = input.add_node(node("a", 10.0, 10.0));
        let b = input.add_node(node("b", 10.0, 10.0));
        input.add_edge(Edge {
            source: first,
            target: a,
        });
        input.add_edge(Edge {
            source: second,
            target: b,
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.clusters.push(ClusterState {
            members: vec![first, second],
            arrangement: ClusterArrangement::Column,
            desired_arrangement: ClusterArrangement::Column,
            padding: 20.0,
            vessel_tala_id: 10001,
            fixed_size: false,
        });
        for member in [first, second] {
            graph.nodes[member.0 as usize].cluster = Some(0);
        }
        graph.node_order = vec![first, a, b];
        let origin = Point { x: 100.25, y: 20.5 };
        let mut entity = graph.nodes[first.0 as usize].clone();
        entity.tala_id = 10001;
        entity.position = Some(origin);
        entity.rect.size = Size {
            width: 20.0,
            height: 60.0,
        };
        let vessel = graph.retain_topology_node(entity);
        graph
            .bind_topology_vessel(TopologyVessel::Cluster(0), vessel)
            .unwrap();
        graph.pending_cluster_vessel_positions.insert(0, origin);
        graph.set_position(first, origin);
        graph.set_position(a, Point { x: 10.25, y: 45.5 });
        graph.set_position(b, Point { x: 20.25, y: 70.5 });
        graph.refresh_node_order_membership();
        let projected = ProjectedAdjacent {
            owner: first,
            tala_id: graph.nodes[first.0 as usize].tala_id,
            container_tala_id: None,
            offset: Point::default(),
            size: Size {
                width: 20.0,
                height: 20.0,
            },
            cluster_member: true,
        };
        let read = |graph: &ArenaGraph| {
            graph.sized_cluster_arrangement_penalty(
                projected,
                (
                    graph.position(a).unwrap(),
                    graph.nodes[a.0 as usize].rect.size,
                ),
                Orientation::Right,
                graph.nodes[a.0 as usize].tala_id,
                &BTreeMap::new(),
            )[1]
        };
        assert_eq!(read(&graph), graph.turn_cost_value());
        // AddCluster reconnects each edge to the actual vessel, while its
        // two original members remain allocated outside Graph.Nodes.
        for edge in &mut graph.edge_records {
            edge.from = vessel;
        }
        assert_eq!(read(&graph), graph.turn_cost_value());
        graph.clear_position(b);
        assert_eq!(read(&graph), 0.0);
    }

    #[test]
    fn bound_cluster_nil_records_match_pairs_in_order_and_keep_live_originals() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", 54.0, 66.0));
        let second = input.add_node(node("second", 54.0, 66.0));
        let peer = input.add_node(node("peer", 52.0, 66.0));
        let first_edge = input.add_edge(Edge {
            source: first,
            target: peer,
        });
        let second_edge = input.add_edge(Edge {
            source: second,
            target: peer,
        });
        let mut pipeline = Pipeline::new(&input, 1, false, true);
        pipeline.graph.clusters.push(ClusterState {
            members: vec![first, second],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 20.0,
            vessel_tala_id: 501,
            fixed_size: false,
        });
        for member in [first, second] {
            pipeline.graph.nodes[member.0 as usize].cluster = Some(0);
        }
        pipeline.graph.node_order = vec![first, peer];
        pipeline
            .prepare_placement_scope_after(None, &BTreeSet::new())
            .unwrap()
            .unwrap();
        let vessel = pipeline
            .graph
            .topology_vessel_node(TopologyVessel::Cluster(0))
            .unwrap();
        pipeline
            .graph
            .set_position(first, Point { x: 193.0, y: 0.0 });
        pipeline
            .graph
            .set_position(second, Point { x: 267.0, y: 0.0 });
        pipeline
            .graph
            .set_position(peer, Point { x: 193.0, y: 128.0 });
        pipeline
            .graph
            .pending_cluster_vessel_positions
            .insert(0, Point { x: 193.0, y: 0.0 });
        // Source matches the first unused current pair, not the edge pointer.
        let edges = [second_edge, first_edge];
        let restored = pipeline
            .graph
            .sized_restored_endpoints(peer, &edges, false)
            .unwrap();
        let nil_sequence_call = pipeline
            .graph
            .sized_restored_endpoints(peer, &edges, true)
            .unwrap();
        assert_eq!(
            nil_sequence_call[0].adjacent.unwrap().source_node,
            Some(first)
        );
        assert_eq!(
            nil_sequence_call[1].adjacent.unwrap().source_node,
            Some(second)
        );
        for (replacement, original) in restored.iter().zip([first, second]) {
            let reference = replacement.adjacent.unwrap();
            assert_eq!(reference.source_node, Some(original));
            assert_eq!(reference.source_owner, Some(vessel));
            let projected = pipeline.graph.abducted_node_projection(reference).unwrap();
            assert_eq!(
                pipeline.graph.projected_absolute_position(projected),
                pipeline.graph.position(original)
            );
        }
        let reference = restored[1].adjacent.unwrap();
        pipeline
            .graph
            .set_position(second, Point { x: 277.0, y: 3.0 });
        let projected = pipeline.graph.abducted_node_projection(reference).unwrap();
        assert_eq!(
            pipeline.graph.projected_absolute_position(projected),
            Some(Point { x: 277.0, y: 3.0 })
        );
        pipeline.graph.nodes[first.0 as usize].scoring_cluster_arrangement =
            Some(ClusterArrangement::Column);
        assert_eq!(
            pipeline
                .graph
                .sized_edge_length_direction(first, &[first_edge, second_edge], None)
                .0,
            Orientation::Bottom
        );
        let vessel_score = pipeline.graph.sized_edge_length(first, true);
        pipeline.graph.nodes[first.0 as usize].nears =
            crate::internal::NodeNearMap::from_raw_keys([Some(peer)]);
        assert_eq!(
            pipeline.graph.sized_edge_length(first, true).to_bits(),
            vessel_score.to_bits()
        );
        pipeline
            .graph
            .set_position(peer, Point { x: 253.0, y: 128.0 });
        let peer_score = pipeline.graph.sized_edge_length(peer, true);
        pipeline.graph.nodes[peer.0 as usize].nears =
            crate::internal::NodeNearMap::from_raw_keys([Some(first)]);
        // The original 54-wide member leaves a six-unit horizontal gap.
        // Expanding it to the 128-wide vessel incorrectly erases that gap.
        let original_near_distance = (6.0_f64 * 6.0 + 62.0 * 62.0).sqrt();
        assert_eq!(
            pipeline.graph.sized_edge_length(peer, true).to_bits(),
            (peer_score + original_near_distance).to_bits()
        );
        pipeline.graph.sized_edge_abductions_present = true;
        assert!(
            pipeline
                .graph
                .sized_restored_endpoints(peer, &edges, false)
                .unwrap()
                .iter()
                .all(|replacement| replacement.node.is_none() && replacement.adjacent.is_none())
        );
        pipeline.graph.sized_edge_abductions_present = false;
        let group = pipeline
            .graph
            .owner_graph_group_entry(
                OwnerGraphId(0),
                super::super::owner_groups::OwnerGroupKind::Cluster,
                Some(vessel),
            )
            .unwrap()
            .flatten()
            .unwrap();
        let mut policy = pipeline.graph.owner_cluster_policy(group).unwrap();
        policy.edge_abductions = Some(
            pipeline
                .graph
                .allocate_owner_abduction_header(&[None], 1, true)
                .unwrap(),
        );
        pipeline
            .graph
            .set_owner_cluster_policy(group, policy)
            .unwrap();
        let context = PollContext {
            polls: Cell::new(0),
            cancel_at: Some(6),
        };
        assert_eq!(
            pipeline
                .graph
                .sized_restored_endpoints_with_owner_frame_checked(
                    peer,
                    &edges,
                    false,
                    None,
                    ScoreContext::new(&context)
                )
                .unwrap_err()
                .to_string(),
            "EdgeLength: context canceled"
        );
        assert_eq!(context.polls.get(), 6);
    }

    struct PollContext {
        polls: Cell<usize>,
        cancel_at: Option<usize>,
    }

    impl limits::Context for PollContext {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<limits::ContextError> {
            let poll = self.polls.get() + 1;
            self.polls.set(poll);
            self.cancel_at
                .filter(|target| poll >= *target)
                .map(|_| limits::ContextError::Cancelled)
        }
    }

    fn checked_scoring_fixture(edge_count: usize) -> (ArenaGraph, NodeId) {
        let mut graph = Graph::default();
        let center = graph.add_node(node("center", 10.0, 10.0));
        let mut neighbors = Vec::new();
        for index in 0..edge_count {
            let adjacent = graph.add_node(node(&format!("adjacent-{index}"), 10.0, 10.0));
            graph.add_edge(Edge {
                source: center,
                target: adjacent,
            });
            neighbors.push(adjacent);
        }
        let mut arena = ArenaGraph::from_input(&graph);
        arena.cell_size = 10.0;
        arena.set_position(center, Point { x: 0.0, y: 0.0 });
        for (index, adjacent) in neighbors.into_iter().enumerate() {
            arena.set_position(
                adjacent,
                Point {
                    x: 100.0 + index as f64 * 20.0,
                    y: 0.0,
                },
            );
        }
        arena.refresh_node_order_membership();
        (arena, center)
    }

    #[test]
    fn symmetry_distinguishes_nil_from_present_empty_abductions() {
        // Original-Go direct caller controls: C has a protruding child A;
        // A's left/right neighbors form a symmetric pair. A nil argument
        // includes A's score, whereas an empty nonnil argument does not.
        let mut input = Graph::default();
        let container = input.add_node(node("C", 60.0, 60.0));
        let mut child_node = node("A", 10.0, 10.0);
        child_node.parent = Some(container);
        let child = input.add_node(child_node);
        let left = input.add_node(node("L", 10.0, 10.0));
        let right = input.add_node(node("R", 10.0, 10.0));
        input.add_edge(Edge {
            source: child,
            target: left,
        });
        input.add_edge(Edge {
            source: child,
            target: right,
        });
        let mut arena = ArenaGraph::from_input(&input);
        // A prepared direct scorer hydrates IsContainer from the actual
        // shared Containers map, as placement-scope metadata does normally.
        arena.nodes[container.0 as usize].scoring_is_container = arena
            .containers
            .get(&Some(container))
            .is_some_and(|children| !children.is_empty());
        for (node, position) in [
            (container, Point { x: 80.0, y: 80.0 }),
            (child, Point { x: 100.0, y: 100.0 }),
            (left, Point { x: 0.0, y: 100.0 }),
            (right, Point { x: 200.0, y: 100.0 }),
        ] {
            arena.set_position(node, position);
        }
        arena.cell_size = 10.0;
        let context = PollContext {
            polls: Cell::new(0),
            cancel_at: None,
        };
        assert_eq!(
            arena
                .sized_symmetry_checked(container, true, ScoreContext::new(&context))
                .unwrap()
                .to_bits(),
            1.0_f64.to_bits()
        );
        assert_eq!(context.polls.get(), 17);
        arena.sized_edge_abductions_present = true;
        let context = PollContext {
            polls: Cell::new(0),
            cancel_at: None,
        };
        assert_eq!(
            arena
                .sized_symmetry_checked(container, true, ScoreContext::new(&context))
                .unwrap()
                .to_bits(),
            0.0_f64.to_bits()
        );
        assert_eq!(context.polls.get(), 2);
    }

    #[test]
    fn checked_sized_scorers_poll_nil_done_before_geometry_and_lazy_cost_reads() {
        let (arena, center) = checked_scoring_fixture(1);
        let positions = arena
            .nodes
            .iter()
            .map(|node| node.position)
            .collect::<Vec<_>>();
        let costs = (
            arena.raw_crossing_cost(),
            arena.raw_turn_cost(),
            arena.raw_non_center_port_cost(),
        );
        for symmetry in [false, true] {
            let context = PollContext {
                polls: Cell::new(0),
                cancel_at: Some(1),
            };
            let result = if symmetry {
                arena.sized_symmetry_checked(center, true, ScoreContext::new(&context))
            } else {
                arena.sized_edge_length_with_cache_mode_materialized_checked(
                    center,
                    false,
                    None,
                    false,
                    false,
                    false,
                    false,
                    ScoreContext::new(&context),
                )
            };
            assert_eq!(
                result.unwrap_err().to_string(),
                "EdgeLength: context canceled"
            );
            assert_eq!(context.polls.get(), 1);
            assert_eq!(
                arena
                    .nodes
                    .iter()
                    .map(|node| node.position)
                    .collect::<Vec<_>>(),
                positions
            );
            assert_eq!(
                (
                    arena.raw_crossing_cost(),
                    arena.raw_turn_cost(),
                    arena.raw_non_center_port_cost()
                ),
                costs
            );
        }
    }

    #[test]
    fn checked_endpoint_preparation_polls_actual_zero_and_sixty_four_boundaries() {
        let (arena, center) = checked_scoring_fixture(65);
        let edges = arena.scoring_edge_ids(center);
        let context = PollContext {
            polls: Cell::new(0),
            cancel_at: None,
        };
        let prepared = arena
            .sized_restored_endpoints_checked(center, &edges, false, ScoreContext::new(&context))
            .unwrap();
        assert!(
            prepared.is_none(),
            "ordinary endpoints retain the allocation-free representation"
        );
        // Two scratch sides and the actual edge traversal each visit 0 and 64.
        assert_eq!(context.polls.get(), 6);
        let cancelled = PollContext {
            polls: Cell::new(0),
            cancel_at: Some(6),
        };
        let result = arena.sized_restored_endpoints_checked(
            center,
            &edges,
            false,
            ScoreContext::new(&cancelled),
        );
        assert_eq!(
            result.unwrap_err().to_string(),
            "EdgeLength: context canceled"
        );
        assert_eq!(cancelled.polls.get(), 6);
    }

    #[test]
    fn checked_sized_scoring_retains_numeric_bits_across_interval_boundary() {
        let (arena, center) = checked_scoring_fixture(65);
        for direction_penalty in [false, true] {
            let expected = arena.sized_edge_length_with_cache_mode_materialized(
                center,
                direction_penalty,
                None,
                false,
                false,
                false,
                false,
            );
            let context = PollContext {
                polls: Cell::new(0),
                cancel_at: None,
            };
            let actual = arena
                .sized_edge_length_with_cache_mode_materialized_checked(
                    center,
                    direction_penalty,
                    None,
                    false,
                    false,
                    false,
                    false,
                    ScoreContext::new(&context),
                )
                .unwrap();
            assert_eq!(actual.to_bits(), expected.to_bits());
            assert!(context.polls.get() > 6);
        }
    }

    #[test]
    fn checked_symmetry_distance_filter_repolls_index_after_removal() {
        let (arena, center_id) = checked_scoring_fixture(0);
        let center = arena.direct_symmetry_neighbor(center_id);
        let neighbor = |identity, x| SymmetryNeighbor {
            identity,
            position: Some(Point { x, y: 0.0 }),
            ..center
        };
        let mut neighbors = vec![
            neighbor(100, 5000.0),
            neighbor(101, 6000.0),
            neighbor(102, 100.0),
        ];
        let context = PollContext {
            polls: Cell::new(0),
            cancel_at: Some(2),
        };
        let error = arena
            .filter_symmetry_neighbors_checked(center, &mut neighbors, ScoreContext::new(&context))
            .unwrap_err();
        assert_eq!(error.to_string(), "EdgeLength: context canceled");
        assert_eq!(context.polls.get(), 2);
        assert_eq!(
            neighbors
                .iter()
                .map(|node| node.identity)
                .collect::<Vec<_>>(),
            vec![101, 102]
        );
        let context = PollContext {
            polls: Cell::new(0),
            cancel_at: None,
        };
        arena
            .filter_symmetry_neighbors_checked(center, &mut neighbors, ScoreContext::new(&context))
            .unwrap();
        assert_eq!(context.polls.get(), 2);
        assert_eq!(
            neighbors
                .iter()
                .map(|node| node.identity)
                .collect::<Vec<_>>(),
            vec![102]
        );
    }

    #[test]
    fn repeated_label_direction_counts_semantic_sources() {
        for arrows in [
            crate::EdgeArrows {
                source: true,
                target: false,
            },
            crate::EdgeArrows {
                source: false,
                target: true,
            },
            crate::EdgeArrows {
                source: true,
                target: true,
            },
            crate::EdgeArrows {
                source: false,
                target: false,
            },
        ] {
            let mut input = Graph::default();
            let first = input.add_node(node("first", 80.0, 80.0));
            let second = input.add_node(node("second", 80.0, 80.0));
            for label in ["first label", "second label", "third label"] {
                let edge = input.add_edge(Edge {
                    source: first,
                    target: second,
                });
                input.set_edge_arrows(edge, arrows);
                input.set_edge_label(
                    edge,
                    Some(crate::EdgeLabel {
                        text: label.into(),
                        size: Size {
                            width: 80.0,
                            height: 20.0,
                        },
                        position: LabelPosition::OutsideBottomCenter,
                        percentage: 0.5,
                    }),
                );
            }
            let arena = ArenaGraph::from_input(&input);
            let first_direction = if arrows.source && !arrows.target {
                Orientation::Right
            } else {
                Orientation::Left
            };
            for (endpoint, expected) in [
                (first, first_direction),
                (second, first_direction.opposite()),
            ] {
                assert_eq!(
                    arena.sized_edge_length_direction(
                        endpoint,
                        &arena.nodes[endpoint.0 as usize].edges.read(),
                        None
                    ),
                    (expected, 10.0, true),
                    "sized source count: {arrows:?}"
                );
                assert_eq!(
                    arena.edge_length_direction(endpoint, true, true),
                    (expected, 10.0),
                    "shared source count: {arrows:?}"
                );
            }
        }
    }

    fn node(name: &str, width: f64, height: f64) -> crate::Node {
        crate::Node {
            d2_near_key: None,
            external_id: name.into(),
            size: Size { width, height },
            declared_size: None,
            d2_authored_geometry: None,
            label_size: None,
            font_size: None,
            label_position: LabelPosition::Unset,
            parent: None,
            locked_position: None,
            constrained_x: None,
            constrained_y: None,
            near: None,
            fixed_width: false,
            fixed_height: false,
            direction: None,
            force_hierarchy: false,
            grid_rows: None,
            grid_columns: None,
            canvas_position: None,
            content_insets: Insets::uniform(60.0),
            layout_margins: Insets::uniform(0.0),
            external_label: None,
            icon_position: None,
            has_icon: false,
            label_aware_grid: false,
            packed_grid: false,
            content_alignment: crate::ContentAlignment::Padding,
            port_spread: 0.0,
            person: false,
            is_3d: false,
            is_multiple: false,
            shape: ShapeKind::Rectangle,
        }
    }

    #[test]
    fn projected_outer_direction_uses_retained_child_depth() {
        for retained_view in 0..4 {
            let mut input = Graph::with_direction(Direction::Right);
            let carrier = input.add_node(node("group", 420.0, 340.0));
            let outside = input.add_node(node("outside", 100.0, 100.0));
            let mut arena = ArenaGraph::from_input(&input);
            let parent_id = arena.nodes[carrier.0 as usize].tala_id;
            let child_id = 30_001;
            let mut child = arena.nodes[carrier.0 as usize].clone();
            child.tala_id = child_id;
            child.scoring_container_parent = Some(parent_id);
            child.scoring_container_ancestors = vec![parent_id];
            match retained_view {
                0 => arena.transaction_external_containers.push(child),
                1 => {
                    arena
                        .transaction_external_container_children
                        .insert(parent_id, vec![child]);
                }
                2 => {
                    arena
                        .transaction_external_aggregate_children
                        .insert(parent_id, vec![child]);
                }
                _ => arena
                    .projected_transaction_nodes
                    .push(ProjectedTransactionNode {
                        structural_container_tala_id: Some(parent_id),
                        structural_ancestor_tala_ids: vec![parent_id],
                        node: child,
                        container_tala_id: Some(parent_id),
                        ancestor_tala_ids: vec![parent_id],
                        child_tala_ids: Vec::new(),
                        edges: Vec::new(),
                    }),
            }
            let projected = ProjectedAdjacent {
                owner: carrier,
                tala_id: child_id,
                container_tala_id: Some(parent_id),
                offset: Point { x: 280.0, y: 200.0 },
                size: Size {
                    width: 80.0,
                    height: 80.0,
                },
                cluster_member: false,
            };
            // The child is absent from this placement graph's node slice.
            // Go restores its pointer and selects the shallower outside node,
            // whose root direction exists; the carrier's depth is irrelevant.
            assert!(
                arena.projected_outer_direction_is_set(Some(projected), None, carrier, outside),
                "retained view {retained_view}"
            );
            assert!(
                arena.projected_outer_direction_is_set(None, Some(projected), outside, carrier),
                "reversed retained view {retained_view}"
            );

            // Only the selected outer endpoint's container controls the gate.
            arena.scoring_directions.clear();
            arena.scoring_directions_by_tala.clear();
            arena
                .scoring_directions_by_tala
                .insert(Some(parent_id), Direction::Right);
            assert!(
                !arena.projected_outer_direction_is_set(Some(projected), None, carrier, outside),
                "inner direction must not count for retained view {retained_view}"
            );
        }
    }

    #[test]
    fn projected_symmetry_matches_an_original_member_before_vessel_substitution() {
        const CHILD_TALA_ID: i64 = 30_001;
        const VESSEL_TALA_ID: i64 = 30_002;
        const SOURCE_CONTAINER_TALA_ID: i64 = 30_003;

        let mut input = Graph::default();
        let coordinator = input.add_node(node("coordinator", 100.0, 100.0));
        let carrier = input.add_node(node("carrier", 100.0, 100.0));
        let left = input.add_node(node("left", 100.0, 100.0));
        let right = input.add_node(node("right", 100.0, 100.0));
        let abducted = input.add_edge(Edge {
            source: coordinator,
            target: carrier,
        });
        input.add_edge(Edge {
            source: coordinator,
            target: left,
        });
        input.add_edge(Edge {
            source: coordinator,
            target: right,
        });

        let mut arena = ArenaGraph::from_input(&input);
        arena.cell_size = 100.0;
        arena.set_position(coordinator, Point { x: 200.0, y: 100.0 });
        arena.set_position(carrier, Point { x: 600.0, y: 100.0 });
        arena.set_position(left, Point { x: 0.0, y: 100.0 });
        arena.set_position(right, Point { x: 400.0, y: 100.0 });
        arena.active_aggregate_vessels_by_member = std::sync::Arc::new(
            std::collections::HashMap::from([(CHILD_TALA_ID, VESSEL_TALA_ID)]),
        );

        let child = ProjectedAdjacent {
            owner: carrier,
            tala_id: CHILD_TALA_ID,
            container_tala_id: Some(SOURCE_CONTAINER_TALA_ID),
            offset: Point::default(),
            size: Size {
                width: 100.0,
                height: 100.0,
            },
            cluster_member: true,
        };
        arena
            .sized_adjacent_overrides
            .insert((coordinator, abducted), child);
        arena
            .sized_edge_abductions
            .push(crate::internal::model::SizedEdgeAbduction {
                source_edge: abducted,
                edge_tala_id: arena.edge_records[abducted.0 as usize].tala_id,
                edge: Some(abducted),
                current_from: Some(AbductionCurrentNodeRef::local(coordinator)),
                current_to: Some(AbductionCurrentNodeRef::local(carrier)),
                originally_from: None,
                originally_to: Some(child.into()),
                originally_from_table_neighbors: Vec::new(),
                originally_to_table_neighbors: Vec::new(),
                originally_from_container: None,
                originally_to_container: None,
                sequence_abduction: false,
                obstructions_from_to: Vec::new(),
                obstructions_to_from: Vec::new(),
                obstruction_sets_from_to: OrderedObstructionInventory::default(),
                obstruction_sets_to_from: OrderedObstructionInventory::default(),
            });
        arena.sized_cluster_distance_boxes.insert(
            (carrier, CHILD_TALA_ID),
            ProjectedClusterDistance {
                offset: Point::default(),
                size: child.size,
                arrangement: ClusterArrangement::Row,
                vessel_tala_id: VESSEL_TALA_ID,
                external_connected: Vec::new(),
            },
        );

        let center = arena.projected_symmetry_neighbor(child);
        assert_eq!(center.container_tala_id, None);
        // The raw OriginallyTo pointer identifies CHILD_TALA_ID. Only after
        // that comparison may getSymmetry substitute VESSEL_TALA_ID. Its
        // direct Container is nil while the aggregate is active, so symmetry
        // uses the same root sibling scope as Go. The unmatched coordinator
        // then contributes its left/right symmetry.
        assert_eq!(arena.projected_original_symmetry(center, true), 2.0 / 3.0);
    }

    #[test]
    fn direct_center_uses_projected_neighbor_scope_for_symmetry_obstructions() {
        const FIRST_TALA_ID: i64 = 10_001;
        const SECOND_TALA_ID: i64 = 10_002;
        const OBSTRUCTION_TALA_ID: i64 = 10_003;

        let mut input = Graph::default();
        let center = input.add_node(node("center", 100.0, 100.0));
        let carrier = input.add_node(node("carrier", 100.0, 100.0));
        let mut arena = ArenaGraph::from_input(&input);
        arena.cell_size = 100.0;
        arena.set_position(center, Point { x: 200.0, y: 100.0 });
        arena.set_position(carrier, Point { x: 0.0, y: 0.0 });

        let carrier_tala_id = arena.nodes[carrier.0 as usize].tala_id;
        let projected_size = Size {
            width: 100.0,
            height: 100.0,
        };
        let first = SymmetryNeighbor {
            identity: FIRST_TALA_ID,
            owner: carrier,
            projected: true,
            container_tala_id: Some(carrier_tala_id),
            position: Some(Point { x: 0.0, y: 0.0 }),
            size: projected_size,
        };
        let second = SymmetryNeighbor {
            identity: SECOND_TALA_ID,
            owner: carrier,
            projected: true,
            container_tala_id: Some(carrier_tala_id),
            position: Some(Point { x: 0.0, y: 200.0 }),
            size: projected_size,
        };

        let template = arena.nodes[carrier.0 as usize].clone();
        let snapshot = |tala_id: i64, position: Point, size: Size| {
            let mut child = template.clone();
            child.tala_id = tala_id;
            child.position = Some(position);
            child.rect.origin = position;
            child.rect.size = size;
            child
        };
        arena.transaction_external_container_children.insert(
            carrier_tala_id,
            vec![
                snapshot(FIRST_TALA_ID, first.positioned(), first.size),
                snapshot(SECOND_TALA_ID, second.positioned(), second.size),
                snapshot(
                    OBSTRUCTION_TALA_ID,
                    Point { x: 120.0, y: 70.0 },
                    Size {
                        width: 60.0,
                        height: 60.0,
                    },
                ),
            ],
        );

        let direct_center = arena.direct_symmetry_neighbor(center);
        assert_eq!(
            arena.symmetry_pair_score_from_center(direct_center, &[first, second]),
            (0.0, Vec::new())
        );

        let clear_position = Point { x: 500.0, y: 500.0 };
        let obstruction = &mut arena
            .transaction_external_container_children
            .get_mut(&carrier_tala_id)
            .unwrap()[2];
        obstruction.position = Some(clear_position);
        obstruction.rect.origin = clear_position;
        assert_eq!(
            arena.symmetry_pair_score_from_center(direct_center, &[first, second]),
            (0.5, vec![FIRST_TALA_ID, SECOND_TALA_ID])
        );
    }

    #[test]
    fn projected_symmetry_skips_ancestor_but_blocks_unrelated_sibling() {
        const PARENT_TALA_ID: i64 = 20_001;
        const ANCESTOR_TALA_ID: i64 = 20_002;
        const CENTER_TALA_ID: i64 = 20_003;
        const FIRST_TALA_ID: i64 = 20_004;
        const SECOND_TALA_ID: i64 = 20_005;
        const BLOCKER_TALA_ID: i64 = 20_006;

        let mut input = Graph::default();
        let carrier = input.add_node(node("carrier", 100.0, 100.0));
        let mut arena = ArenaGraph::from_input(&input);
        arena.cell_size = 100.0;
        arena.set_position(carrier, Point { x: 0.0, y: 0.0 });

        let projected_size = Size {
            width: 100.0,
            height: 100.0,
        };
        let center = SymmetryNeighbor {
            identity: CENTER_TALA_ID,
            owner: carrier,
            projected: true,
            container_tala_id: Some(ANCESTOR_TALA_ID),
            position: Some(Point { x: 200.0, y: 100.0 }),
            size: projected_size,
        };
        let first = SymmetryNeighbor {
            identity: FIRST_TALA_ID,
            owner: carrier,
            projected: true,
            container_tala_id: Some(PARENT_TALA_ID),
            position: Some(Point { x: 0.0, y: 100.0 }),
            size: projected_size,
        };
        let second = SymmetryNeighbor {
            identity: SECOND_TALA_ID,
            owner: carrier,
            projected: true,
            container_tala_id: Some(PARENT_TALA_ID),
            position: Some(Point { x: 400.0, y: 100.0 }),
            size: projected_size,
        };

        let template = arena.nodes[carrier.0 as usize].clone();
        let snapshot =
            |tala_id: i64, position: Point, size: Size, parent: Option<i64>, ancestors: &[i64]| {
                let mut child = template.clone();
                child.tala_id = tala_id;
                child.position = Some(position);
                child.rect.origin = position;
                child.rect.size = size;
                child.scoring_container_parent = parent;
                child.scoring_container_ancestors = ancestors.to_vec();
                child
            };
        arena.transaction_external_container_children.insert(
            ANCESTOR_TALA_ID,
            vec![snapshot(
                CENTER_TALA_ID,
                center.positioned(),
                center.size,
                Some(ANCESTOR_TALA_ID),
                &[ANCESTOR_TALA_ID, PARENT_TALA_ID],
            )],
        );
        arena.transaction_external_container_children.insert(
            PARENT_TALA_ID,
            vec![
                // This box encloses the center and intersects both symmetry
                // rays. Go skips it because the center is its descendant.
                snapshot(
                    ANCESTOR_TALA_ID,
                    Point { x: 150.0, y: 50.0 },
                    Size {
                        width: 200.0,
                        height: 200.0,
                    },
                    Some(PARENT_TALA_ID),
                    &[PARENT_TALA_ID],
                ),
                snapshot(
                    FIRST_TALA_ID,
                    first.positioned(),
                    first.size,
                    Some(PARENT_TALA_ID),
                    &[PARENT_TALA_ID],
                ),
                snapshot(
                    SECOND_TALA_ID,
                    second.positioned(),
                    second.size,
                    Some(PARENT_TALA_ID),
                    &[PARENT_TALA_ID],
                ),
            ],
        );

        assert_eq!(
            arena.symmetry_pair_score_from_center(center, &[first, second]),
            (2.0, vec![FIRST_TALA_ID, SECOND_TALA_ID])
        );

        // An unrelated sibling on the same ray remains a real obstruction.
        arena
            .transaction_external_container_children
            .get_mut(&PARENT_TALA_ID)
            .unwrap()
            .push(snapshot(
                BLOCKER_TALA_ID,
                Point { x: 120.0, y: 120.0 },
                Size {
                    width: 60.0,
                    height: 60.0,
                },
                Some(PARENT_TALA_ID),
                &[PARENT_TALA_ID],
            ));
        assert_eq!(
            arena.symmetry_pair_score_from_center(center, &[first, second]),
            (0.0, Vec::new())
        );
    }

    #[test]
    fn symmetry_empty_neighbors_keeps_nil_position_until_source_geometry_guard() {
        // Go symmetry.go:398 checks cancellation then returns for <2 neighbors,
        // before dereferencing the receiver's TopLeft. No coordinates needed.
        let (mut arena, center) = checked_scoring_fixture(0);
        arena.clear_position(center);
        let context = PollContext {
            polls: Cell::new(0),
            cancel_at: None,
        };
        assert_eq!(
            arena
                .sized_symmetry_checked(center, true, ScoreContext::new(&context))
                .unwrap()
                .to_bits(),
            0.0_f64.to_bits()
        );
        assert_eq!(context.polls.get(), 2);
        assert!(arena.position(center).is_none());
        let context = PollContext {
            polls: Cell::new(0),
            cancel_at: Some(2),
        };
        assert_eq!(
            arena
                .sized_symmetry_checked(center, true, ScoreContext::new(&context))
                .unwrap_err()
                .to_string(),
            "EdgeLength: context canceled"
        );
        assert_eq!(context.polls.get(), 2);
        assert!(arena.position(center).is_none());
    }

    #[test]
    fn retained_symmetry_reads_positioned_child_without_positioning_its_owner() {
        // Original-Go nil-owner controls keep A.TopLeft=(20,20) while C
        // stays nil, including after a missing relative-offset reconciliation.
        let (mut arena, owner) = checked_scoring_fixture(0);
        arena.clear_position(owner);
        let mut child = arena.nodes[owner.0 as usize].clone();
        child.tala_id += 1000;
        child.position = Some(Point { x: 20.0, y: 20.0 });
        let original = AbductedNodeRef {
            source_node: None,
            source_owner: None,
            owner_tala_id: None,
            owner: Some(owner),
            tala_id: child.tala_id,
            container_tala_id: Some(arena.nodes[owner.0 as usize].tala_id),
            offset: Some(Point { x: 20.0, y: 20.0 }),
            size: child.rect.size,
            cluster_member: false,
        };
        arena
            .transaction_external_container_children
            .insert(original.container_tala_id.unwrap(), vec![child]);
        for offset in [original.offset, None] {
            let retained = AbductedNodeRef { offset, ..original };
            let neighbor = arena.abducted_symmetry_neighbor(retained).unwrap();
            assert_eq!(neighbor.identity, original.tala_id);
            assert_eq!(neighbor.position, Some(Point { x: 20.0, y: 20.0 }));
            assert_eq!(
                arena
                    .abducted_symmetry_neighbor_with_active_vessel(retained)
                    .unwrap()
                    .position,
                neighbor.position
            );
            assert!(arena.position(owner).is_none());
        }
    }

    #[test]
    fn nil_owner_symmetry_does_not_invent_geometry_from_offsets_or_stale_aliases() {
        let (mut arena, owner) = checked_scoring_fixture(0);
        arena.clear_position(owner);
        let mut child = arena.nodes[owner.0 as usize].clone();
        child.tala_id += 1000;
        child.position = None;
        let mut stale = child.clone();
        stale.position = Some(Point { x: 999.0, y: 999.0 });
        let original = AbductedNodeRef {
            source_node: None,
            source_owner: None,
            owner_tala_id: None,
            owner: Some(owner),
            tala_id: child.tala_id,
            container_tala_id: Some(arena.nodes[owner.0 as usize].tala_id),
            offset: Some(Point { x: 20.0, y: 20.0 }),
            size: child.rect.size,
            cluster_member: false,
        };
        // An offset alone cannot establish actual child geometry.
        assert!(
            arena
                .abducted_symmetry_neighbor(original)
                .unwrap()
                .position
                .is_none()
        );
        arena
            .transaction_external_container_children
            .insert(1, vec![child]);
        arena
            .transaction_external_aggregate_children
            .insert(2, vec![stale]);
        for offset in [original.offset, None] {
            let retained = AbductedNodeRef { offset, ..original };
            assert!(
                arena
                    .abducted_symmetry_neighbor(retained)
                    .unwrap()
                    .position
                    .is_none()
            );
            assert!(
                arena
                    .abducted_symmetry_neighbor_with_active_vessel(retained)
                    .unwrap()
                    .position
                    .is_none()
            );
        }
        assert!(arena.position(owner).is_none());
    }

    #[test]
    fn missing_original_geometry_stops_at_authoritative_nil_before_stale_alias() {
        let (mut arena, owner) = checked_scoring_fixture(0);
        let mut child = arena.nodes[owner.0 as usize].clone();
        child.tala_id += 1000;
        child.position = None;
        let mut stale = child.clone();
        stale.position = Some(Point { x: 100.0, y: 100.0 });
        let original = AbductedNodeRef {
            source_node: None,
            source_owner: None,
            owner_tala_id: None,
            owner: Some(owner),
            tala_id: child.tala_id,
            container_tala_id: Some(arena.nodes[owner.0 as usize].tala_id),
            offset: None,
            size: child.rect.size,
            cluster_member: false,
        };
        arena
            .transaction_external_container_children
            .insert(1, vec![child]);
        arena
            .transaction_external_aggregate_children
            .insert(2, vec![stale]);
        assert!(arena.abducted_node_projection(original).is_none());
        arena
            .transaction_external_container_children
            .get_mut(&1)
            .unwrap()[0]
            .position = Some(Point { x: 200.0, y: 300.0 });
        let projected = arena
            .abducted_node_projection(original)
            .expect("real child placement");
        assert_eq!(projected.tala_id, original.tala_id);
        let owner_position = arena.position(owner).unwrap();
        assert_eq!(
            projected.offset,
            Point {
                x: 200.0 - owner_position.x,
                y: 300.0 - owner_position.y
            }
        );
    }

    #[test]
    fn symmetry_pair_filters_and_cancellation_precede_nil_center_geometry() {
        let (mut arena, owner) = checked_scoring_fixture(0);
        let mut first = arena.direct_symmetry_neighbor(owner);
        let mut second = first;
        first.identity = 100;
        first.container_tala_id = Some(1);
        second.identity = 101;
        second.container_tala_id = Some(2);
        arena.clear_position(owner);
        let center = arena.direct_symmetry_neighbor(owner);
        let context = PollContext {
            polls: Cell::new(0),
            cancel_at: None,
        };
        let (score, matched) = arena
            .symmetry_pair_score_from_center_checked(
                center,
                &[first, second],
                ScoreContext::new(&context),
            )
            .unwrap();
        assert_eq!(score, 0.0);
        assert!(matched.iter().all(|&flag| !flag));
        assert!(arena.position(owner).is_none());
        let context = PollContext {
            polls: Cell::new(0),
            cancel_at: Some(2),
        };
        assert_eq!(
            arena
                .symmetry_pair_score_from_center_checked(
                    center,
                    &[first, second],
                    ScoreContext::new(&context)
                )
                .unwrap_err()
                .to_string(),
            "EdgeLength: context canceled"
        );
        assert_eq!(context.polls.get(), 2);
        assert!(arena.position(owner).is_none());
    }
    #[test]
    fn sized_candidate_nears_read_live_foreign_boxes_and_nil_positions() {
        // Source NodeEdgeLength takes the minimum over actual Near pointers,
        // not only the peers retained in the temporary component slice.
        let mut input = crate::Graph::default();
        let x = input.add_node(node("x", 53.0, 66.0));
        let mut y_node = node("y", 54.0, 66.0);
        y_node.near = Some(x);
        let y = input.add_node(y_node);
        let mut owner = ArenaGraph::from_input(&input);
        owner.set_position(x, Point { x: 0.0, y: 66.0 });
        owner.set_position(y, Point { x: 132.0, y: 0.0 });
        let workspace = owner.clone();
        let mut foreign = owner.nodes[y.0 as usize].clone();
        foreign.input_id = NodeId(2);
        foreign.tala_id += 100;
        foreign.external_id = "box.a".into();
        foreign.rect.size.width = 53.0;
        foreign.position = Some(Point { x: 60.0, y: 60.0 });
        let foreign_id = foreign.input_id;
        owner.nodes.push(foreign);
        owner.nodes[x.0 as usize].nears.push(foreign_id);
        assert_eq!(workspace.sized_edge_length_with_cache(x, true, None), 79.0);
        assert_eq!(
            workspace.sized_edge_length_with_owner_nears(x, true, None, &owner, x),
            7.0
        );
        owner.set_position(foreign_id, Point { x: 70.0, y: 60.0 });
        assert_eq!(
            workspace.sized_edge_length_with_owner_nears(x, true, None, &owner, x),
            17.0
        );
        owner.clear_position(foreign_id);
        assert_eq!(
            workspace.sized_edge_length_with_owner_nears(x, true, None, &owner, x),
            0.0
        );
        assert_eq!(workspace.sized_edge_length_with_cache(x, true, None), 79.0);
    }
}

#[cfg(test)]
mod borrowed_owner_route_tests {
    use super::*;
    use std::cell::Cell;

    struct HeaderPollContext {
        polls: Cell<usize>,
        old: SharedNodeSlice,
        map: SharedSliceMap,
        replacement: Option<NodeId>,
        cancel_item: bool,
    }
    impl limits::Context for HeaderPollContext {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<limits::ContextError> {
            let poll = self.polls.get() + 1;
            self.polls.set(poll);
            if poll == 1 {
                // Source903 occurs after capture. Replacing the map header
                // must not replace the scorer's retained header.
                self.map.replace(None, SharedNodeSlice::nil()).unwrap();
            }
            if poll == 2 {
                if self.cancel_item {
                    return Some(limits::ContextError::Cancelled);
                }
                // Source907 precedes the actual pointer-slot read.
                self.old.write_capacity_slot(0, self.replacement).unwrap();
            }
            None
        }
    }

    fn fixture(
        child_obstruction: bool,
    ) -> (
        ArenaGraph,
        ArenaGraph,
        Vec<NodeId>,
        OwnerGraphId,
        EdgeId,
        NodeId,
    ) {
        fixture_in_scope(child_obstruction, false)
    }

    fn fixture_in_scope(
        child_obstruction: bool,
        shared_parent: bool,
    ) -> (
        ArenaGraph,
        ArenaGraph,
        Vec<NodeId>,
        OwnerGraphId,
        EdgeId,
        NodeId,
    ) {
        let mut input = Graph::default();
        let a = input.add_node(crate::Node::new(
            "a",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let b = input.add_node(crate::Node::new(
            "b",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let mut obstruction = crate::Node::new(
            "obstruction",
            Size {
                width: 60.0,
                height: 60.0,
            },
        );
        obstruction.parent = child_obstruction.then_some(a);
        let c = input.add_node(obstruction);
        let e = input.add_edge(crate::Edge {
            source: a,
            target: b,
        });
        let parent = shared_parent.then(|| {
            let parent = input.add_node(crate::Node::new(
                "scope",
                Size {
                    width: 300.0,
                    height: 300.0,
                },
            ));
            for child in [a, b, c] {
                input.node_mut(child).unwrap().parent = Some(parent);
            }
            parent
        });
        let mut owner = ArenaGraph::from_input(&input);
        owner.set_position(a, Point { x: 0.0, y: 0.0 });
        owner.set_position(b, Point { x: 200.0, y: 0.0 });
        owner.set_position(c, Point { x: 80.0, y: -20.0 });
        owner.initialize_shared_topology();
        let view = owner.create_owner_graph_view(vec![a, b], vec![e]).unwrap();
        let mut journal = OwnerGraphOwnershipJournal::default();
        owner
            .set_owner_graph_reference(view, &[a, b, c], false, &mut journal)
            .unwrap();
        owner
            .containers
            .replace_header(parent, SharedNodeSlice::from_nodes(&[a]).unwrap());
        let (workspace, mapping) = owner.induced_placement_subgraph(&[a, b]);
        owner
            .set_owner_view_node_mapping(view, mapping.clone())
            .unwrap();
        (owner, workspace, mapping, view, e, c)
    }

    fn score(
        owner: &ArenaGraph,
        workspace: &ArenaGraph,
        mapping: &[NodeId],
        view: OwnerGraphId,
        context: &HeaderPollContext,
    ) -> Result<f64, limits::Error> {
        let frame =
            OwnerScoreFrame::new(owner, owner.topology_arena_id().unwrap(), view, mapping).unwrap();
        workspace.edge_route_penalty_with_owner_frame_checked(
            NodeId(0),
            NodeId(1),
            EdgeId(0),
            None,
            None,
            ProjectedObstructionSource::Current,
            false,
            false,
            Some((&frame, mapping[0], mapping[1])),
            None,
            ScoreContext::new(context),
        )
    }

    #[test]
    fn owner_cluster_side_penalty_uses_actual_external_pointer_geometry() {
        let (mut owner, workspace, mapping, view, edge, vessel) = fixture(false);
        let member = mapping[0];
        let other = mapping[1];
        let mut external = owner.nodes[other.0 as usize].clone();
        external.tala_id = 900_001;
        external.position = Some(Point { x: 300.0, y: 10.0 });
        let foreign = owner.retain_topology_node(external);
        owner
            .set_owner_graph_reference(
                view,
                &[foreign],
                false,
                &mut OwnerGraphOwnershipJournal::default(),
            )
            .unwrap();
        let group = owner.new_owner_group(super::super::owner_groups::OwnerGroupKind::Cluster);
        let nodes = owner
            .allocate_owner_member_header(&[Some(member)], 1)
            .unwrap();
        owner
            .set_owner_group_fields(
                group,
                super::super::shared_topology::OwnerGroupFieldView {
                    vessel: Some(vessel),
                    nodes,
                    graph: Some(view),
                    container: None,
                },
            )
            .unwrap();
        owner
            .set_owner_node_group(
                member,
                super::super::owner_groups::OwnerGroupKind::Cluster,
                Some(group),
            )
            .unwrap();
        let records = owner
            .register_owner_edge_abductions(
                [other, foreign]
                    .into_iter()
                    .map(|external| OwnerEdgeAbduction {
                        source_edge: Some(edge),
                        originally_from: Some(member),
                        originally_to: None,
                        current_from: Some(vessel),
                        current_to: Some(external),
                    })
                    .collect(),
                true,
            )
            .unwrap();
        let header = owner
            .allocate_owner_abduction_header(
                &records
                    .records
                    .iter()
                    .copied()
                    .map(Some)
                    .collect::<Vec<_>>(),
                2,
                true,
            )
            .unwrap();
        owner
            .set_owner_cluster_policy(
                group,
                super::super::owner_groups::OwnerClusterPolicy {
                    arrangement: "Column".into(),
                    edge_abductions: Some(header),
                    ..Default::default()
                },
            )
            .unwrap();
        let read = |owner: &ArenaGraph| {
            let frame =
                OwnerScoreFrame::new(owner, owner.topology_arena_id().unwrap(), view, &mapping)
                    .unwrap();
            workspace
                .sized_owner_cluster_arrangement_penalty_checked(
                    &frame,
                    member,
                    other,
                    Orientation::Left,
                    20.0,
                )
                .unwrap()
                .unwrap()
        };
        assert_eq!(read(&owner), [0.0, 20.0, 5.0]);
        owner.set_position(foreign, Point { x: -200.0, y: 10.0 });
        assert_eq!(read(&owner), [0.0, 0.0, 5.0]);
        owner.clear_position(vessel);
        assert_eq!(read(&owner), [0.0, 0.0, 20.0]);
        owner
            .set_owner_node_group(
                member,
                super::super::owner_groups::OwnerGroupKind::Cluster,
                None,
            )
            .unwrap();
        let frame =
            OwnerScoreFrame::new(&owner, owner.topology_arena_id().unwrap(), view, &mapping)
                .unwrap();
        assert_eq!(
            workspace
                .sized_owner_cluster_arrangement_penalty_checked(
                    &frame,
                    member,
                    other,
                    Orientation::Left,
                    20.0
                )
                .unwrap(),
            None
        );
    }

    #[test]
    fn candidate_route_reads_live_foreign_owner_box_and_current_header() {
        let (mut owner, workspace, mapping, view, edge, obstruction) = fixture(false);
        owner.set_owner_graph_cell_size(view, 20.0).unwrap();
        // The workspace was already cloned with a one-child header. Source
        // scoring reads the current actual header, including a foreign node
        // absent from the dense mapping, and its current nullable TopLeft.
        owner.containers.replace_header(
            None,
            SharedNodeSlice::from_nodes(&[mapping[0], obstruction]).unwrap(),
        );
        let records = OwnerAbductionSlice {
            records: Vec::new(),
            present: true,
            arena: owner.topology_arena_id(),
        };
        let edges = [edge];
        let candidate = |owner: &ArenaGraph| {
            let frame =
                OwnerScoreFrame::new(owner, owner.topology_arena_id().unwrap(), view, &mapping)
                    .unwrap()
                    .with_edge_mapping(&edges)
                    .unwrap()
                    .with_abductions(&records)
                    .unwrap();
            workspace
                .sized_edge_length_with_owner_routes(
                    NodeId(0),
                    true,
                    None,
                    owner,
                    mapping[0],
                    Some(&frame),
                )
                .unwrap()
        };
        let blocked = candidate(&owner);
        owner.set_position(
            obstruction,
            Point {
                x: 1000.0,
                y: 1000.0,
            },
        );
        let clear = candidate(&owner);
        assert!(
            blocked > clear,
            "the actual live box must change the route penalty"
        );
        owner.clear_position(obstruction);
        assert_eq!(
            candidate(&owner).to_bits(),
            clear.to_bits(),
            "nil Source TopLeft must stay nil"
        );
    }

    #[test]
    fn source_903_retains_header_and_907_reads_live_backing_slot() {
        let (owner, workspace, mapping, view, _, obstruction) = fixture(false);
        let old = owner.containers.get_header(&None).unwrap();
        let context = HeaderPollContext {
            polls: Cell::new(0),
            old: old.clone(),
            map: owner.containers.share_map(),
            replacement: Some(obstruction),
            cancel_item: false,
        };
        let result = score(&owner, &workspace, &mapping, view, &context).unwrap();
        assert!(result > 0.0, "live slot replacement must block the route");
        assert_eq!(context.polls.get(), 2);
        assert_eq!(old.get(0), Some(Some(obstruction)));
        assert!(owner.containers.get_header(&None).unwrap().is_nil());
        assert!(
            !owner
                .owner_graph_view(view)
                .unwrap()
                .nodes
                .iter_slots()
                .any(|slot| slot == Some(obstruction))
        );
    }

    #[test]
    fn endpoint_cluster_vessel_is_not_double_counted_as_route_obstruction() {
        use crate::internal::owner_groups::OwnerGroupKind;
        use crate::internal::shared_topology::OwnerGroupFieldView;
        // Source edge_length.go:928–933 guards either replacement endpoint,
        // not every cluster-shaped obstruction. Keep the actual route blocker
        // in the captured owner header and vary only raw cluster identity.
        for endpoint in [0, 1] {
            for (vessel_flag, matching_vessel) in [(true, true), (true, false), (false, false)] {
                let (mut owner, workspace, mapping, view, _, obstruction) =
                    fixture_in_scope(false, true);
                owner
                    .set_owner_cluster_vessel_flag(obstruction, vessel_flag)
                    .unwrap();
                let group = owner.new_owner_group(OwnerGroupKind::Cluster);
                let fields = owner.owner_group_fields(group).unwrap();
                owner
                    .set_owner_group_fields(
                        group,
                        OwnerGroupFieldView {
                            vessel: Some(obstruction),
                            nodes: fields.nodes,
                            graph: None,
                            container: None,
                        },
                    )
                    .unwrap();
                if matching_vessel {
                    owner
                        .set_owner_node_group(
                            mapping[endpoint],
                            OwnerGroupKind::Cluster,
                            Some(group),
                        )
                        .unwrap();
                }
                let old = owner
                    .containers
                    .get_header(&owner.nodes[mapping[0].0 as usize].container)
                    .unwrap();
                let context = HeaderPollContext {
                    polls: Cell::new(0),
                    old,
                    map: owner.containers.share_map(),
                    replacement: Some(obstruction),
                    cancel_item: false,
                };
                let result = score(&owner, &workspace, &mapping, view, &context).unwrap();
                if vessel_flag && matching_vessel {
                    assert_eq!(result, 0.0, "endpoint cluster cost is charged separately");
                } else {
                    assert!(
                        result > 0.0,
                        "unrelated or non-vessel blockers still count: endpoint={endpoint} flag={vessel_flag} matching={matching_vessel}"
                    );
                }
                assert_eq!(
                    context.polls.get(),
                    if matching_vessel { 3 } else { 2 },
                    // A blocked route exits after the first set. A skipped
                    // endpoint vessel visits the second, empty ancestor set.
                    "Source polling precedes vessel guard and retains early route exit"
                );
            }
        }
    }

    #[test]
    fn source_907_cancellation_precedes_invalid_canonical_slot_lookup() {
        let (owner, workspace, mapping, view, _, _) = fixture(false);
        let old = owner.containers.get_header(&None).unwrap();
        old.write_capacity_slot(0, Some(NodeId(999))).unwrap();
        let context = HeaderPollContext {
            polls: Cell::new(0),
            old,
            map: owner.containers.share_map(),
            replacement: None,
            cancel_item: true,
        };
        assert_eq!(
            score(&owner, &workspace, &mapping, view, &context)
                .unwrap_err()
                .to_string(),
            "EdgeLength: context canceled"
        );
        assert_eq!(context.polls.get(), 2);
    }

    #[test]
    fn obstruction_and_diagonal_costs_use_actual_node_graph_receiver() {
        let (owner, workspace, mapping, view, _, obstruction) = fixture(false);
        owner
            .restore_owner_graph_routing_costs(OwnerGraphId(0), (7.0, 99.0, 11.0))
            .unwrap();
        owner
            .restore_owner_graph_routing_costs(view, (17.0, 23.0, 19.0))
            .unwrap();
        workspace.restore_routing_costs((2.0, 3.0, 4.0));
        let context = HeaderPollContext {
            polls: Cell::new(0),
            old: owner.containers.get_header(&None).unwrap(),
            map: owner.containers.share_map(),
            replacement: Some(obstruction),
            cancel_item: false,
        };
        // edge_length.go's obstructed horizontal route adds two turns from
        // node.Graph; neither the map owner nor dense workspace supplies it.
        assert_eq!(
            score(&owner, &workspace, &mapping, view, &context).unwrap(),
            46.0
        );
        let frame =
            OwnerScoreFrame::new(&owner, owner.topology_arena_id().unwrap(), view, &mapping)
                .unwrap();
        let size = Size {
            width: 10.0,
            height: 10.0,
        };
        let diagonal = workspace
            .sized_edge_base_distance_between_with_owner_frame_checked(
                (Point { x: 0.0, y: 0.0 }, size),
                (Point { x: 100.0, y: 100.0 }, size),
                NodeId(0),
                Some(&frame),
            )
            .unwrap();
        assert_eq!(diagonal, 213.0);
        assert_eq!(workspace.raw_routing_costs(), (2.0, 3.0, 4.0));
        assert_eq!(
            owner.owner_graph_routing_costs(OwnerGraphId(0)).unwrap(),
            (7.0, 99.0, 11.0)
        );
        assert_eq!(
            owner.owner_graph_routing_costs(view).unwrap(),
            (17.0, 23.0, 19.0)
        );
    }

    #[test]
    fn canonical_flow_uses_live_container_map_and_pointer_identity() {
        let mut input = Graph::default();
        let size = Size {
            width: 10.0,
            height: 10.0,
        };
        let left = input.add_node(crate::Node::new("left", size));
        let center = input.add_node(crate::Node::new("center", size));
        let down = input.add_node(crate::Node::new("down", size));
        let incoming = input.add_edge(crate::Edge {
            source: left,
            target: center,
        });
        let outgoing = input.add_edge(crate::Edge {
            source: center,
            target: down,
        });
        let mut owner = ArenaGraph::from_input(&input);
        owner.set_position(left, Point { x: -100.0, y: 0.0 });
        owner.set_position(center, Point { x: 0.0, y: 0.0 });
        owner.set_position(down, Point { x: 0.0, y: 100.0 });
        owner.nodes[center.0 as usize].is_container = true;
        // Equal authored IDs cannot merge the two distinct geometric rays.
        owner.nodes[left.0 as usize].tala_id = 7;
        owner.nodes[down.0 as usize].tala_id = 7;
        owner.edge_records[incoming.0 as usize].target_arrow = true;
        owner.edge_records[outgoing.0 as usize].target_arrow = true;
        owner.initialize_shared_topology();
        let view = owner
            .create_owner_graph_view(vec![left, center, down], vec![incoming, outgoing])
            .unwrap();
        let mut journal = OwnerGraphOwnershipJournal::default();
        owner
            .set_owner_graph_reference(view, &[left, center, down], false, &mut journal)
            .unwrap();
        owner
            .containers
            .replace_header(Some(center), SharedNodeSlice::from_nodes(&[]).unwrap());
        owner
            .restore_owner_graph_routing_costs(OwnerGraphId(0), (1.0, 99.0, 2.0))
            .unwrap();
        owner
            .restore_owner_graph_routing_costs(view, (3.0, 23.0, 4.0))
            .unwrap();
        let (workspace, mapping) = owner.induced_placement_subgraph(&[left, center, down]);
        owner
            .set_owner_view_node_mapping(view, mapping.clone())
            .unwrap();
        let records = owner
            .register_owner_edge_abductions(Vec::new(), true)
            .unwrap();
        let score = |owner: &ArenaGraph| {
            let frame =
                OwnerScoreFrame::new(owner, owner.topology_arena_id().unwrap(), view, &mapping)
                    .unwrap()
                    .with_abductions(&records)
                    .unwrap();
            workspace
                .flow_continuity_with_owner_frame_checked(center, None, &frame)
                .unwrap()
        };
        assert_eq!(score(&owner), 23.0);
        owner
            .containers
            .replace_header(Some(center), SharedNodeSlice::from_nodes(&[left]).unwrap());
        assert_eq!(score(&owner), 0.0);
        assert_eq!(
            owner.owner_graph_routing_costs(OwnerGraphId(0)).unwrap(),
            (1.0, 99.0, 2.0)
        );
    }

    #[test]
    fn source_nil_ancestor_and_inverse_descendant_predicates_skip_obstructions() {
        for child in [false, true] {
            let (owner, workspace, mapping, view, _, obstruction) = fixture(child);
            let old = owner.containers.get_header(&None).unwrap();
            let context = HeaderPollContext {
                polls: Cell::new(0),
                old,
                map: owner.containers.share_map(),
                replacement: child.then_some(obstruction),
                cancel_item: false,
            };
            assert_eq!(
                score(&owner, &workspace, &mapping, view, &context).unwrap(),
                0.0
            );
            assert_eq!(context.polls.get(), 2);
        }
    }

    #[test]
    fn unbound_actual_edge_and_grouping_terms_are_rejected() {
        for shared_uncle in [false, true] {
            let (mut owner, workspace, mapping, view, edge, obstruction) = fixture(false);
            let records = owner
                .register_owner_edge_abductions(Vec::new(), true)
                .unwrap();
            if shared_uncle {
                owner
                    .common_uncle_siblings
                    .replace_header(SharedSliceMap::new());
                owner
                    .common_uncle_siblings
                    .insert(mapping[0], vec![mapping[1], obstruction]);
            } else {
                owner.edge_records[edge.0 as usize].target_arrow =
                    !workspace.edge_records[0].target_arrow;
            }
            let frame =
                OwnerScoreFrame::new(&owner, owner.topology_arena_id().unwrap(), view, &mapping)
                    .unwrap()
                    .with_abductions(&records)
                    .unwrap();
            let error = workspace
                .sized_edge_length_with_owner_frame_checked(
                    NodeId(0),
                    false,
                    shared_uncle,
                    false,
                    false,
                    true,
                    &frame,
                    ScoreContext::none(),
                )
                .unwrap_err()
                .to_string();
            assert!(error.contains(if shared_uncle {
                "grouping terms"
            } else {
                "workspace edge differs"
            }));
        }
    }

    #[test]
    fn canonical_incidence_duplicates_are_not_dropped_by_dense_graph_membership() {
        let (mut owner, workspace, mapping, view, edge, _) = fixture(false);
        let records = owner
            .register_owner_edge_abductions(Vec::new(), true)
            .unwrap();
        owner.nodes[mapping[0].0 as usize].edges.push(edge);
        let frame =
            OwnerScoreFrame::new(&owner, owner.topology_arena_id().unwrap(), view, &mapping)
                .unwrap()
                .with_abductions(&records)
                .unwrap();
        assert!(
            workspace
                .sized_edge_length_with_owner_frame_checked(
                    NodeId(0),
                    false,
                    false,
                    false,
                    false,
                    true,
                    &frame,
                    ScoreContext::none(),
                )
                .unwrap_err()
                .to_string()
                .contains("incident edge order")
        );
    }

    fn full_owner_score(
        owner: &ArenaGraph,
        workspace: &ArenaGraph,
        mapping: &[NodeId],
        view: OwnerGraphId,
        records: &OwnerAbductionSlice,
        direction_penalty: bool,
    ) -> f64 {
        let frame = OwnerScoreFrame::new(owner, owner.topology_arena_id().unwrap(), view, mapping)
            .unwrap()
            .with_abductions(records)
            .unwrap();
        workspace
            .sized_edge_length_with_owner_frame_checked(
                NodeId(0),
                direction_penalty,
                false,
                false,
                false,
                true,
                &frame,
                ScoreContext::none(),
            )
            .unwrap()
    }

    #[test]
    fn canonical_direction_and_cell_ignore_stale_workspace_values() {
        let (mut owner, mut workspace, mapping, view, edge, _) = fixture(false);
        let records = owner
            .register_owner_edge_abductions(Vec::new(), true)
            .unwrap();
        owner.edge_records[edge.0 as usize].target_arrow = true;
        workspace.edge_records[0].target_arrow = true;
        owner.directions.clear();
        owner.directions.insert(None, Direction::Up);
        workspace.directions.insert(None, Direction::Down);
        workspace
            .scoring_directions_by_tala
            .insert(None, Direction::Left);
        workspace.cell_size = 999.0;
        owner
            .set_owner_graph_cell_size(OwnerGraphId(0), 91.0)
            .unwrap();
        for cell in [7.0, 10.0, 37.0] {
            owner.set_owner_graph_cell_size(view, cell).unwrap();
            // Go 1188–1246: Top versus the actual semantic Right ray gives
            // compass delta2, specified factor6, and node.Graph.CellSize.
            assert_eq!(
                full_owner_score(&owner, &workspace, &mapping, view, &records, true),
                190.0 + 6.0 * 2.0 * cell * 0.25
            );
            assert_eq!(
                full_owner_score(&owner, &workspace, &mapping, view, &records, false),
                190.0
            );
        }
        assert_eq!(workspace.cell_size, 999.0);
        assert_eq!(owner.owner_graph_cell_size(OwnerGraphId(0)).unwrap(), 91.0);
    }

    #[test]
    fn canonical_direction_uses_actual_source_arrow_and_undirected_axis_weight() {
        let (mut owner, mut workspace, mapping, view, edge, _) = fixture(false);
        let records = owner
            .register_owner_edge_abductions(Vec::new(), true)
            .unwrap();
        owner.set_owner_graph_cell_size(view, 10.0).unwrap();
        owner.directions.clear();
        owner.directions.insert(None, Direction::Left);
        // Source arrows reverse DirectedEndpoints. Bidirectional and no
        // arrows keep declaration From and use the 0.1/0.9 axis mixture.
        for (source, target, expected) in [
            (false, true, 250.0),
            (true, false, 190.0),
            (false, false, 196.0),
            (true, true, 196.0),
        ] {
            owner.edge_records[edge.0 as usize].source_arrow = source;
            owner.edge_records[edge.0 as usize].target_arrow = target;
            workspace.edge_records[0].source_arrow = source;
            workspace.edge_records[0].target_arrow = target;
            assert_eq!(
                full_owner_score(&owner, &workspace, &mapping, view, &records, true),
                expected
            );
        }
        owner.directions.clear();
        for (source, target, expected) in [
            (false, true, 190.75),
            (true, false, 192.25),
            (false, false, 190.0),
            (true, true, 190.0),
        ] {
            owner.edge_records[edge.0 as usize].source_arrow = source;
            owner.edge_records[edge.0 as usize].target_arrow = target;
            workspace.edge_records[0].source_arrow = source;
            workspace.edge_records[0].target_arrow = target;
            assert_eq!(
                full_owner_score(&owner, &workspace, &mapping, view, &records, true),
                expected
            );
        }
    }

    fn original_ref(owner: &ArenaGraph, canonical: NodeId) -> AbductedNodeRef {
        let actual = &owner.nodes[canonical.0 as usize];
        AbductedNodeRef {
            source_node: Some(canonical),
            source_owner: Some(canonical),
            owner_tala_id: Some(actual.tala_id),
            owner: None,
            tala_id: actual.tala_id,
            container_tala_id: actual
                .container
                .map(|parent| owner.nodes[parent.0 as usize].tala_id),
            // Deliberately stale descriptor geometry. The canonical frame
            // reads the real Point/Rect rather than these projected values.
            offset: Some(Point { x: 999.0, y: 999.0 }),
            size: Size {
                width: 999.0,
                height: 999.0,
            },
            cluster_member: false,
        }
    }

    #[test]
    fn canonical_foreign_original_uses_raw_outer_depth_and_current_semantic_pointer() {
        let (mut owner, mut workspace, mapping, view, edge, child) = fixture(false);
        let carrier = mapping[0];
        let adjacent = mapping[1];
        owner.nodes[carrier.0 as usize].is_container = true;
        owner.nodes[carrier.0 as usize].rect.size = Size {
            width: 60.0,
            height: 60.0,
        };
        workspace.nodes[0].rect.size = owner.nodes[carrier.0 as usize].rect.size;
        owner.nodes[child.0 as usize].container = Some(carrier);
        owner.nodes[child.0 as usize].rect.size = Size {
            width: 10.0,
            height: 10.0,
        };
        owner.set_position(child, Point { x: 20.0, y: 0.0 });
        owner.containers.replace_header(
            Some(carrier),
            SharedNodeSlice::from_nodes(&[child]).unwrap(),
        );
        let mut journal = OwnerGraphOwnershipJournal::default();
        owner
            .set_owner_graph_reference(OwnerGraphId(0), &[child], false, &mut journal)
            .unwrap();
        owner.set_owner_graph_cell_size(view, 10.0).unwrap();
        owner.edge_records[edge.0 as usize].source_arrow = false;
        owner.edge_records[edge.0 as usize].target_arrow = false;
        workspace.edge_records[0].source_arrow = false;
        workspace.edge_records[0].target_arrow = false;
        owner.directions.clear();
        owner.directions.insert(Some(carrier), Direction::Down);
        workspace.directions.insert(None, Direction::Up);
        workspace
            .scoring_directions_by_tala
            .insert(None, Direction::Up);
        let original = original_ref(&owner, child);
        workspace.sized_edge_abductions.push(SizedEdgeAbduction {
            source_edge: edge,
            edge_tala_id: owner.edge_records[edge.0 as usize].tala_id,
            edge: Some(EdgeId(0)),
            current_from: Some(AbductionCurrentNodeRef::local_with_source(
                NodeId(0),
                carrier,
            )),
            current_to: Some(AbductionCurrentNodeRef::local_with_source(
                NodeId(1),
                adjacent,
            )),
            originally_from: Some(original),
            originally_to: None,
            originally_from_table_neighbors: Vec::new(),
            originally_to_table_neighbors: Vec::new(),
            originally_from_container: Some(carrier),
            originally_to_container: None,
            sequence_abduction: false,
            obstructions_from_to: Vec::new(),
            obstructions_to_from: Vec::new(),
            obstruction_sets_from_to: OrderedObstructionInventory::default(),
            obstruction_sets_to_from: OrderedObstructionInventory::default(),
        });
        let records = owner
            .from_sized_owner_abductions(&workspace.sized_edge_abductions, true)
            .unwrap();
        // The nested foreign child has a direction, but the selected outer
        // pointer is the root adjacent with no direction. Source does not
        // charge an undirected edge merely because its inner child has one.
        assert_eq!(
            full_owner_score(&owner, &workspace, &mapping, view, &records, true),
            170.0
        );
        assert!(
            !owner
                .owner_graph_view(view)
                .unwrap()
                .nodes
                .iter_slots()
                .any(|slot| slot == Some(child))
        );
        assert_eq!(owner.topology_node_owner(child), Some(OwnerGraphId(0)));
        owner.directions.insert(None, Direction::Right);
        owner.edge_records[edge.0 as usize].target_arrow = true;
        workspace.edge_records[0].target_arrow = true;
        // Actual e.From is the carrier, so orientation is reversed even
        // when its scored box comes from a different original child pointer.
        assert_eq!(
            full_owner_score(&owner, &workspace, &mapping, view, &records, true),
            170.0
        );
        owner.edge_records[edge.0 as usize].source_arrow = true;
        owner.edge_records[edge.0 as usize].target_arrow = false;
        workspace.edge_records[0].source_arrow = true;
        workspace.edge_records[0].target_arrow = false;
        assert_eq!(
            full_owner_score(&owner, &workspace, &mapping, view, &records, true),
            230.0
        );
    }

    struct DirectionPollContext {
        polls: Cell<usize>,
        cancel_at: Option<usize>,
    }

    impl limits::Context for DirectionPollContext {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<limits::ContextError> {
            let next = self.polls.get() + 1;
            self.polls.set(next);
            (self.cancel_at == Some(next)).then_some(limits::ContextError::Cancelled)
        }
    }

    #[test]
    fn canonical_direction_preserves_source_273_polls_and_actual_table_shape() {
        // The historical test name is retained for executed-test continuity;
        // fresh source caller stacks locate this preparation poll at line265.
        let mut input = Graph::default();
        let size = Size {
            width: 10.0,
            height: 10.0,
        };
        let a = input.add_node(crate::Node::new("a", size));
        let b = input.add_node(crate::Node::new("b", size));
        for _ in 0..65 {
            input.add_edge(crate::Edge {
                source: a,
                target: b,
            });
        }
        let mut owner = ArenaGraph::from_input(&input);
        owner.initialize_shared_topology();
        let view = owner
            .create_owner_graph_view(vec![a, b], owner.nodes[a.0 as usize].edges.read())
            .unwrap();
        let (mut workspace, mapping) = owner.induced_placement_subgraph(&[a, b]);
        owner
            .set_owner_view_node_mapping(view, mapping.clone())
            .unwrap();
        let mut journal = OwnerGraphOwnershipJournal::default();
        owner
            .set_owner_graph_reference(view, &[a, b], false, &mut journal)
            .unwrap();
        owner.directions.clear();
        let active = workspace.scoring_edge_ids(NodeId(0)).to_vec();
        let prepare =
            |owner: &ArenaGraph, workspace: &ArenaGraph, context: &DirectionPollContext| {
                let frame =
                    OwnerScoreFrame::new(owner, owner.topology_arena_id().unwrap(), view, &mapping)
                        .unwrap();
                workspace.sized_edge_length_owner_direction_checked(
                    NodeId(0),
                    &active,
                    None,
                    true,
                    &frame,
                    ScoreContext::new(context),
                )
            };
        let context = DirectionPollContext {
            polls: Cell::new(0),
            cancel_at: Some(2),
        };
        assert_eq!(
            prepare(&owner, &workspace, &context)
                .unwrap_err()
                .to_string(),
            "EdgeLength: context canceled"
        );
        assert_eq!(context.polls.get(), 2);
        owner.nodes[a.0 as usize].shape = ShapeKind::SqlTable;
        let context = DirectionPollContext {
            polls: Cell::new(0),
            cancel_at: Some(1),
        };
        assert_eq!(
            prepare(&owner, &workspace, &context).unwrap(),
            (Orientation::Right, 0.5, true)
        );
        assert_eq!(context.polls.get(), 0);
        owner.nodes[a.0 as usize].shape = ShapeKind::Rectangle;
        workspace.nodes[0].shape = ShapeKind::SqlTable;
        let context = DirectionPollContext {
            polls: Cell::new(0),
            cancel_at: None,
        };
        assert_eq!(
            prepare(&owner, &workspace, &context).unwrap(),
            (Orientation::BottomRight, 0.3, true)
        );
        assert_eq!(context.polls.get(), 2);
        owner.directions.insert(None, Direction::Down);
        let context = DirectionPollContext {
            polls: Cell::new(0),
            cancel_at: Some(1),
        };
        assert_eq!(
            prepare(&owner, &workspace, &context).unwrap(),
            (Orientation::Bottom, 6.0, false)
        );
        assert_eq!(context.polls.get(), 0);
    }

    #[test]
    fn canonical_direction_preparation_counts_actual_neighbor_pointers_and_nonnil_equals_current() {
        for strong in [false, true] {
            let mut input = Graph::default();
            let size = Size {
                width: 10.0,
                height: 10.0,
            };
            let a = input.add_node(crate::Node::new("a", size));
            let b = input.add_node(crate::Node::new("b", size));
            let c = input.add_node(crate::Node::new("c", size));
            for adjacent in [b, b, if strong { b } else { c }, c] {
                let edge = input.add_edge(crate::Edge {
                    source: a,
                    target: adjacent,
                });
                input.set_edge_label(
                    edge,
                    Some(crate::EdgeLabel {
                        text: "label".into(),
                        size,
                        position: LabelPosition::OutsideBottomCenter,
                        percentage: 0.5,
                    }),
                );
                input.set_edge_arrows(
                    edge,
                    crate::EdgeArrows {
                        source: false,
                        target: true,
                    },
                );
            }
            let mut owner = ArenaGraph::from_input(&input);
            // Equal authored IDs cannot merge distinct Go node pointers.
            owner.nodes[b.0 as usize].tala_id = 77;
            owner.nodes[c.0 as usize].tala_id = 77;
            owner.initialize_shared_topology();
            let view = owner
                .create_owner_graph_view(vec![a, b, c], owner.nodes[a.0 as usize].edges.read())
                .unwrap();
            let (mut workspace, mapping) = owner.induced_placement_subgraph(&[a, b, c]);
            for edge in &mut workspace.edge_records {
                edge.label = None;
            }
            owner
                .set_owner_view_node_mapping(view, mapping.clone())
                .unwrap();
            let mut journal = OwnerGraphOwnershipJournal::default();
            owner
                .set_owner_graph_reference(view, &[a, b, c], false, &mut journal)
                .unwrap();
            owner.directions.clear();
            let active = workspace.scoring_edge_ids(NodeId(0));
            let restored = vec![
                SizedRestoredEndpoints {
                    node: Some(original_ref(&owner, a)),
                    adjacent: None,
                    abduction: None,
                };
                active.len()
            ];
            let frame =
                OwnerScoreFrame::new(&owner, owner.topology_arena_id().unwrap(), view, &mapping)
                    .unwrap();
            assert_eq!(
                workspace
                    .sized_edge_length_owner_direction_checked(
                        NodeId(0),
                        &active,
                        Some(&restored),
                        true,
                        &frame,
                        ScoreContext::none()
                    )
                    .unwrap(),
                if strong {
                    (Orientation::Left, 10.0, true)
                } else {
                    (Orientation::BottomRight, 0.3, true)
                }
            );
            // This tests source direction preparation only. The complete
            // checked scorer continues to reject unbound label penalties.
        }
    }
}

#[cfg(test)]
mod scoring_scratch_lifetime_tests {
    use super::*;

    #[test]
    fn nested_calls_keep_outer_storage_and_reuse_cleared_inner_storage() {
        USED_SCORING_SCRATCH.with(|pool| pool.borrow_mut().clear());
        let mut outer = ScoringScratchVec::<bool>::take();
        outer.resize(8, true);
        let outer_address = outer.as_ptr();
        let mut inner = ScoringScratchVec::<bool>::take();
        inner.resize(8, true);
        let inner_address = inner.as_ptr();
        assert_ne!(outer_address, inner_address);
        drop(inner);
        let mut reused = ScoringScratchVec::<bool>::take();
        reused.resize(8, false);
        assert_eq!(reused.as_ptr(), inner_address);
        assert!(reused.iter().all(|value| !*value));
        assert!(outer.iter().all(|value| *value));
    }

    #[test]
    fn panic_returns_endpoint_storage_without_old_record_identities() {
        ENDPOINT_SCORING_SCRATCH.with(|pool| pool.borrow_mut().clear());
        let mut address = 0;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut scratch = ScoringScratchVec::<SizedRestoredEndpoints>::take();
            scratch.resize(
                8,
                SizedRestoredEndpoints {
                    abduction: Some((27, true)),
                    ..Default::default()
                },
            );
            address = scratch.as_ptr() as usize;
            panic!("test scratch unwind");
        }));
        assert!(result.is_err());
        ENDPOINT_SCORING_SCRATCH.with(|pool| {
            let pool = pool.borrow();
            assert_eq!(pool.len(), 1);
            assert!(pool[0].is_empty());
            assert!(pool[0].capacity() >= 8);
        });
        let mut reused = ScoringScratchVec::<SizedRestoredEndpoints>::take();
        reused.resize(8, Default::default());
        assert_eq!(reused.as_ptr() as usize, address);
        assert!(reused.iter().all(|value| value.node.is_none()
            && value.adjacent.is_none()
            && value.abduction.is_none()));
    }

    #[test]
    fn oversized_and_nested_storage_retention_is_bounded() {
        USED_SCORING_SCRATCH.with(|pool| pool.borrow_mut().clear());
        let mut oversized = ScoringScratchVec::<bool>::take();
        oversized.resize(MAX_SCORING_SCRATCH_ENTRIES + 1, true);
        drop(oversized);
        USED_SCORING_SCRATCH.with(|pool| assert!(pool.borrow().is_empty()));
        let buffers = (0..MAX_SCORING_SCRATCH_BUFFERS + 3)
            .map(|_| {
                let mut scratch = ScoringScratchVec::<bool>::take();
                scratch.resize(8, true);
                scratch
            })
            .collect::<Vec<_>>();
        drop(buffers);
        USED_SCORING_SCRATCH.with(|pool| {
            let pool = pool.borrow();
            assert_eq!(pool.len(), MAX_SCORING_SCRATCH_BUFFERS);
            assert!(pool.iter().all(
                |buffer| buffer.is_empty() && buffer.capacity() <= MAX_SCORING_SCRATCH_ENTRIES
            ));
        });
    }
}
