"""Contracts for the direct-engine graphjson parity projection."""
import importlib.util
from pathlib import Path
import unittest

from exact_json import decode_output, difference

_spec = importlib.util.spec_from_file_location(
    "check_graphjson_layout", Path(__file__).with_name("check-graphjson-layout.py")
)
_module = importlib.util.module_from_spec(_spec)
assert _spec.loader is not None
_spec.loader.exec_module(_module)
normalized = _module.normalized


class GraphJsonLayoutTests(unittest.TestCase):
    def test_projection_compares_label_icon_route_and_geometry_fields(self):
        go = decode_output(
            b'{"nodes":[{"id":-7,"width":80,"height":40,"topLeft":{"x":-0,"y":4},'
            b'"Label":{"Position":"INSIDE_MIDDLE_CENTER",'
            b'"Width":12,"Height":8},"Icon":{"Position":"OUTSIDE_RIGHT_MIDDLE"}}],'
            b'"edges":[{"id":9,"from":-7,"to":-7,"points":[{"x":1,"y":2}],'
            b'"LabelPercentage":0.25,"Label":{"Position":"OUTSIDE_TOP_CENTER",'
            b'"Width":16,"Height":10}}]}'
        )
        rust = decode_output(
            b'{"nodes":[{"id":-7,"width":80.0,"height":40.0,"top_left":{"x":-0.0,"y":4.0},'
            b'"label":{"position":"INSIDE_MIDDLE_CENTER",'
            b'"width":12.0,"height":8.0},"icon":{"position":"OUTSIDE_RIGHT_MIDDLE"}}],'
            b'"edges":[{"id":9,"from":-7,"to":-7,"points":[{"x":1.0,"y":2.0}],'
            b'"label_percentage":0.25,"label":{"position":"OUTSIDE_TOP_CENTER",'
            b'"width":16.0,"height":10.0}}]}'
        )
        self.assertIsNone(difference(normalized(go), normalized(rust)))

    def test_projection_detects_signed_zero_and_label_divergences(self):
        left = normalized(decode_output(b'{"nodes":[{"id":1,"width":1,"height":1,"topLeft":{"x":-0,"y":0}}]}'))
        right = normalized(decode_output(b'{"nodes":[{"id":1,"width":1,"height":1,"top_left":{"x":0,"y":0}}]}'))
        self.assertIsNotNone(difference(left, right))

        left = normalized(decode_output(b'{"edges":[{"id":1,"from":1,"to":2,"Label":{"Position":"INSIDE_MIDDLE_CENTER"}}]}'))
        right = normalized(decode_output(b'{"edges":[{"id":1,"from":1,"to":2,"label":{"position":"OUTSIDE_TOP_CENTER"}}]}'))
        self.assertIsNotNone(difference(left, right))

    def test_projection_aligns_stable_ids_and_keeps_duplicate_multiplicity(self):
        first = b'{"id":1,"width":1,"height":1,"topLeft":null}'
        second = b'{"id":1,"width":2,"height":1,"topLeft":null}'
        left = normalized(decode_output(b'{"nodes":[' + first + b"," + second + b"]}"))
        same = normalized(decode_output(b'{"nodes":[' + first + b"," + second + b"]}"))
        reversed_items = normalized(decode_output(b'{"nodes":[' + second + b"," + first + b"]}"))
        self.assertIsNone(difference(left, same))
        self.assertIsNotNone(difference(left, reversed_items))
        fewer_duplicates = normalized(decode_output(b'{"nodes":[' + first + b"]}"))
        self.assertIsNotNone(difference(left, fewer_duplicates))

        different_ids = normalized(decode_output(b'{"nodes":[{"id":2,"width":2,"height":1},{"id":1,"width":1,"height":1}]}'))
        reordered_ids = normalized(decode_output(b'{"nodes":[{"id":1,"width":1,"height":1},{"id":2,"width":2,"height":1}]}'))
        self.assertIsNone(difference(different_ids, reordered_ids))


if __name__ == "__main__":
    unittest.main()
