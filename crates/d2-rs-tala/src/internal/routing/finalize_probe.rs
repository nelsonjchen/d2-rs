// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
#![cfg(feature = "diagnostic-traces")]
use super::*;
use serde_json::{Value, json};

pub(in crate::internal::routing) struct State<'a> {
    records: Vec<PublishedRoute>,
    pool: Vec<Option<usize>>,
    order: Vec<Option<usize>>,
    selected: Option<BTreeSet<usize>>,
    work: Option<LocalRouteSearchWork<'a>>,
}
impl<'a> State<'a> {
    pub(in crate::internal::routing) fn new(graph: &ArenaGraph, config: &Value) -> Self {
        let mut records = Vec::new();
        let mut pool = Vec::new();
        for value in config["final_routes"].as_array().into_iter().flatten() {
            if value.is_null() {
                pool.push(None);
                continue;
            }
            if let Some(alias) = value["alias"].as_u64() {
                pool.push(pool[alias as usize]);
                continue;
            }
            let (points, live) = if let Some(edge) = value["from_edge"].as_u64() {
                let route = &graph.edge_records[edge as usize].route;
                (
                    (0..route.len()).map(|i| route.point(i)).collect(),
                    (0..route.len()).map(|i| route.slot(i)).collect(),
                )
            } else {
                (
                    value["points"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|p| Point {
                            x: p[0].as_f64().unwrap(),
                            y: p[1].as_f64().unwrap(),
                        })
                        .collect(),
                    Vec::new(),
                )
            };
            let endpoints = value["ports"].as_array().map(|v| {
                [
                    Point {
                        x: v[0][0].as_f64().unwrap(),
                        y: v[0][1].as_f64().unwrap(),
                    },
                    Point {
                        x: v[1][0].as_f64().unwrap(),
                        y: v[1][1].as_f64().unwrap(),
                    },
                ]
            });
            pool.push(Some(records.len()));
            records.push(PublishedRoute::new(
                value["edge"]
                    .as_i64()
                    .unwrap_or(0)
                    .try_into()
                    .unwrap_or(usize::MAX),
                points,
                None,
                endpoints,
                live,
            ));
        }
        let order = config["final_order"].as_array().map_or_else(
            || pool.clone(),
            |indices| {
                indices
                    .iter()
                    .map(|i| {
                        i.as_i64()
                            .filter(|i| *i >= 0)
                            .and_then(|i| pool[i as usize])
                    })
                    .collect()
            },
        );
        let selected = config["final_select"].as_bool().unwrap_or(false).then(|| {
            config["selected"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|i| i.as_i64().unwrap().try_into().unwrap_or(usize::MAX))
                .collect()
        });
        Self {
            records,
            pool,
            order,
            selected,
            work: None,
        }
    }
    pub(in crate::internal::routing) fn run(
        &mut self,
        graph: &mut ArenaGraph,
        config: &Value,
        limit: u64,
        context: &'a dyn Context,
    ) -> Result<(), String> {
        if config["context"] == "nil" {
            return Err("TALA EdgeRouting requires a context".into());
        }
        self.work = Some(
            LocalRouteSearchWork::with_context(limit, context)
                .map_err(|cause| Error::Guard(cause).to_string())?,
        );
        let updates = selected(
            (!config["nil_graph"].as_bool().unwrap_or(false)).then_some(&*graph),
            &mut self.records,
            &self.order,
            self.selected.as_ref(),
            if config["nil_work"].as_bool().unwrap_or(false) {
                None
            } else {
                self.work.as_ref()
            },
            context,
            "Default",
            config["final_reorder"].as_bool().unwrap_or(false),
        )
        .map_err(|error| error.to_string())?;
        for (edge, points) in updates {
            graph.edge_records[edge].route.replace(points);
        }
        Ok(())
    }
    pub(in crate::internal::routing) fn used(&self) -> u64 {
        self.work.as_ref().map_or(0, LocalRouteSearchWork::used)
    }
    pub(in crate::internal::routing) fn observe(&self) -> Value {
        let point = |p: Point| {
            [
                format!("{:016x}", p.x.to_bits()),
                format!("{:016x}", p.y.to_bits()),
            ]
        };
        json!(self.pool.iter().map(|id| {
            id.map(|id| {
                let route = &self.records[id]; let ports = route.endpoints.unwrap_or([Point { x: 0.0, y: 0.0 }; 2]);
                json!({"id":id+1,"edge":if route.edge == usize::MAX { -1 } else { route.edge as i64 },"points":(0..route.len()).map(|i|point(route.point(i))).collect::<Vec<_>>(),"ports":[point(ports[0]),point(ports[1])]})
            })
        }).collect::<Vec<_>>())
    }
}
