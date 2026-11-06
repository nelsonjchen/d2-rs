"""Frame capture rejects incomplete sessions and validates every emitted PNG."""

import base64
import importlib.util
import struct
import tempfile
import unittest
import zlib
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "frames", Path(__file__).with_name("isometric-frame-scoreboard.py")
)
frames = importlib.util.module_from_spec(spec)
spec.loader.exec_module(frames)


def png(red=0):
    def chunk(name, data):
        return (
            struct.pack(">I", len(data))
            + name
            + data
            + struct.pack(">I", zlib.crc32(name + data))
        )

    return base64.b64encode(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes([0, red, 0, 0])))
        + chunk(b"IEND", b"")
    ).decode()


class Gates(unittest.TestCase):
    def compare(self, want, got):
        with tempfile.TemporaryDirectory() as temp:
            return frames.compare({"want": want}, got, Path(temp))

    def test_success_requires_nonempty_frames(self):
        with tempfile.TemporaryDirectory() as temp, self.assertRaises(ValueError):
            frames.materialize({"frames": [], "error": ""}, Path(temp) / "empty")

    def test_corrupt_equal_bytes_do_not_match(self):
        data = {"frames": [base64.b64encode(b"not png").decode()], "error": ""}
        with self.assertRaises(ValueError):
            self.compare(data, data)

    def test_every_frame_is_validated(self):
        want = {"frames": [png(), png()], "error": ""}
        got = {
            "frames": [png(), base64.b64encode(b"bad second frame").decode()],
            "error": "",
        }
        self.assertEqual(self.compare(want, got), "invalid-native-output")

    def test_missing_and_extra_frames_fail(self):
        want = {"frames": [png(), png()], "error": ""}
        for output in [[png()], [png(), png(), png()]]:
            self.assertEqual(
                self.compare(want, {"frames": output, "error": ""}), "mismatch"
            )

    def test_matching_error_still_compares_emitted_prefix(self):
        want = {"frames": [png()], "error": "encoder stopped"}
        self.assertEqual(self.compare(want, want), "match")
        self.assertEqual(
            self.compare(want, {"frames": None, "error": "encoder stopped"}), "mismatch"
        )
        self.assertEqual(
            self.compare(want, {"frames": [png(255)], "error": "encoder stopped"}),
            "pixel-mismatch",
        )

    def test_complete_error_text_is_required(self):
        self.assertEqual(
            self.compare(
                {"frames": None, "error": "context canceled"},
                {"frames": None, "error": "canceled"},
            ),
            "mismatch",
        )

    def test_full_encoded_bytes_compare_after_pixel_gate(self):
        data = {"frames": [png()], "error": ""}
        self.assertEqual(self.compare(data, data), "match")


if __name__ == "__main__":
    unittest.main()
