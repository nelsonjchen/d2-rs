"""Keep the elk-go P3 schedule reconstruction distinct from final snapshots."""

import importlib.util
from pathlib import Path
import unittest


spec = importlib.util.spec_from_file_location(
    "audit_elk_p3_hierarchy_trace",
    Path(__file__).with_name("audit-elk-p3-hierarchy-trace.py"),
)
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class ElkP3HierarchyTraceTests(unittest.TestCase):
    def test_repeated_final_scope_snapshots_do_not_duplicate_sweeps(self):
        scopes = [
            {"graph_index": 0, "dont_sweep_into": True},
            {"graph_index": 1, "dont_sweep_into": True},
            {"graph_index": 1, "dont_sweep_into": True},
            {"graph_index": 0, "dont_sweep_into": True},
        ]

        self.assertEqual(audit.reconstructed_bottom_up_queue(scopes), [1, 0])

    def test_only_bottom_up_holders_are_scheduled_in_reverse_discovery_order(self):
        scopes = [
            {"graph_index": 0, "dont_sweep_into": True},
            {"graph_index": 1, "dont_sweep_into": False},
            {"graph_index": 2, "dont_sweep_into": True},
        ]

        self.assertEqual(audit.reconstructed_bottom_up_queue(scopes), [2, 0])

    def test_component_runs_keep_repeated_local_roots_and_source_membership(self):
        go_trace = [
            {"stage": "p3-config"},
            {
                "stage": "p3-graph-scope",
                "graph_index": 0,
                "arena_graph": 1,
                "layers": [{"nodes": [0]}, {"nodes": [1]}],
            },
            {"stage": "p3-config"},
            {
                "stage": "p3-graph-scope",
                "graph_index": 0,
                "arena_graph": 3,
                "layers": [{"nodes": [2]}, {"nodes": [3]}],
            },
        ]
        rust_trace = [
            {"operation": "p3_config"},
            {
                "operation": "barycenter_scope_start",
                "value": "layers=[a];[b];port_orders=a:[0];b:[1]",
            },
            {"operation": "p3_config"},
            {
                "operation": "barycenter_scope_start",
                "value": "layers=[c];[d];port_orders=c:[2];d:[3]",
            },
        ]

        go_scopes, rust_scopes = audit.component_run_scope_signatures(
            go_trace,
            rust_trace,
            {"children": [{"id": name} for name in "abcd"]},
        )

        self.assertEqual(go_scopes, [[['a'], ['b']], [['c'], ['d']]])
        self.assertEqual(go_scopes, rust_scopes)


if __name__ == "__main__":
    unittest.main()
