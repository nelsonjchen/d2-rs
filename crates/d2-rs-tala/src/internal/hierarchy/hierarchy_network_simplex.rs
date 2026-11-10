// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Network-simplex ranking for hierarchy placement.
//!
//! Unlike a topological sort, this algorithm optimizes edge lengths while
//! preserving feasible ranks. Stable arena IDs replace the pointer identity of
//! the reference implementation; traversal and tie order remain observable.

use super::*;
use d2_rs_core::stable_sort::StableSort;

#[derive(Clone, Copy, Debug)]
pub(super) struct HierarchyDagEdge {
    pub(super) from: NodeId,
    pub(super) to: NodeId,
    pub(super) id: usize,
    /// Objective coefficient carried by the authored edge.  Undirected and
    /// bidirectional edges are represented by two arcs; the reverse arc has
    /// zero weight, matching TALA's makeSimpleDAG construction.
    pub(super) weight: i64,
}

#[derive(Clone, Debug)]
pub(super) struct HierarchyDag {
    pub(super) nodes: Vec<NodeId>,
    pub(super) edges: Vec<HierarchyDagEdge>,
    /// Stable external IDs used by TALA's cycle-break tie rule. Arena indices
    /// are only storage addresses and do not match layoutgraph entity IDs.
    pub(super) tie_keys: BTreeMap<NodeId, i64>,
}

#[derive(Clone, Debug)]
pub(super) struct HierarchyRank {
    pub(super) level: BTreeMap<NodeId, usize>,
    pub(super) level_count: usize,
}

#[derive(Clone, Debug, Default)]
struct NetworkSimplex {
    levels: BTreeMap<NodeId, i64>,
    tree_root: Option<NodeId>,
    tree_nodes: BTreeSet<NodeId>,
    tree_edges: BTreeSet<usize>,
    edge_to_parent: BTreeMap<NodeId, usize>,
    cut_value: BTreeMap<usize, i64>,
    dfs_entry: BTreeMap<NodeId, usize>,
    dfs_exit: BTreeMap<NodeId, usize>,
    feasible_tree_iterations: usize,
    edge_swap_iterations: usize,
}

impl HierarchyDag {
    pub(super) fn from_scope(nodes: Vec<NodeId>, edges: &[(NodeId, NodeId, bool)]) -> Self {
        Self::from_scope_with_ties(nodes, edges, BTreeMap::new())
    }

    pub(super) fn from_scope_with_ties(
        nodes: Vec<NodeId>,
        edges: &[(NodeId, NodeId, bool)],
        tie_keys: BTreeMap<NodeId, i64>,
    ) -> Self {
        let node_set = nodes.iter().copied().collect::<BTreeSet<_>>();
        let mut dag_edges = Vec::new();
        for &(from, to, directed) in edges {
            if from == to || !node_set.contains(&from) || !node_set.contains(&to) {
                continue;
            }
            let id = dag_edges.len();
            dag_edges.push(HierarchyDagEdge {
                from,
                to,
                id,
                weight: 1,
            });
            if !directed {
                let id = dag_edges.len();
                dag_edges.push(HierarchyDagEdge {
                    from: to,
                    to: from,
                    id,
                    weight: 0,
                });
            }
        }
        let mut dag = Self {
            nodes,
            edges: dag_edges,
            tie_keys,
        };
        dag.reverse_cycle_edges();
        dag.remove_duplicate_edges();
        dag
    }

    fn incident(&self, node: NodeId) -> Vec<usize> {
        self.edges
            .iter()
            .filter(|edge| edge.from == node || edge.to == node)
            .map(|edge| edge.id)
            .collect()
    }

    fn edge(&self, id: usize) -> &HierarchyDagEdge {
        &self.edges[id]
    }

