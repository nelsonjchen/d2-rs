import unittest
import importlib.util
from pathlib import Path

TRACE_CHECKER = Path(__file__).resolve().with_name("check-emitted-trace-parity.py")
SPEC = importlib.util.spec_from_file_location("trace_checker", TRACE_CHECKER)
assert SPEC is not None and SPEC.loader is not None
TRACE_CHECKER_MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(TRACE_CHECKER_MODULE)
difference_profile = TRACE_CHECKER_MODULE.difference_profile
comparable_event = TRACE_CHECKER_MODULE.comparable_event
first_difference = TRACE_CHECKER_MODULE.first_difference
classify_event_family = TRACE_CHECKER_MODULE.classify_event_family

CAPTURE_SCRIPT = Path(__file__).resolve().with_name("capture-layout-trace.py")
CAPTURE_SPEC = importlib.util.spec_from_file_location("trace_capture", CAPTURE_SCRIPT)
assert CAPTURE_SPEC is not None and CAPTURE_SPEC.loader is not None
TRACE_CAPTURE_MODULE = importlib.util.module_from_spec(CAPTURE_SPEC)
CAPTURE_SPEC.loader.exec_module(TRACE_CAPTURE_MODULE)

REPEATABILITY_SCRIPT = Path(__file__).resolve().with_name("check-trace-repeatability.py")
REPEATABILITY_SPEC = importlib.util.spec_from_file_location("trace_repeatability", REPEATABILITY_SCRIPT)
assert REPEATABILITY_SPEC is not None and REPEATABILITY_SPEC.loader is not None
TRACE_REPEATABILITY_MODULE = importlib.util.module_from_spec(REPEATABILITY_SPEC)
REPEATABILITY_SPEC.loader.exec_module(TRACE_REPEATABILITY_MODULE)


class DifferenceProfileTests(unittest.TestCase):
    def test_snapshot_record_order_is_normalized_by_stable_identity(self):
        left = {
            "event": "placement",
            "phase": "after-fit",
            "nodes": [{"id": "b", "x": 2}, {"id": "a", "x": 1}],
        }
        right = {
            "event": "placement",
            "phase": "after-fit",
            "nodes": [{"id": "a", "x": 1}, {"id": "b", "x": 2}],
        }

        self.assertEqual(comparable_event(left, "placement"), comparable_event(right, "placement"))

    def test_duplicate_local_labels_pair_by_stable_numeric_identity(self):
        left = {
            "event": "placement",
            "nodes": [
                {"id": "same-local-name", "stable_id": "20", "x": 2},
                {"id": "same-local-name", "stable_id": "10", "x": 1},
            ],
        }
        right = {
            "event": "placement",
            "nodes": [
                {"id": "same-local-name", "stable_id": "10", "x": 1},
                {"id": "same-local-name", "stable_id": "20", "x": 2},
            ],
        }

        self.assertEqual(comparable_event(left, "placement"), comparable_event(right, "placement"))

    def test_missing_snapshot_identity_is_reported_before_geometry(self):
        left = comparable_event(
            {"event": "placement", "nodes": [{"id": "a", "stable_id": "10", "h": 1}]},
            "placement",
        )
        right = comparable_event(
            {"event": "placement", "nodes": [{"id": "b", "stable_id": "20", "h": 2}]},
            "placement",
        )

        self.assertEqual(first_difference([left], [right]), "$[0].nodes[0].stable_id: '10' != '20'")

    def test_snapshot_event_order_is_still_strict(self):
        first = comparable_event({"event": "stage", "stage": "a", "nodes": []}, "stages")
        second = comparable_event({"event": "stage", "stage": "b", "nodes": []}, "stages")

        self.assertIsNotNone(first_difference([first, second], [second, first]))

    def test_order_only_connected_difference_keeps_membership_visible(self):
        left = [{"event": "gap-connectivity", "ordered_connected": ["a", "b", "c"]}]
        right = [{"event": "gap-connectivity", "ordered_connected": ["b", "a", "c"]}]

        self.assertEqual(
            difference_profile(left, right),
            {
                "different_event_count": 1,
                "event_count_delta": 0,
                "changed_top_level_fields": {"ordered_connected": 1},
                "ordered_connected_only_event_count": 1,
                "ordered_connected_same_members_event_count": 1,
            },
        )
        self.assertIsNotNone(TRACE_CHECKER_MODULE.first_difference(left, right))

    def test_geometry_change_is_not_classified_as_order_only(self):
        left = [{"event": "gap-connectivity", "ordered_connected": ["a", "b"], "x": 1}]
        right = [{"event": "gap-connectivity", "ordered_connected": ["b", "a"], "x": 2}]

        profile = difference_profile(left, right)
        self.assertEqual(profile["changed_top_level_fields"], {"ordered_connected": 1, "x": 1})
        self.assertEqual(profile["ordered_connected_only_event_count"], 0)
        self.assertEqual(profile["ordered_connected_same_members_event_count"], 0)

    def test_connected_traversal_records_normalize_to_stable_integer_ids(self):
        go = TRACE_CAPTURE_MODULE.parse_connected_go_trace(
            'CONNECTED_STEP_GO start=10:root current=11:child excluded=["14:skip"] '
            'enqueued=["12:next" "13:other"] clusters=[30 31] containers=[20 21]'
        )
        rust = TRACE_CAPTURE_MODULE.parse_connected_rust_trace(
            "CONNECTED_STEP_RUST start=10 current=11 excluded=[14] "
            "enqueued=[12, 13] clusters=[30, 31] containers=[20, 21]"
        )

        self.assertEqual(go, rust)

    def test_event_count_difference_is_retained(self):
        profile = difference_profile([{"event": "a"}, {"event": "b"}], [{"event": "a"}])

        self.assertEqual(profile["different_event_count"], 1)
        self.assertEqual(profile["event_count_delta"], 1)
        self.assertEqual(profile["changed_top_level_fields"], {"$event": 1})

    def test_empty_event_family_is_unexercised_not_a_match(self):
        self.assertEqual(classify_event_family([], []), ("unexercised", None))
        self.assertEqual(classify_event_family([], [{"event": "step"}])[0], "divergent")

    def test_repeatability_keeps_map_order_and_finds_first_difference(self):
        left = [{"containers": [11, 12]}]
        right = [{"containers": [12, 11]}]

        self.assertEqual(
            TRACE_REPEATABILITY_MODULE.first_difference_path(left, right),
            "$[0].containers[0]",
        )
        self.assertEqual(TRACE_REPEATABILITY_MODULE.difference_count(left, right), 1)


if __name__ == "__main__":
    unittest.main()
