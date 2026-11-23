// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Upstream large_routing_test.go largeContainerDiagram: #2891's pair order,
// stride, 40 containers and three children each, at the public Graph boundary.
fn large_container_graph(links: usize, nested: bool) -> Graph {
    let mut input = Graph::default();
    let mut names = Vec::new();
    let mut domain = None;
    for c in 0..40 {
        if nested && c % 5 == 0 {
            domain = Some(input.add_node(node(&format!("domain{}", c / 5), 100., 66.)));
        }
        let mut container = node(&format!("c{c}"), 100., 66.);
        container.parent = domain;
        let parent = input.add_node(container);
        for n in 0..3 {
            let mut child = node(&format!("c{c}.n{n}"), 100., 66.);
            child.parent = Some(parent);
            names.push(input.add_node(child));
        }
    }
    let stride = (40 * 39 / 2 * 3 * 3) / links;
    let (mut pair, mut written) = (0, 0);
    for (i, &from) in names.iter().enumerate() {
        for (j, &to) in names.iter().enumerate().skip(i + 1) {
            if i / 3 == j / 3 {
                continue;
            }
            if pair % stride == 0 && written < links {
                let edge = input.add_edge(Edge {
                    source: from,
                    target: to,
                });
                input.set_edge_arrows(
                    edge,
                    EdgeArrows {
                        source: false,
                        target: true,
                    },
                );
                written += 1;
            }
            pair += 1;
        }
    }
    input
}
fn check_large_container_routing(links: usize, nested: bool, seed: i64) {
    let input = large_container_graph(links, nested);
    let result = crate::Engine
        .layout(
            &input,
            &crate::LayoutOptions {
                seeds: vec![seed],
                max_concurrency: 1,
            },
        )
        .unwrap();
    assert_eq!(result.boxes.len(), if nested { 168 } else { 160 });
    assert_eq!(result.routes.len(), links);
    for route in result.routes.values() {
        assert!(route.len() >= 2);
    }
}
#[test]
fn large_container_routing_within_work_limits() {
    for seed in crate::LayoutOptions::default().seeds {
        check_large_container_routing(100, false, seed);
    }
}
#[test]
fn large_nested_routing_in_standard_suite() {
    // 9a46c7f2d moves the 230-edge case into the normal suite. Sketch and
    // rendering live outside this crate; exercise the same graph and seed.
    check_large_container_routing(230, true, 1);
}