    fn reverse_cycle_edges(&mut self) {
        // Direct translation of Graph.findCycleEdges plus reverseEdges. Every
        // DAG edge has a target arrow, so incoming/outgoing is `to`/`from`.
        let mut incoming = self
            .nodes
            .iter()
            .copied()
            .map(|node| (node, BTreeSet::new()))
            .collect::<BTreeMap<_, _>>();
        let mut outgoing = incoming.clone();
        let mut remaining = self.nodes.iter().copied().collect::<BTreeSet<_>>();
        for edge in &self.edges {
            outgoing.get_mut(&edge.from).unwrap().insert(edge.id);
            incoming.get_mut(&edge.to).unwrap().insert(edge.id);
        }

        let remove_node = |node: NodeId,
                           incoming: &mut BTreeMap<NodeId, BTreeSet<usize>>,
                           outgoing: &mut BTreeMap<NodeId, BTreeSet<usize>>,
                           remaining: &mut BTreeSet<NodeId>| {
            let mut removed = Vec::new();
            removed.extend(incoming[&node].iter().copied());
            removed.extend(outgoing[&node].iter().copied());
            for edge_id in removed {
                let edge = self.edge(edge_id);
                incoming.get_mut(&edge.to).unwrap().remove(&edge_id);
                outgoing.get_mut(&edge.from).unwrap().remove(&edge_id);
            }
            remaining.remove(&node);
        };

        let mut reversed = BTreeSet::new();
        while !remaining.is_empty() {
            loop {
                let sinks = remaining
                    .iter()
                    .copied()
                    .filter(|node| outgoing[node].is_empty())
                    .collect::<Vec<_>>();
                if sinks.is_empty() {
                    break;
                }
                for node in sinks {
                    remove_node(node, &mut incoming, &mut outgoing, &mut remaining);
                }
            }
            loop {
                let sources = remaining
                    .iter()
                    .copied()
                    .filter(|node| incoming[node].is_empty())
                    .collect::<Vec<_>>();
                if sources.is_empty() {
                    break;
                }
                for node in sources {
                    remove_node(node, &mut incoming, &mut outgoing, &mut remaining);
                }
            }
            let Some(node) = remaining.iter().copied().max_by_key(|node| {
                let out = outgoing[node].len() as isize;
                let inn = incoming[node].len() as isize;
                // TALA then prefers more incident edges and lower node ID.
                (
                    out - inn,
                    out + inn,
                    std::cmp::Reverse(self.tie_keys.get(node).copied().unwrap_or(node.0 as i64)),
                )
            }) else {
                continue;
            };
            if crate::internal::trace_env_enabled("TALA_TRACE_HIER_DETAIL") {
                eprintln!(
                    "HIER_CYCLE_MAX_RUST node={} in={} out={}",
                    node.0,
                    incoming[&node].len(),
                    outgoing[&node].len()
                );
            }
            reversed.extend(incoming[&node].iter().copied());
            for edge_id in incoming[&node].clone() {
                let from = self.edge(edge_id).from;
                outgoing.get_mut(&from).unwrap().remove(&edge_id);
                incoming.get_mut(&node).unwrap().remove(&edge_id);
            }
            remove_node(node, &mut incoming, &mut outgoing, &mut remaining);
        }
        for edge in &mut self.edges {
            if reversed.contains(&edge.id) {
                if crate::internal::trace_env_enabled("TALA_TRACE_HIER_DETAIL") {
                    eprintln!("HIER_CYCLE_RUST {}:{}->{}", edge.id, edge.from.0, edge.to.0);
                }
                std::mem::swap(&mut edge.from, &mut edge.to);
            }
        }
    }

    fn remove_duplicate_edges(&mut self) {
        // makeSimpleDAG keeps the first arc for an endpoint pair and folds
        // every later arc's rank weight into it.  This matters for parallel
        // authored edges and for the zero-weight half of undirected edges.
        let mut first = BTreeMap::<(NodeId, NodeId), usize>::new();
        let mut combined: Vec<HierarchyDagEdge> = Vec::with_capacity(self.edges.len());
        for edge in self.edges.drain(..) {
            if let Some(&index) = first.get(&(edge.from, edge.to)) {
                combined[index].weight += edge.weight;
            } else {
                first.insert((edge.from, edge.to), combined.len());
                combined.push(edge);
            }
        }
        self.edges = combined;
        for (id, edge) in self.edges.iter_mut().enumerate() {
            edge.id = id;
        }
    }
}

impl NetworkSimplex {
    fn rank(dag: &HierarchyDag) -> Option<HierarchyRank> {
        let mut simplex = Self::default();
        for &node in &dag.nodes {
            let level = simplex.longest_path_level(dag, node)?;
            simplex.levels.insert(node, level);
        }
        simplex.make_feasible_tree(dag)?;
        simplex.traverse_tree(dag);
        simplex.assign_cut_values(dag);
        while let Some(leaving) = simplex.find_tree_edge_to_remove() {
            if simplex.edge_swap_iterations == 100 {
                return None;
            }
            simplex.edge_swap_iterations += 1;
            let entering = simplex.find_non_tree_edge_to_replace(dag, leaving)?;
            simplex.swap_edges(dag, leaving, entering);
        }
        let level_count = simplex.normalize_levels()?;
        Some(HierarchyRank {
            level: simplex
                .levels
                .into_iter()
                .map(|(node, level)| (node, level as usize))
                .collect(),
            level_count,
        })
    }

    fn longest_path_level(&mut self, dag: &HierarchyDag, node: NodeId) -> Option<i64> {
        if let Some(&level) = self.levels.get(&node) {
            return (level != -1).then_some(level);
        }
        self.levels.insert(node, -1);
        let mut max = -1;
        for edge_id in dag.incident(node) {
            let edge = dag.edge(edge_id);
            if edge.to != node {
                continue;
            }
            max = max.max(self.longest_path_level(dag, edge.from)?);
        }
        let level = max + 1;
        self.levels.insert(node, level);
        Some(level)
    }

    fn slack(&self, dag: &HierarchyDag, edge_id: usize) -> i64 {
        let edge = dag.edge(edge_id);
        self.levels[&edge.from] - self.levels[&edge.to] + 1
    }

    fn make_feasible_tree(&mut self, dag: &HierarchyDag) -> Option<()> {
        self.tree_root = self
            .levels
            .iter()
            .filter(|(_, level)| **level == 0)
            .map(|(&node, _)| node)
            .max_by_key(|node| (dag.incident(*node).len(), std::cmp::Reverse(*node)));
        loop {
            self.make_tree(dag)?;
            if self.tree_nodes.len() == dag.nodes.len() {
                return Some(());
            }
            if self.feasible_tree_iterations == 100 {
                return None;
            }
            self.feasible_tree_iterations += 1;
            let edge_id = self.find_min_incident_edge(dag)?;
            let edge = dag.edge(edge_id);
            let mut delta = self.slack(dag, edge_id);
            if !self.tree_nodes.contains(&edge.to) {
                delta = -delta;
            }
            self.move_tree_nodes(delta);
        }
    }

