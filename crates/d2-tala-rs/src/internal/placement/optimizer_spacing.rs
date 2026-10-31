// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// node.go780–925 DeltaTo, as float64 spacing consumed by overlap predicates.
pub(super) fn owner_spacing_delta(owner:&ArenaGraph,node:NodeId,other:NodeId,point:Point)->f64 {
    owner_spacing_delta_at(owner,node,other,point,owner.position(other))
}
/// `owner_spacing_delta` with `other`'s TopLeft already read; it is consulted
/// only for the orientation, exactly where the live read was.
pub(super) fn owner_spacing_delta_at(owner:&ArenaGraph,node:NodeId,other:NodeId,point:Point,other_position:Option<Point>)->f64 {
    use super::{Orientation,ShapeKind};
    use super::loop_offsets::{loop_offset_toward,loop_spacing_units};
    let first=&owner.nodes[node.0 as usize];let second=&owner.nodes[other.0 as usize];
    let mut horizontal=20.;let mut vertical=20.;let mut connected=false;
    let mut max_width=f64::NEG_INFINITY;let mut max_height=f64::NEG_INFINITY;
    owner.for_each_owner_node_incident(node, |&edge| {
        let edge=edge.unwrap_or_else(||panic!("layout invariant violated: spacing check encountered an incomplete edge"));
        let (from,to)=owner.owner_edge_endpoint_pair(edge).expect("bound spacing endpoint");
        let (Some(from),Some(to))=(from,to)else{panic!("layout invariant violated: spacing check encountered an incomplete edge");};
        let adjacent=if from==node{to}else{from};
        if adjacent==other{connected=true;let edge=&owner.edge_records[edge.0 as usize];if edge.min_width>max_width{max_width=edge.min_width.trunc();}if edge.min_height>max_height{max_height=edge.min_height.trunc();}}
    }).expect("bound spacing incidents");
    if connected{horizontal=60.;vertical=60.;}
    if first.shape==ShapeKind::SqlTable||second.shape==ShapeKind::SqlTable{horizontal=120.;}
    if max_width>horizontal{horizontal=max_width;}if max_height>vertical{vertical=max_height;}
    let has_loops=|node:&super::ArenaNode|node.loop_offsets.as_ref().is_some_and(|map|!map.is_empty());
    let no_margin=|m:super::Insets|m.top==0.&&m.right==0.&&m.bottom==0.&&m.left==0.;
    if horizontal==vertical&&!has_loops(first)&&!has_loops(second)&&no_margin(first.layout_margins)&&no_margin(second.layout_margins){return horizontal;}
    let orientation=other_position.map_or(Orientation::None,|p|owner.sized_box_orientation((point,first.rect.size),(p,second.rect.size)));
    if has_loops(first)||has_loops(second){
        let loops=20i64.wrapping_add(loop_spacing_units(loop_offset_toward(first.loop_offsets.as_ref(),orientation.opposite()))).wrapping_add(loop_spacing_units(loop_offset_toward(second.loop_offsets.as_ref(),orientation))) as f64;
        if loops>horizontal{horizontal=loops;}if loops>vertical{vertical=loops;}
    }
    let margins=|m:super::Insets,side:Orientation|->(i64,i64){let (x,y)=match side{
        Orientation::Top=>(0.,m.top),Orientation::Bottom=>(0.,m.bottom),Orientation::Left=>(m.left,0.),Orientation::Right=>(m.right,0.),
        Orientation::TopLeft=>(m.left,m.top),Orientation::TopRight=>(m.right,m.top),Orientation::BottomLeft=>(m.left,m.bottom),Orientation::BottomRight=>(m.right,m.bottom),Orientation::None=>(0.,0.),
    };(loop_spacing_units(x),loop_spacing_units(y))};
    let (ax,ay)=margins(first.layout_margins,orientation.opposite());let (bx,by)=margins(second.layout_margins,orientation);
    let margin_x=ax.wrapping_add(bx) as f64;let margin_y=ay.wrapping_add(by) as f64;
    if margin_x>horizontal{horizontal=margin_x;}if margin_y>vertical{vertical=margin_y;}
    match orientation{Orientation::Top|Orientation::Bottom=>vertical,Orientation::Left|Orientation::Right=>horizontal,_=>if horizontal<vertical{horizontal}else{vertical}}
}
