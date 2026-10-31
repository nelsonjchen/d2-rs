"""Contracts for stable identities in the Dagre rank-operation trace gate."""

import importlib.util
from pathlib import Path
import unittest


spec = importlib.util.spec_from_file_location(
    "compare_dagre_rank_ops", Path(__file__).with_name("compare-dagre-rank-ops.py")
)
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class DagreRankOperationTraceTests(unittest.TestCase):
    def test_empty_traces_are_inapplicable_only_when_both_engines_succeed(self):
        self.assertEqual(gate.comparison_status(0, 0, [], [], None), "inapplicable")
        self.assertEqual(gate.comparison_status(0, 0, [{"phase": "x"}], [], {}), "divergent")
        self.assertEqual(gate.comparison_status(0, 1, [], [], None), "failed")

    def test_go_mapper_records_decode_json_and_keep_stable_paths(self):
        fixture = Path("fixture.d2")
        stderr = (
            'GO_MAP id="1" object="outer.child"\n'
            'GO_MAP id="2" object="quoted.\\\"id"\n'
            'GO_RANK_OP {"phase":"input"}\n'
        )
        self.assertEqual(
            gate.parse_go_object_map(stderr, fixture),
            {"1": "outer.child", "2": 'quoted."id'},
        )

    def test_numeric_aliases_are_normalized_recursively(self):
        fixture = Path("fixture.d2")
        event = {
            "nodes": [{"id": "object:1", "parent": "nesting-root"}],
            "constraints": [{"source": "object:1", "target": "border:0"}],
            "tree": [{"source": "object:1", "target": "object:2"}],
            "leave": {"source": "object:2", "target": "object:1"},
            "candidate": ["object:2", "object:not-numeric"],
        }
        normalized = gate.normalize_go_event_ids(
            event, {"1": "z.parent", "2": "a.child"}, fixture
        )
        self.assertEqual(normalized["nodes"][0]["id"], "object:z.parent")
        self.assertEqual(normalized["nodes"][0]["parent"], "nesting-root")
        self.assertEqual(normalized["constraints"][0]["target"], "border:0")
        self.assertEqual(normalized["constraints"][0]["source"], "object:z.parent")
        self.assertEqual(
            normalized["tree"][0], {"source": "object:a.child", "target": "object:z.parent"}
        )
        self.assertEqual(
            normalized["leave"], {"source": "object:a.child", "target": "object:z.parent"}
        )
        self.assertEqual(normalized["candidate"], ["object:a.child", "object:not-numeric"])

    def test_go_mapper_ids_are_scoped_to_each_dagre_layout_call(self):
        stderr = (
            'GO_MAP id="0" object="first.scope"\n'
            'GO_RANK_OP {"phase":"first","nodes":[{"id":"object:0"}]}\n'
            'GO_MAP id="0" object="second.scope"\n'
            'GO_RANK_OP {"phase":"second","nodes":[{"id":"object:0"}]}\n'
        )
        events = gate.go_trace_events(stderr, Path("fixture.d2"))
        self.assertEqual(events[0]["nodes"][0]["id"], "object:first.scope")
        self.assertEqual(events[1]["nodes"][0]["id"], "object:second.scope")

    def test_malformed_duplicate_and_unmapped_object_records_fail(self):
        fixture = Path("fixture.d2")
        with self.assertRaisesRegex(ValueError, "malformed"):
            gate.parse_go_object_map('GO_MAP id="1" object=bad\n', fixture)
        with self.assertRaisesRegex(ValueError, "repeats mapper ID"):
            gate.parse_go_object_map(
                'GO_MAP id="1" object="a"\nGO_MAP id="1" object="b"\n', fixture
            )
        with self.assertRaisesRegex(ValueError, "unmapped Dagro object ID"):
            gate.normalize_go_event_ids(
                {"id": "object:9"}, {}, fixture
            )


if __name__ == "__main__":
    unittest.main()
