// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// optimizer_spatial_index.go doesOverlap and optimization_guard.go legacy scan.
impl OptimizerSpatialIndex {
    pub(super) fn does_overlap(&mut self,owner:&mut ArenaGraph,node:Option<NodeId>,point:Option<&dyn SpatialPoint>,exceptions:&super::container_slices::SharedNodeSlice,guard:&mut OptimizationWorkGuard<'_>)->Result<bool,OptimizationError>{
        // Keep low-degree scans free of cache bookkeeping and of the extra
        // per-pair cache argument. Both modes retain the same work loop.
        let prepare=point.is_some()&&node.is_some_and(|node| {
            owner.topology_node_owner(node).is_some()
                &&guard.context().stable_sized_owner()
                &&owner.nodes[node.0 as usize].edges.len()>2
        });
        if prepare{
            self.does_overlap_mode::<true>(owner,node,point,exceptions,guard)
        }else{
            self.does_overlap_mode::<false>(owner,node,point,exceptions,guard)
        }
    }
    fn does_overlap_mode<const PREPARE_SPACING:bool>(&mut self,owner:&mut ArenaGraph,node:Option<NodeId>,point:Option<&dyn SpatialPoint>,exceptions:&super::container_slices::SharedNodeSlice,guard:&mut OptimizationWorkGuard<'_>)->Result<bool,OptimizationError>{
        let (Some(node),Some(point))=(node,point)else{return Err(invalid(guard,"overlap check requires a node, graph, and point"));};
        let Some(receiver)=owner.topology_node_owner(node)else{return Err(invalid(guard,"overlap check requires a node, graph, and point"));};
        let header=owner.owner_graph_members_read(receiver).map_err(|e|OptimizationError::Invalid(e.to_string()))?;
        if header.len()>10000||exceptions.len()>10000{return Err(invalid(guard,"overlap inputs exceed node limit 10000"));}
        if PREPARE_SPACING{self.spacing_edges.begin();}
        let p=point.get();let size=owner.nodes[node.0 as usize].rect.size;
        let indexed=self.graph==owner.topology_arena_id().map(|arena|(arena,receiver))&&self.node_count==header.len()&&self.spatial_usable&&p.x.is_finite()&&p.y.is_finite()&&size.width.is_finite()&&size.height.is_finite()&&size.width>=0.&&size.height>=0.&&(p.x+size.width).is_finite()&&(p.y+size.height).is_finite();
        if indexed{
            drop(header);
            let (candidates,result)=self.query(owner,[p.x-500.,p.y-500.,p.x+size.width+500.,p.y+size.height+500.],guard);result?;
            let p=point.get();let size=owner.nodes[node.0 as usize].rect.size;let right=p.x+size.width;let bottom=p.y+size.height;
            for index in candidates{
                step(owner,guard)?;
                let graph=owner.topology_node_owner(node).expect("runtime error: invalid memory address or nil pointer dereference");
                let other=owner.owner_graph_members_read(graph).expect("bound overlap graph")
                    .get(index).expect("runtime error: index out of range");
                if overlap_candidate(owner,node,other,point,exceptions,right,bottom,false,PREPARE_SPACING.then_some(&mut self.spacing_edges),guard)?{return Ok(true);}
            }
            finish(owner,guard)?;Ok(false)
        }else{
            // Go ranges over the header copied here. An equal header kept
            // from an earlier scan is that same copy.
            let header=match self.retained_members.take(){
                Some(retained) if retained.same_header(&header)=>{drop(header);retained}
                _=>{let retained=header.clone();drop(header);retained}
            };
            let p=point.get();let size=owner.nodes[node.0 as usize].rect.size;let right=p.x+size.width;let bottom=p.y+size.height;
            let result='scan:{
                for other in header.borrowed_slots(){
                    if let Err(error)=step(owner,guard){break 'scan Err(error);}
                    if other.is_none(){break 'scan Err(invalid(guard,"found a nil graph node"));}
                    match overlap_candidate(owner,node,other,point,exceptions,right,bottom,true,PREPARE_SPACING.then_some(&mut self.spacing_edges),guard){
                        Ok(true)=>break 'scan Ok(true),
                        Ok(false)=>{}
                        Err(error)=>break 'scan Err(error),
                    }
                }
                // Source's legacy scan returns false without an explicit Finish.
                Ok(false)
            };
            self.retained_members=Some(header);
            result
        }
    }
}
// Called once per graph member in the scan loops; inlining removes a
// helper call per member and folds the constant `full_scan`.
#[inline(always)]
fn overlap_candidate(owner:&mut ArenaGraph,node:NodeId,other:Option<NodeId>,point:&dyn SpatialPoint,exceptions:&super::container_slices::SharedNodeSlice,right:f64,bottom:f64,full_scan:bool,spacing_edges:Option<&mut SpacingEdges>,guard:&mut OptimizationWorkGuard<'_>)->Result<bool,OptimizationError>{
    if other==Some(node){return Ok(false);}
    for exception in exceptions.borrowed_slots(){step(owner,guard)?;if exception==other{return Ok(false);}}
    let other=other.expect("runtime error: invalid memory address or nil pointer dereference");
    let Some(other_position)=owner.position(other)else{return Ok(false);};
    if full_scan{
        let p=point.get();let size=owner.nodes[node.0 as usize].rect.size;let other_size=owner.nodes[other.0 as usize].rect.size;
        if p.x>other_position.x+other_size.width+500.||p.x+size.width+500.<other_position.x||p.y>other_position.y+other_size.height+500.||p.y+size.height+500.<other_position.y{return Ok(false);}
    }
    let edge_count=owner.owner_node_incident_count(node) as u64;
    #[cfg(feature="diagnostic-traces")]
    guard.add_with_owner(edge_count,owner)?;
    #[cfg(not(feature="diagnostic-traces"))]
    guard.add(edge_count)?;
    // No poll separates the spacing read from the predicate below, so one
    // read of the proposal and of other's TopLeft serves both.
    let p=point.get();let other_position=owner.position(other);
    let delta=match spacing_edges {
        Some(edges)=>edges.delta(owner,node,other,p,other_position),
        None=>owner_spacing_delta_at(owner,node,other,p,other_position),
    };
    let other_position=other_position.expect("runtime error: invalid memory address or nil pointer dereference");let other_size=owner.nodes[other.0 as usize].rect.size;
    Ok(p.x<other_position.x+other_size.width+delta&&right+delta>other_position.x&&p.y<other_position.y+other_size.height+delta&&bottom+delta>other_position.y)
}