    fn make_tree(&mut self, dag: &HierarchyDag) -> Option<()> {
        self.tree_nodes.clear();
        self.tree_edges.clear();
        let root = self.tree_root?;
        self.tree_nodes.insert(root);
        self.dfs_make_tree(dag, root);
        Some(())
    }

    fn find_min_incident_edge(&self, dag: &HierarchyDag) -> Option<usize> {
        dag.edges
            .iter()
            .filter(|edge| {
                self.tree_nodes.contains(&edge.from) != self.tree_nodes.contains(&edge.to)
            })
            .filter_map(|edge| {
                let slack = self.slack(dag, edge.id);
                (slack != 0).then_some((slack, edge.id))
            })
            .min_by_key(|(slack, _)| *slack)
            .map(|(_, edge)| edge)
    }

    fn move_tree_nodes(&mut self, delta: i64) {
        for &node in &self.tree_nodes {
            *self.levels.get_mut(&node).unwrap() += delta;
        }
    }

    fn normalize_levels(&mut self) -> Option<usize> {
        let min = *self.levels.values().min()?;
        let max = *self.levels.values().max()?;
        if min != 0 {
            self.move_tree_nodes(-min);
        }
        Some((max - min + 1) as usize)
    }

    fn dfs_make_tree(&mut self, dag: &HierarchyDag, root: NodeId) {
        for edge_id in dag.incident(root) {
            let edge = dag.edge(edge_id);
            let adjacent = if edge.from == root {
                edge.to
            } else {
                edge.from
            };
            if self.tree_nodes.contains(&adjacent) || self.slack(dag, edge_id) != 0 {
                continue;
            }
            self.tree_nodes.insert(adjacent);
            self.tree_edges.insert(edge_id);
            self.dfs_make_tree(dag, adjacent);
        }
    }

    fn traverse_tree(&mut self, dag: &HierarchyDag) {
        self.dfs_entry.clear();
        self.dfs_exit.clear();
        self.edge_to_parent.clear();
        if let Some(root) = self.tree_root {
            self.dfs_traverse_tree(dag, root, 1);
        }
    }

    fn dfs_traverse_tree(&mut self, dag: &HierarchyDag, root: NodeId, mut entry: usize) -> usize {
        self.dfs_entry.insert(root, entry);
        for edge_id in dag.incident(root) {
            if !self.tree_edges.contains(&edge_id) {
                continue;
            }
            let edge = dag.edge(edge_id);
            let adjacent = if edge.from == root {
                edge.to
            } else {
                edge.from
            };
            if self.dfs_entry.contains_key(&adjacent) {
                continue;
            }
            self.edge_to_parent.insert(adjacent, edge_id);
            entry = self.dfs_traverse_tree(dag, adjacent, entry);
        }
        self.dfs_exit.insert(root, entry);
        entry + 1
    }

    fn assign_cut_values(&mut self, dag: &HierarchyDag) {
        self.cut_value.clear();
        let mut nodes = self.tree_nodes.iter().copied().collect::<Vec<_>>();
        nodes.stable_sort_by_key(|node| self.dfs_exit[node]);
        for child in nodes {
            let Some(&parent_edge) = self.edge_to_parent.get(&child) else {
                continue;
            };
            let edge = dag.edge(parent_edge);
            let child_is_tail = edge.to != child;
            let mut cut = 1;
            for edge_id in dag.incident(child) {
                if edge_id == parent_edge {
                    continue;
                }
                let targeted_to_child = dag.edge(edge_id).to == child;
                let previous = self.cut_value.get(&edge_id).copied().unwrap_or_default();
                if child_is_tail != targeted_to_child {
                    cut = cut - previous + 1;
                } else {
                    cut = cut + previous - 1;
                }
            }
            self.cut_value.insert(parent_edge, cut);
        }
    }

    fn find_tree_edge_to_remove(&self) -> Option<usize> {
        let mut edges = self.tree_edges.iter().copied().collect::<Vec<_>>();
        edges.sort_unstable();
        edges
            .into_iter()
            .find(|edge| self.cut_value.get(edge).copied().unwrap_or_default() < 0)
    }

    fn descendant_of(&self, node: NodeId, root: NodeId) -> bool {
        self.dfs_entry[&root] <= self.dfs_exit[&node]
            && self.dfs_exit[&node] <= self.dfs_exit[&root]
    }

    fn find_non_tree_edge_to_replace(&self, dag: &HierarchyDag, leaving: usize) -> Option<usize> {
        let leaving = dag.edge(leaving);
        let (tail, flipped) = if self.dfs_exit[&leaving.from] > self.dfs_exit[&leaving.to] {
            (leaving.to, true)
        } else {
            (leaving.from, false)
        };
        let candidates = dag
            .edges
            .iter()
            .filter(|edge| !self.tree_edges.contains(&edge.id))
            .filter(|edge| {
                self.descendant_of(edge.from, tail) == flipped
                    && self.descendant_of(edge.to, tail) != flipped
            })
            .map(|edge| edge.id)
            .collect::<Vec<_>>();
        candidates
            .into_iter()
            .min_by_key(|edge| self.slack(dag, *edge))
    }

