// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
#![cfg(feature = "diagnostic-traces")]
use super::*;
use serde_json::{Value, json};

fn items(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}
fn index(value: &Value) -> Option<usize> {
    value.as_i64().and_then(|v| usize::try_from(v).ok())
}
fn node(value: &Value) -> Option<NodeId> {
    index(value).map(|v| NodeId(v as u32))
}
fn edge(value: &Value) -> Option<EdgeId> {
    index(value).map(|v| EdgeId(v as u32))
}
fn node_header(value: &Value) -> SharedNodeSlice {
    if value.is_null() {
        return SharedNodeSlice::nil();
    }
    let values = items(&value["values"]);
    let capacity = value["cap"].as_u64().unwrap_or(values.len() as u64) as usize;
    let header = SharedNodeSlice::with_len_cap(values.len(), capacity).unwrap();
    for (i, value) in values.iter().enumerate() {
        header.write_capacity_slot(i, node(value)).unwrap();
    }
    header
}

struct Context {
    polls: RefCell<Vec<i64>>,
    used: Cell<i64>,
    cancel_at: usize,
    deadline: bool,
    done: bool,
}
impl limits::Context for Context {
    fn done(&self) -> Option<bool> {
        self.done.then_some(true)
    }
    fn transaction_work_used(&self, used: i64) {
        self.used.set(used);
    }
    fn error(&self) -> Option<limits::ContextError> {
        let mut polls = self.polls.borrow_mut();
        polls.push(self.used.get());
        (self.cancel_at > 0 && polls.len() >= self.cancel_at).then_some(if self.deadline {
            limits::ContextError::DeadlineExceeded
        } else {
            limits::ContextError::Cancelled
        })
    }
}

