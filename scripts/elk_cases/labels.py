"""elk-go Layered edge-label matrices (crates/d2-rs-elk/src/model_order_contract.rs).

Each public function returns the [{name, input}] cases of
crates/d2-rs-elk/src/fixtures/original_<function>.json in fixture order,
recovered from the 2026-10-16 center-label probes.
"""
import copy
import json

from elk_cases import curated


def compound():
    """elk-go curated `09_compound`: an INCLUDE_CHILDREN RIGHT cluster beside an outside node."""
    return curated.load("09_compound")


def center_label_sides():
    rows = []
    base = compound()
    for direction in ["DOWN", "RIGHT", "UP", "LEFT"]:
        for mode in ["ALWAYS_UP", "ALWAYS_DOWN", "DIRECTION_UP", "DIRECTION_DOWN", "SMART_UP", "SMART_DOWN"]:
            for owner in ["root", "child"]:
                for profile in ["single", "stacked", "parallel", "reversed"]:
                    g = json.loads(json.dumps(base))
                    g["layoutOptions"]["elk.direction"] = direction
                    if owner == "root":
                        g["children"] = g["children"][0]["children"]
                        g["edges"] = [dict(id="inside", sources=["cluster.a"], targets=["cluster.b"])]
                        scope = g
                    else:
                        scope = g["children"][0]
                    opts = scope.setdefault("layoutOptions", {})
                    opts["elk.layered.edgeLabels.sideSelection"] = mode
                    opts["elk.layered.cycleBreaking.strategy"] = "GREEDY_MODEL_ORDER"
                    opts["elk.spacing.edgeLabel"] = 3
                    opts["elk.spacing.labelLabel"] = 2
                    labels = [dict(text="label", width=36, height=12, layoutOptions={"elk.edgeLabels.inline": False})]
                    if profile == "stacked":
                        labels = [dict(text=t, width=w, height=h, layoutOptions={"elk.edgeLabels.inline": False})
                                  for t, w, h in [("first", 12, 5), ("second", 36, 11), ("third", 17, 7)]]
                    scope["edges"][0]["labels"] = labels
                    if profile in ["parallel", "reversed"]:
                        scope["edges"].append(dict(
                            id="second-inside", sources=["cluster.b" if profile == "reversed" else "cluster.a"],
                            targets=["cluster.a" if profile == "reversed" else "cluster.b"],
                            labels=[dict(text="other", width=19, height=9, layoutOptions={"elk.edgeLabels.inline": False})]))
                    rows.append(dict(name="-".join([direction.lower(), mode.lower(), owner, profile]), input=g))
    return rows


def owned_edges():
    """The curated compound with its inside edge owned by the root, the cluster, a grandparent wrapper or two siblings."""
    rows = []
    for labelled in [True, False]:
        for direction in ["DOWN", "RIGHT", "UP", "LEFT"]:
            for owner in ["root", "child", "grandchild", "siblings"]:
                g = compound()
                g["layoutOptions"]["elk.direction"] = direction
                cluster = g["children"][0]
                if labelled:
                    cluster["edges"][0]["labels"] = [dict(text="inside", width=36, height=12)]
                if owner == "root":
                    g["edges"].append(cluster.pop("edges")[0])
                elif owner == "grandchild":
                    g["children"][0] = dict(id="outer", width=0, height=0, children=[cluster])
                elif owner == "siblings":
                    second = json.loads(json.dumps(cluster).replace("cluster.", "second."))
                    second["id"] = "second"
                    second["edges"][0]["id"] = "second-inside"
                    g["children"].append(second)
                    g["edges"].append(dict(id="second-outside", sources=["second"], targets=["outside"]))
                rows.append(dict(name=f"{'labelled' if labelled else 'plain'}-{direction.lower()}-{owner}-owned",
                                 input=g))
    return rows


def inline_label_stacks():
    """center_label_sides' stacked-label profile with every label inline."""
    rows = []
    for row in center_label_sides():
        if not row["name"].endswith("-stacked"):
            continue
        g = copy.deepcopy(row["input"])
        scope = g if row["name"].split("-")[2] == "root" else g["children"][0]
        for label in scope["edges"][0]["labels"]:
            label["layoutOptions"]["elk.edgeLabels.inline"] = True
        rows.append(dict(name=row["name"] + "-inline", input=g))
    return rows
