"""Contracts for the exact-bit Dagre boundary gate."""

import contextlib
import importlib.util
import io
import json
import math
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("dagre_stages", Path(__file__).with_name("check-dagre-stages.py"))
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


def object_line(engine, identity="a", values=(0.0, 1.0, 2.0, 3.0)):
    bits = ",".join(f"{struct.unpack('>Q', struct.pack('>d', value))[0]:016x}" for value in values)
    parent = ' parent="parent with space"' if engine == "go" else ""
    return f"{engine.upper()}_OBJ {json.dumps(identity)} " + " ".join(map(str, values)) + parent + f" bits={bits}\n"


def trace(engine, identity="a", values=(0.0, 1.0, 2.0, 3.0), *, include_final=False):
    column = 0 if engine == "rust" else 1
    return "".join(f"{engine.upper()}_PHASE {pair[column]}\n" + object_line(engine, identity, values) for pair in gate.phase_pairs(include_final))


def edge_line(engine, identity="1:a -> 1:b #0", points=((0.0, 1.0), (2.0, 3.0))):
    bits = ";".join(",".join(f"{struct.unpack('>Q', struct.pack('>d', value))[0]:016x}" for value in point) for point in points)
    return f"{engine.upper()}_EDGE {json.dumps(identity)} point_count={len(points)} bits={bits}\n"


def route_trace(engine, edges=(("1:a -> 1:b #0", ((0.0, 1.0), (2.0, 3.0))),), *, include_final=False):
    column = 0 if engine == "rust" else 1
    return "".join(f"{engine.upper()}_PHASE {pair[column]}\n" + object_line(engine)
                   + f"{engine.upper()}_EDGESET count={len(edges)}\n"
                   + "".join(edge_line(engine, identity, points) for identity, points in edges)
                   for pair in gate.phase_pairs(include_final))


