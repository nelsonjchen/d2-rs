"""Contract checks for the isometric capture inventory and surface comparator."""
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
spec = importlib.util.spec_from_file_location("isometric_scoreboard", ROOT / "scripts/isometric-scoreboard.py")
iso = importlib.util.module_from_spec(spec)
spec.loader.exec_module(iso)


class InventoryTests(unittest.TestCase):
    def test_folder_boards_are_traversed_without_being_outputs(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            board = root / "case" / "dagre" / "board.exp.json"
            board.parent.mkdir(parents=True)
            board.write_text(json.dumps({"isFolderOnly": True, "layers": [
                {"steps": [{}, {}]}, {"isFolderOnly": True, "scenarios": [{}]}]}))
            saved = iso.TESTDATA
            iso.TESTDATA = root
            try:
                self.assertEqual(iso.expected_boards([{"board": str(board)}]), [
                    "case/dagre/isometric.layers.0",
                    "case/dagre/isometric.layers.0.steps.0",
                    "case/dagre/isometric.layers.0.steps.1",
                    "case/dagre/isometric.layers.1.scenarios.0"])
            finally:
                iso.TESTDATA = saved

    def test_surface_removal_preserves_all_outer_svg_bytes(self):
        svg = '<svg><defs><g id="surface-0"><g><path/></g></g></defs><use href="#surface-0"/><g transform="scale(1200 1200)"><defs/><g/></g></svg>'
        # Match Go's explicit group serialization (no self-closing groups).
        svg = svg.replace('<g/>', '<g></g>')
        self.assertEqual(iso.stub_surfaces(svg), '<svg><defs><g id="surface-0">§</g></defs><use href="#surface-0"/><g transform="scale(1200 1200)">§</g></svg>')

    def test_successful_process_with_no_outputs_still_reports_missing(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            package = root / "trace/d2renderers/d2isometric/d2isometricimg"
            package.mkdir(parents=True)
            board = root / "data/case/dagre/board.exp.json"
            board.parent.mkdir(parents=True)
            board.write_text("{}")
            capture = root / "capture"
            capture.mkdir()
            completed = SimpleNamespace(poll=lambda: 0, wait=lambda: 0)
            with patch.object(iso, "TRACE", root / "trace"), patch.object(iso, "TESTDATA", root / "data"), patch.object(iso.references, "command", return_value=iso.references.SOURCE_COMMIT), patch.object(iso.subprocess, "Popen", return_value=completed):
                code, rows, failure = iso.writer_tier(capture, [{"board": str(board), "out": str(capture / "rust/case/dagre")}])
            self.assertEqual(code, 0)
            self.assertIsNone(failure)
            self.assertEqual(rows, [{"board": "case/dagre/isometric", "outcome": "go-missing"}])

    def test_empty_filter_inventory_cannot_be_mistaken_for_the_corpus(self):
        rows = iso.requests(Path('/unused'), 'mesh', 'no-such-upstream-case-123456')
        self.assertEqual(rows, [])
        self.assertEqual(iso.expected_boards(rows), [])


class DirectContracts(unittest.TestCase):
    def test_explicit_asset_profile_never_claims_a_golden_match(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            capture, requests = self.fixture(root)
            with patch.object(iso, "TESTDATA", root / "data"):
                self.assertEqual(iso.compare_direct(capture, requests, "svg", {}, source_profile=True)[0][0]["outcome"], "source-match")
                (root / "data/case/dagre/isometric.exp.svg").unlink()
                self.assertEqual(iso.compare_direct(capture, requests, "svg", {})[0][0]["outcome"], "go-unreproduced")
                self.assertEqual(iso.compare_direct(capture, requests, "svg", {}, "budget-exceeded", True)[0][0]["outcome"], "incomplete-run")
                (capture / "rust/case/dagre/isometric.svg").write_text("<svg>changed</svg>")
                self.assertEqual(iso.compare_direct(capture, requests, "svg", {}, source_profile=True)[0][0]["outcome"], "mismatch")
                for side in ("go", "rust"):
                    (capture / side / "case/dagre/isometric.svg").write_text("broken SVG")
                self.assertEqual(iso.compare_direct(capture, requests, "svg", {}, source_profile=True)[0][0]["outcome"], "go-invalid-output")

    def test_explicit_assets_require_complete_unchanged_inputs(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            asset = root / "icon.svg"
            asset.write_text("<svg/>")
            manifest = root / "assets.json"
            value = {"https://example.test/icon.svg": {"path": str(asset), "mime": "image/svg+xml", "sha256": iso.references.digest(asset)}}
            manifest.write_text(json.dumps(value))
            self.assertEqual(iso.asset_sources(manifest), value)
            asset.write_text("changed")
            with self.assertRaisesRegex(ValueError, "hash differs"):
                iso.asset_sources(manifest)
            asset.unlink()
            with self.assertRaises(FileNotFoundError):
                iso.asset_sources(manifest)
            manifest.write_text("{}")
            with self.assertRaisesRegex(ValueError, "nonempty"):
                iso.asset_sources(manifest)

    def fixture(self, root):
        board = root / "data/case/dagre/board.exp.json"
        board.parent.mkdir(parents=True)
        board.write_text("{}")
        golden = board.with_name("isometric.exp.svg")
        golden.write_text("<svg></svg>")
        capture = root / "capture"
        for side in ("go", "rust"):
            path = capture / side / "case/dagre/isometric.svg"
            path.parent.mkdir(parents=True)
            path.write_bytes(golden.read_bytes())
        return capture, [{"board": str(board)}]

    def test_whole_board_missing_duplicate_empty_and_stale_outputs_fail(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            capture, requests = self.fixture(root)
            with patch.object(iso, "TESTDATA", root / "data"):
                rows, _ = iso.compare_direct(capture, requests, "svg", {"go": 0, "rust": 0})
                self.assertEqual(rows[0]["outcome"], "match")
                native = capture / "rust/case/dagre/isometric.svg"
                native.unlink()
                self.assertEqual(iso.compare_direct(capture, requests, "svg", {})[0][0]["outcome"], "rust-missing")
                native.write_bytes(b"")
                self.assertEqual(iso.compare_direct(capture, requests, "svg", {})[0][0]["outcome"], "rust-empty")
                native.with_suffix(".err").write_text("error")
                self.assertEqual(iso.compare_direct(capture, requests, "svg", {})[0][0]["outcome"], "rust-duplicate")
                native.with_suffix(".err").unlink()
                source = capture / "go/case/dagre/isometric.svg"
                source.write_text("stale")
                self.assertEqual(iso.compare_direct(capture, requests, "svg", {})[0][0]["outcome"], "go-unreproduced")
                source.unlink()
                self.assertEqual(iso.compare_direct(capture, requests, "svg", {})[0][0]["outcome"], "go-missing")

    def test_process_failure_and_capture_limit_revoke_all_matches(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            capture, requests = self.fixture(root)
            with patch.object(iso, "TESTDATA", root / "data"):
                for runs, failure in (({"go": 1}, None), ({}, "budget-exceeded")):
                    self.assertEqual(iso.compare_direct(capture, requests, "svg", runs, failure)[0][0]["outcome"], "incomplete-run")
                with self.assertRaisesRegex(ValueError, "empty capture"):
                    iso.compare_direct(capture, [], "svg", {})

    def test_finder_metadata_is_explicit_and_extra_boards_still_fail(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            capture, requests = self.fixture(root)
            (capture / "go/.DS_Store").write_text("Finder")
            (capture / "rust/extra.svg").write_text("<svg/>")
            with patch.object(iso, "TESTDATA", root / "data"):
                rows, metadata = iso.compare_direct(capture, requests, "svg", {})
            self.assertEqual(len(metadata["go"]), 1)
            self.assertEqual(metadata["go"][0]["path"], ".DS_Store")
            self.assertEqual([r["outcome"] for r in rows], ["match", "unexpected-board"])

    def test_direct_guard_covers_total_individual_files_and_free_space(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "file").write_bytes(b"12")
            self.assertEqual(iso.direct_guard(root, 1), "budget-exceeded")
            with patch.object(iso.shutil, "disk_usage", return_value=SimpleNamespace(free=1)):
                self.assertEqual(iso.direct_guard(root, 100), "free-space-budget-exceeded")
            with (root / "file").open("wb") as stream:
                stream.truncate((128 << 20) + 1)
            self.assertEqual(iso.direct_guard(root, 1 << 30), "file-budget-exceeded")

    def test_direct_scene_comparison_preserves_booleans_and_null_slices(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            capture, requests = self.fixture(root)
            for side, value in (("go", {"x": True, "items": None}), ("rust", {"x": 1, "items": []})):
                path = capture / side / "case/dagre/isometric.svg"
                path.unlink()
                path.with_suffix(".json").write_text(json.dumps(value))
            with patch.object(iso, "TESTDATA", root / "data"):
                self.assertEqual(iso.compare_direct(capture, requests, "scene", {})[0][0]["outcome"], "mismatch")


class TextContracts(unittest.TestCase):
    def test_primitive_triangle_order_identity_and_numeric_types_are_strict(self):
        want = {"triangles": [{"material": 1, "castShadow": True,
                              "v": [{"p": [0.1, 2, 3]}, {"p": [4, 5, 6]}]}],
                "materials": [{"color": [1, 2, 3, 255]}], "error": ""}
        self.assertEqual(iso.compare_text_calls([{"want": want}], [want]), [])
        for change in (lambda v: v["triangles"][0]["v"].reverse(),
                       lambda v: v["triangles"][0].update(material=2),
                       lambda v: v["triangles"][0].update(castShadow=1),
                       lambda v: v["triangles"][0]["v"][0]["p"].__setitem__(0, 0.10000000000000002),
                       lambda v: v.update(error="limit"),
                       lambda v: v.update(triangles=None)):
            actual = json.loads(json.dumps(want))
            change(actual)
            self.assertTrue(iso.compare_text_calls([{"want": want}], [actual]))

    def test_primitive_instrumentation_restores_both_sources_on_exception(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            package = Path("d2renderers/d2isometric/d2isometricimg")
            sources = {"geometry.go": b"\n".join(
                f"func (b *meshBuilder) {method}(".encode()
                for method in ("box", "sphere", "cylinder", "flat")),
                "native_shapes.go": b"func (b *meshBuilder) extrudedProfile("}
            for name in ("trace", "upstream"):
                (root / name / package).mkdir(parents=True)
                for source, original in sources.items():
                    (root / name / package / source).write_bytes(original)
            with (patch.object(iso, "TRACE", root / "trace"),
                  patch.object(iso, "UPSTREAM", root / "upstream"),
                  self.assertRaisesRegex(RuntimeError, "stop"), iso.primitive_instrumentation()):
                self.assertIn(b"d2rsOriginalBox", (root / "trace" / package / "geometry.go").read_bytes())
                raise RuntimeError("stop")
            for source, original in sources.items():
                self.assertEqual((root / "trace" / package / source).read_bytes(), original)
            self.assertFalse((root / "trace" / package / "d2rs_primitive_hook_test.go").exists())

    def test_text_inventory_and_every_output_field_are_compared(self):
        calls = [{"want": {"layout": {"Lines": ["a"]}, "pixels": "sha", "surface": "<g></g>"}}]
        actual = [calls[0]["want"].copy()]
        self.assertEqual(iso.compare_text_calls(calls, actual), [])
        for invalid in ([], actual + actual, None, ["bad"]):
            self.assertTrue(iso.compare_text_calls(calls, invalid))
        for field in calls[0]["want"]:
            changed = actual[0].copy()
            changed[field] = "wrong"
            self.assertEqual(iso.compare_text_calls(calls, [changed])[0]["fields"], [field])
        self.assertTrue(iso.compare_text_calls(calls, [{**actual[0], "extra": True}]))

    def test_native_budget_failure_reaps_the_process(self):
        process = SimpleNamespace(pid=42, poll=lambda: None, wait=Mock(return_value=0))
        with (patch.object(iso.subprocess, "Popen", return_value=process),
              patch.object(iso.os, "killpg") as kill,
              self.assertRaisesRegex(iso.CaptureLimit, "budget-exceeded")):
            iso.bounded_native(["unused"], None, lambda: "budget-exceeded")
        kill.assert_called_once_with(42, iso.signal.SIGTERM)
        self.assertGreaterEqual(process.wait.call_count, 1)

    def test_text_instrumentation_restores_source_after_exception(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            package = Path("d2renderers/d2isometric/d2isometricimg")
            original = b"func (p *textPainter) texture("
            for name in ("trace", "upstream"):
                (root / name / package).mkdir(parents=True)
                (root / name / package / "text.go").write_bytes(original)
            with (patch.object(iso, "TRACE", root / "trace"),
                  patch.object(iso, "UPSTREAM", root / "upstream"),
                  self.assertRaisesRegex(RuntimeError, "stop"), iso.text_instrumentation()):
                self.assertIn(b"d2rsOriginalTexture", (root / "trace" / package / "text.go").read_bytes())
                raise RuntimeError("stop")
            self.assertEqual((root / "trace" / package / "text.go").read_bytes(), original)
            self.assertFalse((root / "trace" / package / "d2rs_text_hook_test.go").exists())

    def test_rich_instrumentation_restores_exact_source_after_exception(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            package = Path("d2renderers/d2isometric/d2isometricimg")
            original = b"// retained body\nfunc (p *richLabelPainter) texture("
            for name in ("trace", "upstream"):
                (root / name / package).mkdir(parents=True)
                (root / name / package / "rich_labels.go").write_bytes(original)
            with (patch.object(iso, "TRACE", root / "trace"),
                  patch.object(iso, "UPSTREAM", root / "upstream"),
                  self.assertRaisesRegex(RuntimeError, "stop"), iso.text_instrumentation(rich=True)):
                self.assertIn(b"d2rsOriginalTexture", (root / "trace" / package / "rich_labels.go").read_bytes())
                self.assertTrue((root / "trace" / package / "d2rs_rich_hook_test.go").exists())
                raise RuntimeError("stop")
            self.assertEqual((root / "trace" / package / "rich_labels.go").read_bytes(), original)
            self.assertFalse((root / "trace" / package / "d2rs_rich_hook_test.go").exists())

    def test_rich_viewport_pixels_and_complete_surface_are_all_required(self):
        want = {"viewBox": {"X": 0, "Y": 0, "Width": 30, "Height": 40},
                "width": 8, "height": 16, "pixels": "sha", "surface": "<g>complete</g>"}
        self.assertEqual(iso.compare_text_calls([{"want": want}], [want]), [])
        for field in want:
            changed = want.copy()
            del changed[field]
            self.assertEqual(iso.compare_text_calls([{"want": want}], [changed])[0]["fields"], [field])


if __name__ == '__main__':
    unittest.main()
