import base64
import importlib.util
import io
import tempfile
import unittest
from pathlib import Path

from PIL import Image

spec = importlib.util.spec_from_file_location(
    "gif_scoreboard", Path(__file__).with_name("isometric-gif-scoreboard.py")
)
SCORE = importlib.util.module_from_spec(spec)
spec.loader.exec_module(SCORE)


def result(colors=("red", "blue"), delay=80, loop=0):
    frames = [Image.new("RGB", (4, 4), c) for c in colors]
    data = io.BytesIO()
    frames[0].save(
        data,
        format="GIF",
        save_all=True,
        append_images=frames[1:],
        duration=delay,
        loop=loop,
        disposal=1,
    )
    return {"data": base64.b64encode(data.getvalue()).decode(), "error": ""}


class GateTests(unittest.TestCase):
    def compare(self, expected, actual):
        with tempfile.TemporaryDirectory() as directory:
            return SCORE.compare(expected, actual, Path(directory))

    def test_complete_animation(self):
        self.assertEqual(self.compare(result(), result()), ("match", 2))

    def test_missing_frame_fails(self):
        self.assertEqual(self.compare(result(), result(("red",)))[0], "mismatch")

    def test_extra_frame_fails(self):
        self.assertEqual(
            self.compare(result(), result(("red", "blue", "green")))[0], "mismatch"
        )

    def test_every_frame_is_checked(self):
        self.assertEqual(
            self.compare(result(), result(("red", "green")))[0], "mismatch"
        )

    def test_delay_and_loop_are_checked(self):
        self.assertEqual(self.compare(result(), result(delay=90))[0], "mismatch")
        self.assertEqual(self.compare(result(), result(loop=1))[0], "mismatch")

    def test_missing_or_corrupt_artifact_fails(self):
        for value in [
            "",
            base64.b64encode(b"GIF89a;").decode(),
            base64.b64encode(base64.b64decode(result()["data"])[:-1]).decode(),
        ]:
            self.assertEqual(
                self.compare(result(), {"data": value, "error": ""})[0],
                "invalid-native-output",
            )

    def test_complete_errors_and_no_partial_output(self):
        error = {"data": None, "error": "context canceled"}
        self.assertEqual(self.compare(error, error), ("match", 0))
        self.assertEqual(
            self.compare(error, {"data": None, "error": "canceled"})[0], "mismatch"
        )
        self.assertEqual(
            self.compare(error, {**result(), "error": error["error"]})[0],
            "invalid-native-output",
        )

    def test_unsupported_is_not_passing(self):
        self.assertEqual(
            self.compare(result(), {"unsupported": True})[0], "unsupported"
        )


if __name__ == "__main__":
    unittest.main()
