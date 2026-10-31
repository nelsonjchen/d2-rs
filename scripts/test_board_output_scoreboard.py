#!/usr/bin/env python3
"""Complete board output inventory/hash gates, including filesystem aliases."""

import base64
import copy
import hashlib
import importlib.util
import json
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "boards", Path(__file__).with_name("board-output-scoreboard.py")
)
boards = importlib.util.module_from_spec(spec)
spec.loader.exec_module(boards)


def row():
    data = b"<svg/>"
    name = "output/e\u0301.svg"
    disk_name = "output/é.svg"
    manifest = {
        "format": "d2-board-output",
        "version": 1,
        "files": [{"path": "e\u0301.svg", "sha256": hashlib.sha256(data).hexdigest()}],
    }
    encode = lambda value: base64.b64encode(value).decode()
    return {
        "name": "alias",
        "error": "",
        "ownedErrors": {},
        "files": {
            disk_name: encode(data),
            "output/.d2-board-output-manifest-v1.json": encode(
                json.dumps(manifest).encode()
            ),
        },
        "owned": {name: encode(data)},
    }


class Gates(unittest.TestCase):
    def test_preserves_disk_and_declared_spellings(self):
        files, owned = boards.validate_files(row())
        self.assertIn("output/é.svg", files)
        self.assertIn("output/e\u0301.svg", owned)
        altered = copy.deepcopy(row())
        altered["files"]["extra.svg"] = "eA=="
        self.assertNotEqual(boards.validate_files(altered), (files, owned))

    def test_missing_owned_output_fails(self):
        bad = row()
        bad["owned"] = {}
        with self.assertRaises(AssertionError):
            boards.validate_files(bad)

    def test_empty_or_changed_bytes_fail(self):
        for data in ["", "eA=="]:
            bad = row()
            bad["owned"]["output/e\u0301.svg"] = data
            with self.assertRaises(AssertionError):
                boards.validate_files(bad)

    def test_read_errors_fail(self):
        bad = row()
        bad["ownedErrors"] = {"output/e\u0301.svg": "missing"}
        with self.assertRaises(AssertionError):
            boards.validate_files(bad)

    def test_missing_or_corrupt_manifest_fails(self):
        for data in [None, "eA=="]:
            bad = row()
            if data is None:
                del bad["files"]["output/.d2-board-output-manifest-v1.json"]
            else:
                bad["files"]["output/.d2-board-output-manifest-v1.json"] = data
            with self.assertRaises((AssertionError, ValueError)):
                boards.validate_files(bad)


if __name__ == "__main__":
    unittest.main()
