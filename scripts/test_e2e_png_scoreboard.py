"""Original PNG call observations cannot pass incomplete evidence."""
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parent))
import isometric_png_scoreboard

def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result

SCORE = load('e2e_png_score', 'e2e-png-scoreboard.py')
PNG = load('png_helpers', 'test_render_artifact_gate.py').png


class OriginalPNGTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.capture, self.replay = self.root / 'capture', self.root / 'replay'
        self.name = 'set/case/elk/isometric.png'
        self.golden = self.root / 'golden.png'
        self.golden.write_bytes(PNG())
        self.go = self.capture / 'go' / self.name
        self.native = self.replay / 'native' / self.name
        for path in [self.go, self.native]:
            path.parent.mkdir(parents=True)
            path.write_bytes(PNG(level=0))
        self.option = self.capture / 'options' / self.name.replace('.png', '.json')
        self.option.parent.mkdir(parents=True)
        self.option.write_text('{}')
        self.events = [dict(Action='pass', Test='TestPNG'), dict(Action='pass', Test='TestPNG/' + self.name[:-4])]
        self.run = dict(exit=0, failure=None)

    def score(self):
        with patch.object(SCORE, 'expected_outputs', return_value={self.name: self.golden}):
            return SCORE.score(self.root, self.capture, self.replay, self.events, self.run)[0][0]['outcome']

    def test_exact_pixels_with_different_encodings(self):
        self.assertEqual(self.score(), 'match')

    def test_missing_extra_and_corrupt_native(self):
        self.native.unlink()
        self.assertEqual(self.score(), 'mismatch')
        for value in [b'', PNG()[:-1], b'not a PNG', PNG(pixels=b'\x10\x20\x31')]:
            self.native.write_bytes(value)
            self.assertEqual(self.score(), 'mismatch')
        self.native.write_bytes(PNG())
        self.native.with_name('extra.png').write_bytes(PNG())
        self.assertEqual(self.score(), 'mismatch')

    def test_original_call_and_assertion_inventories(self):
        self.option.unlink()
        self.assertEqual(self.score(), 'mismatch')
        self.option.write_text('{}')
        for event in self.events.copy():
            self.events.remove(event)
            self.assertEqual(self.score(), 'mismatch')
            self.events.append(event)

    def test_timeouts_and_nonzero_exit_never_pass(self):
        for run in [dict(exit=1, failure=None), dict(exit=0, failure='budget-exceeded'), dict(exit=-9, failure='timeout')]:
            self.run = run
            self.assertEqual(self.score(), 'mismatch')

    def test_unreproduced_golden_never_passes(self):
        self.go.write_bytes(PNG(pixels=b'\x10\x20\x31'))
        self.native.write_bytes(self.go.read_bytes())
        self.assertEqual(self.score(), 'mismatch')

    def test_empty_inventory_cannot_pass(self):
        with patch.object(SCORE, 'expected_outputs', return_value={}):
            rows, inventory = SCORE.score(self.root, self.capture, self.replay, self.events, self.run)
        self.assertEqual(rows, [])
        self.assertFalse(inventory['complete'])


if __name__ == '__main__':
    unittest.main()
