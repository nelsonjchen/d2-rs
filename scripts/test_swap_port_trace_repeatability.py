"""Tests for exact port-trace repeatability comparison helpers."""

import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "swap_port_repeatability",
    Path(__file__).with_name("audit-swap-port-trace-repeatability.py"),
)
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class SwapPortTraceRepeatabilityTests(unittest.TestCase):
    def test_difference_path_identifies_the_first_ordered_event(self):
        left = [{"event": "swap-port-node", "node": "first"}]
        right = [{"event": "swap-port-node", "node": "second"}]
        self.assertEqual(audit.first_difference_path(left, right), "$[0].node")

    def test_side_schedule_retains_source_iteration_order(self):
        events = [
            {"event": "swap-port-side-order", "phase": "before-sort", "node": "n", "side": "Right", "edges": ["a"]},
            {"event": "swap-port-side-order", "phase": "after-sort", "node": "n", "side": "Right", "edges": ["a"]},
            {"event": "swap-port-side-order", "phase": "before-sort", "node": "n", "side": "Left", "edges": ["b"]},
        ]
        self.assertEqual(
            audit.side_schedule(events),
            [
                {"node": "n", "side": "Right", "edges": ["a"]},
                {"node": "n", "side": "Left", "edges": ["b"]},
            ],
        )


if __name__ == "__main__":
    unittest.main()
