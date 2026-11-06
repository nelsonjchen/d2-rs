"""elk-go Layered spacing matrices (crates/d2-rs-elk/src/model_order_contract.rs).

Each public function returns the [{name, input}] cases of
crates/d2-rs-elk/src/fixtures/original_<function>.json in fixture order,
recovered from the 2026-10-17 spacing probes.
"""
import copy

from elk_cases import curated

DIRECTIONS = ["RIGHT", "LEFT", "DOWN", "UP"]


def label_pair_spacing():
    rows = []
    for direction in DIRECTIONS:
        for inline in [False, True]:
            for edge_node in [10, 40]:
                for edge_edge in [10, 17.25]:
                    for seed in [1, 42]:
                        inp = {"id": "root",
                               "layoutOptions": {"elk.algorithm": "layered", "elk.direction": direction,
                                                 "elk.layered.considerModelOrder.strategy": "NODES_AND_EDGES",
                                                 "elk.layered.nodePlacement.bk.fixedAlignment": "BALANCED",
                                                 "elk.spacing.edgeNode": edge_node, "elk.spacing.edgeEdge": edge_edge,
                                                 "elk.randomSeed": seed},
                               "children": [{"id": "a", "width": 50, "height": 40}] +
                                           [{"id": f"b{i}", "width": 40 + i * 4, "height": 30 + i * 2}
                                            for i in range(3)],
                               "edges": [{"id": f"e{i}", "sources": ["a"], "targets": [f"b{i}"],
                                          "labels": [{"text": f"label{i}", "width": 14 + i * 2, "height": 8 + i * 2,
                                                      "layoutOptions": {"elk.edgeLabels.inline": inline}}]}
                                         for i in range(3)]}
                        rows.append(dict(name=f"label-spacing-{direction.lower()}-inline{inline}-node{edge_node}-"
                                              f"edge{edge_edge}-seed{seed}", input=inp))
    return rows


def external_port_spacing():
    """Port-port spacing on the elk-go curated `14_moon_issue9_single_container` graph."""
    rows = []
    base = curated.load("14_moon_issue9_single_container")
    for direction in DIRECTIONS:
        for spacing in [None, 0, 7.25, 23.57]:
            for edge_spacing in [10, 40]:
                for seed in [1, 42]:
                    for setting in ["root-only", "every-scope"]:
                        inp = copy.deepcopy(base)
                        opts = inp["layoutOptions"]
                        opts["elk.direction"] = direction
                        opts["elk.randomSeed"] = seed
                        opts["elk.spacing.edgeNode"] = edge_spacing
                        opts["elk.layered.spacing.edgeNodeBetweenLayers"] = edge_spacing

                        def configure(node):
                            if not node.get("children"):
                                return
                            o = node.setdefault("layoutOptions", {})
                            o.pop("elk.spacing.portPort", None)
                            if spacing is not None:
                                o["elk.spacing.portPort"] = spacing
                            if setting == "every-scope":
                                for child in node.get("children", []):
                                    configure(child)

                        configure(inp)
                        rows.append(dict(name=f"boundary-spacing-{direction.lower()}-port{spacing}-edge{edge_spacing}-"
                                              f"seed{seed}-{setting}", input=inp))
    for direction in DIRECTIONS:
        for key in ["org.eclipse.elk.spacing.portPort", "elk.spacing.portPort", "spacing.portPort"]:
            for value in [23.57, "23.57", " 7.25f ", "23.57D"]:
                inp = copy.deepcopy(base)
                inp["layoutOptions"]["elk.direction"] = direction

                def configure(node):
                    if not node.get("children"):
                        return
                    node.setdefault("layoutOptions", {})[key] = value
                    for child in node.get("children", []):
                        configure(child)

                configure(inp)
                rows.append(dict(name=f"boundary-option-{direction.lower()}-{key}-{repr(value)}", input=inp))
    return rows