    fn swap_edges(&mut self, dag: &HierarchyDag, leaving: usize, entering: usize) {
        self.tree_edges.remove(&leaving);
        self.tree_edges.insert(entering);
        self.traverse_tree(dag);
        self.assign_cut_values(dag);
        let mut nodes = self.tree_nodes.iter().copied().collect::<Vec<_>>();
        nodes.stable_sort_by_key(|node| {
            (self.dfs_entry[node], std::cmp::Reverse(self.dfs_exit[node]))
        });
        for node in nodes.into_iter().skip(1) {
            let edge = dag.edge(self.edge_to_parent[&node]);
            let parent = if edge.from == node {
                edge.to
            } else {
                edge.from
            };
            let level = if edge.to == node {
                self.levels[&parent] + 1
            } else {
                self.levels[&parent] - 1
            };
            self.levels.insert(node, level);
        }
    }
}

pub(super) fn rank_hierarchy_dag(dag: &HierarchyDag) -> Option<HierarchyRank> {
    ReferenceRankSimplex::rank(dag)
}

/// Faithful Rust implementation of v0.9.0's rank graph and graphical
/// network-simplex solver.  The older `NetworkSimplex` above is retained for
/// its focused recovery tests, but production hierarchy assignment uses this
/// implementation.  In particular, it preserves authored edge weights,
/// entity-ID node ordering, pivot ordering, and the exact feasible-tree
/// exchange rules from rank_graph.go/rank_simplex.go.
struct ReferenceRankEdge {
    from: usize,
    to: usize,
    weight: i64,
    id: usize,
}

struct ReferenceRankProblem {
    nodes: Vec<NodeId>,
    edges: Vec<ReferenceRankEdge>,
    outgoing_start: Vec<usize>,
    pivot_order: Vec<usize>,
    incident_start: Vec<usize>,
    incident_edges: Vec<usize>,
    balance: Vec<i64>,
}

impl ReferenceRankProblem {
    fn new(dag: &HierarchyDag) -> Option<Self> {
        let mut nodes = dag.nodes.clone();
        nodes.stable_sort_by_key(|node| {
            (
                dag.tie_keys.get(node).copied().unwrap_or(node.0 as i64),
                node.0,
            )
        });
        let mut index = BTreeMap::new();
        for (i, node) in nodes.iter().copied().enumerate() {
            index.insert(node, i);
        }

        let mut edges = Vec::with_capacity(dag.edges.len());
        for edge in &dag.edges {
            let from = *index.get(&edge.from)?;
            let to = *index.get(&edge.to)?;
            if from == to || edge.weight <= 0 {
                return None;
            }
            edges.push(ReferenceRankEdge {
                from,
                to,
                weight: edge.weight,
                id: edge.id,
            });
        }
        edges.stable_sort_by_key(|edge| (edge.from, edge.to, edge.id));

        let mut outgoing_start = vec![0usize; nodes.len() + 1];
        for edge in &edges {
            outgoing_start[edge.from + 1] += 1;
        }
        for node in 0..nodes.len() {
            outgoing_start[node + 1] += outgoing_start[node];
        }
        let mut pivot_order = (0..edges.len()).collect::<Vec<_>>();
        pivot_order.stable_sort_by_key(|&edge| edges[edge].id);

        if crate::internal::trace_env_enabled("TALA_TRACE_RANK_DETAIL") {
            eprint!("RANK_PROBLEM_RUST nodes=");
            for node in &nodes {
                eprint!(
                    "{}:{},",
                    dag.tie_keys.get(node).copied().unwrap_or(node.0 as i64),
                    node.0
                );
            }
            eprint!(" edges=");
            for (i, edge) in edges.iter().enumerate() {
                eprint!(
                    "{}:{}->{}:w{}:id{},",
                    i,
                    dag.tie_keys
                        .get(&nodes[edge.from])
                        .copied()
                        .unwrap_or(nodes[edge.from].0 as i64),
                    dag.tie_keys
                        .get(&nodes[edge.to])
                        .copied()
                        .unwrap_or(nodes[edge.to].0 as i64),
                    edge.weight,
                    edge.id
                );
            }
            eprint!(" piv=");
            for edge in &pivot_order {
                eprint!("{},", edge);
            }
            eprintln!();
        }

        let mut degree = vec![0usize; nodes.len()];
        let mut balance = vec![0i64; nodes.len()];
        for edge in &edges {
            degree[edge.from] += 1;
            degree[edge.to] += 1;
            balance[edge.from] = balance[edge.from].checked_add(edge.weight)?;
            balance[edge.to] = balance[edge.to].checked_sub(edge.weight)?;
        }
        let mut incident_start = vec![0usize; nodes.len() + 1];
        for node in 0..nodes.len() {
            incident_start[node + 1] = incident_start[node] + degree[node];
        }
        let mut incident_edges = vec![0usize; edges.len() * 2];
        let mut next = incident_start[..nodes.len()].to_vec();
        for &edge_index in &pivot_order {
            let edge = &edges[edge_index];
            incident_edges[next[edge.from]] = edge_index;
            next[edge.from] += 1;
            incident_edges[next[edge.to]] = edge_index;
            next[edge.to] += 1;
        }
        Some(Self {
            nodes,
            edges,
            outgoing_start,
            pivot_order,
            incident_start,
            incident_edges,
            balance,
        })
    }

