#!/usr/bin/env python3
"""Original-call inventory, registration-input and stylesheet/error gates."""
import importlib.util
import json
import unittest
from pathlib import Path


def module(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + '.py'))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


fonts = module('svg-font-scoreboard')
original = module('test_svg_admission_scoreboard')
original.scoreboard = fonts.base


class CaptureGate(original.CaptureGate):
    expected_count = 6

    def setUp(self):
        super().setUp()
        fixture = Path(__file__).resolve().parents[1] / 'crates/d2-rs-render/tests/fixtures/svg-font-api.json'
        cases = json.loads(fixture.read_text())
        for path in (self.capture / 'go').glob('*.json'):
            path.unlink()
        for i, case in enumerate(cases):
            (self.capture / 'go' / f'{i}.json').write_text(json.dumps(case))
        (self.capture / 'inventory.json').write_text(json.dumps({c['test']: 1 for c in cases}))
        self.seal()

    def test_registration_is_input(self):
        path = next(p for p in (self.capture / 'go').glob('*.json') if json.loads(p.read_text())['request']['api'] == 'Render')
        case = json.loads(path.read_text())
        case['request']['families'] = []
        path.write_text(json.dumps(case))
        self.seal()
        with self.assertRaises(AssertionError):
            fonts.base.verify(self.capture)

    def test_theme_error_returns_empty_stylesheet(self):
        for response in [{'error': 'bad', 'output': 'stylesheet'}, {'error': 400, 'output': ''}, {'error': 'bad'}]:
            with self.subTest(response=response), self.assertRaises(AssertionError):
                fonts.validate_case({'name':'TestTheme#0', 'test':'TestTheme', 'request':{'api':'ThemeCSS'}, 'got':response})

    def test_render_document_is_complete(self):
        case = next(json.loads(p.read_text()) for p in (self.capture / 'go').glob('*.json') if json.loads(p.read_text())['request']['api'] == 'Render')
        case['got']['output'] = ''
        with self.assertRaises(AssertionError):
            fonts.validate_case(case)


if __name__ == '__main__':
    unittest.main()
