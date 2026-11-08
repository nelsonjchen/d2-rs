"""The reducer replays widened-format findings with their exact flags."""
import importlib.util
from pathlib import Path
import unittest
from unittest import mock


MODULE_PATH = Path(__file__).with_name("fuzz-minimize.py")
SPEC = importlib.util.spec_from_file_location("fuzz_minimize", MODULE_PATH)
MINIMIZER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MINIMIZER)


class FuzzMinimizeTests(unittest.TestCase):
    def test_kind_forwards_output_format_and_render_flags(self):
        calls = []

        def run(binary, directory, layout, name, output_format, render_flags):
            calls.append((binary, layout, name, output_format, tuple(render_flags)))
            return 0, "", binary

        with mock.patch.object(MINIMIZER, "run", run):
            kind = MINIMIZER.kind(
                "rust",
                "go",
                "a",
                "elk",
                "png",
                ["--sketch", "--scale=0.5"],
            )

        self.assertEqual(kind, "output-diff")
        self.assertEqual(
            calls,
            [
                ("go", "elk", "render", "png", ("--sketch", "--scale=0.5")),
                ("rust", "elk", "render", "png", ("--sketch", "--scale=0.5")),
            ],
        )


if __name__ == "__main__":
    unittest.main()
