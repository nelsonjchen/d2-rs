// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Raw diagnostic observer; no production argument construction.
use super::*;
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
fn wire(edge: &Edge) -> Value {
    let id = |node: &NodeRef| match node.identity {
        NodeIdentity::Ovg(id) => id,
        _ => panic!("probe node"),
    };
    json!([id(&edge.from), id(&edge.to), bits(edge.distance)])
}
fn header(values: &[Edge], capacity: usize) -> Value {
    json!({"nil":capacity==0,"cap":capacity,"values":values.iter().map(wire).collect::<Vec<_>>()})
}
fn make_edge(v: &Value, nodes: &[NodeRef]) -> Edge {
    Edge {
        from: nodes[v[0].as_u64().unwrap() as usize].clone(),
        to: nodes[v[1].as_u64().unwrap() as usize].clone(),
        distance: number(&v[2]),
    }
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
fn state(set: &EdgeSet, nodes: &[NodeRef]) -> Value {
    let axis_header = |v: &Vec<f64>| json!({"cap":v.capacity(),"nil":false,"values":v.iter().map(|v|bits(*v)).collect::<Vec<_>>()});
    let buckets = |m: &crate::internal::routing::routed_edge_set::AxisBuckets| {
        let mut rows = m
            .values()
            .map(|(axis, edges)| json!({"axis":bits(*axis),"edges":header(edges,edges.capacity())}))
            .collect::<Vec<_>>();
        rows.sort_by_key(|v| v["axis"].as_str().unwrap().to_owned());
        rows
    };
    let mut keys = set
        .keys
        .values()
        .chain(set.nan_edges.iter())
        .map(wire)
        .collect::<Vec<_>>();
    keys.sort_by_key(Value::to_string);
    json!({"len":set.len(),"keys":keys,"h":axis_header(&set.horizontal_axes),"v":axis_header(&set.vertical_axes),"horizontal":buckets(&set.horizontal),"vertical":buckets(&set.vertical),"nodes":nodes.iter().map(|node|{let p=node.point();json!([bits(p.x),bits(p.y)])}).collect::<Vec<_>>(),"point_aliases":nodes.iter().map(|n|nodes.iter().position(|other|n.shared.as_ref().unwrap().same_allocation(other.shared.as_ref().unwrap())).unwrap()).collect::<Vec<_>>()})
}
pub(in crate::internal) fn run(value: Value) -> Value {
    Value::Array(value.as_array().unwrap().iter().map(|row| {
        let config=&row["edge_set"];
        let mut nodes=config["nodes"].as_array().unwrap().iter().enumerate().map(|(id,p)|NodeRef{identity:NodeIdentity::Ovg(id),value:point(p),shared:Some(SharedOwnerPoint::new(point(p)))}).collect::<Vec<_>>();
        if let Some(aliases)=config.get("aliases").and_then(Value::as_array) { for a in aliases { nodes[a[0].as_u64().unwrap() as usize].shared=nodes[a[1].as_u64().unwrap() as usize].shared.clone(); } }
        let mut set=EdgeSet::default();let mut observations=Vec::new();
        for op in config["operations"].as_array().unwrap() {
            let guard=Guard{config:&op["guard"],nodes:&nodes,used:Cell::new(0),events:RefCell::new(Vec::new())};
            let budget=op.get("guard").filter(|v|!v.is_null()).map(|_|&guard as &dyn Work);
            let mut error=false;
            let result=match op["kind"].as_str().unwrap() {
                "mutate"=>{nodes[integer(op,"node",0)].shared.as_ref().unwrap().set(point(&op["point"]));Value::Null},
                "contains"=>json!(set.contains(&make_edge(&op["edge"],&nodes))),
                "add"=>{let edge=make_edge(&op["edge"],&nodes);if budget.is_some(){error=set.add_guarded(edge,budget).is_err();}else{set.add(edge);} Value::Null},
                "intersect"=>match set.intersects(&make_edge(&op["edge"],&nodes),budget){Ok(v)=>json!(v),Err(())=>{error=true;json!(false)}},
                "overlap"=>match set.overlapping(&make_edge(&op["edge"],&nodes),budget){Ok(v)=>header(&v,v.capacity()),Err(())=>{error=true;header(&[],0)}},
                _=>panic!("probe operation")
            };
            observations.push(json!({"result":result,"error":error,"used":guard.used.get(),"events":guard.events.into_inner(),"state":state(&set,&nodes)}));
        }
        json!({"name":row["name"],"observations":observations})
    }).collect())
}
