"""Failure gates for original public API call captures."""

import base64
import copy
import io
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import api_scoreboard as api
from PIL import Image


def page_result():
    data = io.BytesIO()
    Image.new("RGBA", (64, 64), (255, 255, 255, 255)).save(data, format="PNG")
    return {
        "result": {
            "PNG": base64.b64encode(data.getvalue()).decode(),
            "Links": [],
            "Width": 64,
            "Height": 64,
        },
        "error": "",
    }


class APIContracts(unittest.TestCase):
    def test_empty_missing_and_extra_outputs_fail(self):
        call = {"request": {"api": "page"}, "want": page_result()}
        for calls, results in [
            ([], []),
            ([call], []),
            ([call], [call["want"], call["want"]]),
        ]:
            with self.assertRaises(ValueError):
                api.compare_calls(calls, results, 0)

    def test_complete_png_and_inventory_are_required(self):
        result = page_result()
        self.assertTrue(api.valid_result("page", result))
        for corrupt in [
            "",
            base64.b64encode(b"broken").decode(),
            result["result"]["PNG"][:-12],
        ]:
            changed = copy.deepcopy(result)
            changed["result"]["PNG"] = corrupt
            self.assertFalse(api.valid_result("page", changed))
        for key, value in [("Links", None), ("Width", 63)]:
            changed = copy.deepcopy(result)
            changed["result"][key] = value
            self.assertFalse(api.valid_result("page", changed))

    def test_invalid_sources_and_failed_processes_cannot_match(self):
        call = {"request": {"api": "page"}, "want": page_result()}
        self.assertEqual(
            api.compare_calls([call], [call["want"]], 0)[0]["outcome"], "match"
        )
        self.assertEqual(
            api.compare_calls([call], [call["want"]], 1)[0]["outcome"], "mismatch"
        )
        call["want"]["result"]["PNG"] = ""
        self.assertEqual(
            api.compare_calls([call], [call["want"]], 0)[0]["outcome"],
            "go-invalid-output",
        )

    def test_error_results_must_not_publish_partial_output(self):
        self.assertTrue(
            api.valid_result("page", {"result": None, "error": "context canceled"})
        )
        self.assertFalse(
            api.valid_result("page", {"result": {}, "error": "context canceled"})
        )

    def test_full_annotation_and_png_bytes_compare(self):
        call = {"request": {"api": "page"}, "want": page_result()}
        changed = copy.deepcopy(call["want"])
        changed["result"]["Links"] = [{"URL": "different"}]
        self.assertEqual(
            api.compare_calls([call], [changed], 0)[0]["outcome"], "mismatch"
        )
        changed = page_result()
        data = io.BytesIO()
        Image.new("RGBA", (64, 64), (0, 0, 0, 255)).save(data, format="PNG")
        changed["result"]["PNG"] = base64.b64encode(data.getvalue()).decode()
        self.assertEqual(
            api.compare_calls([call], [changed], 0)[0]["outcome"], "mismatch"
        )

    def test_svg_requires_complete_xml_and_full_bytes(self):
        options = {"Format": "svg"}

        def result(data):
            return {"result": base64.b64encode(data).decode(), "error": ""}

        valid = result(
            b'<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><path d="M0 0L1 1"/></svg>'
        )
        self.assertTrue(api.valid_result("render", valid, options))
        for data in [
            b"",
            b"<svg",
            b'<svg width="64" height="64"><path/></svg>',
            b'<svg xmlns="http://www.w3.org/2000/svg" width="0" height="64"><path/></svg>',
            b'<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"/>',
        ]:
            self.assertFalse(api.valid_result("render", result(data), options))
        call = {"request": {"api": "render", "options": options}, "want": valid}
        self.assertEqual(api.compare_calls([call], [valid], 0)[0]["outcome"], "match")
        changed = result(base64.b64decode(valid["result"]).replace(b"L1 1", b"L2 2"))
        self.assertEqual(
            api.compare_calls([call], [changed], 0)[0]["outcome"], "mismatch"
        )

    def test_streaming_guards_cover_native_outputs(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            output = root / "output"
            with output.open("wb") as stream:
                stream.truncate((128 << 20) + 1)
            with patch.object(api.iso, "direct_guard", return_value=None):
                self.assertEqual(
                    api.api_guard(root, [output]), "native output exceeds file budget"
                )
            output.unlink()
            with patch.object(api.iso, "direct_guard", return_value="free-space gate"):
                self.assertEqual(api.api_guard(root, []), "free-space gate")


if __name__ == "__main__":
    unittest.main()
