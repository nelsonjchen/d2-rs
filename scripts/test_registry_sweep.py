"""Registry sweep: every board of a multi-board output is compared."""
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).parent))
SPEC = importlib.util.spec_from_file_location("registry_sweep", Path(__file__).with_name("registry-sweep.py"))
SWEEP = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SWEEP)

FAKE = """#!{python}
import os, sys
out = sys.argv[-1]
child = {child!r}
if child is None:
    open(out, "w").write('<svg xmlns="http://www.w3.org/2000/svg"/>')
elif child == "empty":
    os.makedirs(out[:-4])
else:
    os.makedirs(out[:-4] + "/layers")
    open(out[:-4] + "/index.svg", "w").write('<svg xmlns="http://www.w3.org/2000/svg"/>')
    open(out[:-4] + "/layers/l1.svg", "w").write('<svg xmlns="http://www.w3.org/2000/svg"><text>' + child + '</text></svg>')
"""


class RegistrySweepTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)

    def cli(self, name, child):
        path = self.root / name
        path.write_text(FAKE.format(python=sys.executable, child=child))
        path.chmod(0o755)
        return str(path)

    def check(self, go_child, rust_child):
        go = self.cli("go", go_child)
        registry = SWEEP.mapseed.Registry(self.root / "registry.json")
        job = (self.cli("rust", rust_child), go, "flags", "x", "a -> b\n", ["--sketch"], "svg",
               SWEEP.mapseed.Oracle(Path(go)), (1, 2), registry)
        result = SWEEP.check(job)
        return result[5], result[10]

    def test_multiboard_directories_compare_every_board(self):
        self.assertEqual(self.check("same", "same"), (None, None))
        kind, resolution = self.check("one", "two")
        self.assertEqual((kind, resolution.classification), ("output-diff", "mismatch"))

    def test_single_file_and_empty_outputs(self):
        self.assertEqual(self.check(None, None)[0], None)
        self.assertEqual(self.check(None, "same")[0], "output-diff")
        self.assertEqual(self.check("same", "empty")[0], "rust-invalid-output")


if __name__ == "__main__":
    unittest.main()
