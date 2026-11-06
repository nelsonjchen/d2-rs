"""elk-go Layered model-order matrices (crates/d2-rs-elk/src/model_order_contract.rs).

Each public function returns the [{name, input}] cases of
crates/d2-rs-elk/src/fixtures/original_<function>.json in fixture order,
recovered from the 2026-10-17 model-order probes.
"""
import copy
import itertools

MODEL = "elk.layered.considerModelOrder.strategy"


def _nested_model(direction, order, edge_order, ownership):
    opts = {"elk.algorithm": "layered", "elk.direction": direction, "elk.hierarchyHandling": "INCLUDE_CHILDREN",
            MODEL: "NODES_AND_EDGES", "elk.layered.nodePlacement.bk.fixedAlignment": "BALANCED", "elk.randomSeed": 1}
    inner = [{"id": f"box.{a}-{b}", "sources": ["box." + a], "targets": ["box." + b]}
             for a, b in [("a", "b"), ("b", "c"), ("a", "c")]]
    if edge_order:
        inner.reverse()
    box = {"id": "box", "width": 80, "height": 60, "layoutOptions": opts.copy(),
           "children": [{"id": "box." + n, "width": 30 + ord(n) - 97, "height": 20 + 2 * (ord(n) - 97)} for n in order]}
    inp = {"id": "root", "layoutOptions": opts, "children": [box, {"id": "outside", "width": 40, "height": 30}],
           "edges": [{"id": "entry", "sources": ["outside"], "targets": ["box.a"]}]}
    if ownership == "root":
        inp["edges"] += inner
    else:
        box["edges"] = inner
    return dict(name=f'model-{direction.lower()}-{"".join(order)}-reverse{edge_order}-{ownership}', input=inp)


def nested_input_model_sort():
    rows = [_nested_model(direction, order, edge_order, ownership)
            for direction in ["RIGHT", "LEFT", "DOWN", "UP"]
            for order in itertools.permutations("abc")
            for edge_order in [False, True]
            for ownership in ["root", "child"]]
    variants = []
    for row in rows:
        for suffix in ["seed42", "root-none", "child-none", "both-none"]:
            inp = copy.deepcopy(row["input"])
            box = inp["children"][0]
            if suffix == "seed42":
                inp["layoutOptions"]["elk.randomSeed"] = 42
                box["layoutOptions"]["elk.randomSeed"] = 42
            if suffix in ("root-none", "both-none"):
                inp["layoutOptions"][MODEL] = "NONE"
            if suffix in ("child-none", "both-none"):
                box["layoutOptions"][MODEL] = "NONE"
            variants.append(dict(name=row["name"] + "-" + suffix, input=inp))
    return rows + variants
