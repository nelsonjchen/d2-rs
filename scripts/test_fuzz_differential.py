"""The fuzz driver exposes formats/flags and includes upstream test scripts."""
import importlib.util
import json
from pathlib import Path
import random
import tempfile
import unittest


MODULE_PATH = Path(__file__).with_name("fuzz-differential.py")
SPEC = importlib.util.spec_from_file_location("fuzz_differential", MODULE_PATH)
FUZZ = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FUZZ)


class FuzzDriverTests(unittest.TestCase):
    def test_format_and_exact_render_flags_are_selectable(self):
        options = FUZZ.parse_args([
            "d2-rs", "--format=png", "--layout=elk",
            "--render-flag=--sketch", "--render-flag=--scale=0.25",
        ])
        self.assertEqual(options.format, "png")
        self.assertEqual(options.layout, "elk")
        self.assertEqual(options.render_flag, ["--sketch", "--scale=0.25"])

    def test_upstream_d2_and_raw_test_scripts_join_e2e_inputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cases, upstream = root / "cases", root / "upstream"
            cases.mkdir(); upstream.mkdir()
            (cases / "one.json").write_text(json.dumps({"script": "e2e"}))
            (upstream / "plain.d2").write_text("plain")
            (upstream / "sample_test.go").write_text(
                "var cases = []struct{ script string }{{script: `from test`}}"
            )
            scripts = dict(FUZZ.load_scripts(cases, upstream))
        self.assertEqual(scripts["e2e:one"], "e2e")
        self.assertEqual(scripts["upstream:plain.d2"], "plain")
        self.assertEqual(scripts["upstream:sample_test.go:script:0"], "from test")

    def test_pdf_rejects_the_live_clock_oracle(self):
        with self.assertRaisesRegex(SystemExit, "clock-injected"):
            FUZZ.main(["d2-rs", "--format=pdf", "--jobs=0"])

    def test_pinned_go_panics_are_unscored_and_retain_rust_outcome(self):
        self.assertTrue(FUZZ.go_crashed(2, "warn: option ignored\npanic: nil pointer"))
        self.assertTrue(FUZZ.go_crashed(-11, ""))
        self.assertFalse(FUZZ.go_crashed(1, "input.d2: panic: is not a keyword"))
        self.assertEqual(FUZZ.rust_outcome(101, ""), "rust-panic")
        self.assertEqual(FUZZ.rust_outcome("timeout", ""), "rust-hang")

    def test_text_mutation_is_deterministic_and_keeps_source(self):
        first = FUZZ.mutate_text("a -> b", random.Random(17))
        second = FUZZ.mutate_text("a -> b", random.Random(17))
        self.assertEqual(first, second)
        self.assertTrue(first.startswith("a -> b\n"))
        self.assertTrue(any(block in first for block in FUZZ.TEXT_BLOCKS))

    def test_text_family_is_selectable(self):
        options = FUZZ.parse_args(["d2-rs", "--mutation-family=text"])
        self.assertEqual(options.mutation_family, "text")


if __name__ == "__main__":
    unittest.main()
