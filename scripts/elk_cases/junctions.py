"""elk-go Layered port-alignment and hierarchy-junction matrices
(crates/d2-rs-elk/src/model_order_contract.rs).

Each public function returns the [{name, input}] cases (plus `raw_input`,
the exact stdin text, where the capture recorded one) of
crates/d2-rs-elk/src/fixtures/original_<function>.json in fixture order,
recovered from the 2026-10-17 alignment, junction and label-merge probes.
"""
import copy
import json


def _port(name, x, y, side):
    return dict(id=name, x=x, y=y, width=0, height=0, layoutOptions={"elk.port.side": side})


def _alignment(direction, alignment, multiplicity, key):
    children = [dict(id="a", width=40, height=40),
                dict(id="b", width=30, height=20, ports=[_port("b.w", 0, 10, "WEST"), _port("b.e", 30, 10, "EAST")],
                     layoutOptions={"elk.portConstraints": "FIXED_POS", key: alignment}),
                dict(id="c", width=96, height=50),
                dict(id="d", width=40, height=40, ports=[_port("d.w", 0, 20, "WEST")],
                     layoutOptions={"elk.portConstraints": "FIXED_POS"})]
    edges = [dict(id="ab", sources=["a"], targets=["b.w"]), dict(id="ac", sources=["a"], targets=["c"]),
             dict(id="cd", sources=["c"], targets=["d.w"])] + \
            [dict(id=f"bd{i}", sources=["b.e"], targets=["d.w"]) for i in range(multiplicity)]
    inp = dict(id="root", layoutOptions={"elk.algorithm": "layered", "elk.direction": direction,
                                         "elk.layered.considerModelOrder.strategy": "NODES_AND_EDGES",
                                         "elk.layered.thoroughness": 1}, children=children, edges=edges)
    return dict(name=f"align-{direction.lower()}-{alignment}-edges{multiplicity}-" +
                     ("canonical" if key.startswith("org.") else "short"), input=inp)


def shared_port_alignment():
    return [_alignment(direction, alignment, multiplicity, key)
            for direction in ["RIGHT", "DOWN", "LEFT", "UP"]
            for alignment in ["AUTOMATIC", "LEFT", "RIGHT", "CENTER", "TOP", "BOTTOM"]
            for multiplicity in [1, 3]
            for key in ["elk.alignment", "org.eclipse.elk.alignment"]]


def hierarchy_junctions():
    base = _alignment("RIGHT", "AUTOMATIC", 1, "elk.alignment")["input"]
    rows = []
    for direction in ["RIGHT", "DOWN", "LEFT", "UP"]:
        for mode in ["root-owned", "child-owned", "outbound", "inbound"]:
            for multiplicity in [1, 3]:
                for seed in [1, 42]:
                    children = copy.deepcopy(base["children"])
                    edges = copy.deepcopy(base["edges"])
                    edges = edges[:3] + [dict(id=f"bd{i}", sources=["b.e"], targets=["d.w"]) for i in range(multiplicity)]
                    group = dict(id="g", width=0, height=0, children=children)
                    inp = dict(id="root", layoutOptions=dict(base["layoutOptions"]) | {
                        "elk.hierarchyHandling": "INCLUDE_CHILDREN", "elk.direction": direction, "elk.randomSeed": seed},
                               children=[group], edges=[])
                    if mode == "child-owned":
                        group["edges"] = edges
                    else:
                        inp["edges"] = edges
                    if mode in ["outbound", "inbound"]:
                        outside = dict(id="outside", width=50, height=40,
                                       ports=[dict(id="outside.p", x=0 if mode == "outbound" else 50, y=20, width=0,
                                                   height=0,
                                                   layoutOptions={"elk.port.side": "WEST" if mode == "outbound" else "EAST"})],
                                       layoutOptions={"elk.portConstraints": "FIXED_POS"})
                        inp["children"].append(outside)
                        inp["edges"] += [dict(id="cross" + str(i), sources=[node if mode == "outbound" else "outside.p"],
                                              targets=["outside.p" if mode == "outbound" else node])
                                         for i, node in enumerate(["b.e", "c"])]
                    rows.append(dict(name=f"junction-hier-{direction.lower()}-{mode}-edges{multiplicity}-seed{seed}",
                                     input=inp))
    return rows


def hierarchy_labels():
    """hierarchy_junctions with one label per edge at every level, outside or inline."""
    rows = []
    for row in hierarchy_junctions():
        for inline in [False, True]:
            graph = copy.deepcopy(row["input"])

            def decorate(node):
                for index, edge in enumerate(node.get("edges", [])):
                    edge["labels"] = [dict(id="label-" + edge["id"], text="edge-" + edge["id"], width=28 + index * 7,
                                           height=12, layoutOptions={"elk.edgeLabels.inline": inline})]
                for child in node.get("children", []):
                    decorate(child)

            decorate(graph)
            rows.append(dict(name=row["name"] + "-inline" + str(inline), input=graph, raw_input=json.dumps(graph) + "\n"))
    return rows


def north_south_switch_sides():
    """allowNonFlowPortsToSwitchSides spellings on every port of the root-owned junction graph."""
    junctions = {row["name"]: row["input"] for row in hierarchy_junctions()}
    rows = []
    for direction in ["right", "down", "left", "up"]:
        base = junctions["junction-hier-" + direction + "-root-owned-edges1-seed1"]
        for form, value in [("bool-true", True), ("bool-false", False), ("string-true", "true"),
                            ("string-false", "false"), ("string-1", "1"), ("string-0", "0"), ("string-t", "t"),
                            ("string-f", "f")]:
            for spelling, key in [("canonical", "org.eclipse.elk.layered.allowNonFlowPortsToSwitchSides"),
                                  ("short", "elk.layered.allowNonFlowPortsToSwitchSides")]:
                graph = copy.deepcopy(base)
                for node in graph["children"][0]["children"]:
                    for port in node.get("ports", []):
                        port.setdefault("layoutOptions", {})[key] = value
                rows.append(dict(name="switch-" + direction + "-" + form + "-" + spelling, input=graph))
    return rows
