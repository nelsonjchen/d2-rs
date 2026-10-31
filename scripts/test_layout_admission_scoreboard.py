#!/usr/bin/env python3
"""Failure gates for complete original graph-admission inventories and results."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('admission', Path(__file__).with_name('layout-admission-scoreboard.py'))
admission = importlib.util.module_from_spec(spec); spec.loader.exec_module(admission)

def fixture():
    return [dict(package=p, test=t, call=i, input=dict(engine=e, objects=2, edges=[[0, 1]], canceled=False), result=dict(error=f'{p}/{t}/{i}')) for p, (_, e, tests) in admission.PACKAGES.items() for t, count in tests.items() for i in range(count)]

class Gates(unittest.TestCase):
    def test_complete(self):
        calls = fixture()
        self.assertEqual(len(admission.compare(calls, [c['result'] for c in calls])), 12)
        self.assertTrue(all(r['outcome'] == 'match' for r in admission.compare(calls, [c['result'] for c in calls])))

    def test_missing_extra_duplicate_original(self):
        for change in ['missing', 'extra', 'duplicate', 'reordered']:
            calls = fixture()
            if change == 'missing': calls.pop()
            elif change == 'extra': calls.append(copy.deepcopy(calls[-1]))
            elif change == 'duplicate': calls[5] = copy.deepcopy(calls[4])
            else: calls[5], calls[6] = calls[6], calls[5]
            with self.assertRaises(AssertionError): admission.compare(calls, [])

    def test_missing_extra_reordered_native(self):
        calls = fixture(); results = [c['result'] for c in calls]
        for wrong in [results[:-1], results+[results[-1]], list(reversed(results))]:
            self.assertTrue(any(r['outcome'] == 'mismatch' for r in admission.compare(calls, wrong)))

    def test_complete_errors_and_native_failure(self):
        calls = fixture()
        for value in [{'error':'changed'}, {'failure':{'exit':None, 'failure':'timeout'}}, {'error':calls[0]['result']['error'], 'extra':True}]:
            results = copy.deepcopy([c['result'] for c in calls]); results[0] = value
            self.assertEqual(admission.compare(calls, results)[0]['outcome'], 'mismatch')

    def test_expected_outputs_cannot_enter_requests(self):
        calls = fixture(); calls[0]['input']['expected'] = calls[0]['result']
        with self.assertRaises(AssertionError): admission.compare(calls, [])

if __name__ == '__main__': unittest.main()
