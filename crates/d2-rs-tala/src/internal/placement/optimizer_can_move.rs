// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// optimizer_spatial_index.go69–90: indexed ordinary nodes, live descendant scans.
impl OptimizerSpatialIndex {
    pub(super) fn can_move(&mut self,owner:&mut ArenaGraph,receiver:OwnerGraphId,node:Option<NodeId>,point:Option<&dyn SpatialPoint>,guard:&mut OptimizationWorkGuard<'_>)->Result<bool,OptimizationError>{
        let (Some(node),Some(point))=(node,point)else{return Err(invalid(guard,"movement check requires a node, graph, and point"));};
        let Some(graph)=owner.topology_node_owner(node)else{return Err(invalid(guard,"movement check requires a node, graph, and point"));};
        if owner.position(node)==Some(point.get()){return Ok(true);}
        let descendants=owner.nodes[node.0 as usize].is_container||owner.owner_is_cluster_vessel(node)||owner.owner_graph_group_entry(receiver,super::owner_groups::OwnerGroupKind::Sequence,Some(node)).map_err(|e|OptimizationError::Invalid(e.to_string()))?.flatten().is_some();
        let mut fallback=OptimizerSpatialIndex::default();
        let (index,graph)=if descendants{(&mut fallback,graph)}else{(self,receiver)};
        // Default index forces optimizerCanMove's live occupancy/overlap scan.
        let (_,occupied,result)=index.is_occupied(owner,graph,point,guard);result?;
        if occupied{return Ok(false);}
        let exceptions=super::container_slices::SharedNodeSlice::nil();
        Ok(!index.does_overlap(owner,Some(node),Some(point),&exceptions,guard)?)
    }
}
