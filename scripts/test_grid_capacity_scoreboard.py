#!/usr/bin/env python3
"""Fail-closed inventory and complete-result comparison for Grid admission."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('grid', Path(__file__).with_name('grid-capacity-scoreboard.py'))
grid = importlib.util.module_from_spec(spec); spec.loader.exec_module(grid)

class GateTests(unittest.TestCase):
    def setUp(self):
        self.calls = [dict(test=name, input=dict(operation='construct', rows=1000,columns=1000,objects=1),
            result=dict(error='',rows=1000,columns=1000,occupied_rows=1,occupied_columns=1))
            for name,count in grid.TESTS.items() for _ in range(count)]
        self.results = [c['result'] for c in self.calls]

    def test_complete_results_match(self):
        self.assertTrue(all(r['outcome']=='match' for r in grid.compare(self.calls,self.results)))

    def test_missing_extra_original_calls_fail(self):
        for calls in [self.calls[:-1],self.calls+[self.calls[0]]]:
            with self.assertRaises(AssertionError): grid.compare(calls,self.results)

    def test_missing_extra_native_results_do_not_pass(self):
        for results in [[],self.results[:-1],self.results+[self.results[0]]]:
            self.assertTrue(all(r['outcome']=='mismatch' for r in grid.compare(self.calls,results)))

    def test_full_error_and_occupied_extent_compare(self):
        for replacement in [dict(error='truncated error'),dict(error='',rows=1000,columns=1000,occupied_rows=1000,occupied_columns=1000)]:
            results = copy.deepcopy(self.results); results[-1] = replacement
            self.assertEqual(grid.compare(self.calls,results)[-1]['outcome'],'mismatch')

    def test_expectations_never_enter_request(self):
        calls = copy.deepcopy(self.calls); calls[0]['input']['expected_error']=''
        with self.assertRaises(AssertionError): grid.compare(calls,self.results)

    def test_boolean_is_not_a_dimension(self):
        results = copy.deepcopy(self.results); results[-1]['occupied_rows'] = True
        self.assertEqual(grid.compare(self.calls,results)[-1]['outcome'],'mismatch')

if __name__ == '__main__': unittest.main()