    fn longest_path_levels(&self) -> Option<Vec<i64>> {
        let mut indegree = vec![0usize; self.nodes.len()];
        for edge in &self.edges {
            indegree[edge.to] += 1;
        }
        let mut ready = std::collections::BinaryHeap::<std::cmp::Reverse<usize>>::new();
        for (node, degree) in indegree.iter().copied().enumerate() {
            if degree == 0 {
                ready.push(std::cmp::Reverse(node));
            }
        }
        let mut levels = vec![0i64; self.nodes.len()];
        let mut visited = 0usize;
        while let Some(std::cmp::Reverse(node)) = ready.pop() {
            visited += 1;
            for edge_index in self.outgoing_start[node]..self.outgoing_start[node + 1] {
                let edge = &self.edges[edge_index];
                levels[edge.to] = levels[edge.to].max(levels[node].checked_add(1)?);
                indegree[edge.to] -= 1;
                if indegree[edge.to] == 0 {
                    ready.push(std::cmp::Reverse(edge.to));
                }
            }
        }
        (visited == self.nodes.len()).then_some(levels)
    }
}

struct ReferenceRankSimplex {
    problem: ReferenceRankProblem,
    cut_value: Vec<i64>,
}

impl ReferenceRankSimplex {
    fn rank(dag: &HierarchyDag) -> Option<HierarchyRank> {
        if dag.nodes.is_empty() {
            return Some(HierarchyRank {
                level: BTreeMap::new(),
                level_count: 0,
            });
        }
        let problem = ReferenceRankProblem::new(dag)?;
        let mut levels = problem.longest_path_levels()?;
        if crate::internal::trace_env_enabled("TALA_TRACE_RANK_DETAIL") {
            eprint!("RANK_INITIAL_RUST ");
            for (i, level) in levels.iter().enumerate() {
                eprint!(
                    "{}:{},",
                    dag.tie_keys
                        .get(&problem.nodes[i])
                        .copied()
                        .unwrap_or(problem.nodes[i].0 as i64),
                    level
                );
            }
            eprintln!();
        }
        if problem.edges.is_empty() {
            return Some(HierarchyRank {
                level: problem.nodes.iter().copied().map(|n| (n, 0)).collect(),
                level_count: 1,
            });
        }
        let mut solver = Self {
            cut_value: vec![0; problem.edges.len()],
            problem,
        };
        let mut tree = solver.feasible_tree(&mut levels)?;
        if crate::internal::trace_env_enabled("TALA_TRACE_RANK_DETAIL") {
            eprint!("RANK_TREE_RUST ");
            for (i, present) in tree.iter().copied().enumerate() {
                if present {
                    eprint!("{}(id{}),", i, solver.problem.edges[i].id);
                }
            }
            eprintln!();
            eprint!("RANK_FEASIBLE_RUST ");
            for (i, level) in levels.iter().enumerate() {
                eprint!("{}:{},", i, level);
            }
            eprintln!();
        }
        solver.optimize(&mut levels, &mut tree)?;
        let min = *levels.iter().min()?;
        let max = *levels.iter().max()?;
        for level in &mut levels {
            *level -= min;
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_RANK_DETAIL") {
            eprint!("RANK_FINAL_RUST ");
            for (i, level) in levels.iter().enumerate() {
                eprint!(
                    "{}:{},",
                    dag.tie_keys
                        .get(&solver.problem.nodes[i])
                        .copied()
                        .unwrap_or(solver.problem.nodes[i].0 as i64),
                    level
                );
            }
            eprintln!();
        }
        let mut result = BTreeMap::new();
        for (index, node) in solver.problem.nodes.iter().copied().enumerate() {
            result.insert(node, levels[index] as usize);
        }
        Some(HierarchyRank {
            level: result,
            level_count: (max - min + 1) as usize,
        })
    }

    fn slack(&self, edge_index: usize, levels: &[i64]) -> Option<i64> {
        levels[self.problem.edges[edge_index].to]
            .checked_sub(levels[self.problem.edges[edge_index].from])?
            .checked_sub(1)
    }

