#!/usr/bin/env python3
"""Failure gates for original CLI admission observations."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('admission', Path(__file__).with_name('cli-admission-scoreboard.py'))
admission = importlib.util.module_from_spec(spec); spec.loader.exec_module(admission)

def fixture():
    files = {'input.d2': {'data': 'eDp7eX0K', 'mode': 0o600}, 'output.svg': None}
    return [dict(test=name, call=i, input=dict(directory='/capture/work', args=['fmt', 'input.d2'], canceled=i==0, files=copy.deepcopy(files)), result=dict(error='context canceled' if i==0 else 'unsupported', stdout='', files=copy.deepcopy(files))) for i, name in enumerate(admission.NAMES)]

class Gates(unittest.TestCase):
    def test_complete(self):
        calls = fixture()
        self.assertTrue(all(r['outcome'] == 'match' for r in admission.compare(calls, [c['result'] for c in calls])))

    def test_missing_extra_duplicate_reordered_original_call_fails(self):
        for change in ['missing', 'extra', 'duplicate', 'reordered']:
            calls = fixture()
            if change == 'missing': calls.pop()
            elif change == 'extra': calls.append(copy.deepcopy(calls[-1]))
            elif change == 'duplicate': calls[1] = copy.deepcopy(calls[0])
            else: calls.reverse()
            with self.assertRaises(AssertionError): admission.compare(calls, [])

    def test_missing_extra_reordered_native_result_fails(self):
        calls = fixture(); results = [c['result'] for c in calls]
        for wrong in [[], results[:-1], results+[results[-1]], list(reversed(results))]:
            self.assertTrue(any(r['outcome'] == 'mismatch' for r in admission.compare(calls, wrong)))

    def test_exact_error_stdout_bytes_and_mode(self):
        calls = fixture()
        for field in ['error', 'stdout', 'input-bytes', 'mode', 'output']:
            results = copy.deepcopy([c['result'] for c in calls])
            if field in ['error', 'stdout']: results[0][field] += 'changed'
            elif field == 'input-bytes': results[0]['files']['input.d2']['data'] = 'eAo='
            elif field == 'mode': results[0]['files']['input.d2']['mode'] = 0o644
            else: results[0]['files']['output.svg'] = {'data':'eAo=', 'mode':0o600}
            self.assertEqual(admission.compare(calls, results)[0]['outcome'], 'mismatch')

    def test_expected_results_cannot_enter_request(self):
        calls = fixture(); calls[0]['input']['expected'] = calls[0]['result']
        with self.assertRaises(AssertionError): admission.compare(calls, [])

if __name__ == '__main__': unittest.main()
