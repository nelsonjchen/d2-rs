"""elk-go Fixed and Random provider controls (crates/d2-rs-elk/src/{fixed_layout,random_layout}.rs).

Each public function returns the [{name, input}] cases of
crates/d2-rs-elk/src/fixtures/original_<function>.json in fixture order,
recovered from the 2026-10-19 Fixed and Random qualification controls (their
inline generators were not retained; these reproduce the recorded inputs
exactly).
"""
import copy


GEOMETRY_KEYS = ["id", "text", "x", "y", "width", "height", "children", "ports", "labels", "edges", "sections",
                 "startPoint", "endPoint", "bendPoints"]


def _keep(value, keys):
    if isinstance(value, dict):
        return {k: _keep(v, keys) for k, v in value.items() if k in keys}
    if isinstance(value, list):
        return [_keep(v, keys) for v in value]
    return value


def project_fixed(output):
    """The geometry fields of a raw Fixed output (fixed_layout.rs compares exactly these)."""
    return _keep(output, GEOMETRY_KEYS)


def project_random(output):
    """Geometry plus edge sources/targets/container of a raw Random output (random_layout.rs)."""
    return _keep(output, GEOMETRY_KEYS + ["sources", "targets", "container"])


def _root(algorithm, children, edges=None, **options):
    graph = {"id": "root", "layoutOptions": {"elk.algorithm": algorithm, **options}, "children": children}
    if edges is not None:
        graph["edges"] = edges
    return graph


def _a(**extra):
    return {"id": "a", "width": 40, "height": 30, **extra}


def fixed():
    at = dict(x=10, y=20)
    minimum = {"elk.nodeSize.constraints": "MINIMUM_SIZE"}
    pair = [_a(), {"id": "b", "width": 40, "height": 30, "x": 50}]
    section = {"startPoint": {"x": 1000, "y": 1000}, "endPoint": {"x": 1200, "y": 1200},
               "bendPoints": [{"x": 100, "y": 150}], "id": "es0"}
    rows = [
        ("shape-positions", _root("fixed", [_a(**at)])),
        ("negative-positions", _root("fixed", [_a(x=-60, y=-80)])),
        ("multiple", _root("fixed", [_a(**at), {"id": "b", "width": 80, "height": 10, "x": 120, "y": -10}])),
        ("asymmetric", _root("fixed", [_a(**at)], **{"elk.padding": "[top=10,left=20,bottom=30,right=40]"})),
        ("root-minimum", _root("fixed", [_a(**at)], **minimum, **{"elk.nodeSize.minimum": "(300,200)"})),
        ("fixed-size", dict(_root("fixed", [_a(**at)], **{"elk.nodeSize.fixedGraphSize": True}), width=123,
                            height=456)),
        ("position-property", _root("fixed", [_a(**at, layoutOptions={"elk.position": "(100,200)"})])),
        ("minimum-with-position", _root("fixed", [_a(layoutOptions={
            "elk.position": "(5,6)", **minimum, "elk.nodeSize.minimum": "(80,90)"})])),
        ("minimum-without-position", _root("fixed", [_a(layoutOptions={**minimum, "elk.nodeSize.minimum": "(80,90)"})])),
        ("position-without-minimum", _root("fixed", [_a(layoutOptions={"elk.position": "(5,6)", **minimum})])),
        ("node-label", _root("fixed", [_a(**at, labels=[{"text": "L", "width": 30, "height": 10,
                                                          "layoutOptions": {"elk.position": "(50,60)"}}])])),
    ]
    for fixed_pos in [False, True]:
        options = {"elk.position": "(5,6)", **minimum, "elk.nodeSize.minimum": "(80,60)"}
        if fixed_pos:
            options["elk.portConstraints"] = "FIXED_POS"
        rows.append((f"resize-port-{fixed_pos}", _root("fixed", [_a(layoutOptions=options, ports=[
            {"id": "p", "x": 40, "y": 15, "width": 0, "height": 0}])], **{"elk.direction": "RIGHT"})))
    rows += [
        ("resize-label", _root("fixed", [_a(layoutOptions={"elk.position": "(5,6)", **minimum,
                                                           "elk.nodeSize.minimum": "(80,60)"},
                                           labels=[{"text": "L", "x": 40, "y": 10, "width": 10, "height": 10}])])),
        ("existing-sections", _root("fixed", copy.deepcopy(pair), [
            {"id": "e", "sources": ["a"], "targets": ["b"], "sections": [copy.deepcopy(section)]}])),
        ("bend-chain", _root("fixed", copy.deepcopy(pair), [
            {"id": "e", "sources": ["a"], "targets": ["b"], "sections": [copy.deepcopy(section)],
             "layoutOptions": {"elk.bendPoints": "(1,2);(150,160);(8,9)"}}])),
        ("edge-label", _root("fixed", copy.deepcopy(pair), [
            {"id": "e", "sources": ["a"], "targets": ["b"], "labels": [
                {"text": "edge", "width": 40, "height": 20, "layoutOptions": {"elk.position": "(100,200)"}}]}])),
    ]
    return [dict(name=name, input=graph) for name, graph in rows]


