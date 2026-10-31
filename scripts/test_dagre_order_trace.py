"""Contracts for multi-invocation Dagre order-trace comparisons."""

import importlib.util
from pathlib import Path
import unittest


spec = importlib.util.spec_from_file_location(
    "check_dagre_order_trace", Path(__file__).with_name("check-dagre-order-trace.py")
)
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


def go_group(object_id, node_ids):
    lines = [f'GO_MAP id="0" object="{object_id}"', "GO_DAGRO_PHASE border-segments"]
    for index, node_id in enumerate(node_ids):
        lines.append(f'GO_DAGRO_NODE index={index} id="{node_id}" parent=""')
    layer = ",".join(f'"{node_id}"' for node_id in node_ids)
    layers = f"[[{layer}]]"
    lines.extend(
        [
            f"GO_DAGRE_ORDER phase=init iteration=-1 crossings_bits=none layers={layers}",
            f"GO_DAGRE_ORDER phase=sweep iteration=0 crossings_bits=0000000000000000 layers={layers}",
            f"GO_DAGRE_ORDER phase=best iteration=-1 crossings_bits=0000000000000000 layers={layers}",
            "GO_DAGRO_PHASE order",
        ]
    )
    return lines


def rust_group(external_ids):
    lines = []
    for index, external_id in enumerate(external_ids):
        lines.append(f"RUST_DAGRE_ORDER_NODE index={index} id={100 + index}")
        lines.append(f'RUST_DAGRE_OBJECT index={index} external_id="{external_id}"')
    layers = "[" + ",".join(str(index) for index in range(len(external_ids))) + "]"
    normalized = f"[{layers}]"
    lines.extend(
        [
            f"RUST_DAGRE_LAYERS phase=init iteration=-1 crossings_bits=none layers={normalized}",
            f"RUST_DAGRE_LAYERS phase=sweep iteration=0 crossings_bits=0000000000000000 layers={normalized}",
            f"RUST_DAGRE_LAYERS phase=best iteration=-1 crossings_bits=0000000000000000 layers={normalized}",
        ]
    )
    return lines


class DagreOrderTraceTests(unittest.TestCase):
    def test_go_id_maps_are_scoped_across_nested_layout_invocations(self):
        stderr = "\n".join(go_group("first.path", ["0"]) + go_group("second.path", ["0"]))
        groups = gate.parse_go_trace(stderr)
        self.assertEqual(len(groups), 2)
        self.assertEqual(groups[0]["objects"], {"0": "first.path"})
        self.assertEqual(groups[1]["objects"], {"0": "second.path"})

    def test_rust_groups_match_go_sweeps_and_stable_object_paths(self):
        go_groups = gate.parse_go_trace(
            "\n".join(go_group("first.path", ["0"]) + go_group("second.path", ["0"]))
        )
        rust_trace = "\n".join(rust_group(["first.path"]) + rust_group(["second.path"]))
        rust_events, rust_maps, rust_ids = gate.parse_rust_trace(rust_trace)
        result = gate.compare_trace_groups(go_groups, rust_events, rust_maps, rust_ids)
        self.assertEqual(result["status"], "matched")
        self.assertEqual(result["go_order_groups"], 2)

    def test_group_multiplicity_and_identity_mismatches_are_not_hidden(self):
        go_groups = gate.parse_go_trace("\n".join(go_group("expected.path", ["0"])))
        rust_trace = "\n".join(rust_group(["other.path"]))
        rust_events, rust_maps, rust_ids = gate.parse_rust_trace(rust_trace)
        result = gate.compare_trace_groups(go_groups, rust_events, rust_maps, rust_ids)
        self.assertEqual(result["status"], "divergent")
        self.assertTrue(result["trace_multiplicity_matches"])
        self.assertFalse(result["group_order_matches"])
        self.assertEqual(
            [group["missing_engine"] for group in result["groups"]],
            ["rust", "go"],
        )
        self.assertEqual(
            gate.compare_trace_groups([], [], [], []) ["status"], "inapplicable"
        )

    def test_exact_scopes_align_around_an_extra_enclosing_rust_pass(self):
        go_groups = gate.parse_go_trace(
            "\n".join(go_group("leaf.path", ["0"]))
        )
        rust_trace = "\n".join(rust_group(["root.path", "leaf.path"]) + rust_group(["leaf.path"]))
        rust_events, rust_maps, rust_ids = gate.parse_rust_trace(rust_trace)
        result = gate.compare_trace_groups(go_groups, rust_events, rust_maps, rust_ids)
        self.assertEqual(result["status"], "divergent")
        self.assertFalse(result["group_order_matches"])
        self.assertTrue(result["groups"][0]["matched"])
        self.assertEqual(result["groups"][0]["rust_group"], 1)
        self.assertEqual(result["groups"][1]["missing_engine"], "go")

    def test_invocation_order_remains_part_of_trace_parity(self):
        go_groups = gate.parse_go_trace(
            "\n".join(go_group("first.path", ["0"]) + go_group("second.path", ["0"]))
        )
        rust_trace = "\n".join(rust_group(["second.path"]) + rust_group(["first.path"]))
        rust_events, rust_maps, rust_ids = gate.parse_rust_trace(rust_trace)
        result = gate.compare_trace_groups(go_groups, rust_events, rust_maps, rust_ids)
        self.assertEqual(result["status"], "divergent")
        self.assertTrue(all(group["matched"] for group in result["groups"]))
        self.assertFalse(result["group_order_matches"])

    def test_no_order_events_is_a_valid_empty_rust_trace(self):
        self.assertEqual(gate.parse_rust_trace(""), ([], [], []))


if __name__ == "__main__":
    unittest.main()