class DagreStageTests(unittest.TestCase):
    def route_difference(self, rust_text, go_text, *, include_final=False):
        return gate.first_difference(gate.parse_trace(rust_text, "rust", include_final=include_final),
                                     gate.parse_trace(go_text, "go", include_final=include_final),
                                     rust_routes=gate.parse_route_trace(rust_text, "rust", include_final=include_final),
                                     go_routes=gate.parse_route_trace(go_text, "go", include_final=include_final),
                                     include_final=include_final)

    def test_matching_snapshots_with_quoted_and_unicode_identity(self):
        identity = 'a."spaced \\ key".\N{SNOWMAN}'
        rust = gate.parse_trace(trace("rust", identity), "rust")
        go = gate.parse_trace(trace("go", identity), "go")
        self.assertIsNone(gate.first_difference(rust, go))
        self.assertEqual(list(rust["rank_nodes"]), [identity])

    def test_every_bit_matters_including_negative_zero(self):
        for changed in (-0.0, math.nextafter(0.0, 1.0)):
            with self.subTest(changed=changed):
                rust = gate.parse_trace(trace("rust", values=(changed, 1.0, 2.0, 3.0)), "rust")
                go = gate.parse_trace(trace("go"), "go")
                difference = gate.first_difference(rust, go)
                self.assertEqual(difference["kind"], "coordinate_bits")
                self.assertEqual(difference["rust_phase"], "rank_nodes")
                self.assertEqual(difference["field"], "x")
                self.assertNotEqual(difference["rust_bits"], difference["go_bits"])

    def test_object_membership_is_not_filtered(self):
        rust = gate.parse_trace(trace("rust", "extra"), "rust")
        go = gate.parse_trace(trace("go", "missing"), "go")
        difference = gate.first_difference(rust, go)
        self.assertEqual(difference["missing_in_rust"], ["missing"])
        self.assertEqual(difference["extra_in_rust"], ["extra"])

    def test_extracted_scope_identity_prefix_is_explicit_and_collision_checked(self):
        rust = gate.parse_trace(trace("rust", "b.AA"), "rust")
        go = gate.parse_trace(trace("go", "AA"), "go")
        normalized = gate.strip_identity_prefix(rust, "b.")
        self.assertIsNone(gate.first_difference(normalized, go))
        rust["fit"]["AA"] = rust["fit"]["b.AA"]
        with self.assertRaisesRegex(gate.TraceError, "collapses multiple objects"):
            gate.strip_identity_prefix(rust, "b.")

    def test_extracted_scope_prefix_rebuilds_utf8_route_identity_lengths(self):
        original = "11:scope.a.\N{SNOWMAN} -> 7:scope.b #0"
        routes = {"rank_nodes": [{"identity": original, "points": [(0, 0)]}]}
        normalized = gate.strip_edge_identity_prefix(routes, "scope.")
        self.assertEqual(normalized["rank_nodes"][0]["identity"], "5:a.\N{SNOWMAN} -> 1:b #0")
        self.assertEqual(normalized["rank_nodes"][0]["points"], [(0, 0)])

    def test_extracted_scope_route_prefix_rejects_collisions_and_bad_lengths(self):
        collisions = {
            "rank_nodes": [
                {"identity": "7:scope.a -> 7:scope.b #0", "points": []},
                {"identity": "1:a -> 1:b #0", "points": []},
            ]
        }
        with self.assertRaisesRegex(gate.TraceError, "collapses multiple route identities"):
            gate.strip_edge_identity_prefix(collisions, "scope.")
        malformed = {"rank_nodes": [{"identity": "2:a -> 1:b #0", "points": []}]}
        with self.assertRaisesRegex(gate.TraceError, "malformed canonical edge identity"):
            gate.strip_edge_identity_prefix(malformed, "scope.")

    def test_missing_duplicate_empty_and_reordered_phases_fail(self):
        original = trace("rust")
        first = "RUST_PHASE rank_nodes\n" + object_line("rust")
        second = "RUST_PHASE rank_spacing\n" + object_line("rust")
        cases = [
            (original.replace(first, ""), "missing required phases"),
            (original + first, "duplicate required phase"),
            (original.replace(first, "RUST_PHASE rank_nodes\n"), "empty required snapshots"),
            (original.replace(first + second, second + first), "out of order"),
        ]
        for malformed, expected in cases:
            with self.subTest(expected=expected), self.assertRaisesRegex(gate.TraceError, expected):
                gate.parse_trace(malformed, "rust")

    def test_duplicate_and_unscoped_objects_fail(self):
        with self.assertRaisesRegex(gate.TraceError, "duplicate object"):
            gate.parse_trace(trace("rust") + object_line("rust"), "rust")
        with self.assertRaisesRegex(gate.TraceError, "before its phase"):
            gate.parse_trace(object_line("rust") + trace("rust"), "rust")

    def test_malformed_quotes_bits_and_decimal_mismatch_fail(self):
        original = trace("rust")
        cases = [
            (original.replace('"a"', r'"a\u{a0}"', 1), "invalid JSON quoted identity"),
            (original.replace("bits=0000000000000000", "bits=0", 1), "16-digit IEEE-754"),
            (original.replace('"a" 0.0', '"a" 0.25', 1), "disagrees"),
        ]
        for malformed, expected in cases:
            with self.subTest(expected=expected), self.assertRaisesRegex(gate.TraceError, expected):
                gate.parse_trace(malformed, "rust")

    def test_additional_named_stages_do_not_satisfy_required_stages(self):
        extra = "RUST_PHASE before_bk\n" + object_line("rust", "unrelated")
        parsed = gate.parse_trace(extra + trace("rust"), "rust")
        self.assertEqual(list(parsed), [rp for rp, _ in gate.PHASES])
        with self.assertRaisesRegex(gate.TraceError, "missing required phases"):
            gate.parse_trace(extra, "rust")

    def test_first_difference_uses_phase_order(self):
        rust = gate.parse_trace(trace("rust"), "rust")
        go = gate.parse_trace(trace("go"), "go")
        rust["source_cross"]["a"]["bits"][3] += 1
        rust["fit"]["a"]["bits"][0] += 1
        difference = gate.first_difference(rust, go)
        self.assertEqual((difference["rust_phase"], difference["field"]), ("source_cross", "height"))

    def test_timeout_and_failed_process_are_not_inapplicable(self):
        with patch.object(gate.subprocess, "run", side_effect=subprocess.TimeoutExpired("binary", 1)):
            metadata, _ = gate.run(Path("binary"), Path("fixture"), Path("out"), "rust", 1)
        self.assertIn("failure", metadata)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = root / "input.d2"
            fixture.write_text("a")
            with patch.object(gate, "run", return_value=({"exit_code": 1, "failure": "not implemented"}, "")):
                result = gate.compare_fixture(Path("rust"), Path("go"), fixture, root, 1)
            self.assertEqual(result["status"], "failed")

    def test_nested_go_and_rust_passes_can_be_selected_symmetrically(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = root / "input.d2"
            fixture.write_text("a")
            rust_text = trace("rust", "scope.a") + trace("rust", "scope.b")
            go_text = trace("go", "a") + trace("go", "b")
            captures = [({"exit_code": 0}, rust_text), ({"exit_code": 0}, go_text)]
            with patch.object(gate, "run", side_effect=captures):
                result = gate.compare_fixture(Path("rust"), Path("go"), fixture, root, 1,
                                               rust_pass=1, go_pass=1, rust_id_prefix="scope.")
            self.assertEqual(result["status"], "matched")
            self.assertEqual((result["rust_pass_count"], result["rust_pass_selected"]), (2, 1))
            self.assertEqual((result["go_pass_count"], result["go_pass_selected"]), (2, 1))

    def test_nested_pass_selection_rejects_out_of_range_go_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = root / "input.d2"
            fixture.write_text("a")
            captures = [({"exit_code": 0}, trace("rust")),
                        ({"exit_code": 0}, trace("go") + trace("go"))]
            with patch.object(gate, "run", side_effect=captures):
                result = gate.compare_fixture(Path("rust"), Path("go"), fixture, root, 1, go_pass=2)
            self.assertEqual(result["status"], "failed")
            self.assertIn("outside 0..2", result["reason"])

    def test_raw_rust_rank_boundary_maps_to_go_dagro(self):
        rust_text = trace("rust").replace("RUST_PHASE rank_nodes\n", "RUST_PHASE raw_rank_nodes\n", 1)
        rust = gate.parse_trace(rust_text, "rust", rust_rank_phase="raw_rank_nodes")
        go = gate.parse_trace(trace("go"), "go", rust_rank_phase="raw_rank_nodes")
        self.assertIsNone(gate.first_difference(rust, go, rust_rank_phase="raw_rank_nodes"))
        self.assertEqual(gate.phase_pairs(rust_rank_phase="raw_rank_nodes")[0],
                         ("raw_rank_nodes", "dagro"))

    def test_raw_rust_boundary_splits_each_nested_invocation(self):
        rust_text = trace("rust").replace("RUST_PHASE rank_nodes\n", "RUST_PHASE raw_rank_nodes\n", 1)
        rust_text += trace("rust").replace("RUST_PHASE rank_nodes\n", "RUST_PHASE raw_rank_nodes\n", 1)
        self.assertEqual(len(gate.split_pass_traces(rust_text, "rust", rust_rank_phase="raw_rank_nodes")), 2)

    def test_report_has_hashes_scope_and_failing_exit(self):
        for rust_trace, expected_exit, expected_status in [(trace("rust"), 0, "matched"),
                                                          (trace("rust", "different"), 1, "divergent"),
                                                          ("", 1, "failed")]:
            with self.subTest(expected=expected_status), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root / "tests/d2").mkdir(parents=True)
                (root / "tests/d2/input.d2").write_text("a")
                for name in ("rust", "go"):
                    (root / name).write_bytes(name.encode())
                output = root / "report.json"
                argv = ["check-dagre-stages.py", "input", "--rust", str(root / "rust"), "--go", str(root / "go"),
                        "--output", str(output), "--summary"]
                captures = [({"exit_code": 0}, rust_trace), ({"exit_code": 0}, trace("go"))]
                with patch.object(gate, "ROOT", root), patch.object(gate, "run", side_effect=captures), \
                     patch("sys.argv", argv), contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(gate.main(), expected_exit)
                report = json.loads(output.read_text())
                self.assertEqual(report["scope"], gate.SCOPE)
                self.assertEqual(report["results"][0]["status"], expected_status)
                self.assertEqual(report["results"][0]["fixture_sha256"], gate.digest(root / "tests/d2/input.d2"))
                self.assertEqual(report["binaries"]["rust"]["sha256"], gate.digest(root / "rust"))

    def test_route_identities_are_opaque_and_json_quoted(self):
        edges = [('24:a." -> escaped\\key" -> 3:\N{SNOWMAN} #0', ((0.0, 1.0), (2.0, 3.0))),
                 ('24:a." -> escaped\\key" -> 3:\N{SNOWMAN} #1', ())]
        self.assertIsNone(self.route_difference(route_trace("rust", edges), route_trace("go", edges)))
        parsed = gate.parse_route_trace(route_trace("rust", edges), "rust")
        self.assertEqual([edge["identity"] for edge in parsed["rank_nodes"]], [edge[0] for edge in edges])
        self.assertEqual(parsed["rank_nodes"][1]["points"], [])

    def test_edgeless_route_snapshots_require_explicit_zero_count(self):
        self.assertIsNone(self.route_difference(route_trace("rust", ()), route_trace("go", ())))
        for text in (trace("rust"), route_trace("rust", ()).replace("RUST_EDGESET count=0\n", "", 1)):
            with self.subTest(text=text), self.assertRaisesRegex(gate.TraceError, "missing EDGESET"):
                gate.parse_route_trace(text, "rust")

    def test_route_markers_counts_identities_and_bit_payloads_are_strict(self):
        original = route_trace("rust")
        cases = [
            (original.replace("RUST_EDGESET count=1", "RUST_EDGESET count=1\nRUST_EDGESET count=1", 1), "duplicate EDGESET"),
            (original.replace("RUST_EDGESET count=1", "RUST_EDGESET count=2", 1), "EDGESET count=2 disagrees"),
            (original.replace("RUST_EDGESET count=1", "RUST_EDGESET count=-1", 1), "expected EDGESET count=N"),
            (original.replace("point_count=2", "point_count=3", 1), "point_count=3 disagrees"),
            (original.replace(edge_line("rust"), edge_line("rust").replace("bits=0000000000000000", "bits=0", 1), 1), "two 16-digit"),
            (original.replace('"1:a -> 1:b #0"', r'"bad\x20escape"', 1), "invalid JSON quoted identity"),
            (original.replace('"1:a -> 1:b #0"', '""', 1), "empty canonical edge identity"),
            (original + edge_line("rust"), "duplicate edge identity"),
            (edge_line("rust") + original, "before its phase"),
            (original.replace("RUST_EDGESET count=1", "RUST_EDGESET", 1), "expected EDGESET count=N"),
            (original.replace("RUST_EDGE ", "RUST_EDGE", 1), "malformed EDGE record"),
        ]
        for malformed, message in cases:
            with self.subTest(message=message), self.assertRaisesRegex(gate.TraceError, message):
                gate.parse_route_trace(malformed, "rust")

    def test_route_phase_validation_cannot_hide_missing_or_duplicate_snapshots(self):
        original = route_trace("rust")
        first = "RUST_PHASE rank_nodes\n" + object_line("rust") + "RUST_EDGESET count=1\n" + edge_line("rust")
        second = first.replace("rank_nodes", "rank_spacing", 1)
        cases = [(original.replace(first, ""), "missing required route phases"),
                 (original + first, "duplicate required phase"),
                 (original.replace(first + second, second + first), "out of order")]
        for malformed, message in cases:
            with self.subTest(message=message), self.assertRaisesRegex(gate.TraceError, message):
                gate.parse_route_trace(malformed, "rust")

    def test_edge_membership_and_order_are_both_compared(self):
        edges = (("1:a -> 1:b #0", ((0.0, 1.0),)), ("1:a -> 1:b #1", ((0.0, 1.0),)))
        difference = self.route_difference(route_trace("rust", edges[::-1]), route_trace("go", edges))
        self.assertEqual((difference["kind"], difference["edge_index"]), ("edge_order", 0))
        difference = self.route_difference(route_trace("rust", edges[:1]), route_trace("go", edges))
        self.assertEqual(difference["kind"], "edge_membership")
        self.assertEqual(difference["missing_in_rust"], ["1:a -> 1:b #1"])

    def test_route_point_count_order_and_exact_signed_zero_bits_are_compared(self):
        go_edges = (("1:a -> 1:b #0", ((0.0, 1.0), (2.0, 3.0))),)
        cases = [(((0.0, 1.0),), "route_point_count"),
                 (((2.0, 3.0), (0.0, 1.0)), "route_coordinate_bits"),
                 (((-0.0, 1.0), (2.0, 3.0)), "route_coordinate_bits")]
        for points, kind in cases:
            with self.subTest(points=points):
                difference = self.route_difference(route_trace("rust", ((go_edges[0][0], points),)), route_trace("go", go_edges))
                self.assertEqual(difference["kind"], kind)
                self.assertEqual(difference["rust_phase"], "rank_nodes")
                if kind == "route_coordinate_bits":
                    self.assertEqual((difference["point_index"], difference["field"]), (0, "x"))

    def test_earlier_route_divergence_precedes_later_object_divergence(self):
        rust_text, go_text = route_trace("rust"), route_trace("go")
        rust = gate.parse_trace(rust_text, "rust")
        rust["fit"]["a"]["bits"][0] += 1
        rust_routes = gate.parse_route_trace(rust_text, "rust")
        rust_routes["rank_spacing"][0]["points"][0] = (1, 2)
        difference = gate.first_difference(rust, gate.parse_trace(go_text, "go"), rust_routes=rust_routes,
                                           go_routes=gate.parse_route_trace(go_text, "go"))
        self.assertEqual((difference["rust_phase"], difference["kind"]), ("rank_spacing", "route_coordinate_bits"))

    def test_routes_are_optional_but_the_flag_requires_evidence_in_the_report(self):
        for require_routes, include_routes, expected in [(False, False, "matched"), (True, False, "failed"), (True, True, "matched")]:
            with self.subTest(require_routes=require_routes, include_routes=include_routes), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root / "tests/d2").mkdir(parents=True)
                (root / "tests/d2/input.d2").write_text("a")
                for name in ("rust", "go"):
                    (root / name).write_bytes(name.encode())
                output = root / "report.json"
                argv = ["check-dagre-stages.py", "input", "--rust", str(root / "rust"), "--go", str(root / "go"),
                        "--output", str(output), "--summary"] + (["--routes"] if require_routes else [])
                make_trace = route_trace if include_routes else trace
                captures = [({"exit_code": 0}, make_trace("rust")), ({"exit_code": 0}, make_trace("go"))]
                with patch.object(gate, "ROOT", root), patch.object(gate, "run", side_effect=captures), \
                     patch("sys.argv", argv), contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(gate.main(), int(expected != "matched"))
                report = json.loads(output.read_text())
                self.assertEqual(report["routes_checked"], require_routes)
                self.assertEqual(report["results"][0]["status"], expected)
                if require_routes and include_routes:
                    stage = report["results"][0]["stages"][0]
                    self.assertEqual((stage["rust_edges"], stage["rust_route_points"]), (1, 2))

    def test_final_requires_five_complete_ordered_phases_without_mutating_defaults(self):
        text = route_trace("rust", include_final=True)
        for parse in (gate.parse_trace, gate.parse_route_trace):
            with self.subTest(parser=parse.__name__):
                self.assertEqual(len(parse(text, "rust", include_final=True)), 5)
                self.assertEqual(len(parse(text, "rust")), 4)
                with self.assertRaisesRegex(gate.TraceError, "missing required.*phases"):
                    parse(route_trace("rust"), "rust", include_final=True)
                final = "RUST_PHASE final\n" + object_line("rust") + "RUST_EDGESET count=1\n" + edge_line("rust")
                with self.assertRaisesRegex(gate.TraceError, "duplicate required phase"):
                    parse(text + final, "rust", include_final=True)
                with self.assertRaisesRegex(gate.TraceError, "out of order"):
                    parse(final + text.replace(final, ""), "rust", include_final=True)
        self.assertEqual(len(gate.PHASES), 4)
        self.assertEqual(gate.phase_pairs(), gate.PHASES)

    def test_final_edgeless_stage_requires_its_own_marker(self):
        text = route_trace("rust", (), include_final=True)
        final = "RUST_PHASE final\n" + object_line("rust") + "RUST_EDGESET count=0\n"
        incomplete = text.replace(final, "RUST_PHASE final\n" + object_line("rust"))
        self.assertEqual(len(gate.parse_route_trace(text, "rust", include_final=True)), 5)
        with self.assertRaisesRegex(gate.TraceError, "missing EDGESET marker in phase 'final'"):
            gate.parse_route_trace(incomplete, "rust", include_final=True)
        self.assertEqual(len(gate.parse_route_trace(incomplete, "rust")), 4)

    def test_final_route_and_object_differences_are_reported_at_final_only(self):
        rust_text, go_text = route_trace("rust", include_final=True), route_trace("go", include_final=True)
        self.assertIsNone(self.route_difference(rust_text, go_text, include_final=True))
        final_edge = "RUST_PHASE final\n" + object_line("rust") + "RUST_EDGESET count=1\n" + edge_line("rust")
        changed = final_edge.replace(edge_line("rust"), edge_line("rust", points=((-0.0, 1.0), (2.0, 3.0))))
        changed_text = rust_text.replace(final_edge, changed)
        difference = self.route_difference(changed_text, go_text, include_final=True)
        self.assertEqual((difference["rust_phase"], difference["kind"]), ("final", "route_coordinate_bits"))
        self.assertIsNone(self.route_difference(changed_text, go_text))
        rust_objects = gate.parse_trace(rust_text, "rust", include_final=True)
        rust_objects["final"]["a"]["bits"][2] += 1
        difference = gate.first_difference(rust_objects, gate.parse_trace(go_text, "go", include_final=True), include_final=True)
        self.assertEqual((difference["rust_phase"], difference["kind"]), ("final", "coordinate_bits"))

    def test_final_cli_flag_reports_its_scope_and_enforces_missing_final(self):
        for include_evidence in (False, True):
            with self.subTest(include_evidence=include_evidence), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root / "tests/d2").mkdir(parents=True)
                (root / "tests/d2/input.d2").write_text("a")
                for name in ("rust", "go"):
                    (root / name).write_bytes(name.encode())
                output = root / "report.json"
                argv = ["check-dagre-stages.py", "input", "--rust", str(root / "rust"), "--go", str(root / "go"),
                        "--output", str(output), "--summary", "--routes", "--final"]
                captures = [({"exit_code": 0}, route_trace(engine, include_final=include_evidence)) for engine in ("rust", "go")]
                with patch.object(gate, "ROOT", root), patch.object(gate, "run", side_effect=captures), \
                     patch("sys.argv", argv), contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(gate.main(), int(not include_evidence))
                report = json.loads(output.read_text())
                self.assertTrue(report["routes_checked"])
                self.assertTrue(report["final_checked"])
                self.assertEqual(report["scope"], "five boundary snapshots, not operation-level trace parity")
                self.assertEqual(report["phase_mapping"][-1], {"go": "final", "rust": "final"})
                self.assertEqual(report["results"][0]["status"], "matched" if include_evidence else "failed")
                if include_evidence:
                    self.assertEqual(len(report["results"][0]["stages"]), 5)


if __name__ == "__main__":
    unittest.main()
