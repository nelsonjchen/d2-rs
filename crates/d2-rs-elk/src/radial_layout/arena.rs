// Copyright (c) 2017, 2020 Kiel University and others. EPL-2.0.
// elk-go/internal/radial/provider.go: graph navigation and geometry transfer.
use super::*;
pub(super) struct Node {
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub incoming: Vec<usize>,
    pub outgoing: Vec<usize>,
    pub position: V,
    pub size: V,
    pub original: V,
    pub order: i32,
    pub margins: [f64; 4],
    pub angle: f64,
}
impl Node {
    pub fn center(&self) -> V {
        self.position.add(self.size.scale(0.5))
    }
}
pub(super) struct Edge {
    pub sources: Vec<(usize, bool)>,
    pub targets: Vec<usize>,
    pub container: usize,
}
pub(super) struct Arena {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub ids: BTreeMap<String, usize>,
}
impl Arena {
    pub fn import(graph: &ElkGraph, micro: bool) -> Self {
        let mut nodes = vec![];
        let mut ids = BTreeMap::new();
        let mut endpoints = BTreeMap::new();
        let mut ports = BTreeMap::new();
        fn insert(
            node: &ElkNode,
            parent: Option<usize>,
            nodes: &mut Vec<Node>,
            ids: &mut BTreeMap<String, usize>,
            endpoints: &mut BTreeMap<String, (usize, bool)>,
            ports: &mut BTreeMap<usize, Vec<String>>,
        ) -> usize {
            let n = nodes.len();
            let o = node.layout_options.as_ref().unwrap_or(&Value::Null);
            let (px, py) = point(o, "position").unwrap_or_default();
            let margins = option(o, "margins")
                .map(|v| {
                    let p = padding(&serde_json::json!({"org.eclipse.elk.padding":v}));
                    [p[0], p[3], p[2], p[1]]
                })
                .unwrap_or([0.0; 4]);
            nodes.push(Node {
                parent,
                children: vec![],
                incoming: vec![],
                outgoing: vec![],
                position: V {
                    x: node.x,
                    y: node.y,
                },
                size: V {
                    x: node.width,
                    y: node.height,
                },
                original: V { x: px, y: py },
                order: integer(o, "radial.orderId").unwrap_or(0),
                margins,
                angle: number(o, "radial.rotation.targetAngle", 0.0),
            });
            ids.insert(node.id.clone(), n);
            endpoints.insert(node.id.clone(), (n, false));
            for p in &node.ports {
                endpoints.insert(p.id.clone(), (n, true));
                ports.entry(n).or_default().push(p.id.clone());
            }
            for c in &node.children {
                let child = insert(c, Some(n), nodes, ids, endpoints, ports);
                nodes[n].children.push(child)
            }
            n
        }
        let root = ElkNode {
            id: graph.id.clone(),
            x: graph.x.unwrap_or(0.0),
            y: graph.y.unwrap_or(0.0),
            width: graph.width.unwrap_or(0.0),
            height: graph.height.unwrap_or(0.0),
            children: graph.children.clone(),
            edges: vec![],
            ports: vec![],
            labels: vec![],
            layout_options: Some(graph.layout_options.clone()),
        };
        insert(
            &root,
            None,
            &mut nodes,
            &mut ids,
            &mut endpoints,
            &mut ports,
        );
        let containers = crate::edge_container_ids(graph, &graph.edges);
        let mut edges = vec![];
        let mut port_incoming = BTreeMap::<String, Vec<usize>>::new();
        for (e, wire) in graph.edges.iter().enumerate() {
            let sources = wire
                .sources
                .iter()
                .filter_map(|id| endpoints.get(id).copied())
                .collect::<Vec<_>>();
            let targets = wire
                .targets
                .iter()
                .filter_map(|id| endpoints.get(id).map(|x| x.0))
                .collect();
            for &(source, is_port) in &sources {
                if !is_port {
                    nodes[source].outgoing.push(e)
                }
            }
            for target in &wire.targets {
                if let Some(&(n, is_port)) = endpoints.get(target) {
                    if is_port {
                        port_incoming.entry(target.clone()).or_default().push(e)
                    } else {
                        nodes[n].incoming.push(e)
                    }
                }
            }
            edges.push(Edge {
                sources,
                targets,
                container: ids.get(&containers[e]).copied().unwrap_or(0),
            });
        }
        for (n, port_ids) in ports {
            for id in port_ids {
                if let Some(incoming) = port_incoming.get(&id) {
                    nodes[n].incoming.extend(incoming)
                }
            }
        }
        let mut result = Self { nodes, edges, ids };
        if micro {
            let spacing = number(&graph.layout_options, "spacing.labelNode", 5.0);
            for child in &graph.children {
                let n = result.ids[&child.id];
                result.nodes[n].margins = node_margins(graph, child, spacing)
            }
        }
        result
    }
    pub fn successors(&self, n: usize) -> Vec<usize> {
        self.nodes[n]
            .outgoing
            .iter()
            .filter_map(|&e| {
                let e = &self.edges[e];
                if e.sources.first()?.1 {
                    return None;
                }
                let &target = e.targets.first()?;
                (!self.nodes[n].children.contains(&target)).then_some(target)
            })
            .collect()
    }
    pub fn tree_parent(&self, n: usize) -> Option<usize> {
        self.edges[*self.nodes[n].incoming.first()?]
            .sources
            .first()
            .map(|x| x.0)
    }
    pub fn largest(&self, n: usize) -> f64 {
        let mut result = 0.0_f64;
        for &c in &self.nodes[n].children {
            result = result.max(self.nodes[c].size.length());
            result = result.max(self.largest(c))
        }
        result
    }
    pub fn publish(&self, wire: &mut [ElkNode]) {
        for node in wire {
            let n = &self.nodes[self.ids[&node.id]];
            node.x = n.position.x;
            node.y = n.position.y;
            self.publish(&mut node.children)
        }
    }
    pub fn route(&self, graph: &mut ElkGraph, n: usize) {
        for &e in &self.nodes[n].outgoing {
            let edge = &self.edges[e];
            if edge.sources.first().is_none_or(|s| s.1) {
                continue;
            }
            let Some(&target) = edge.targets.first() else {
                continue;
            };
            if self.nodes[n].parent != Some(edge.container) {
                continue;
            }
            let (a, b) = clipped(
                self.nodes[n].center(),
                self.nodes[target].center(),
                self.nodes[n].size,
                self.nodes[target].size,
            );
            let wire = &mut graph.edges[e];
            if wire.sections.is_empty() {
                wire.sections.push(ElkEdgeSection {
                    id: Some(format!("{}_s0", wire.id)),
                    start: a.point(),
                    end: b.point(),
                    bends: vec![],
                    incoming_shape: None,
                    outgoing_shape: None,
                })
            }
            let section = &mut wire.sections[0];
            section.start = a.point();
            section.end = b.point();
            section.bends.clear();
            self.route(graph, target)
        }
    }
}
pub(super) fn node_margins(graph: &ElkGraph, node: &ElkNode, spacing: f64) -> [f64; 4] {
    let mut min = V {
        x: node.x,
        y: node.y,
    };
    let mut max = V {
        x: node.x + node.width,
        y: node.y + node.height,
    };
    let mut union = |x: f64, y: f64, w: f64, h: f64| {
        min.x = min.x.min(x);
        min.y = min.y.min(y);
        max.x = max.x.max(x + w);
        max.y = max.y.max(y + h);
        // Rectangle.Union stores size, then MaxX/MaxY reconstruct the end.
        // Preserve those subtraction/addition boundaries on every union.
        max.x = min.x + (max.x - min.x);
        max.y = min.y + (max.y - min.y);
    };
    for label in &node.labels {
        union(
            node.x + label.x,
            node.y + label.y,
            label.width,
            label.height,
        )
    }
    for port in &node.ports {
        union(node.x + port.x, node.y + port.y, port.width, port.height)
    }
    for edge in &graph.edges {
        for (incoming, ends) in [(false, &edge.sources), (true, &edge.targets)] {
            for endpoint in ends {
                let port = node.ports.iter().find(|p| &p.id == endpoint);
                if endpoint != &node.id && port.is_none() {
                    continue;
                }
                for label in &edge.labels {
                    let placement = label
                        .layout_options
                        .as_ref()
                        .and_then(|o| option(o, "edgeLabels.placement"))
                        .and_then(Value::as_str)
                        .unwrap_or("CENTER");
                    if placement != if incoming { "HEAD" } else { "TAIL" } {
                        continue;
                    }
                    let mut x = node.x;
                    let mut y = node.y;
                    if let Some(port) = port {
                        x += port.x;
                        y += port.y;
                        match option(&port.layout_options, "port.side")
                            .and_then(Value::as_str)
                            .unwrap_or("UNDEFINED")
                        {
                            "WEST" => x -= spacing + spacing + label.width,
                            "NORTH" => {
                                x += port.width + spacing;
                                y -= spacing + spacing + label.height
                            }
                            "SOUTH" => {
                                x += port.width + spacing;
                                y += port.height + spacing + spacing
                            }
                            _ => x += port.width + spacing + spacing,
                        }
                    } else if incoming {
                        x -= spacing + label.width
                    } else {
                        x += node.width + spacing
                    }
                    union(x, y, label.width, label.height)
                }
            }
        }
    }
    [
        (node.y - min.y).max(0.0),
        (max.x - (node.x + node.width)).max(0.0),
        (max.y - (node.y + node.height)).max(0.0),
        (node.x - min.x).max(0.0),
    ]
}