impl ArenaGraph {
    pub(in crate::internal) fn probe_owner_preflight(
        &mut self,
        name: &str,
        config: &Value,
        limit: i64,
        cancel_at: usize,
        deadline: bool,
    ) -> Value {
        self.initialize_shared_topology();
        let receiver = OwnerGraphId(0);
        // Requests explicitly assign each header with make(len,cap). These are
        // test operations on real storage, not normalization of observations.
        let members = node_header(&config["nodes"]);
        self.shared_topology.views[0]
            .as_mut()
            .unwrap()
            .nodes
            .replace(members);
        let edge_config = &config["edges"];
        let values = items(&edge_config["values"])
            .iter()
            .map(edge)
            .collect::<Vec<_>>();
        let header = self
            .allocate_owner_incident_header(
                &values,
                edge_config["cap"].as_u64().unwrap_or(values.len() as u64) as usize,
                !edge_config.is_null(),
            )
            .unwrap();
        self.set_owner_graph_edge_header(receiver, header).unwrap();
        for item in items(&config["incidents"]) {
            let owner = node(&item["node"]).unwrap();
            let value = &item["header"];
            let values = items(&value["values"]).iter().map(edge).collect::<Vec<_>>();
            let header = self
                .allocate_owner_incident_header(
                    &values,
                    value["cap"].as_u64().unwrap_or(values.len() as u64) as usize,
                    !value.is_null(),
                )
                .unwrap();
            self.set_owner_node_incident_header(owner, header).unwrap();
        }
        self.containers.share_map().clear();
        for item in items(&config["containers"]) {
            self.containers
                .replace_header(node(&item["key"]), node_header(&item["header"]));
        }
        for item in items(&config["node_fields"]) {
            let id = node(&item["node"]).unwrap();
            if item.get("parent").is_some() {
                self.nodes[id.0 as usize].container = node(&item["parent"]);
            }
            if let Some(flag) = item["container"].as_bool() {
                self.nodes[id.0 as usize].is_container = flag;
            }
            if let Some(flag) = item["graph"].as_bool() {
                self.set_owner_node_graph_reference(id, flag.then_some(receiver))
                    .unwrap();
            }
            if let Some(flag) = item["cluster_vessel"].as_bool() {
                self.set_owner_cluster_vessel_flag(id, flag).unwrap();
            }
            if item.get("nears").is_some() {
                self.nodes[id.0 as usize].nears =
                    NodeNearMap::from_raw_keys(items(&item["nears"]).iter().map(node));
            }
            if item.get("requirements").is_some() {
                self.shared_topology.long_distance_requirements.insert(
                    id,
                    super::node_requirements_map::NodeRequirementsMap::from_raw_keys(
                        items(&item["requirements"]).iter().map(node),
                    ),
                );
            }
        }
        for item in items(&config["endpoints"]) {
            let id = edge(&item["edge"]).unwrap();
            if item.get("from").is_some() {
                self.set_owner_edge_endpoint(id, false, node(&item["from"]))
                    .unwrap();
            }
            if item.get("to").is_some() {
                self.set_owner_edge_endpoint(id, true, node(&item["to"]))
                    .unwrap();
            }
        }
        for item in items(&config["routes"]) {
            let id = edge(&item["edge"]).unwrap();
            let original = self.edge_records[id.0 as usize].route.clone();
            let values = items(&item["header"]["values"]);
            let header = SharedPointSlice::with_len_cap(
                values.len(),
                item["header"]["cap"]
                    .as_u64()
                    .unwrap_or(values.len() as u64) as usize,
            )
            .unwrap();
            for (i, value) in values.iter().enumerate() {
                header
                    .write_capacity_slot(i, index(value).and_then(|i| original.slot(i)))
                    .unwrap();
            }
            self.edge_records[id.0 as usize]
                .route
                .replace_header(header);
        }
        if config.get("directions").is_some() {
            self.shared_topology.placement_directions = Some(
                items(&config["directions"])
                    .iter()
                    .map(|v| (node(v), Orientation::Bottom))
                    .collect(),
            );
        }
        let mut herds = Vec::new();
        for item in items(&config["herds"]) {
            let herd = self.new_owner_herd();
            for (key, opposite) in [("opposite", true), ("same", false)] {
                for value in items(&item[key]) {
                    self.pair_owner_herd(Some(&herd), node(value), opposite)
                        .unwrap();
                }
            }
            herds.push(herd);
        }
        for item in items(&config["node_herds"]) {
            self.set_owner_node_herd(
                node(&item["node"]).unwrap(),
                index(&item["herd"]).map(|i| herds[i].clone()),
            )
            .unwrap();
        }
        let mut hierarchies = Vec::new();
        let mut hierarchy_maps = Vec::<OwnerHierarchyMap>::new();
        for item in items(&config["hierarchies"]) {
            let hierarchy = self.new_owner_hierarchy();
            let levels = if let Some(i) = index(&item["shared_map"]) {
                hierarchy_maps[i].clone()
            } else {
                let levels = self.allocate_owner_hierarchy_map(!item["levels"].is_null());
                for entry in items(&item["levels"]) {
                    self.write_owner_hierarchy_level(
                        &levels,
                        node(&entry["node"]),
                        entry["level"].as_i64().unwrap_or(0),
                    )
                    .unwrap();
                }
                levels
            };
            self.set_owner_hierarchy_fields(
                &hierarchy,
                OwnerHierarchyFields {
                    levels: levels.clone(),
                    level_count: item["level_count"].as_i64().unwrap_or(0),
                },
            )
            .unwrap();
            hierarchy_maps.push(levels);
            hierarchies.push(hierarchy);
        }
        for item in items(&config["node_hierarchies"]) {
            self.set_owner_node_hierarchy(
                node(&item["node"]).unwrap(),
                index(&item["hierarchy"]).map(|i| hierarchies[i].clone()),
            )
            .unwrap();
        }
        let mut abductions = Vec::new();
        for item in items(&config["abductions"]) {
            let registered = self
                .register_owner_edge_abductions(
                    vec![OwnerEdgeAbduction {
                        source_edge: edge(&item["edge"]),
                        originally_from: node(&item["original_from"]),
                        originally_to: node(&item["original_to"]),
                        current_from: node(&item["current_from"]),
                        current_to: node(&item["current_to"]),
                    }],
                    true,
                )
                .unwrap();
            abductions.push(registered.records[0]);
        }
        let mut groups = Vec::new();
        for item in items(&config["groups"]) {
            let kind = if item["kind"].as_str() == Some("sequence") {
                OwnerGroupKind::Sequence
            } else {
                OwnerGroupKind::Cluster
            };
            let id = self.new_owner_group(kind);
            self.owner_group(id).unwrap().set_fields(OwnerGroupFields {
                vessel: node(&item["vessel"]),
                nodes: node_header(&item["nodes"]),
                graph: item["graph"].as_bool().unwrap_or(true).then_some(receiver),
                container: node(&item["container"]),
            });
            if let Some(value) = item.get("abductions") {
                let values = items(&value["values"]);
                let slots = values
                    .iter()
                    .map(|value| index(value).map(|i| abductions[i]))
                    .collect::<Vec<_>>();
                let header = self
                    .allocate_owner_abduction_header(
                        &slots,
                        value["cap"].as_u64().unwrap_or(values.len() as u64) as usize,
                        !value.is_null(),
                    )
                    .unwrap();
                if kind == OwnerGroupKind::Sequence {
                    self.set_owner_sequence_abductions(id, Some(header))
                        .unwrap();
                } else {
                    let group = self.owner_group(id).unwrap();
                    let mut policy = group.policy();
                    policy.edge_abductions = Some(header);
                    group.set_policy(policy);
                }
            }
            groups.push(id);
        }
        for item in items(&config["group_maps"]) {
            let group = index(&item["group"]).map(|i| groups[i]);
            let map = if item["kind"].as_str() == Some("sequence") {
                &self.shared_topology.views[0].as_ref().unwrap().sequences
            } else {
                &self.shared_topology.views[0].as_ref().unwrap().clusters
            };
            map.insert(node(&item["key"]), group).unwrap();
        }
        for item in items(&config["node_groups"]) {
            let kind = if item["kind"].as_str() == Some("sequence") {
                OwnerGroupKind::Sequence
            } else {
                OwnerGroupKind::Cluster
            };
            self.set_owner_node_group(
                node(&item["node"]).unwrap(),
                kind,
                index(&item["group"]).map(|i| groups[i]),
            )
            .unwrap();
        }
        let tree_values = items(&config["trees"]);
        let mut trees = Vec::new();
        for item in tree_values {
            trees.push(self.new_owner_tree(node(&item["node"])).unwrap());
        }
        for (i, item) in tree_values.iter().enumerate() {
            let values = items(&item["children"]["values"])
                .iter()
                .map(|v| index(v).map(|i| trees[i]))
                .collect::<Vec<_>>();
            let children = if item["children"].is_null() {
                OwnerTreeSlice::nil()
            } else {
                OwnerTreeSlice::from_slots(
                    &values,
                    item["children"]["cap"]
                        .as_u64()
                        .unwrap_or(values.len() as u64) as usize,
                )
                .unwrap()
            };
            let mut fields = self.owner_tree(trees[i]).unwrap().fields();
            fields.parent = index(&item["parent"]).map(|i| trees[i]);
            fields.children = children;
            fields.sentinel_edge = edge(&item["edge"]);
            self.set_owner_tree_fields(trees[i], fields).unwrap();
        }
        for item in items(&config["roots"]) {
            let values = items(&item["header"]["values"])
                .iter()
                .map(|v| index(v).map(|i| trees[i]))
                .collect::<Vec<_>>();
            let header = if item["header"].is_null() {
                OwnerTreeSlice::nil()
            } else {
                OwnerTreeSlice::from_slots(
                    &values,
                    item["header"]["cap"]
                        .as_u64()
                        .unwrap_or(values.len() as u64) as usize,
                )
                .unwrap()
            };
            self.shared_topology.views[0]
                .as_ref()
                .unwrap()
                .tree_roots
                .insert(node(&item["key"]), header)
                .unwrap();
        }
        if !items(&config["aliases"]).is_empty() {
            self.shared_topology.views[0].as_mut().unwrap().node_to_tree =
                SharedOwnerTreeMap::allocated();
        }
        for item in items(&config["aliases"]) {
            self.shared_topology.views[0]
                .as_ref()
                .unwrap()
                .node_to_tree
                .insert(node(&item["node"]), index(&item["tree"]).map(|i| trees[i]))
                .unwrap();
        }
        for (key, hubs) in [("hubs", true), ("siblings", false)] {
            let map = SharedSliceMap::new();
            for item in items(&config[key]) {
                map.replace(node(&item["key"]), node_header(&item["header"]))
                    .unwrap();
            }
            let view = self.shared_topology.views[0].as_mut().unwrap();
            if hubs {
                view.hubs.replace_header(map);
            } else {
                view.common_uncle_siblings.replace_header(map);
            }
        }
        let record_observations = self.probe_owner_record_program(
            items(&config["record_steps"]),
            &hierarchies,
            &mut hierarchy_maps,
            &abductions,
        );
        let context = Context {
            polls: RefCell::new(Vec::new()),
            used: Cell::new(0),
            cancel_at,
            deadline,
            done: config["done"].as_bool().unwrap_or(false),
        };
        let mut work = None;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if config["nil_graph"].as_bool() == Some(true) {
                return self.validate_owner_graph_topology(
                    None,
                    Some(&context),
                    "SimplifyEdgeRoutes",
                );
            }
            let ctx = if config["nil_context"].as_bool() == Some(true) {
                None
            } else {
                Some(&context as &dyn limits::Context)
            };
            work = Some(WorkGuard::new(
                ctx,
                "SimplifyEdgeRoutes",
                limits::MAX_ENGINE_WORK_UNITS,
            )?);
            let guard = work.as_mut().unwrap();
            guard.set_limit(limit);
            self.validate_owner_graph_topology_with_guard(receiver, guard)
        }));
        let used = work.as_ref().map_or(0, WorkGuard::used);
        let (error, panic) = match outcome {
            Ok(result) => (result.err().map(|e| e.to_string()), None),
            Err(payload) => (
                None,
                Some(
                    payload
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_else(|| "non-string panic".into()),
                ),
            ),
        };
        let mut result = json!({"name":name,"error":error,"panic":panic,"used":used,"polls":context.polls.into_inner()});
        if !record_observations.is_empty() {
            result["records"] = json!(record_observations);
        }
        result
    }

    fn probe_owner_record_state(
        &self,
        hierarchies: &[OwnerHierarchy],
        retained: &[OwnerHierarchyMap],
        abductions: &[OwnerAbductionId],
    ) -> Value {
        let mut record_ids = BTreeMap::<usize, usize>::new();
        let mut map_ids = Vec::<OwnerHierarchyMap>::new();
        let mut map_value = |map: &OwnerHierarchyMap| {
            let id = if map.is_nil() {
                0
            } else if let Some(i) = map_ids.iter().position(|old| old.same_allocation(map)) {
                i + 1
            } else {
                map_ids.push(map.clone());
                map_ids.len()
            };
            json!({"id":id,"nil":map.is_nil(),"entries":map.entries().into_iter().map(|(node,level)|json!([node.map(|n|n.0),level])).collect::<Vec<_>>()})
        };
        let records = hierarchies
            .iter()
            .map(|hierarchy| {
                let next = record_ids.len() + 1;
                let id = *record_ids.entry(hierarchy.allocation_key()).or_insert(next);
                let fields = self.owner_hierarchy_fields(hierarchy).unwrap();
                json!({"id":id,"level_count":fields.level_count,"map":map_value(&fields.levels)})
            })
            .collect::<Vec<_>>();
        let maps = retained.iter().map(&mut map_value).collect::<Vec<_>>();
        let nodes = (0..self.nodes.len())
            .map(|i| {
                self.owner_node_hierarchy(NodeId(i as u32))
                    .unwrap()
                    .map(|hierarchy| {
                        let next = record_ids.len() + 1;
                        *record_ids.entry(hierarchy.allocation_key()).or_insert(next)
                    })
            })
            .collect::<Vec<_>>();
        let edges = (0..self.edge_records.len())
            .map(|i| {
                json!([
                    self.owner_edge_endpoint(EdgeId(i as u32), false)
                        .unwrap()
                        .map(|n| n.0),
                    self.owner_edge_endpoint(EdgeId(i as u32), true)
                        .unwrap()
                        .map(|n| n.0)
                ])
            })
            .collect::<Vec<_>>();
        let abductions=abductions.iter().map(|&id| {
            let record=self.owner_edge_abduction(id).unwrap();
            json!({"edge":record.source_edge.map(|e|e.0),"original_from":record.originally_from.map(|n|n.0),"original_to":record.originally_to.map(|n|n.0),"current_from":record.current_from.map(|n|n.0),"current_to":record.current_to.map(|n|n.0)})
        }).collect::<Vec<_>>();
        let incidents=(0..self.nodes.len()).map(|i| {
            let header=self.owner_node_incident_header(NodeId(i as u32)).unwrap();
            json!({"nil":header.is_nil(),"len":header.len(),"cap":header.capacity(),"slots":(0..header.capacity()).map(|i|header.read_capacity_slot(i).unwrap().map(|edge|edge.0)).collect::<Vec<_>>()})
        }).collect::<Vec<_>>();
        json!({"hierarchies":records,"retained_maps":maps,"nodes":nodes,"edges":edges,"incidents":incidents,"abductions":abductions})
    }

    fn probe_owner_record_program(
        &mut self,
        steps: &[Value],
        hierarchies: &[OwnerHierarchy],
        maps: &mut Vec<OwnerHierarchyMap>,
        abductions: &[OwnerAbductionId],
    ) -> Vec<Value> {
        if steps.is_empty() {
            return Vec::new();
        }
        let mut observations = vec![
            json!({"op":"initial","value":null,"panic":null,"error":null,"state":self.probe_owner_record_state(hierarchies,maps,abductions)}),
        ];
        let mut snapshots = Vec::new();
        for step in steps {
            let op = step["op"].as_str().unwrap();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                match op {
                    "levels" => {
                        maps.push(
                            self.owner_hierarchy_levels(
                                &hierarchies[index(&step["hierarchy"]).unwrap()],
                            )
                            .unwrap(),
                        );
                    }
                    "replace_map" => {
                        let hierarchy = &hierarchies[index(&step["hierarchy"]).unwrap()];
                        let mut fields = self.owner_hierarchy_fields(hierarchy).unwrap();
                        fields.levels = if let Some(i) = index(&step["map"]) {
                            maps[i].clone()
                        } else {
                            self.allocate_owner_hierarchy_map(false)
                        };
                        self.set_owner_hierarchy_fields(hierarchy, fields).unwrap();
                    }
                    "put" => {
                        self.write_owner_hierarchy_level(
                            &maps[index(&step["map"]).unwrap()],
                            node(&step["node"]),
                            step["level"].as_i64().unwrap(),
                        )
                        .unwrap();
                    }
                    "delete" => {
                        self.delete_owner_hierarchy_level(
                            &maps[index(&step["map"]).unwrap()],
                            node(&step["node"]),
                        )
                        .unwrap();
                    }
                    "set_count" => {
                        let hierarchy = &hierarchies[index(&step["hierarchy"]).unwrap()];
                        let mut fields = self.owner_hierarchy_fields(hierarchy).unwrap();
                        fields.level_count = step["level_count"].as_i64().unwrap();
                        self.set_owner_hierarchy_fields(hierarchy, fields).unwrap();
                    }
                    "bind" => {
                        self.set_owner_node_hierarchy(
                            node(&step["node"]).unwrap(),
                            index(&step["hierarchy"]).map(|i| hierarchies[i].clone()),
                        )
                        .unwrap();
                    }
                    "level" => {
                        return json!(
                            self.owner_hierarchy_level(node(&step["node"]).unwrap())
                                .unwrap()
                        );
                    }
                    "snapshot" => {
                        snapshots.push(self.owner_topology_snapshot());
                    }
                    "rollback" => {
                        self.restore_owner_topology_snapshot(
                            snapshots.pop().expect("saved snapshot"),
                        );
                    }
                    "apply" => {
                        let records = items(&step["records"])
                            .iter()
                            .map(|v| abductions[index(v).unwrap()])
                            .collect();
                        self.apply_owner_edge_abductions(&OwnerAbductionSlice {
                            records,
                            present: true,
                            arena: self.topology_arena_id(),
                        });
                    }
                    _ => panic!("unknown record operation"),
                }
                Value::Null
            }));
            let (value, panic) = match outcome {
                Ok(value) => (value, None),
                Err(payload) => (
                    Value::Null,
                    Some(
                        payload
                            .downcast_ref::<String>()
                            .cloned()
                            .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                            .unwrap_or_else(|| "non-string panic".into()),
                    ),
                ),
            };
            observations.push(json!({"op":op,"value":value,"error":null,"panic":panic,"state":self.probe_owner_record_state(hierarchies,maps,abductions)}));
        }
        observations
    }
}
