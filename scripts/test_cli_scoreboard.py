#!/usr/bin/env python3
"""Failure gates for the CLI scoreboard; no native build required."""
import importlib.util
import hashlib
import json
import shutil
from pathlib import Path
import tempfile
import unittest
import zipfile
from unittest.mock import patch


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(file))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


SCORE = load("cli_score", "cli-scoreboard.py")
DELTA = load("cli_delta", "cli-scoreboard-delta.py")


class CLIGateTests(unittest.TestCase):
    def test_release_profile_requires_fresh_go_assertions_and_exact_artifacts(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            name = "TestCLI_E2E/stdout_pptx"
            runs = {key: {"failure": None} for key in ["go", "rust", "go_release"]}
            goldens = root / "e2etests-cli/testdata"
            goldens.mkdir(parents=True)
            source = b"source-development-version"
            release = b"release-version"
            # Historical goldens must qualify against the original source run.
            (goldens / "TestCLI_E2E").mkdir()
            (goldens / "TestCLI_E2E/stdout_pptx.exp.pptx").write_bytes(source)
            for engine, data in [("go", source), ("rust", release), ("go-release", release)]:
                directory = root / engine / "cases" / name / "stdout"
                directory.mkdir(parents=True)
                (directory / "000").write_bytes(data)
                (root / f"{engine}.jsonl").write_text(json.dumps({"Test": name, "Action": "pass"}) + "\n")
            with patch.object(SCORE, "UPSTREAM", root):
                ordinary = SCORE.score([name], root, runs)
                self.assertEqual(ordinary["cases"][0]["outcome"], "artifact-mismatch")
                qualified = SCORE.score([name], root, runs, pptx_release=True)
                self.assertEqual(qualified["cases"][0]["outcome"], "contract-match")
                self.assertFalse(qualified["cases"][0]["source_comparison"]["matched"])
                self.assertEqual(qualified["goldens"][0]["outcome"], "golden-mismatch")
                self.assertEqual(DELTA.delta(ordinary, qualified)["regressions"], [])
                for data in [b"", release[:-1], release + b"extra"]:
                    (root / "go-release/cases" / name / "stdout/000").write_bytes(data)
                    result = SCORE.score([name], root, runs, pptx_release=True)
                    self.assertEqual(result["cases"][0]["outcome"], "artifact-mismatch")
                (root / "go-release.jsonl").write_text("")
                result = SCORE.score([name], root, runs, pptx_release=True)
                self.assertEqual(result["cases"][0]["outcome"], "go-release-unreproduced")
                (root / "go-release.jsonl").write_text(json.dumps({"Test": name, "Action": "pass"}) + "\n")
                (root / "go-release/cases" / name / "stdout/000").write_bytes(release)
                original = root / "go/cases" / name / "stdout/000"
                for missing in [False, True]:
                    if missing:
                        original.unlink()
                    else:
                        original.write_bytes(b"")
                    result = SCORE.score([name], root, runs, pptx_release=True)
                    self.assertEqual(result["cases"][0]["outcome"], "source-capture-failed")
                    self.assertFalse(result["cases"][0]["matched"])

    def test_reuse_cannot_switch_release_profiles(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            binary = root / "binary"
            binary.write_bytes(b"native")
            (root / "runs.json").write_text('{}')
            for captured, requested in [(None, True), (SCORE.PPTX_RELEASE, False),
                                        ({**SCORE.PPTX_RELEASE, "version": "v0.8.1"}, True)]:
                (root / "manifest.json").write_text(json.dumps({
                    "rust_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                    "pdf_clock": None, "pptx_release": captured,
                }))
                arguments = ["cli-scoreboard.py", "--reuse", "--rust", str(binary),
                             "--capture", str(root), "--output", str(root / "report.json")]
                if requested:
                    arguments.append("--pptx-release")
                with patch("sys.argv", arguments), patch.object(SCORE, "install", return_value=[]), \
                     self.assertRaises(SystemExit) as result:
                    SCORE.main()
                self.assertEqual(result.exception.code, 2)
                self.assertFalse((root / "report.json").exists())

    def test_validate_capture_install_is_idempotent_and_keeps_original_write(self):
        with tempfile.TemporaryDirectory() as tmp:
            trace = Path(tmp)
            for relative in ["e2etests-cli/main_test.go", "e2etests-cli/raster_test.go", "d2cli/validate.go", "lib/pdf/pdf.go"]:
                target = trace / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(SCORE.UPSTREAM / relative, target)
            with patch.object(SCORE, "TRACE", trace), patch.object(SCORE, "verify_source"), \
                 patch.object(SCORE.subprocess, "check_output", return_value=SCORE.references.SOURCE_COMMIT + "\n"):
                self.assertEqual(len(SCORE.install()), 70)
                first = {path.relative_to(trace): path.read_bytes() for path in trace.rglob("*.go")}
                self.assertEqual(len(SCORE.install()), 70)
                self.assertEqual(first, {path.relative_to(trace): path.read_bytes() for path in trace.rglob("*.go")})
            original = (SCORE.UPSTREAM / "d2cli/validate.go").read_text()
            instrumented = (trace / "d2cli/validate.go").read_text()
            self.assertEqual(instrumented.replace("fmt.Fprintf(d2rsValidateOutput(ms), ", "fmt.Printf("), original)
            writer = (trace / "d2cli/d2rs_validate_output.go").read_text()
            self.assertIn("d2rsValidateWriter(os.Stdout, ms.Stdout)", writer)
            self.assertIn("io.MultiWriter(process, captured)", writer)
            original_pdf = (SCORE.UPSTREAM / "lib/pdf/pdf.go").read_text()
            self.assertEqual((trace / "lib/pdf/pdf.go").read_text().replace("\n\td2rsPDFClock(newGofPDF)", ""), original_pdf)

    def test_pdf_dates_and_order_are_never_normalized(self):
        with tempfile.TemporaryDirectory() as tmp:
            left, right = Path(tmp) / "go.pdf", Path(tmp) / "rust.pdf"
            data = b"%PDF-1.4\n/CreationDate (D:20000102030405)\n/ModDate (D:20000102030405)\n%%EOF\n"
            left.write_bytes(data)
            right.write_bytes(data)
            self.assertTrue(SCORE.compare_file(left, right)["matched"])
            for changed in [data.replace(b"20000102030405", b"20000102030406"), data.replace(b"CreationDate", b"ModDate")]:
                right.write_bytes(changed)
                self.assertFalse(SCORE.compare_file(left, right)["matched"])

    def test_reuse_rejects_a_missing_or_changed_clock_profile(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            binary = root / "binary"
            binary.write_bytes(b"native")
            (root / "runs.json").write_text('{}')
            arguments = ["cli-scoreboard.py", "--reuse", "--pdf-epoch", "0", "--rust", str(binary),
                         "--capture", str(root), "--output", str(root / "report.json")]
            for clock in ["no-manifest", None, {"epoch_seconds": 1, "timezone": "UTC", "catalog_order": "FPDF SetCatalogSort(true)"}]:
                if clock != "no-manifest":
                    (root / "manifest.json").write_text(json.dumps({"rust_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "pdf_clock": clock}))
                with patch("sys.argv", arguments), patch.object(SCORE, "install", return_value=[]), \
                     self.assertRaises(SystemExit) as result:
                    SCORE.main()
                self.assertEqual(result.exception.code, 2)
                self.assertFalse((root / "report.json").exists())

    def test_stdout_requires_complete_exact_bytes(self):
        with tempfile.TemporaryDirectory() as tmp:
            go, rust = Path(tmp) / "go", Path(tmp) / "rust"
            for directory in [go, rust]:
                (directory / "stdout").mkdir(parents=True)
            expected = b"Success! [/same path/input.d2] is valid D2.\n"
            (go / "stdout/000").write_bytes(expected)
            self.assertFalse(SCORE.compare_case(go, rust)["matched"])
            for actual in [b"", expected.rstrip(), expected.replace(b"same path", b"other path"), expected + b"extra"]:
                (rust / "stdout/000").write_bytes(actual)
                self.assertFalse(SCORE.compare_case(go, rust)["matched"])
            (rust / "stdout/000").write_bytes(expected)
            self.assertTrue(SCORE.compare_case(go, rust)["matched"])

    def test_missing_and_empty_artifacts_fail(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "out.svg"
            self.assertFalse(SCORE.compare_file(path, path)["matched"])
            path.touch()
            self.assertFalse(SCORE.compare_file(path, path)["matched"])

    def test_complete_case_inventory_required(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            go, rust = root / "go", root / "rust"
            go.mkdir()
            self.assertFalse(SCORE.compare_case(go, rust)["matched"])
            rust.mkdir()
            (go / "a.svg").write_text("<svg/>")
            (rust / "a.svg").write_text("<svg/>")
            (go / "b.svg").write_text("<svg/>")
            self.assertFalse(SCORE.compare_case(go, rust)["matched"])

    def test_corrupt_png_cannot_match_itself(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "out.png"
            path.write_bytes(b"corrupt")
            self.assertFalse(SCORE.compare_file(path, path)["matched"])

    def test_empty_inputs_are_distinct_from_empty_outputs(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for engine in ["go", "rust"]:
                directory = root / engine
                (directory / "files").mkdir(parents=True)
                (directory / "files/icon.svg").touch()
                (directory / "empty-inputs.json").write_text('{"icon.svg":true}')
            self.assertTrue(SCORE.compare_case(root / "go", root / "rust")["matched"])
            for engine in ["go", "rust"]:
                (root / engine / "files/output.png").touch()
            self.assertFalse(SCORE.compare_case(root / "go", root / "rust")["matched"])

    def test_svg_version_allowance_preserves_content(self):
        with tempfile.TemporaryDirectory() as tmp:
            left, right = Path(tmp) / "a.svg", Path(tmp) / "b.svg"
            left.write_text('<svg data-d2-version="old"><text>v0.9.0</text></svg>')
            right.write_text('<svg data-d2-version="new"><text>v0.9.0</text></svg>')
            self.assertTrue(SCORE.compare_file(left, right)["matched"])
            self.assertFalse(SCORE.compare_file(left, right, golden=True)["matched"])
            right.write_text('<svg data-d2-version="new"><text>changed</text></svg>')
            self.assertFalse(SCORE.compare_file(left, right)["matched"])

    def test_gif_decodes_every_frame_and_delay(self):
        from PIL import Image
        with tempfile.TemporaryDirectory() as tmp:
            left, right = Path(tmp) / "a.gif", Path(tmp) / "b.gif"
            frames = [Image.new("RGB", (4, 4), color) for color in ["red", "blue"]]
            frames[0].save(left, save_all=True, append_images=frames[1:], duration=100, loop=0, optimize=False)
            frames[0].save(right, save_all=True, append_images=frames[1:], duration=100, loop=0, optimize=True)
            self.assertNotEqual(left.read_bytes(), right.read_bytes())
            self.assertTrue(SCORE.compare_file(left, right)["matched"])
            frames[0].save(right, save_all=True, append_images=frames[1:], duration=200, loop=0)
            self.assertFalse(SCORE.compare_file(left, right)["matched"])
            right.write_bytes(right.read_bytes()[:-1])
            self.assertFalse(SCORE.compare_file(right, right)["matched"])

    def test_pptx_development_version_is_visible_and_remains_nonpassing(self):
        with tempfile.TemporaryDirectory() as tmp:
            for suffix in [".pptx", ""]:
                left, right = Path(tmp) / ("go" + suffix), Path(tmp) / ("rust" + suffix)
                for path, version in [(left, "0.8.1"), (right, "0.9.0")]:
                    with zipfile.ZipFile(path, "w") as archive:
                        archive.writestr("docProps/app.xml", f'<Properties xmlns="urn:test"><AppVersion>{version}</AppVersion></Properties>')
                result = SCORE.compare_file(left, right)
                self.assertFalse(result["matched"])
                self.assertEqual(result["metadata_differences"][0]["field"], "docProps/app.xml/AppVersion")
                self.assertTrue(SCORE.compare_file(right, right)["matched"])

    def test_every_golden_and_case_remains_even_without_events(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            goldens = root / "e2etests-cli/testdata/TestCLI_E2E"
            goldens.mkdir(parents=True)
            (goldens / "live.exp.svg").write_text("<svg/>")
            (goldens / "obsolete.exp.svg").write_text("<svg/>")
            for engine in ["go", "rust"]:
                (root / f"{engine}.jsonl").write_text("")
            with patch.object(SCORE, "UPSTREAM", root):
                report = SCORE.score(["TestCLI_E2E/live"], root,
                                     {engine: {"failure": None} for engine in ["go", "rust"]})
            self.assertEqual(report["summary"]["cases"], {"go-unreproduced": 1})
            self.assertEqual(report["summary"]["goldens"], {"go-unreproduced": 1, "unbound-golden": 1})

    def test_missing_lost_or_duplicate_matches_are_regressions(self):
        before = {"cases": [{"name": "a", "outcome": "contract-match"}], "goldens": []}
        for rows in [[], [{"name": "a", "outcome": "rust-failed"}], before["cases"] * 2]:
            result = DELTA.delta(before, {"cases": rows, "goldens": []})
            self.assertTrue(result["regressions"])

    def test_assertions_remain_a_separate_guard_from_artifact_bytes(self):
        passed = {"name": "pdf", "go": "pass", "rust": "pass", "outcome": "artifact-mismatch"}
        failed = {**passed, "rust": "fail", "outcome": "rust-failed"}
        before = {"cases": [failed], "goldens": []}
        after = {"cases": [passed], "goldens": []}
        self.assertEqual(DELTA.delta(before, after)["improvements"],
                         ["assertions/pdf: non-passing -> pass"])
        self.assertTrue(DELTA.delta(after, before)["regressions"])
        for outcome in ["capture-failed", "unsupported-native-writer"]:
            self.assertFalse(DELTA.assertion_matches({"cases": [{**passed, "outcome": outcome}]}))
        self.assertFalse(DELTA.assertion_matches({**after, "runs": {"rust": {"failure": "timeout"}}}))

    def test_budget_exhaustion_cannot_count_a_passed_case(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for engine in ["go", "rust"]:
                (root / f"{engine}.jsonl").write_text('{"Test":"TestCLI_E2E/live","Action":"pass"}\n')
                (root / engine / "cases/TestCLI_E2E/live").mkdir(parents=True)
            with patch.object(SCORE, "UPSTREAM", root):
                report = SCORE.score(["TestCLI_E2E/live"], root,
                                     {"go": {"failure": None}, "rust": {"failure": "budget-exceeded"}})
            self.assertEqual(report["summary"]["cases"], {"capture-failed": 1})


if __name__ == "__main__":
    unittest.main()
