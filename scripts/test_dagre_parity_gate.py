"""Verify Dagre report failures cannot be counted as geometry parity."""

import contextlib
import base64
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("dagre_gate", Path(__file__).with_name("check-dagre-parity.py"))
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class DagreGateTests(unittest.TestCase):
    def test_diagnostic_normalization_preserves_meaning_and_order(self):
        go = "success: successfully compiled graph.d2 to /tmp/go-case/steps/1.svg in 2.5ms\nerr: failure one\nerr: failure two"
        rust = "success: successfully compiled graph.d2 to /tmp/rust-case/steps/1.svg in 125µs\nerr: failure one\nerr: failure two"
        normalized = gate.comparable_diagnostics(go, Path("/tmp/go-case.svg"))
        self.assertEqual(normalized, gate.comparable_diagnostics(rust, Path("/tmp/rust-case.svg")))
        self.assertNotEqual(normalized, gate.comparable_diagnostics(rust.replace("failure one", "different error"), Path("/tmp/rust-case.svg")))
        self.assertNotEqual(normalized, gate.comparable_diagnostics(rust.replace("failure one\nerr: failure two", "failure two\nerr: failure one"), Path("/tmp/rust-case.svg")))
        self.assertNotEqual(normalized, gate.comparable_diagnostics(rust.replace("steps/1.svg", "steps/2.svg"), Path("/tmp/rust-case.svg")))
        self.assertEqual(gate.comparable_diagnostics("err: 2.5ms /tmp/go-case.svg", Path("/tmp/go-case.svg")), "err: 2.5ms /tmp/go-case.svg")

    def test_execution_classification_and_report_exit(self):
        cases = [
            ((1, "not implemented", None), (0, "", None), "divergent", 1),
            ((0, "", None), (1, "unsupported by Dagre", None), "divergent", 1),
            ((1, "unsupported", None), (1, "unsupported by Dagre", None), "divergent", 1),
            ((1, "unsupported by Dagre", None), (1, "unsupported by Dagre", None), "inapplicable", 0),
            ((None, "", "timed out"), (0, "", None), "failed", 1),
            ((0, "", None), (None, "", "timed out"), "failed", 1),
            ((None, "", "could not execute"), (1, "unsupported by Dagre", None), "failed", 1),
        ]
        for rust_run, go_run, expected, exit_code in cases:
            with self.subTest(rust=rust_run, go=go_run), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root / "tests/d2").mkdir(parents=True)
                (root / "tests/d2/test.d2").write_text("a -> b")
                rust_binary = root / "rust"
                go_binary = root / "go"
                rust_binary.write_bytes(b"rust executable")
                go_binary.write_bytes(b"go executable")
                report_path = root / "report.json"
                args = ["check-dagre-parity.py", "test", "--rust", str(rust_binary), "--go", str(go_binary),
                        "--output", str(report_path), "--timeout", "7", "--summary"]
                output = io.StringIO()
                with patch.object(gate, "ROOT", root), patch.object(gate, "run", side_effect=[rust_run, go_run]) as run, \
                     patch("sys.argv", args), contextlib.redirect_stdout(output):
                    self.assertEqual(gate.main(), exit_code)
                report = json.loads(report_path.read_text())
                self.assertEqual(report["results"][0]["status"], expected)
                self.assertTrue(report["results"][0]["reason"])
                self.assertEqual(report["matched"], 0)
                self.assertEqual(report["rust_binary_sha256"], hashlib.sha256(b"rust executable").hexdigest())
                self.assertEqual(report["go_binary_sha256"], hashlib.sha256(b"go executable").hexdigest())
                self.assertEqual(report["timeout_seconds"], 7)
                self.assertEqual(run.call_args_list[0].args[-1], 7)
                self.assertEqual(len(output.getvalue().splitlines()), 1)

    def test_successful_exit_requires_readable_svg(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "tests/d2").mkdir(parents=True)
            (root / "tests/d2/test.d2").write_text("a")
            with patch.object(gate, "ROOT", root), patch.object(gate, "run", return_value=(0, "", None)):
                result = gate.compare_fixture(Path("rust"), Path("go"), "test", root / "tests/d2/test.d2", root, 1)
            self.assertEqual(result["status"], "failed")
            self.assertIn("could not read generated SVG", result["reason"])

    def test_multiboard_directories_compare_every_svg(self):
        for step_width, expected in [(10, "matched"), (11, "divergent")]:
            with self.subTest(step_width=step_width), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root / "tests/d2").mkdir(parents=True)
                (root / "tests/d2/test.d2").write_text("a")
                svg = '<svg viewBox="0 0 100 100"><g class="shape"><rect width="{}" height="10"/></g></svg>'

                def write_multiboard(_binary, _source, output, _timeout):
                    board_directory = output.with_suffix("")
                    (board_directory / "steps").mkdir(parents=True, exist_ok=True)
                    (board_directory / "index.svg").write_text(svg.format(10))
                    width = step_width if output.name.startswith("rust-") else 10
                    (board_directory / "steps/1.svg").write_text(svg.format(width))
                    return 0, "", None

                with patch.object(gate, "ROOT", root), patch.object(gate, "run", side_effect=write_multiboard):
                    result = gate.compare_fixture(
                        Path("rust"), Path("go"), "test", root / "tests/d2/test.d2", root, 1
                    )
                self.assertEqual(result["status"], expected)
                self.assertEqual(result["board_count"], 2)
                self.assertEqual(
                    [board["path"] for board in result["boards"]], ["index.svg", "steps/1.svg"]
                )
                self.assertEqual(result["boards"][0]["status"], "matched")
                self.assertEqual(result["boards"][1]["status"], expected)

    def test_multiboard_path_set_difference_is_a_layout_divergence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "tests/d2").mkdir(parents=True)
            (root / "tests/d2/test.d2").write_text("a")
            svg = '<svg viewBox="0 0 100 100"/>'

            def write_multiboard(_binary, _source, output, _timeout):
                board_directory = output.with_suffix("")
                board_directory.mkdir(parents=True, exist_ok=True)
                (board_directory / "index.svg").write_text(svg)
                if output.name.startswith("go-"):
                    (board_directory / "steps").mkdir()
                    (board_directory / "steps/1.svg").write_text(svg)
                return 0, "", None

            with patch.object(gate, "ROOT", root), patch.object(gate, "run", side_effect=write_multiboard):
                result = gate.compare_fixture(
                    Path("rust"), Path("go"), "test", root / "tests/d2/test.d2", root, 1
                )
            self.assertEqual(result["status"], "divergent")
            self.assertEqual(
                result["board_path_difference"],
                {"rust_missing": ["steps/1.svg"], "go_missing": []},
            )

    def test_missing_fixture_is_failure_without_execution(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(gate, "run") as run:
            root = Path(directory)
            with patch.object(gate, "ROOT", root):
                result = gate.compare_fixture(Path("rust"), Path("go"), "missing", root / "missing.d2", root, 1)
            self.assertEqual(result["status"], "failed")
            run.assert_not_called()

    def test_geometry_results_still_distinguish_matches_and_divergence(self):
        for width, expected in [(10, "matched"), (11, "divergent")]:
            with self.subTest(width=width), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root / "tests/d2").mkdir(parents=True)
                (root / "tests/d2/test.d2").write_text("a")
                svg = '<svg viewBox="0 0 100 100"><g class="shape"><rect width="{}" height="10"/></g></svg>'
                (root / "rust-test.svg").write_text(svg.format(width))
                (root / "go-test.svg").write_text(svg.format(10))
                with patch.object(gate, "ROOT", root), patch.object(gate, "run", return_value=(0, "", None)):
                    result = gate.compare_fixture(Path("rust"), Path("go"), "test", root / "tests/d2/test.d2", root, 1)
                self.assertEqual(result["status"], expected)

    def test_shape_geometry_descends_through_filter_groups_but_ignores_mask_defs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            go_svg = root / "go.svg"
            rust_svg = root / "rust.svg"
            go_svg.write_text(
                '<svg viewBox="0 0 100 100"><g class="shape">'
                '<rect x="10" y="20" width="30" height="40"/>'
                '</g></svg>'
            )
            rust_svg.write_text(
                '<svg viewBox="0 0 100 100"><g class="shape">'
                '<g filter="url(#shadow)"><rect x="10" y="20" width="30" height="40"/></g>'
                '<defs><mask><rect x="0" y="0" width="100" height="100"/>'
                '<path d="M0 0L1 1"/></mask></defs>'
                '</g></svg>'
            )
            self.assertEqual(gate.geometry(go_svg), gate.geometry(rust_svg))

    def test_code_label_light_and_dark_backgrounds_count_as_one_translated_shape(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            go_svg = root / "go.svg"
            rust_svg = root / "rust.svg"
            go_svg.write_text(
                '<svg viewBox="0 0 424 839"><g id="code">'
                '<g class="shape"/>'
                '<g class="light-code" transform="translate(40 216)">'
                '<rect width="148" height="205" class="shape"/></g>'
                '<g class="dark-code" transform="translate(40 216)">'
                '<rect width="148" height="205" class="shape"/></g>'
                '</g></svg>'
            )
            rust_svg.write_text(
                '<svg viewBox="0 0 424 839"><g id="code">'
                '<g class="shape"><rect x="40" y="216" width="148" '
                'height="205"/></g></g></svg>'
            )
            self.assertEqual(gate.geometry(go_svg), gate.geometry(rust_svg))

    def test_route_geometry_normalizes_line_and_path_encodings(self):
        semantic_id = base64.b64encode(b"(a -- )[0]").decode("ascii")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            go_svg = root / "go.svg"
            rust_svg = root / "rust.svg"
            go_svg.write_text(
                f'<svg><g id="{semantic_id}"><path class="connection" d="M 1 2 H 4"/></g></svg>'
            )
            rust_svg.write_text(
                f'<svg><g id="{semantic_id}"><line x1="1" y1="2" x2="4" y2="2" stroke-dasharray="4,2"/></g></svg>'
            )
            self.assertEqual(gate.connection_geometry(go_svg), gate.connection_geometry(rust_svg))

    def test_double_dash_edge_is_not_mistaken_for_a_sequence_lifeline(self):
        edge_id = "more.container.(finally -- sequence)[0]"
        semantic_id = base64.b64encode(edge_id.encode("utf-8")).decode("ascii")
        self.assertFalse(gate.is_sequence_lifeline_id(edge_id))
        with tempfile.TemporaryDirectory() as directory:
            svg_path = Path(directory) / "diagram.svg"
            svg_path.write_text(
                f'<svg><g id="{semantic_id}"><path class="connection" d="M 1 2 L 3 4"/></g></svg>'
            )
            geometry = gate.connection_geometry(svg_path)
        self.assertEqual(geometry["lifelines"], [])
        self.assertEqual(geometry["edges"][0]["edge"], edge_id)

    def test_arrowhead_marker_paths_are_not_layout_routes(self):
        semantic_id = base64.b64encode(b"a <-> b").decode("ascii")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            go_svg = root / "go.svg"
            rust_svg = root / "rust.svg"
            go_svg.write_text(
                f'<svg><g id="{semantic_id}"><marker><path class="connection" '
                'd="M4.8,0 4.8,18"/></marker><path class="connection" '
                'd="M 1 2 L 3 4"/></g></svg>'
            )
            rust_svg.write_text(
                f'<svg><g id="{semantic_id}"><path class="connection" '
                'd="M 1 2 L 3 4"/></g></svg>'
            )
            self.assertEqual(gate.connection_geometry(go_svg), gate.connection_geometry(rust_svg))

    def test_route_divergence_is_reported_even_when_shapes_match(self):
        semantic_id = base64.b64encode(b"a -> b").decode("ascii")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "test.d2").write_text("a -> b")
            (root / "tests/d2").mkdir(parents=True)
            (root / "tests/d2/test.d2").write_text("a -> b")
            rust_svg = f'<svg viewBox="0 0 100 100"><g class="shape"><rect width="10" height="10"/></g><g id="{semantic_id}"><path class="connection" d="M 0 0 L 10 10"/></g></svg>'
            go_svg = f'<svg viewBox="0 0 100 100"><g class="shape"><rect width="10" height="10"/></g><g id="{semantic_id}"><path class="connection" d="M 0 0 L 10 11"/></g></svg>'

            def write_fixture(_binary, _source, output, _timeout):
                output.write_text(rust_svg if output.name.startswith("rust-") else go_svg)
                return 0, "", None

            with patch.object(gate, "ROOT", root), patch.object(gate, "run", side_effect=write_fixture):
                result = gate.compare_fixture(Path("rust"), Path("go"), "test", root / "test.d2", root, 1)
            self.assertEqual(result["status"], "divergent")
            self.assertNotEqual(result["difference"]["rust_connections"], result["difference"]["go_connections"])

    def test_timeout_preserves_partial_stderr(self):
        with patch.object(gate.subprocess, "run", side_effect=subprocess.TimeoutExpired("binary", 1, stderr=b"partial log")):
            exit_code, stderr, failure = gate.run(Path("binary"), "test", Path("out.svg"), 1)
        self.assertIsNone(exit_code)
        self.assertEqual(stderr, "partial log")
        self.assertIn("timed out after 1 seconds", failure)

    def test_spawn_error_and_signal_are_failures(self):
        with patch.object(gate.subprocess, "run", side_effect=FileNotFoundError("missing")):
            self.assertIn("could not execute", gate.run(Path("binary"), "test", Path("out.svg"), 1)[2])
        with patch.object(gate.subprocess, "run", return_value=subprocess.CompletedProcess([], -9, "", "")):
            self.assertIn("signal 9", gate.run(Path("binary"), "test", Path("out.svg"), 1)[2])


if __name__ == "__main__":
    unittest.main()