    fn feasible_tree(&mut self, levels: &mut [i64]) -> Option<Vec<bool>> {
        let node_count = self.problem.nodes.len();
        let mut tree = vec![false; self.problem.edges.len()];
        let mut in_tree = vec![false; node_count];
        in_tree[0] = true;
        let mut tree_node_count = 1usize;
        let mut queue = vec![0usize];
        self.grow_tight(
            levels,
            &mut tree,
            &mut in_tree,
            &mut tree_node_count,
            &mut queue,
        )?;

        while tree_node_count < node_count {
            let mut best_slack = i64::MAX;
            let mut boundary = Vec::new();
            for &edge_index in &self.problem.pivot_order {
                let edge = &self.problem.edges[edge_index];
                if in_tree[edge.from] == in_tree[edge.to] {
                    continue;
                }
                let slack = self.slack(edge_index, levels)?;
                if slack < best_slack {
                    best_slack = slack;
                    boundary.clear();
                    boundary.push(edge_index);
                } else if slack == best_slack {
                    boundary.push(edge_index);
                }
            }
            let selected = *boundary.first()?;
            let selected_tail_in_tree = in_tree[self.problem.edges[selected].from];
            let delta = if selected_tail_in_tree {
                best_slack
            } else {
                best_slack.checked_neg()?
            };
            for (node, present) in in_tree.iter().copied().enumerate() {
                if present {
                    levels[node] = levels[node].checked_add(delta)?;
                }
            }

            queue.clear();
            for edge_index in boundary {
                let edge = &self.problem.edges[edge_index];
                if in_tree[edge.from] == in_tree[edge.to] {
                    continue;
                }
                let tail_in_tree = in_tree[edge.from];
                if best_slack != 0 && tail_in_tree != selected_tail_in_tree {
                    continue;
                }
                if self.slack(edge_index, levels)? != 0 {
                    continue;
                }
                let outside = if tail_in_tree { edge.to } else { edge.from };
                tree[edge_index] = true;
                in_tree[outside] = true;
                tree_node_count += 1;
                queue.push(outside);
            }
            if queue.is_empty() {
                return None;
            }
            self.grow_tight(
                levels,
                &mut tree,
                &mut in_tree,
                &mut tree_node_count,
                &mut queue,
            )?;
        }
        Some(tree)
    }

    fn grow_tight(
        &self,
        levels: &[i64],
        tree: &mut [bool],
        in_tree: &mut [bool],
        tree_node_count: &mut usize,
        queue: &mut Vec<usize>,
    ) -> Option<()> {
        let mut cursor = 0;
        while cursor < queue.len() {
            let node = queue[cursor];
            let start = self.problem.incident_start[node];
            let end = self.problem.incident_start[node + 1];
            for offset in start..end {
                let edge_index = self.problem.incident_edges[offset];
                let edge = &self.problem.edges[edge_index];
                let other = if edge.from == node {
                    edge.to
                } else {
                    edge.from
                };
                if in_tree[other] || self.slack(edge_index, levels)? != 0 {
                    continue;
                }
                tree[edge_index] = true;
                in_tree[other] = true;
                *tree_node_count += 1;
                queue.push(other);
            }
            cursor += 1;
        }
        queue.clear();
        Some(())
    }

    fn optimize(&mut self, levels: &mut [i64], tree: &mut [bool]) -> Option<()> {
        loop {
            self.compute_cut_values(tree)?;
            let leaving = self
                .problem
                .pivot_order
                .iter()
                .copied()
                .find(|&edge| tree[edge] && self.cut_value[edge] < 0);
            let Some(leaving) = leaving else {
                return Some(());
            };
            let head = self.head_component(tree, leaving)?;
            let mut entering = None;
            let mut minimum_slack = i64::MAX;
            for &edge_index in &self.problem.pivot_order {
                let edge = &self.problem.edges[edge_index];
                if tree[edge_index] || !head[edge.from] || head[edge.to] {
                    continue;
                }
                let slack = self.slack(edge_index, levels)?;
                if slack < minimum_slack {
                    minimum_slack = slack;
                    entering = Some(edge_index);
                }
            }
            let entering = entering?;
            if crate::internal::trace_env_enabled("TALA_TRACE_RANK_DETAIL") {
                eprint!("RANK_HEAD_RUST ");
                for (i, present) in head.iter().copied().enumerate() {
                    if present {
                        eprint!("{},", i);
                    }
                }
                eprintln!();
                eprintln!(
                    "RANK_PIVOT_RUST leave={} enter={} min={} cut={}",
                    self.problem.edges[leaving].id,
                    self.problem.edges[entering].id,
                    minimum_slack,
                    self.cut_value[leaving]
                );
            }
            for (node, in_head) in head.iter().copied().enumerate() {
                if in_head {
                    levels[node] = levels[node].checked_add(minimum_slack)?;
                }
            }
            if self.slack(entering, levels)? != 0 {
                return None;
            }
            tree[leaving] = false;
            tree[entering] = true;
            if crate::internal::trace_env_enabled("TALA_TRACE_RANK_DETAIL") {
                eprint!("RANK_AFTER_RUST ");
                for (i, level) in levels.iter().enumerate() {
                    eprint!("{}:{},", i, level);
                }
                eprintln!();
            }
        }
    }

