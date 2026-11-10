// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Raw diagnostic observer; no production argument construction.
use super::super::routed_edge_set::NodeIdentity;
use super::*;
use crate::internal::owner_points::SharedOwnerPoint;
use d2_rs_core::stable_sort::StableSort;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
fn integer(v: &Value, key: &str, default: usize) -> usize {
    v.get(key)
        .and_then(Value::as_u64)
        .map_or(default, |n| n as usize)
}
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn bits(v: f64) -> String {
    format!("{:016x}", v.to_bits())
}
fn point(v: &Value) -> Point {
    Point {
        x: number(&v[0]),
        y: number(&v[1]),
    }
}
fn wire(index: &StaticEdges, edge: &StaticEdge) -> Value {
    let id = |node: Option<u32>| node.expect("probe node") as usize;
    let [from, to] = index.edge_ovg(edge);
    json!([id(from), id(to), bits(edge.distance)])
}
fn header(index: &StaticEdges, capacity: usize) -> Value {
    json!({"nil":capacity==0,"cap":capacity,"values":index.edges.iter().map(|edge| wire(index, edge)).collect::<Vec<_>>()})
}
struct Guard<'a> {
    config: &'a Value,
    nodes: &'a [NodeRef],
    used: Cell<u64>,
    events: RefCell<Vec<Value>>,
}
impl Guard<'_> {
    fn call(&self, kind: &str, units: u64) -> Result<(), ()> {
        self.events
            .borrow_mut()
            .push(json!([kind, units, self.used.get()]));
        let count = self.events.borrow().len();
        if let Some(mutations) = self.config.get("mutations").and_then(Value::as_array) {
            for mutation in mutations {
                if integer(mutation, "at", 0) == count {
                    self.nodes[integer(mutation, "node", 0)]
                        .shared
                        .as_ref()
                        .unwrap()
                        .set(point(&mutation["point"]));
                }
            }
        }
        if integer(self.config, "fail_at", 0) == count {
            return Err(());
        }
        let limit = integer(self.config, "limit", 1000000) as u64;
        if units > limit.saturating_sub(self.used.get()) {
            return Err(());
        }
        self.used.set(self.used.get() + units);
        Ok(())
    }
}
impl Work for Guard<'_> {
    fn step(&self) -> Result<(), ()> {
        self.call("step", 1)
    }
    fn add(&self, units: u64) -> Result<(), ()> {
        self.call("add", units)
    }
    fn check(&self) -> Result<(), ()> {
        self.call("check", 0)
    }
}
fn pointers(values: &Vec<usize>) -> Value {
    json!({"nil":values.capacity()==0,"cap":values.capacity(),"values":values})
}
fn state(index: &StaticEdgeIndex, nodes: &[NodeRef]) -> Value {
    let axes = |axes: &Axes| {
        let mut rows = axes
            .buckets
            .values()
            .map(|bucket| json!({"axis":bits(bucket.axis),"edges":pointers(&bucket.edges)}))
            .collect::<Vec<_>>();
        rows.stable_sort_by_key(|row| row["axis"].as_str().unwrap().to_owned());
        rows
    };
    json!({"edges":header(index,index.edges.capacity()),"horizontal":axes(&index.horizontal),"vertical":axes(&index.vertical),"nearby":({let mut v=index.nearby.iter().collect::<Vec<_>>();v.sort_unstable();v}),"overlapping":({let mut v=index.overlapping.iter().collect::<Vec<_>>();v.sort_unstable_by_key(|(edge,_)|*edge);v.into_iter().map(|(edge,routes)|json!([edge,pointers(routes)])).collect::<Vec<_>>()}),"nodes":nodes.iter().map(|n|{let p=n.point();json!([bits(p.x),bits(p.y)])}).collect::<Vec<_>>(),"point_aliases":nodes.iter().map(|n|nodes.iter().position(|o|n.shared.as_ref().unwrap().same_allocation(o.shared.as_ref().unwrap())).unwrap()).collect::<Vec<_>>()})
}
pub(in crate::internal) fn run(value: Value) -> Value {
    Value::Array(value.as_array().unwrap().iter().map(|row|{
        let config=&row["static_index"];
        let mut nodes=config["nodes"].as_array().unwrap().iter().enumerate().map(|(id,p)|NodeRef{identity:NodeIdentity::Ovg(id),value:point(p),shared:Some(SharedOwnerPoint::new(point(p)))}).collect::<Vec<_>>();
        if let Some(aliases)=config.get("aliases").and_then(Value::as_array) {for a in aliases {nodes[a[0].as_u64().unwrap() as usize].shared=nodes[a[1].as_u64().unwrap() as usize].shared.clone();}}
        let mut build=super::super::ovg_ports::PortBuild::default();
        for node in &nodes {let id=build.allocate_node(node.point());build.nodes[id].write().point_ref=node.shared.clone();}
        for (connection,edge) in config["connections"].as_array().unwrap().iter().enumerate(){
            if let Some(changes)=config.get("connection_mutations").and_then(Value::as_array){for change in changes{if integer(change,"before",usize::MAX)==connection{nodes[integer(change,"node",0)].shared.as_ref().unwrap().set(point(&change["point"]));}}}
            build.connect(edge[0].as_u64().unwrap() as usize,edge[1].as_u64().unwrap() as usize,None).unwrap();
        }
        if let Some(changes)=config.get("before_bind").and_then(Value::as_array){for change in changes{nodes[integer(change,"node",0)].shared.as_ref().unwrap().set(point(&change["point"]));}}
        let mut index=StaticEdgeIndex::from_build(&build);let mut observations=Vec::new();
        for op in config["operations"].as_array().unwrap() {
            let guard=Guard{config:&op["guard"],nodes:&nodes,used:Cell::new(0),events:RefCell::new(Vec::new())};
            let mut error=false;
            match op["kind"].as_str().unwrap(){
                "mutate"=>nodes[integer(op,"node",0)].shared.as_ref().unwrap().set(point(&op["point"])),
                "snapshot"=>{},
                "publish"=>{
                    let id=integer(op,"route",0);let route=&config["routes"][id];let refs=route["nodes"].as_array().unwrap();
                    let visible=!route.get("invisible").and_then(Value::as_bool).unwrap_or(false);
                    error=index.mark_route(refs.len(),id,visible,|slot|nodes[refs[slot].as_u64().unwrap() as usize].clone(),Some(&guard))
                        .and_then(|()|guard.check()).is_err();
                },_=>panic!("static index probe operation")
            }
            observations.push(json!({"error":error,"used":guard.used.get(),"events":guard.events.into_inner(),"state":state(&index,&nodes)}));
        }
        json!({"name":row["name"],"observations":observations})
    }).collect())
}
