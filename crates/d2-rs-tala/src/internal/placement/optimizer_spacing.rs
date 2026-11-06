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

/// A single overlap call has no topology changes when its context opts into
/// stable sized ownership. Prepare DeltaTo's incident-edge maxima once, after
/// the first pair's work charge. Geometry and directional margins stay live.
#[derive(Debug, Default)]
struct SpacingEdges {
    slots: Vec<(u64, f64, f64)>,
    generation: u64,
    prepared: bool,
}
impl SpacingEdges {
    fn begin(&mut self) {
        self.prepared = false;
    }
    fn delta(
        &mut self,
        owner: &ArenaGraph,
        node: NodeId,
        other: NodeId,
        point: Point,
        other_position: Option<Point>,
    ) -> f64 {
        if !self.prepared {
            self.generation = self.generation.wrapping_add(1);
            if self.generation == 0 {
                self.slots.fill((0, 0., 0.));
                self.generation = 1;
            }
            self.slots
                .resize(self.slots.len().max(owner.nodes.len()), (0, 0., 0.));
            owner
                .for_each_owner_node_incident(node, |&edge| {
                    let edge = edge.expect(
                        "layout invariant violated: spacing check encountered an incomplete edge",
                    );
                    let (from, to) = owner
                        .owner_edge_endpoint_pair(edge)
                        .expect("bound spacing endpoint");
                    let (Some(from), Some(to)) = (from, to) else {
                        panic!(
                            "layout invariant violated: spacing check encountered an incomplete edge"
                        );
                    };
                    let adjacent = if from == node { to } else { from };
                    // Only an in-range other node can be queried by this scan.
                    let Some(slot) = self.slots.get_mut(adjacent.0 as usize) else {
                        return;
                    };
                    if slot.0 != self.generation {
                        *slot = (self.generation, f64::NEG_INFINITY, f64::NEG_INFINITY);
                    }
                    let edge = &owner.edge_records[edge.0 as usize];
                    if edge.min_width > slot.1 {
                        slot.1 = edge.min_width.trunc();
                    }
                    if edge.min_height > slot.2 {
                        slot.2 = edge.min_height.trunc();
                    }
                })
                .expect("bound spacing incidents");
            self.prepared = true;
        }
        let slot = self.slots[other.0 as usize];
        spacing_delta_with_edges(
            owner,
            node,
            other,
            point,
            other_position,
            (slot.0 == self.generation).then_some((slot.1, slot.2)),
        )
    }
}

// Keep the direct edge scan's original single-call path as well as the
// prepared path: extracting this calculation must not add a call per pair.
#[inline(always)]
fn spacing_delta_with_edges(owner:&ArenaGraph,node:NodeId,other:NodeId,point:Point,other_position:Option<Point>,edges:Option<(f64,f64)>)->f64 {
    use super::{Orientation,ShapeKind};
    use super::loop_offsets::{loop_offset_toward,loop_spacing_units};
    let first=&owner.nodes[node.0 as usize];let second=&owner.nodes[other.0 as usize];
    let mut horizontal=20.;let mut vertical=20.;
    let connected=edges.is_some();
    let (max_width,max_height)=edges.unwrap_or((f64::NEG_INFINITY,f64::NEG_INFINITY));
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

#[cfg(test)]
mod spacing_tests {
    use super::*;
    use crate::internal::limits::{Context, ContextError};
    use std::cell::Cell;

    #[test]
    fn overlap_spacing_reuse_keeps_live_calls_and_source_work() {
        struct CountingContext {
            stable: bool,
            polls: Cell<usize>,
        }
        impl Context for CountingContext {
            fn stable_sized_owner(&self) -> bool {
                self.stable
            }
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<ContextError> {
                self.polls.set(self.polls.get() + 1);
                None
            }
        }
        // Exercise both optimizerDoesOverlap's full scan and the indexed walk.
        for count in [4, 100] {
            let mut input = crate::Graph::default();
            for i in 0..count {
                input.add_node(crate::Node::new(
                    format!("n{i}"),
                    crate::Size {
                        width: 20.,
                        height: 20.,
                    },
                ));
            }
            for to in [0, 1, 1, 2] {
                input.add_edge(crate::Edge {
                    source: NodeId(0),
                    target: NodeId(to),
                });
            }
            let mut owner = ArenaGraph::from_input(&input);
            for i in 0..count {
                owner.set_position(
                    NodeId(i),
                    Point {
                        x: f64::from(i) * 1000.,
                        y: 0.,
                    },
                );
            }
            owner.set_position(NodeId(2), Point { x: 1000., y: 300. });
            owner.set_position(NodeId(3), Point { x: 900., y: 300. });
            let contexts = [false, true].map(|stable| CountingContext {
                stable,
                polls: Cell::new(0),
            });
            let mut indexes = [
                OptimizerSpatialIndex::default(),
                OptimizerSpatialIndex::default(),
            ];
            let mut guards = contexts.each_ref().map(|context| {
                OptimizationWorkGuard::new(Some(context), "overlap-spacing", 1_000_000).unwrap()
            });
            for (index, guard) in indexes.iter_mut().zip(&mut guards) {
                index.rebuild(&mut owner, OwnerGraphId(0), guard).unwrap();
            }
            let exceptions = super::super::container_slices::SharedNodeSlice::nil();
            // Topology and edge minima may change between calls. Parallel
            // edges retain their incidence order and directional maximum.
            for (widths, expected) in [([0., 0.], false), ([120., 150.], true), ([0., 0.], false)] {
                owner.edge_records[1].min_width = widths[0];
                owner.edge_records[2].min_width = widths[1];
                let point = Point { x: 900., y: 0. };
                for (index, guard) in indexes.iter_mut().zip(&mut guards) {
                    assert_eq!(
                        index
                            .does_overlap(
                                &mut owner,
                                Some(NodeId(0)),
                                Some(&point),
                                &exceptions,
                                guard
                            )
                            .unwrap(),
                        expected
                    );
                }
                assert_eq!(guards[0].used(), guards[1].used());
                assert_eq!(contexts[0].polls.get(), contexts[1].polls.get());
            }
            // An early budget failure still occurs before spacing evaluation.
            for (index, guard) in indexes.iter_mut().zip(&mut guards) {
                guard.set_limit(guard.used());
                assert!(
                    index
                        .does_overlap(
                            &mut owner,
                            Some(NodeId(0)),
                            Some(&Point { x: 900., y: 0. }),
                            &exceptions,
                            guard
                        )
                        .is_err()
                );
            }
            assert_eq!(guards[0].used(), guards[1].used());
            assert_eq!(contexts[0].polls.get(), contexts[1].polls.get());
        }
    }
}
