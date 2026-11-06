"""Timeline uses the same strict whole-animation gate as the image API."""
import importlib.util
import tempfile
import unittest
from pathlib import Path

import test_isometric_gif_scoreboard as gif_gate

spec = importlib.util.spec_from_file_location(
    "timeline_scoreboard", Path(__file__).with_name("isometric-timeline-scoreboard.py")
)
SCORE = importlib.util.module_from_spec(spec)
spec.loader.exec_module(SCORE)

class TimelineGateTests(gif_gate.GateTests):
    def compare(self, expected, actual):
        with tempfile.TemporaryDirectory() as directory:
            return SCORE.compare(expected, actual, Path(directory))

if __name__ == "__main__":
    unittest.main()
