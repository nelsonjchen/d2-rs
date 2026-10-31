#!/usr/bin/env python3
"""Fail-closed original search inventories and ordered partition comparison."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('grid', Path(__file__).with_name('grid-search-scoreboard.py'))
grid = importlib.util.module_from_spec(spec); spec.loader.exec_module(grid)


class GateTests(unittest.TestCase):
    def setUp(self):
        self.calls = [dict(test=name,input=dict(values=[1.,2.,3.],ids=['a','b','c'],groups=2,gap=40,target=23.),
            result=[['a'],['b','c']]) for name in sorted(grid.original_names())]
        self.results = [c['result'] for c in self.calls]

    def test_complete_results_match(self):
        self.assertTrue(all(r['outcome']=='match' for r in grid.compare(self.calls,self.results)))

    def test_missing_extra_duplicate_original_calls_fail(self):
        for calls in [self.calls[:-1],self.calls+[self.calls[0]],self.calls[:-1]+[self.calls[0]]]:
            with self.assertRaises(AssertionError): grid.compare(calls,self.results)

    def test_wrong_original_subtest_does_not_pass(self):
        calls = copy.deepcopy(self.calls); calls[0]['test']+='-unknown'
        with self.assertRaises(AssertionError): grid.compare(calls,self.results)

    def test_missing_extra_native_results_do_not_pass(self):
        for results in [[],self.results[:-1],self.results+[self.results[0]]]:
            self.assertTrue(all(r['outcome']=='mismatch' for r in grid.compare(self.calls,results)))

    def test_partition_order_and_contents_compare(self):
        for replacement in [[['a'],['c','b']],[['a','b'],['c']],[['a'],['b']]]:
            results = copy.deepcopy(self.results); results[-1] = replacement
            self.assertTrue(any(row['outcome']=='mismatch' for row in grid.compare(self.calls,results)))

    def test_expectations_never_enter_request(self):
        calls = copy.deepcopy(self.calls); calls[0]['input']['expected'] = calls[0]['result']
        with self.assertRaises(AssertionError): grid.compare(calls,self.results)


if __name__ == '__main__': unittest.main()