def random_seeded():
    styled = {"elk.padding": "[top=10,left=20,bottom=30,right=40]", "elk.spacing.nodeNode": 17.1234567,
              "elk.aspectRatio": 1.23456789}
    n = lambda i, w, h, **extra: {"id": f"n{i}", "width": w, "height": h, **extra}
    e = lambda i, s, t: {"id": f"e{i}", "sources": [s], "targets": [t]}
    port = lambda size: [{"id": "p0", "x": 40, "y": 15, "width": size, "height": size}]

    def shapes(seed):
        opts = {"elk.randomSeed": seed}
        return [
            _root("random", [n(0, 40, 30)], **opts),
            _root("random", [n(0, 40, 30), n(1, 80, 50)], **opts, **styled),
            _root("random", [n(0, 20, 80), n(1, 200, 30), n(2, 50, 60), n(3, 80, 40)], [e(0, "n0", "n1")], **opts),
            _root("random", [n(0, 40, 30), n(1, 60, 50)], [e(0, "n0", "n1"), e(1, "n1", "n1")], **opts, **styled),
            _root("random", [n(0, 40, 30, ports=port(2)), n(1, 80, 50)],
                  [e(0, "n0", "n1"), e(1, "n1", "n1"), e(2, "p0", "n1")], **opts),
        ]

    rows = []
    for seed in [1, -1, 42, "2147483647", "-2147483648"]:
        rows += [dict(name=f"{seed}-{k}", input=g) for k, g in enumerate(shapes(seed))]
    section = {"id": "existing", "startPoint": {"x": 1, "y": 2}, "endPoint": {"x": 3, "y": 4},
               "bendPoints": [{"x": 5, "y": 6}]}
    sections = copy.deepcopy(rows[-1]["input"])
    sections["edges"][0]["sections"] = [copy.deepcopy(section)]
    hyper = copy.deepcopy(sections)
    hyper["edges"][0].update(sources=["n0", "p0"], targets=["n1", "n0"])
    hyper["edges"][0]["sections"] = hyper["edges"][0].pop("sections")
    port_zero = copy.deepcopy(hyper)
    port_zero["children"][0]["ports"] = port(0)
    rows += [dict(name="sections", input=sections), dict(name="hyper", input=hyper),
             dict(name="port-zero", input=port_zero)]

    def nested(owned=True, **group_options):
        group = {"id": "g", "width": 60, "height": 50,
                 "children": [{"id": "a", "width": 40, "height": 30}, {"id": "b", "width": 20, "height": 10}]}
        inner = [{"id": "inner", "sources": ["a"], "targets": ["b"]}]
        if owned:
            group["edges"] = inner
        if group_options:
            group["layoutOptions"] = group_options
        graph = _root("random", [group, {"id": "x", "width": 30, "height": 20}], **{"elk.randomSeed": "42"})
        if not owned:
            graph["edges"] = inner
        return graph

    root_no_layout = nested()
    root_no_layout["layoutOptions"]["elk.noLayout"] = True
    rows += [dict(name="owned-nested", input=nested()), dict(name="flat-nested", input=nested(owned=False)),
             dict(name="skipped-nested", input=nested(**{"elk.noLayout": True})),
             dict(name="root-no-layout", input=root_no_layout)]
    return rows
