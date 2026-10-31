#!/usr/bin/env python3
"""Complete original-call inventory, IEEE input and document-output gates."""
import base64
import binascii
import importlib.util
import json
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path
from fixture_recipe import cached_fixture  # oracle-cache artifact (docs/FIXTURES.md)


def module(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + '.py'))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


isometric = module('svg-isometric-scoreboard')
original = module('test_svg_admission_scoreboard')
original.scoreboard = isometric.base
SVG = base64.b64encode(b'<svg xmlns="http://www.w3.org/2000/svg"/>').decode()


class CaptureGate(original.CaptureGate):
    expected_count = 13

    def setUp(self):
        super().setUp()
        for path in (self.capture / 'go').glob('*.json'):
            row = json.loads(path.read_text())
            row.update(test=row['name'].rsplit('#', 1)[0], request={'api': 'Render'},
                       got={'output': SVG, 'error': ''})
            path.write_text(json.dumps(row))
        self.seal()

    def test_full_svg_inventory(self):
        for output in ['', 'garbage', base64.b64encode(b'<svg>').decode(), None]:
            with self.subTest(output=output), self.assertRaises((AssertionError, binascii.Error, ET.ParseError)):
                isometric.validate_response({'api': 'Render'}, {'output': output, 'error': ''})
        for output in [None, [], [SVG, ''], [SVG, None]]:
            with self.subTest(output=output), self.assertRaises((AssertionError, binascii.Error, ET.ParseError)):
                isometric.validate_response({'api': 'RenderMultiboard'}, {'output': output, 'error': ''})
        isometric.validate_response({'api': 'RenderMultiboard'}, {'output': [SVG, SVG], 'error': ''})

    def test_complete_error_envelope(self):
        for response in [{'error': 'error'}, {'error': 400, 'output': None}, {'error': 'error', 'output': SVG}]:
            with self.subTest(response=response), self.assertRaises(AssertionError):
                isometric.validate_response({'api': 'Render'}, response)

    def test_retained_inf_input_and_original_fixture_inventory(self):
        fixture = cached_fixture("crates/d2-rs-isometric/tests/fixtures/svg-isometric-api.json")
        rows = json.loads(fixture.read_text())
        self.assertEqual(len(rows), 13)
        inf = [r for r in rows if '/infinite_scale#' in r['name']]
        self.assertEqual(len(inf), 1)
        self.assertEqual(inf[0]['request']['scaleBits'], 0x7ff0000000000000)
        self.assertIsNone(inf[0]['request']['opts']['Scale'])
        for row in rows:
            isometric.validate_response(row['request'], row['got'])


if __name__ == '__main__':
    unittest.main()
