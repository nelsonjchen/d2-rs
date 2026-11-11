"""The shape-trace sweep must generate deterministic, shape-heavy D2 graphs."""
import importlib.util
import random
import re
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
SCRIPT = Path(__file__).with_name("shape-trace-differential.py")
spec = importlib.util.spec_from_file_location("shape_trace_differential", SCRIPT)
sweep = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sweep)


class ShapeTraceDifferentialTests(unittest.TestCase):
    def cases(self, seed, count=200):
        rng = random.Random(seed)
        return [sweep.generate(rng) for _ in range(count)]

    def test_same_seed_same_cases(self):
        self.assertEqual(self.cases(7), self.cases(7))
        self.assertNotEqual(self.cases(7), self.cases(8))

    def test_every_case_has_traced_shapes_and_an_edge(self):
        for source in self.cases(1):
            shapes = re.findall(r"shape: ([\w-]+)", source)
            self.assertTrue(shapes)
            self.assertTrue(set(shapes) <= set(sweep.SHAPES))
            self.assertRegex(source, r"(->|<-|<->|--)")

    def test_sweep_covers_every_shape_and_wedge_threshold(self):
        text = "".join(self.cases(1, 400))
        for shape in sweep.SHAPES:
            self.assertIn(f"shape: {shape}", text)
        for size in (25, 26, 27, 34, 35, 36):
            self.assertIn(f"width: {size}", text)

    def test_edges_reference_declared_nodes(self):
        for source in self.cases(3):
            declared = set(re.findall(r"^(n\d+):", source, re.M))
            for left, right in re.findall(r"^((?:box\.)?n\d+) \S+ ((?:box\.)?n\d+)", source, re.M):
                self.assertIn(left.removeprefix("box."), declared)
                self.assertIn(right.removeprefix("box."), declared)


if __name__ == "__main__":
    unittest.main()
