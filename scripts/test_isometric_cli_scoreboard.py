#!/usr/bin/env python3
"""Failure gates for the complete CLI isometric output comparison."""

import importlib.util
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "score", Path(__file__).with_name("isometric-cli-scoreboard.py")
)
score = importlib.util.module_from_spec(spec)
spec.loader.exec_module(score)


class Gates(unittest.TestCase):
    def test_corpus_retains_named_source_contracts(self):
        rows = score.cases()
        self.assertEqual(len(rows), 50)
        self.assertEqual(len({r["name"] for r in rows}), 50)
        self.assertIn("svg-multiple-stdout", {r["name"] for r in rows})
        self.assertIn("source-nan", {r["name"] for r in rows})

    def test_missing_outputs_and_capture_failures_never_match(self):
        row = {"exit": 0, "failure": None, "logs": {}, "outputs": {}}
        case = {"fail": False, "suffix": "svg", "args": []}
        self.assertEqual(
            score.compare(case, row, row, Path("."), Path(".")), "render-failed"
        )
        row = dict(row, failure="budget-exceeded")
        self.assertEqual(
            score.compare(case, row, row, Path("."), Path(".")), "capture-failed"
        )

    def test_corrupt_equal_png_bytes_never_pass(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            (directory / "out.png").write_bytes(b"not png")
            row = {
                "exit": 0,
                "failure": None,
                "logs": {},
                "outputs": {"out.png": "same"},
            }
            with self.assertRaises(ValueError):
                score.compare(
                    {"fail": False, "suffix": "png", "args": []},
                    row,
                    row,
                    directory,
                    directory,
                )

    def test_error_requires_nonzero_and_complete_logs(self):
        row = {"exit": 0, "failure": None, "logs": {}, "outputs": {}}
        case = {"fail": True}
        self.assertEqual(
            score.compare(case, row, row, Path("."), Path(".")), "error-mismatch"
        )
        row = dict(row, exit=1)
        self.assertEqual(
            score.compare(case, row, row, Path("."), Path(".")), "error-match"
        )
        self.assertEqual(
            score.compare(
                case, row, dict(row, logs={"stderr": "other"}), Path("."), Path(".")
            ),
            "error-mismatch",
        )

    def test_paged_corpus_preserves_all_original_static_cases(self):
        rows = score.paged_cases()
        self.assertEqual(len(rows), 24)
        self.assertEqual(len({r["name"] for r in rows}), 24)
        self.assertEqual(sum(r.get("preserve", False) for r in rows), 4)
        self.assertEqual(sum(r["fail"] for r in rows), 8)

    def test_changed_failure_destination_never_passes(self):
        row = {
            "exit": 1,
            "failure": None,
            "logs": {},
            "outputs": {},
            "preserved": False,
        }
        self.assertEqual(
            score.compare(
                {"fail": True, "preserve": True}, row, row, Path("."), Path(".")
            ),
            "publication-mismatch",
        )

    def test_gif_corpus_covers_source_mode_and_timeline_selection(self):
        rows = score.gif_cases()
        self.assertEqual(len(rows), 16)
        self.assertEqual(len({r["name"] for r in rows}), 16)
        self.assertEqual(sum(r["fail"] for r in rows), 3)
        self.assertIn("gif-nested-folder", {r["name"] for r in rows})
        self.assertIn("gif-selected-leaf", {r["name"] for r in rows})
        self.assertIn("gif-source-traffic", {r["name"] for r in rows})

    def test_gif_gate_checks_every_frame_delay_loop_and_inventory(self):
        from PIL import Image

        with tempfile.TemporaryDirectory() as temp:
            left, right = Path(temp) / "go", Path(temp) / "rust"
            left.mkdir()
            right.mkdir()
            first = Image.new("RGB", (2, 2), "red")
            second = Image.new("RGB", (2, 2), "blue")

            def save(path, delay=70, loop=0, color=second):
                first.save(
                    path / "out.gif",
                    save_all=True,
                    append_images=[color],
                    duration=[30, delay],
                    loop=loop,
                )

            case = {"fail": False, "suffix": "gif", "args": []}
            go = {
                "exit": 0,
                "failure": None,
                "logs": {},
                "outputs": {"out.gif": "go hash"},
            }
            rust = dict(go, outputs={"out.gif": "different encoded hash"})
            save(left)
            save(right)
            self.assertEqual(score.compare(case, go, rust, left, right), "source-match")
            for kw in [
                {"delay": 80},
                {"loop": 1},
                {"color": Image.new("RGB", (2, 2), "green")},
            ]:
                save(right, **kw)
                self.assertEqual(
                    score.compare(case, go, rust, left, right), "frame-mismatch"
                )
            self.assertEqual(
                score.compare(
                    case, go, dict(rust, outputs={"extra.gif": "x"}), left, right
                ),
                "source-mismatch",
            )
            (right / "out.gif").write_bytes(b"GIF89a;")
            with self.assertRaises((ValueError, OSError)):
                score.compare(case, go, rust, left, right)
            (right / "out.gif").unlink()
            with self.assertRaises(OSError):
                score.compare(case, go, rust, left, right)

    def test_corrupt_equal_pdf_and_pptx_never_pass(self):
        for suffix in ("pdf", "pptx"):
            with self.subTest(suffix=suffix), tempfile.TemporaryDirectory() as temp:
                directory = Path(temp)
                name = "out." + suffix
                (directory / name).write_bytes(b"not a document")
                row = {
                    "exit": 0,
                    "failure": None,
                    "logs": {},
                    "outputs": {name: "same"},
                }
                with self.assertRaises((ValueError, score.zipfile.BadZipFile)):
                    score.compare(
                        {"fail": False, "suffix": suffix, "args": []},
                        row,
                        row,
                        directory,
                        directory,
                    )


if __name__ == "__main__":
    unittest.main()
