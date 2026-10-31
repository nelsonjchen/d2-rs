// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// sized_optimizer.go528–707: guarded rings, relative-offset cache and occupancy.
fn checked_limit(guard:&OptimizationWorkGuard<'_>)->OptimizationError{
    OptimizationError::ResourceLimit{location:guard.location().into(),detail:super::limits::OptimizationResourceLimitDetail::CheckedPlacementCount(1_000_000)}
}
fn check_offset(checked:&mut Option<CheckedOffsets>,point:Point,guard:&OptimizationWorkGuard<'_>)->Result<bool,OptimizationError>{
    let Some(checked)=checked else{return Ok(false);};let key=key(point);
    if checked.contains(&key){return Ok(true);}
    if checked.len()>=1_000_000{return Err(checked_limit(guard));}
    checked.insert(key);Ok(false)
}
impl OptimizerSpatialIndex {
    pub(super) fn closest_distance(&mut self,owner:&mut ArenaGraph,receiver:OwnerGraphId,node:Option<NodeId>,point:Option<&dyn SpatialPoint>,cell:f64,origin:Option<Point>,minimize:bool,checked:&mut Option<CheckedOffsets>,guard:&mut OptimizationWorkGuard<'_>)->Result<f64,OptimizationError>{
        // Go's findClosestUnoccupiedDistanceGuarded has a VALUE receiver.
        // Queries change its local candidate header, while writes to an
        // existing backing remain visible through the original header.
        let original=self.candidates.clone();
        let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||self.closest_distance_value(owner,receiver,node,point,cell,origin,minimize,checked,guard)));
        self.candidates=original;
        match result{Ok(result)=>result,Err(payload)=>std::panic::resume_unwind(payload)}
    }
    fn closest_distance_value(&mut self,owner:&mut ArenaGraph,receiver:OwnerGraphId,node:Option<NodeId>,point:Option<&dyn SpatialPoint>,cell:f64,origin:Option<Point>,minimize:bool,checked:&mut Option<CheckedOffsets>,guard:&mut OptimizationWorkGuard<'_>)->Result<f64,OptimizationError>{
        let (Some(node),Some(point))=(node,point)else{return Err(invalid(guard,"placement search requires a positioned node, graph, and point"));};
        if owner.topology_node_owner(node).is_none()||owner.position(node).is_none(){return Err(invalid(guard,"placement search requires a positioned node, graph, and point"));}
        if cell<=0.||!cell.is_finite(){return Err(invalid(guard,"placement search requires a finite positive cell size"));}
        if checked.as_ref().is_some_and(|map|map.len()>1_000_000){return Err(checked_limit(guard));}
        for current in 0..=100 {
            step(owner,guard)?;let distance=current as f64*cell;let mut x=distance;let mut y=0.;
            while x>=-distance {
                step(owner,guard)?;
                if y!=0. {
                    let p=point.get();let candidate=Point{x:((p.x+x)/cell).round()*cell,y:((p.y-y)/cell).round()*cell};
                    if origin.is_none_or(|o|candidate.x>=o.x&&candidate.y>=o.y){
                        let size=owner.nodes[node.0 as usize].rect.size;
                        if self.find_unoccupied(owner,receiver,node,point,x,-y,size.width,size.height,cell,origin,minimize,checked,guard)?{return Ok(current as f64);}
                    }
                }
                let p=point.get();let candidate=Point{x:((p.x+x)/cell).round()*cell,y:((p.y+y)/cell).round()*cell};
                if origin.is_none_or(|o|candidate.x>=o.x&&candidate.y>=o.y){
                    let size=owner.nodes[node.0 as usize].rect.size;
                    if self.find_unoccupied(owner,receiver,node,point,x,y,size.width,size.height,cell,origin,minimize,checked,guard)?{return Ok(current as f64);}
                }
                if x>0.{y+=cell;}else{y-=cell;}x-=cell;
            }
        }
        let p=point.get();let debug=owner_search_debug_id(owner,Some(node))?;
        Err(OptimizationError::Invalid(format!("could not find closest unoccupied distance for p=&{{{} {}}}, node={debug}",super::owner_placement::go_general_float(p.x),super::owner_placement::go_general_float(p.y))))
    }
    fn find_unoccupied(&mut self,owner:&mut ArenaGraph,receiver:OwnerGraphId,node:NodeId,point:&dyn SpatialPoint,x:f64,y:f64,width:f64,height:f64,cell:f64,origin:Option<Point>,minimize:bool,checked:&mut Option<CheckedOffsets>,guard:&mut OptimizationWorkGuard<'_>)->Result<bool,OptimizationError>{
        step(owner,guard)?;
        let skip=check_offset(checked,Point{x,y},guard)?;
        if !skip&&!self.point_occupied(owner,receiver,node,point,x,y,cell,origin,guard)?{return Ok(true);}
        if !minimize{return Ok(false);}
        let mut i=x-width;
        while i<=x {let mut j=y-height;while j<=y {
            step(owner,guard)?;
            if i==x&&j==y{j+=cell;continue;}
            if check_offset(checked,Point{x:i,y:j},guard)?{j+=cell;continue;}
            if !self.point_occupied(owner,receiver,node,point,i,j,cell,origin,guard)?{return Ok(true);}
            j+=cell;
        }i+=cell;}
        Ok(false)
    }
    fn point_occupied(&mut self,owner:&mut ArenaGraph,receiver:OwnerGraphId,node:NodeId,point:&dyn SpatialPoint,x:f64,y:f64,cell:f64,origin:Option<Point>,guard:&mut OptimizationWorkGuard<'_>)->Result<bool,OptimizationError>{
        let p=point.get();let proposed=Point{x:((p.x+x)/cell).round()*cell,y:((p.y+y)/cell).round()*cell};
        if origin.is_some_and(|o|proposed.x<o.x||proposed.y<o.y){return Ok(true);}
        let (_,occupied,result)=self.is_occupied(owner,receiver,&proposed,guard);result?;
        if occupied{return Ok(true);}
        let exceptions=match self.single_exception.take(){
            Some((cached,exceptions)) if cached==node=>exceptions,
            _=>super::container_slices::SharedNodeSlice::from_nodes(&[node]).expect("single overlap exception"),
        };
        let result=self.does_overlap(owner,Some(node),Some(&proposed),&exceptions,guard);
        self.single_exception=Some((node,exceptions));
        result
    }
}
pub(super) fn owner_search_debug_id(owner:&ArenaGraph,node:Option<NodeId>)->Result<String,OptimizationError>{
    use super::owner_groups::OwnerGroupKind;
    let Some(node)=node else{return Ok("nil".into());};let live=&owner.nodes[node.0 as usize];if let Some(id)=&live.d2_id{return Ok(id.clone());}
    let graph=owner.topology_node_owner(node);
    let cluster=owner.owner_is_cluster_vessel(node);
    let group=if cluster{let graph=graph.expect("runtime error: invalid memory address or nil pointer dereference");Some((true,owner.owner_graph_group_entry(graph,OwnerGroupKind::Cluster,Some(node)).map_err(|e|OptimizationError::Invalid(e.to_string()))?.flatten().expect("runtime error: invalid memory address or nil pointer dereference")))}else if let Some(graph)=graph{owner.owner_graph_group_entry(graph,OwnerGroupKind::Sequence,Some(node)).map_err(|e|OptimizationError::Invalid(e.to_string()))?.map(|group|(false,group.expect("runtime error: invalid memory address or nil pointer dereference")))}else{None};
    if let Some((cluster,group))=group{
        let fields=owner.owner_group_fields(group).map_err(|e|OptimizationError::Invalid(e.to_string()))?;let mut ids=Vec::new();for member in fields.nodes.iter_slots(){ids.push(owner_search_debug_id(owner,member)?);}
        if cluster{let policy=owner.owner_cluster_policy(group).map_err(|e|OptimizationError::Invalid(e.to_string()))?;return Ok(format!("Cluster vessel of: [{}]; Arrangement: {}",ids.join(", "),policy.arrangement));}
        return Ok(format!("Sequence vessel of: [{}]",ids.join(", ")));
    }
    Ok(live.tala_id.to_string())
}
