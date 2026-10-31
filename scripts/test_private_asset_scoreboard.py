#!/usr/bin/env python3
"""Failure gates for original paired private-network CLI calls."""
import base64
import copy
import importlib.util
import io
from pathlib import Path
import tempfile
import unittest

from PIL import Image

spec = importlib.util.spec_from_file_location("policy", Path(__file__).with_name("private-asset-scoreboard.py"))
policy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(policy)


def encoded(data):
    return base64.b64encode(data).decode()


def fixture():
    png = io.BytesIO()
    Image.new("RGB", (2, 2), (255, 0, 0)).save(png, format="PNG")
    payloads = dict(svg=b'<svg xmlns="http://www.w3.org/2000/svg"/>', png=png.getvalue(),
                    pdf=b"%PDF-1.3\n/Type /Page\n/Subtype /Image\n%%EOF\n")
    calls = []
    for fmt in ["svg", "png", "pdf"]:
        source = "a: {icon: http://127.0.0.1:12345/" + fmt + "/icon.svg}"
        for allowed in [False, True]:
            result = dict(data=encoded(payloads[fmt] if allowed else b""), error="" if allowed else "complete error",
                          requests=1 if allowed else 0,
                          files={"input.d2": dict(data=encoded(source.encode()), mode=0o600)})
            calls.append(dict(test=policy.TEST + "/" + fmt, source=source, allowed=allowed,
                              args=(["--allow-private-network"] if allowed else []) + ["--isometric", "--scale=.1", "--stdout-format=" + fmt, "-"],
                              results=dict(go=result, native=copy.deepcopy(result))))
    return calls


class Gates(unittest.TestCase):
    def setUp(self):
        self.calls = fixture()

    def rows(self, execution=None, events=None):
        with tempfile.TemporaryDirectory() as directory:
            return policy.compare(self.calls, execution or dict(exit=0, failure=None),
                                  events if events is not None else {policy.TEST: "pass", **{policy.TEST + "/" + fmt: "pass" for fmt in ["svg", "png", "pdf"]}},
                                  Path(directory))

    def native(self, index=0):
        return self.calls[index]["results"]["native"]

    def test_complete_calls(self):
        self.assertTrue(all(r["outcome"] == "match" for r in self.rows()))

    def test_missing_call(self):
        self.calls.pop()
        with self.assertRaises(AssertionError): self.rows()

    def test_extra_call(self):
        self.calls.append(self.calls[-1])
        with self.assertRaises(AssertionError): self.rows()

    def test_timeout(self):
        self.assertTrue(all(r["outcome"] == "execution-failed" for r in self.rows(dict(exit=-15, failure="timeout"))))

    def test_budget_exceeded(self):
        self.assertTrue(all(r["outcome"] == "execution-failed" for r in self.rows(dict(exit=-15, failure="budget-exceeded"))))

    def test_missing_original_assertion(self):
        self.assertTrue(all(r["outcome"] == "execution-failed" for r in self.rows(events={})))

    def test_original_assertion_failed(self):
        self.assertTrue(all(r["outcome"] == "execution-failed" for r in self.rows(events={policy.TEST: "fail"})))

    def test_default_policy_fetch(self):
        self.native()["requests"] = 1
        self.assertEqual(self.rows()[0]["outcome"], "mismatch")

    def test_permission_did_not_fetch(self):
        self.native(1)["requests"] = 0
        self.assertEqual(self.rows()[1]["outcome"], "mismatch")

    def test_bool_request_count(self):
        self.native()["requests"] = False
        self.assertEqual(self.rows()[0]["outcome"], "mismatch")

    def test_error_is_complete(self):
        self.native()["error"] += " another detail"
        self.assertEqual(self.rows()[0]["outcome"], "mismatch")

    def test_error_cannot_write_stdout(self):
        self.native()["data"] = encoded(b"partial")
        self.assertEqual(self.rows()[0]["outcome"], "mismatch")

    def test_missing_input(self):
        self.native()["files"].clear()
        self.assertEqual(self.rows()[0]["outcome"], "mismatch")

    def test_extra_output(self):
        self.native()["files"]["extra"] = dict(data="", mode=0o600)
        self.assertEqual(self.rows()[0]["outcome"], "mismatch")

    def test_input_mode(self):
        self.native()["files"]["input.d2"]["mode"] = 0o644
        self.assertEqual(self.rows()[0]["outcome"], "mismatch")

    def test_go_policy_failure_never_passes(self):
        self.calls[0]["results"]["go"]["requests"] = 1
        with self.assertRaises(AssertionError): self.rows()

    def test_svg_bytes_not_normalized(self):
        self.native(1)["data"] = encoded(b'<svg xmlns="http://www.w3.org/2000/svg"> </svg>')
        self.assertEqual(self.rows()[1]["outcome"], "mismatch")

    def test_corrupt_svg(self):
        self.native(1)["data"] = encoded(b"<svg>")
        with self.assertRaises(policy.ET.ParseError): self.rows()

    def test_corrupt_png(self):
        self.native(3)["data"] = encoded(b"broken")
        self.assertEqual(self.rows()[3]["outcome"], "mismatch")

    def test_corrupt_pdf(self):
        self.native(5)["data"] = encoded(b"%PDF-1.3\n%%EOF")
        with self.assertRaises(AssertionError): self.rows()

    def test_pdf_bytes_not_normalized(self):
        self.native(5)["data"] = encoded(b"%PDF-1.3\n/Type /Page\n/Subtype /Image\n/CreationDate(changed)\n%%EOF")
        self.assertEqual(self.rows()[5]["outcome"], "mismatch")


if __name__ == "__main__":
    unittest.main()