    fn compute_cut_values(&mut self, tree: &[bool]) -> Option<()> {
        let node_count = self.problem.nodes.len();
        if tree.iter().filter(|present| **present).count() != node_count - 1 {
            return None;
        }
        let unvisited = usize::MAX;
        let mut parent = vec![unvisited; node_count];
        let mut parent_edge = vec![usize::MAX; node_count];
        parent[0] = usize::MAX - 1;
        let mut order = Vec::with_capacity(node_count);
        let mut stack = vec![0usize];
        while let Some(node) = stack.pop() {
            order.push(node);
            for offset in self.problem.incident_start[node]..self.problem.incident_start[node + 1] {
                let edge_index = self.problem.incident_edges[offset];
                if !tree[edge_index] || edge_index == parent_edge[node] {
                    continue;
                }
                let edge = &self.problem.edges[edge_index];
                let other = if edge.from == node {
                    edge.to
                } else {
                    edge.from
                };
                if parent[other] != unvisited {
                    return None;
                }
                parent[other] = node;
                parent_edge[other] = edge_index;
                stack.push(other);
            }
        }
        if order.len() != node_count {
            return None;
        }
        let mut subtree = self.problem.balance.clone();
        self.cut_value.fill(0);
        for &node in order.iter().rev().take(node_count.saturating_sub(1)) {
            let edge_index = parent_edge[node];
            let edge = &self.problem.edges[edge_index];
            let amount = subtree[node];
            self.cut_value[edge_index] = if edge.from == node {
                amount
            } else if edge.to == node {
                amount.checked_neg()?
            } else {
                return None;
            };
            subtree[parent[node]] = subtree[parent[node]].checked_add(amount)?;
        }
        (subtree[0] == 0).then_some(())
    }

