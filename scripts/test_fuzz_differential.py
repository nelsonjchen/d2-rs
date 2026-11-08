"""The fuzz driver exposes formats/flags and includes upstream test scripts."""
import importlib.util
import json
from pathlib import Path
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


if __name__ == "__main__":
    unittest.main()
