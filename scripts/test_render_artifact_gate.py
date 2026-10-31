"""Test the raster parity harness with broken evidence, without running D2."""
import contextlib
import hashlib
import importlib.util
import io
import json
import struct
import subprocess
import tempfile
import unittest
import zlib
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "render_artifact_gate", Path(__file__).with_name("check-render-parity.py")
)
CHECKER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECKER)


def chunk(kind, payload):
    return (struct.pack(">I", len(payload)) + kind + payload
            + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF))


def png(width=1, height=1, pixels=b"\x10\x20\x30", *, raw=None, level=6):
    stride = width * 3
    if raw is None:
        raw = b"".join(b"\0" + pixels[y * stride:(y + 1) * stride] for y in range(height))
    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, level)) + chunk(b"IEND", b""))


class RenderArtifactTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.go = self.root / "go/output.png"
        self.rust = self.root / "rust/output.png"
        self.go.parent.mkdir()
        self.rust.parent.mkdir()

    def boards(self, output, names, content=None):
        for name in names:
            path = output.with_suffix("") / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(png() if content is None else content)

    def compare(self):
        return CHECKER.compare_png_outputs(self.go, self.rust)

    def manifest(self, output, entries=None):
        directory = output.with_suffix("")
        if entries is None:
            entries = [{"path": path.relative_to(directory).as_posix(),
                        "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
                       for path in sorted(directory.rglob("*.png"))]
        (directory / CHECKER.BOARD_OUTPUT_MANIFEST).write_text(json.dumps(
            {"format": "d2-board-output", "version": 1, "files": entries, "directories": []}))

    def test_board_manifests_compare_inventory_not_png_bytes(self):
        self.boards(self.go, ["index.png", "d.png"], png(level=0))
        self.boards(self.rust, ["index.png", "d.png"], png(level=9))
        self.manifest(self.go)
        self.manifest(self.rust)
        self.assertTrue(self.compare()["matched"])

    def test_one_sided_board_manifest_never_matches(self):
        self.boards(self.go, ["index.png"])
        self.boards(self.rust, ["index.png"])
        self.manifest(self.go)
        self.assertFalse(self.compare()["matched"])

    def test_board_manifest_with_wrong_hash_never_matches(self):
        for output in (self.go, self.rust):
            self.boards(output, ["index.png"])
            self.manifest(output, [{"path": "index.png", "sha256": "0" * 64}])
        self.assertFalse(self.compare()["matched"])

    def test_two_missing_outputs_never_match(self):
        self.assertFalse(self.compare()["matched"])

    def test_one_missing_output_never_matches(self):
        for present in (self.go, self.rust):
            with self.subTest(present=present):
                present.write_bytes(png())
                self.assertFalse(self.compare()["matched"])
                present.unlink()

    def test_two_empty_board_directories_never_match(self):
        self.go.with_suffix("").mkdir()
        self.rust.with_suffix("").mkdir()
        self.assertFalse(self.compare()["matched"])

    def test_empty_collector_result_cannot_become_a_vacuous_match(self):
        with patch.object(CHECKER, "collect_png_outputs", return_value={}):
            self.assertFalse(self.compare()["matched"])

    def test_equal_pixels_with_different_png_encoding_match(self):
        self.go.write_bytes(png(level=0))
        self.rust.write_bytes(png(level=9))
        self.assertNotEqual(self.go.read_bytes(), self.rust.read_bytes())
        result = self.compare()
        self.assertTrue(result["matched"])
        self.assertEqual(result["different_pixels"], 0)

    def test_all_nested_boards_must_match(self):
        names = ["index.png", "steps/one.png", "steps/two.png"]
        self.boards(self.go, names)
        self.boards(self.rust, reversed(names))
        result = self.compare()
        self.assertTrue(result["matched"])
        self.assertEqual(result["go_artifacts"], names)
        self.assertEqual(len(result["artifacts"]), 3)

    def test_missing_extra_and_renamed_boards_fail(self):
        self.boards(self.go, ["index.png", "nested/one.png"])
        for names in (["index.png"], ["index.png", "nested/two.png"],
                      ["index.png", "nested/one.png", "extra.png"]):
            with self.subTest(names=names):
                self.boards(self.rust, names)
                result = self.compare()
                self.assertFalse(result["matched"])
                self.assertEqual(result["error"], "PNG artifact inventory differs")
                for path in self.rust.with_suffix("").rglob("*.png"):
                    path.unlink()

    def test_changed_pixel_in_one_board_fails(self):
        self.boards(self.go, ["index.png", "nested/one.png"])
        self.boards(self.rust, ["index.png", "nested/one.png"])
        (self.rust.with_suffix("") / "nested/one.png").write_bytes(png(pixels=b"\x10\x20\x31"))
        result = self.compare()
        self.assertFalse(result["matched"])
        self.assertEqual(result["artifacts"][1]["different_pixels"], 1)

    def test_board_dimension_difference_fails(self):
        self.boards(self.go, ["index.png"])
        self.boards(self.rust, ["index.png"], png(2, 1, b"\x10\x20\x30" * 2))
        result = self.compare()
        self.assertFalse(result["matched"])
        self.assertEqual(result["artifacts"][0]["rust_size"], [2, 1])

    def test_singleton_and_board_tree_cannot_hide_each_other(self):
        self.go.write_bytes(png())
        self.rust.write_bytes(png())
        self.boards(self.rust, ["unexamined.png"])
        result = self.compare()
        self.assertFalse(result["matched"])
        self.assertIn("ambiguous", result["error"])

    def test_non_png_board_artifact_fails(self):
        self.boards(self.go, ["index.png"])
        self.boards(self.rust, ["index.png"])
        (self.rust.with_suffix("") / "partial.txt").write_text("partial render")
        self.assertFalse(self.compare()["matched"])

    def test_identically_corrupt_images_do_not_match(self):
        valid = png()
        bad_crc = bytearray(valid)
        bad_crc[29] ^= 1
        header = valid[:33]
        compressed = zlib.compress(b"\0\x10\x20\x30")
        corruptions = {
            "missing IEND": valid[:-12],
            "bad CRC": bytes(bad_crc),
            "trailing bytes": valid + b"ignored",
            "extra decoded row": png(raw=b"\0\x10\x20\x30" * 2),
            "short decoded row": png(raw=b"\0\x10\x20"),
            "empty canvas": png(0, 1, b""),
            "truncated chunk": valid[:20],
            "duplicate IHDR": header + valid[8:],
            "missing IDAT": header + chunk(b"IEND", b""),
            "trailing zlib data": header + chunk(b"IDAT", compressed + b"ignored") + chunk(b"IEND", b""),
            "incomplete zlib stream": header + chunk(b"IDAT", compressed[:-1]) + chunk(b"IEND", b""),
            "bad filter": png(raw=b"\5\x10\x20\x30"),
            "IDAT before IHDR": valid[:8] + chunk(b"IDAT", compressed) + valid[8:],
            "duplicate PLTE": header + chunk(b"PLTE", b"\0\0\0") * 2 + valid[33:],
        }
        for name, data in corruptions.items():
            with self.subTest(name=name):
                self.go.write_bytes(data)
                self.rust.write_bytes(data)
                result = self.compare()
                self.assertFalse(result["matched"])
                self.assertIn("error", result)

    def test_rgb_transparency_cannot_be_ignored(self):
        valid = png()
        self.go.write_bytes(valid)
        self.rust.write_bytes(valid[:33] + chunk(b"tRNS", struct.pack(">HHH", 16, 32, 48)) + valid[33:])
        result = self.compare()
        self.assertFalse(result["matched"])
        self.assertIn("transparency", result["error"])

    def test_all_supported_png_filters_decode_complete_rows(self):
        # Hand-authored 2x2 RGB rows exercise left, up and upper-left predictors.
        encoded = {
            0: bytes([0, 50, 20, 30, 10, 50, 60, 0, 90, 25, 35, 55, 65, 80]),
            1: bytes([1, 50, 20, 30, 216, 30, 30, 1, 90, 25, 35, 221, 40, 45]),
            2: bytes([2, 50, 20, 30, 10, 50, 60, 2, 40, 5, 5, 45, 15, 20]),
            3: bytes([3, 50, 20, 30, 241, 40, 45, 3, 65, 15, 20, 5, 28, 33]),
            4: bytes([4, 50, 20, 30, 216, 30, 30, 4, 40, 5, 5, 5, 15, 20]),
        }
        for kind, raw in encoded.items():
            with self.subTest(kind=kind):
                self.go.write_bytes(png(2, 2, raw=raw))
                self.assertEqual(CHECKER.decode_png(self.go),
                                 (2, 2, bytes([50, 20, 30, 10, 50, 60, 90, 25, 35, 55, 65, 80])))


class RenderArtifactCliTests(unittest.TestCase):
    def run_checker(self, *, content=None, mode="single", exit_code=0, timeout=False, mixed=False,
                    layout=None, default_go=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            go_binary, rust_binary = root / "go-renderer", root / "rust-renderer"
            go_binary.write_bytes(b"test Go binary")
            rust_binary.write_bytes(b"test Rust binary")
            source = root / "first.d2"
            source.write_text("a -> b")
            report = root / "report.json"
            args = ["check-render-parity.py", "--rust", str(rust_binary),
                    "--output", str(report), "--timeout", "1"]
            if not default_go:
                args.extend(["--go", str(go_binary)])
            if layout is not None:
                args.extend(["--layout", layout])
            args.append(f"first={source}")
            if mixed:
                second = root / "second.d2"
                second.write_text("b -> c")
                args.append(f"second={second}")
            data = png() if content is None else content

            def render(command, **kwargs):
                if layout in {"tala", "tala-rs"} and "--layout=tala" not in command:
                    return subprocess.CompletedProcess(command, 1, b"", b"unsupported layout")
                output = Path(command[-1])
                omit = mode == "missing" or (mixed and command[0] == str(rust_binary)
                                              and Path(command[-2]).name == "second.d2")
                if output.name != "-" and not omit:
                    if mode == "boards":
                        for name in ["index.png", "steps/one.png"]:
                            board = output.with_suffix("") / name
                            board.parent.mkdir(parents=True, exist_ok=True)
                            board.write_bytes(data)
                    else:
                        output.write_bytes(data)
                if timeout:
                    raise subprocess.TimeoutExpired(command, kwargs.get("timeout", 1))
                # Also supports the previous stdout-based checker for a real before/after test.
                stdout = data if output.name == "-" and not omit else b""
                return subprocess.CompletedProcess(command, exit_code, stdout, b"test error" if exit_code else b"")

            with patch.object(CHECKER.subprocess, "run", side_effect=render), \
                 patch.object(CHECKER.references, "GO_ORACLE", go_binary), \
                 patch("sys.argv", args), contextlib.redirect_stdout(io.StringIO()):
                code = CHECKER.main()
            return code, json.loads(report.read_text())

    def test_tala_uses_the_current_builtin_engine_on_both_sides(self):
        for layout in ["tala", "tala-rs"]:
            with self.subTest(layout=layout):
                code, report = self.run_checker(layout=layout)
                self.assertEqual(code, 0)
                self.assertTrue(report["fixtures"][0]["matched"])

    def test_default_oracle_and_report_revision_follow_the_manifest(self):
        code, report = self.run_checker(default_go=True)
        self.assertEqual(code, 0)
        self.assertEqual(Path(report["reference"]["go_binary"]).name, "go-renderer")
        self.assertEqual(report["reference"]["d2_commit"], CHECKER.references.SOURCE_COMMIT)

    def test_success_without_outputs_is_a_reported_failure(self):
        code, report = self.run_checker(mode="missing")
        self.assertEqual(code, 1)
        self.assertFalse(report["fixtures"][0]["matched"])
        self.assertIn("error", report["fixtures"][0])

    def test_identically_corrupt_outputs_fail(self):
        code, report = self.run_checker(content=png()[:-12])
        self.assertEqual(code, 1)
        self.assertFalse(report["fixtures"][0]["matched"])

    def test_identical_partial_outputs_with_nonzero_exits_fail(self):
        code, report = self.run_checker(exit_code=2)
        self.assertEqual(code, 1)
        self.assertFalse(report["fixtures"][0]["matched"])
        self.assertEqual(report["fixtures"][0]["go_exit"], 2)

    def test_timeout_with_a_partial_output_is_reported_as_failure(self):
        code, report = self.run_checker(timeout=True)
        self.assertEqual(code, 1)
        self.assertFalse(report["fixtures"][0]["matched"])
        self.assertIn("timed out", report["fixtures"][0]["error"])

    def test_valid_single_image_run_passes(self):
        code, report = self.run_checker()
        self.assertEqual(code, 0)
        self.assertTrue(report["fixtures"][0]["matched"])

    def test_valid_complete_multiboard_run_passes(self):
        code, report = self.run_checker(mode="boards")
        self.assertEqual(code, 0)
        self.assertTrue(report["fixtures"][0]["matched"])
        self.assertEqual(len(report["fixtures"][0]["artifacts"]), 2)

    def test_one_failed_fixture_makes_the_whole_run_fail(self):
        code, report = self.run_checker(mixed=True)
        self.assertEqual(code, 1)
        self.assertEqual([row["matched"] for row in report["fixtures"]], [True, False])

    def test_empty_suite_cannot_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "renderer"
            binary.write_bytes(b"test")
            with patch.object(CHECKER, "DEFAULT_FIXTURES", {}), \
                 patch("sys.argv", ["check-render-parity.py", "--go", str(binary), "--rust", str(binary)]), \
                 contextlib.redirect_stderr(io.StringIO()), \
                 self.assertRaises(SystemExit) as failure:
                CHECKER.main()
            self.assertEqual(failure.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
