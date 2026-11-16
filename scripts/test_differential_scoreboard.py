"""Failure-oriented contracts for the CLI differential harness."""
import importlib.util
import io
from contextlib import redirect_stderr
from pathlib import Path
import random
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("differential", Path(__file__).with_name("differential-scoreboard.py"))
D = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(D)


class DifferentialContracts(unittest.TestCase):
    def setUp(self):
        D.budget_exhausted.clear()
        D.budget_used = 0
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def binary(self, name, body):
        path = self.root / name
        path.write_text(f"#!{sys.executable}\nfrom pathlib import Path\n" + body)
        path.chmod(0o755)
        return path

    def compare(self, go_body, rust_body, source="a", options=None, oracle=None, registry=None):
        go = self.binary("go", go_body)
        # By default the fake Go is its own seeded build: it ignores the seed.
        args = SimpleNamespace(go=go, rust=self.binary("rust", rust_body), timeout=2, tala_timeout=2, png_timeout=2,
                               mapseed_oracle=D.mapseed.Oracle(oracle or go), mapseed_seeds=(1, 2, 3),
                               mapseed_registry_entries=registry)
        return D.compare(("control", source, "original", options or []), args)

    # Map seed 2 takes the other map order; unset, the stock order.
    SEEDED_GO = ('import os\nPath("output.svg").write_text("<svg>" + '
                 '{"2": "<g/>"}.get(os.environ.get("D2RS_GO_MAPSEED"), "") + "</svg>")\n')

    def test_mapseed_match_passes_and_records_seeds(self):
        row = self.compare(self.SEEDED_GO, 'Path("output.svg").write_text("<svg><g/></svg>")\n')
        self.assertEqual((row["outcome"], row["mapseed"]["mapseed_seeds"], row["mapseed"]["go_variants"]),
                         ("mapseed-match", [2], 2))
        self.assertFalse(D.failed(row))

    def test_known_mapseed(self):
        registry = D.mapseed.Registry(self.root / "registry.json")
        registry.add(D.job_identity("a", [], {}), "a", [], "test", "test", "test")
        row = self.compare(self.SEEDED_GO, 'Path("output.svg").write_text("<svg><g/></svg>")\n', registry=registry)
        self.assertEqual(row["outcome"], "known-mapseed")
        self.assertFalse(D.failed(row))

    def test_go_nondeterministic_without_rust_match_fails_with_cluster(self):
        row = self.compare(self.SEEDED_GO, 'Path("output.svg").write_text("<svg><a/></svg>")\n')
        self.assertEqual((row["outcome"], row["mapseed"]["go_variants"]), ("go-nondeterministic-no-rust-match", 2))
        self.assertTrue(D.failed(row))
        self.assertEqual(D.cluster_label(row), "go-nondeterministic-no-rust-match: svg element: svg")

    def test_missing_seeded_oracle_fails(self):
        with redirect_stderr(io.StringIO()):
            row = self.compare(self.SEEDED_GO, 'Path("output.svg").write_text("<svg><g/></svg>")\n',
                               oracle=self.root / "absent-mapseed")
        self.assertEqual(row["outcome"], "mapseed-oracle-missing")
        self.assertTrue(D.failed(row))

    def test_oracle_stamp_evidence(self):
        base = "x: go1.27.0\n\tbuild\tGOARCH=arm64\n"
        stamp = '\tbuild\t-ldflags="-X github.com/d2lang/d2/lib/version.Version=v0.9.0"\n'
        self.assertTrue(D.stamped(base + stamp + "\tbuild\tvcs.modified=false\n"))
        # Go omits -ldflags from build info under -trimpath; a clean tree is then required.
        self.assertTrue(D.stamped(base + "\tbuild\t-trimpath=true\n\tbuild\tvcs.modified=false\n"))
        self.assertFalse(D.stamped(base + "\tbuild\t-trimpath=true\n\tbuild\tvcs.modified=true\n"))
        self.assertFalse(D.stamped(base + '\tbuild\t-ldflags="-s -w"\n\tbuild\t-trimpath=true\n\tbuild\tvcs.modified=false\n'))
        self.assertFalse(D.stamped(base + "\tbuild\tvcs.modified=false\n"))

    def test_trimpath_oracle_passes_provenance_but_unstamped_does_not(self):
        arch = {"x86_64": "amd64", "aarch64": "arm64"}[D.platform.machine()]
        metadata = (f"go: {D.references.MANIFEST['d2']['go_toolchain']}\n\tbuild\t-trimpath=true\n\tbuild\tGOARCH={arch}\n"
                    f"\tbuild\tvcs.revision={D.references.SOURCE_COMMIT}\n\tbuild\tvcs.modified=false\n")
        binary = self.binary("oracle", "")

        def fake(version):
            return lambda argv, **kw: metadata if argv[0] == "go" else version + "\n"
        with patch.object(D.subprocess, "check_output", fake("v0.9.0")):
            self.assertIn("build_metadata", D.provenance(binary, True))
        with patch.object(D.subprocess, "check_output", fake("v0.8.1-HEAD")), self.assertRaises(ValueError):
            D.provenance(binary, True)
        with patch.object(D.subprocess, "check_output", lambda argv, **kw: metadata.replace("vcs.modified=false", "vcs.modified=true")
                          if argv[0] == "go" else "v0.9.0\n"), self.assertRaises(ValueError):
            D.provenance(binary, True)

    def test_seed_reaches_the_environment(self):
        self.assertEqual(D.environment(None, 7)[D.mapseed.ENV], "7")
        self.assertNotIn(D.mapseed.ENV, D.environment(None))

    def test_multiboard_success_requires_complete_file_set_and_bytes(self):
        body = 'Path("output").mkdir()\nPath("output/a.svg").write_text("<svg/>")\n'
        self.assertEqual(self.compare(body, body)["outcome"], "match")
        row = self.compare(body, body + 'Path("output/b.svg").write_text("<svg/>")\n')
        self.assertEqual(row["cluster"], "output inventory")
        self.assertEqual(self.compare(body, body.replace("<svg/>", "<svg><g/></svg>"))["outcome"], "mismatch")

    def test_rust_exit_is_checked_when_go_writes_a_directory(self):
        row = self.compare('Path("output").mkdir()\nPath("output/a.svg").write_text("<svg/>")\n', 'raise SystemExit(1)\n')
        self.assertEqual(row["cluster"], "exit 0 vs 1")
        self.assertTrue(D.failed(row))

    def test_rust_cannot_inherit_go_artifacts(self):
        row = self.compare('Path("output.svg").write_text("<svg/>")\n', '')
        self.assertEqual(row["outcome"], "mismatch")
        self.assertEqual(row["rust"]["files"], {})

    def test_language_imports_are_recreated_for_each_engine(self):
        go = self.binary("go", 'assert Path("nested/model.d2").read_text() == "a"\nPath("nested/model.d2").write_text("changed")\n')
        rust = self.binary("rust", 'assert Path("nested/model.d2").read_text() == "a"\n')
        args = SimpleNamespace(go=go, rust=rust, timeout=2)
        row = D.compare(("import", "x: @nested/model", "language-import", [], {"nested/model.d2": "a"}), args)
        self.assertEqual(row["outcome"], "match")
        self.assertEqual(row["go"]["files"], {})

    def test_language_queue_is_seeded_and_separate(self):
        jobs = list(D.language_jobs(123, 100))
        self.assertEqual(jobs, list(D.language_jobs(123, 100)))
        self.assertNotEqual(jobs, list(D.language_jobs(124, 100)))
        self.assertEqual(len(jobs), 100)
        self.assertTrue(all(job[2].startswith("language-") for job in jobs))

    def test_stdout_differences_are_scored(self):
        self.assertEqual(self.compare('print("go")\n', 'print("rust")\n')["cluster"], "stdout")

    def test_corrupt_svg_cannot_match(self):
        body = 'Path("output.svg").write_text("broken")\n'
        row = self.compare(body, body)
        self.assertEqual(row["outcome"], "invalid-output")
        self.assertTrue(D.failed(row))

    def test_missing_and_corrupt_png_cannot_match(self):
        for body in ('', 'Path("output.png").write_bytes(b"not png")\n'):
            self.assertEqual(self.compare(body, body, options=["--png"])["outcome"], "invalid-output")

    def test_go_crash_retains_rust_result(self):
        row = self.compare('import sys\nprint("panic: controlled", file=sys.stderr)\nraise SystemExit(2)\n', 'raise SystemExit(3)\n')
        self.assertEqual(row["outcome"], "go-crash-or-timeout")
        self.assertEqual(row["rust"]["exit"], 3)
        self.assertFalse(D.failed(row))

    def test_process_launch_failure_is_scored(self):
        args = SimpleNamespace(go=self.root / "missing-go", rust=self.root / "missing-rust", timeout=2)
        row = D.compare(("missing-binaries", "a", "original", []), args)
        self.assertEqual(row["outcome"], "process-failure")
        self.assertTrue(D.failed(row))

    def test_timeout_is_failure_even_for_go_crash_bucket(self):
        self.assertTrue(D.failed({"outcome": "go-crash-or-timeout", "go": {"exit": "timeout"}}))
        self.assertTrue(D.failed({"outcome": "go-crash-or-timeout", "rust": {"exit": -11}}))
        self.assertTrue(D.failed({"outcome": "go-crash-or-timeout", "rust": {"exit": 0, "invalid_output": ["output.svg"]}}))

    def test_mutations_always_change_the_source(self):
        rng = random.Random(20261102)
        for source in ("", "a", "# comments\n", "shape: rectangle\n"):
            for _ in range(100):
                result, _ = D.mutate(source, rng)
                self.assertNotEqual(result, source)

    def test_rendering_matrix_is_seeded_and_covers_theme_cross_product(self):
        jobs = list(D.rendering_jobs(20261104, 5000))
        self.assertEqual(jobs, list(D.rendering_jobs(20261104, 5000)))
        matrix = [options for _, _, family, options in jobs if family == "render-theme-matrix"]
        combinations = {(next(o for o in options if o.startswith("--theme=")),
                         next(o for o in options if o.startswith("--dark-theme=")),
                         "--sketch" in options, D.format_of(options)) for options in matrix}
        self.assertEqual(len(matrix), 20 * 20 * 2 * 5)
        self.assertEqual(len(combinations), len(matrix))

    def test_rendering_detail_matrix_is_seeded_and_capped(self):
        jobs = list(D.rendering_detail_jobs(20261104, 720))
        self.assertEqual(len(jobs), 720)
        self.assertEqual(jobs, list(D.rendering_detail_jobs(20261104, 720)))
        self.assertEqual(jobs[:10], list(D.rendering_detail_jobs(20261104, 10)))
        self.assertEqual(len({name for name, _, _, _ in jobs}), 8)
        self.assertEqual(sum("--sketch" in options for _, _, _, options in jobs), 360)

    def test_empty_source_reports_the_branch_actually_taken(self):
        result, kind = D._mutate_once("", SimpleNamespace(randrange=lambda _: 1))
        self.assertEqual(kind, "edge")
        self.assertIn(" -> ", result)

    def test_edge_mutation_filters_reserved_keys(self):
        rng = random.Random(1)
        source = "shape: rectangle\nlabel: hello\na\nb\n"
        for _ in range(100):
            result, kind = D.mutate(source, rng)
            if kind == "edge":
                self.assertIn(result[len(source):].strip(), ("a -> b", "b -> a"))

    def test_only_version_attributes_are_normalized(self):
        self.assertEqual(D.normalize_svg(b'<svg data-d2-version="v1" d2Version="v2">v1</svg>'), b'<svg data-d2-version="" d2Version="">v1</svg>')

    def test_fmt_rewritten_files_are_compared(self):
        row = self.compare('Path("input.d2").write_text("a -> b\\n")\n',
                           'Path("input.d2").write_text("a->b\\n")\n', options=["fmt"])
        self.assertEqual(row["cluster"], "file bytes: fmt")

    def test_binary_stdout_is_preserved(self):
        body = 'import sys\nsys.stdout.buffer.write(b"DIR\\x00binary")\n'
        row = self.compare(body, body.replace("binary", "different"), options=["--stdout", "--format=txt"])
        self.assertEqual(row["outcome"], "mismatch")

    def test_clock_does_not_inherit_user_options(self):
        with patch.dict(D.os.environ, {"D2_THEME": "999", "SOURCE_DATE_EPOCH": "123"}):
            self.assertNotIn("D2_THEME", D.environment())
            self.assertNotIn("SOURCE_DATE_EPOCH", D.environment())
            self.assertEqual(D.environment(456)["SOURCE_DATE_EPOCH"], "456")

    def test_matrix_reaches_new_formats_and_real_target(self):
        rng = random.Random(20261102)
        flags = set()
        for index in range(2000):
            flags.update(D.widened_options([], index, "layers: {differential_layer: {a -> b}}", rng))
        for flag in ("--format=pdf", "--format=pptx", "--format=gif", "--format=txt", "--stdout",
                     "--dark-theme=200", "--center", "--isometric", "--target=layers.differential_layer"):
            self.assertIn(flag, flags)
        corpus = [(str(i), "a -> b\n") for i in range(100)]
        expanded = list(D.expanded_jobs(corpus, 20261102, 1200))
        self.assertEqual(expanded, list(D.expanded_jobs(corpus, 20261102, 1200)))
        self.assertEqual(len(expanded), 1200)
        for offset in range(0, len(expanded), 3):
            group = expanded[offset:offset + 3]
            self.assertEqual({job[3][0] for job in group}, {"--layout=dagre", "--layout=elk", "--layout=tala"})
            self.assertEqual(len({job[1] for job in group}), 1)
        flags = {flag for job in expanded for flag in job[3]}
        for flag in ("--theme=7", "--theme=8", "--theme=302", "--theme=303", "--dark-theme=201",
                     "--format=png", "--format=pdf", "--format=pptx", "--format=gif",
                     "--target=layers.differential_layer", "--animate-interval=125"):
            self.assertIn(flag, flags)

    def test_transparent_png_is_valid_and_corruption_fails(self):
        import struct
        import zlib
        def chunk(kind, payload):
            return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload))
        png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 6, 0, 0, 0))
        png += chunk(b"IDAT", zlib.compress(b"\0\0\0\0\0")) + chunk(b"IEND", b"")
        body = f'Path("output.png").write_bytes({png!r})\n'
        self.assertEqual(self.compare(body, body, options=["--png"])["outcome"], "match")
        corrupt = png[:-1] + bytes([png[-1] ^ 1])
        body = f'Path("output.png").write_bytes({corrupt!r})\n'
        self.assertEqual(self.compare(body, body, options=["--png"])["outcome"], "invalid-output")

    def test_artifact_budget_failure_is_not_match(self):
        with patch.object(D, "FILE_LIMIT", 8):
            self.assertEqual(self.compare('print("x" * 100)\n', '')["outcome"], "budget-exceeded")


if __name__ == "__main__":
    unittest.main()