    fn head_component(&self, tree: &[bool], leaving: usize) -> Option<Vec<bool>> {
        let mut head = vec![false; self.problem.nodes.len()];
        let start = self.problem.edges[leaving].to;
        head[start] = true;
        let mut queue = vec![start];
        let mut cursor = 0;
        while cursor < queue.len() {
            let node = queue[cursor];
            if crate::internal::trace_env_enabled("TALA_TRACE_RANK_DETAIL") && leaving == 30 {
                eprint!("RANK_HEAD_SCAN_RUST node={} edges=", node);
            }
            for offset in self.problem.incident_start[node]..self.problem.incident_start[node + 1] {
                let edge_index = self.problem.incident_edges[offset];
                if crate::internal::trace_env_enabled("TALA_TRACE_RANK_DETAIL") && leaving == 30 {
                    eprint!("{}:{}", edge_index, tree[edge_index]);
                }
                if edge_index == leaving || !tree[edge_index] {
                    continue;
                }
                let edge = &self.problem.edges[edge_index];
                let other = if edge.from == node {
                    edge.to
                } else {
                    edge.from
                };
                if !head[other] {
                    head[other] = true;
                    queue.push(other);
                }
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_RANK_DETAIL") && leaving == 30 {
                eprintln!();
            }
            cursor += 1;
        }
        (queue.len() != self.problem.nodes.len()).then_some(head)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_helpers_match_recovered_descendant_incident_and_move_behavior() {
        let root = NodeId(0);
        let child = NodeId(1);
        let outside = NodeId(2);
        let dag = HierarchyDag {
            nodes: vec![root, child, outside],
            edges: vec![
                HierarchyDagEdge {
                    from: root,
                    to: child,
                    id: 0,
                    weight: 1,
                },
                HierarchyDagEdge {
                    from: root,
                    to: outside,
                    id: 1,
                    weight: 1,
                },
            ],
            tie_keys: BTreeMap::new(),
        };
        let mut simplex = NetworkSimplex {
            levels: BTreeMap::from([(root, 4), (child, 3), (outside, 0)]),
            tree_nodes: BTreeSet::from([root, child]),
            dfs_entry: BTreeMap::from([(root, 1), (child, 2), (outside, 3)]),
            dfs_exit: BTreeMap::from([(root, 4), (child, 3), (outside, 5)]),
            ..NetworkSimplex::default()
        };

        assert!(simplex.descendant_of(child, root));
        assert!(!simplex.descendant_of(outside, root));
        assert_eq!(simplex.find_min_incident_edge(&dag), Some(1));

        simplex.move_tree_nodes(-2);
        assert_eq!(simplex.levels[&root], 2);
        assert_eq!(simplex.levels[&child], 1);
        assert_eq!(simplex.levels[&outside], 0);
    }

    #[test]
    fn feasible_tree_moves_only_the_current_tree_then_normalizes_all_levels() {
        let a = NodeId(0);
        let b = NodeId(1);
        let c = NodeId(2);
        let d = NodeId(3);
        // a→b→d is the initial zero-slack tree. c→d has slack -1,
        // so TALA moves the current tree by -1 before the next tree walk can
        // absorb c.
        let dag = HierarchyDag {
            nodes: vec![a, b, c, d],
            edges: vec![
                HierarchyDagEdge {
                    from: a,
                    to: b,
                    id: 0,
                    weight: 1,
                },
                HierarchyDagEdge {
                    from: b,
                    to: d,
                    id: 1,
                    weight: 1,
                },
                HierarchyDagEdge {
                    from: c,
                    to: d,
                    id: 2,
                    weight: 1,
                },
            ],
            tie_keys: BTreeMap::new(),
        };
        let mut simplex = NetworkSimplex {
            levels: BTreeMap::from([(a, 0), (b, 1), (c, 0), (d, 2)]),
            ..NetworkSimplex::default()
        };

        simplex
            .make_feasible_tree(&dag)
            .expect("recovered feasible tree");
        assert_eq!(simplex.tree_root, Some(a));
        assert_eq!(simplex.feasible_tree_iterations, 1);
        assert_eq!(simplex.tree_nodes, BTreeSet::from([a, b, c, d]));
        assert_eq!(simplex.tree_edges, BTreeSet::from([0, 1, 2]));
        assert_eq!(
            simplex.levels,
            BTreeMap::from([(a, -1), (b, 0), (c, 0), (d, 1)])
        );

        assert_eq!(simplex.normalize_levels(), Some(3));
        assert_eq!(
            simplex.levels,
            BTreeMap::from([(a, 0), (b, 1), (c, 1), (d, 2)])
        );
    }

    #[test]
    fn find_and_swap_tree_edge_rebuilds_parent_cut_and_level_state() {
        let root = NodeId(0);
        let child = NodeId(1);
        let outside = NodeId(2);
        let dag = HierarchyDag {
            nodes: vec![root, child, outside],
            edges: vec![
                HierarchyDagEdge {
                    from: root,
                    to: child,
                    id: 0,
                    weight: 1,
                },
                HierarchyDagEdge {
                    from: root,
                    to: outside,
                    id: 1,
                    weight: 1,
                },
                HierarchyDagEdge {
                    from: child,
                    to: outside,
                    id: 2,
                    weight: 1,
                },
            ],
            tie_keys: BTreeMap::new(),
        };
        let mut simplex = NetworkSimplex {
            levels: BTreeMap::from([(root, 0), (child, 1), (outside, 1)]),
            tree_root: Some(root),
            tree_nodes: BTreeSet::from([root, child, outside]),
            tree_edges: BTreeSet::from([0, 1]),
            edge_to_parent: BTreeMap::from([(child, 0), (outside, 1)]),
            dfs_entry: BTreeMap::from([(root, 1), (child, 2), (outside, 4)]),
            dfs_exit: BTreeMap::from([(root, 6), (child, 3), (outside, 5)]),
            ..NetworkSimplex::default()
        };
        let entering = simplex
            .find_non_tree_edge_to_replace(&dag, 0)
            .expect("cross-cut replacement edge");
        assert_eq!(entering, 2);

        simplex.swap_edges(&dag, 0, entering);
        assert_eq!(simplex.tree_edges, BTreeSet::from([1, 2]));
        assert_eq!(
            simplex.edge_to_parent,
            BTreeMap::from([(child, 2), (outside, 1)])
        );
        assert_eq!(simplex.cut_value, BTreeMap::from([(1, 2), (2, 0)]));
        assert_eq!(
            simplex.levels,
            BTreeMap::from([(root, 0), (child, 0), (outside, 1)])
        );
    }

    #[test]
    fn tree_edge_removal_uses_only_negative_cut_values() {
        let mut simplex = NetworkSimplex {
            tree_edges: BTreeSet::from([0, 1, 2]),
            cut_value: BTreeMap::from([(0, 4), (1, -3), (2, 0)]),
            ..NetworkSimplex::default()
        };
        assert_eq!(simplex.find_tree_edge_to_remove(), Some(1));

        simplex.cut_value.insert(1, 0);
        assert_eq!(simplex.find_tree_edge_to_remove(), None);
    }

    #[test]
    fn longest_path_leveling_rejects_an_unreversed_cycle() {
        let first = NodeId(0);
        let second = NodeId(1);
        let dag = HierarchyDag {
            nodes: vec![first, second],
            edges: vec![
                HierarchyDagEdge {
                    from: first,
                    to: second,
                    id: 0,
                    weight: 1,
                },
                HierarchyDagEdge {
                    from: second,
                    to: first,
                    id: 1,
                    weight: 1,
                },
            ],
            tie_keys: BTreeMap::new(),
        };
        let mut simplex = NetworkSimplex::default();
        assert_eq!(simplex.longest_path_level(&dag, first), None);
        assert_eq!(simplex.levels[&first], -1);
        assert_eq!(simplex.levels[&second], -1);
    }

    #[test]
    fn recovered_grid_nested_hierarchy_ranking_matches_pristine_arm64_capture() {
        let nodes = (0..7).map(NodeId).collect::<Vec<_>>();
        // a→b→c→a, d→e→g, d→f→g, b→g.
        let dag = HierarchyDag::from_scope(
            nodes,
            &[
                (NodeId(0), NodeId(1), true),
                (NodeId(1), NodeId(2), true),
                (NodeId(2), NodeId(0), true),
                (NodeId(3), NodeId(4), true),
                (NodeId(4), NodeId(6), true),
                (NodeId(3), NodeId(5), true),
                (NodeId(5), NodeId(6), true),
                (NodeId(1), NodeId(6), true),
            ],
        );
        let result = rank_hierarchy_dag(&dag).expect("recovered network simplex");
        assert_eq!(result.level_count, 3);
        assert_eq!(result.level[&NodeId(0)], 0);
        assert_eq!(result.level[&NodeId(1)], 1);
        assert_eq!(result.level[&NodeId(2)], 2);
        assert_eq!(result.level[&NodeId(3)], 0);
        assert_eq!(result.level[&NodeId(4)], 1);
        assert_eq!(result.level[&NodeId(5)], 1);
        assert_eq!(result.level[&NodeId(6)], 2);
    }
}
