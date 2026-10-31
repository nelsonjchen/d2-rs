#!/usr/bin/env python3
"""Failure gates for the original compound cancellation observations."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('cancellation', Path(__file__).with_name('tala-cancellation-scoreboard.py'))
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)

class Gates(unittest.TestCase):
    def setUp(self):
        graph = dict(nodes=[{'point':{'x':1,'y':2}}]*9, edges=[{'points':[{'x':3,'y':4}]}]*6)
        self.calls = []
        self.results = []
        for test, count in m.TESTS.items():
            for index in range(count):
                pre = test.endswith('RoutedInterior')
                probe = not pre and index == 0
                cancel = {'kind':'pre'} if pre else {'kind':'probe'} if probe else {'kind':'fraction','fraction_index':index-1}
                schedule = {} if pre else {'checks_before':0,'checks_after':30 if probe else [10,20,29][index-1], 'limit':0 if probe else [10,20,29][index-1]}
                result = dict(graph=copy.deepcopy(graph) if probe else None, cause='' if probe else 'context canceled', input_after=copy.deepcopy(graph))
                self.calls.append(dict(test=test,call=index,input=dict(graph=dict(wire={},view=copy.deepcopy(graph)),cancel=cancel),source_schedule=schedule,diagnostic='' if probe else 'EdgeRouting: context canceled',result=result))
                native = dict(checks=1 if pre else 12 if probe else [4,8,11][index-1],limit=0 if pre or probe else [4,8,11][index-1],probe_checks=0 if pre or probe else 12)
                self.results.append(dict(result=copy.deepcopy(result),diagnostic=result['cause'],native_schedule=native))

    def test_complete_inventory_and_distinct_poll_schedules(self):
        self.assertEqual([r['outcome'] for r in m.compare(self.calls,self.results)],['match']*2)

    def test_missing_extra_duplicate_or_empty_calls_rejected(self):
        for calls in [[],self.calls[:-1],self.calls+[self.calls[-1]]]:
            with self.assertRaises(AssertionError):m.compare(calls,self.results)

    def test_missing_extra_or_empty_native_results_fail(self):
        for results in [[],self.results[:-1],self.results+[self.results[-1]]]:
            self.assertTrue(all(r['outcome']=='mismatch' for r in m.compare(self.calls,results)))

    def test_geometry_input_null_candidate_and_cause_compared(self):
        for call_index,key,value in [(0,'graph',{}),(0,'cause',''),(0,'input_after',{}),(1,'graph',{}),(1,'cause','context canceled')]:
            changed=copy.deepcopy(self.results);changed[call_index]['result'][key]=value
            self.assertIn('mismatch',[r['outcome'] for r in m.compare(self.calls,changed)])

    def test_native_missing_or_unfired_cancellation_boundary_fails(self):
        for key,value in [('checks',0),('limit',True),('probe_checks',0)]:
            changed=copy.deepcopy(self.results);changed[-1]['native_schedule'][key]=value
            self.assertEqual(m.compare(self.calls,changed)[1]['outcome'],'mismatch')

    def test_source_fraction_recipe_and_input_preservation_required(self):
        for key,value in [('source_schedule',{'checks_before':0,'checks_after':7,'limit':7}),('result',dict(graph=None,cause='context canceled',input_after={}))]:
            calls=copy.deepcopy(self.calls);calls[-1][key]=value
            with self.assertRaises(AssertionError):m.compare(calls,self.results)

    def test_expected_output_in_native_request_rejected(self):
        self.calls[0]['input']['expected']=self.calls[0]['result']
        with self.assertRaises(AssertionError):m.compare(self.calls,self.results)

    def test_transport_preserves_values_and_raw_capture(self):
        original=copy.deepcopy(self.calls[0]['input']);converted=m.native_input(original)
        self.assertTrue(m.same_json(original,converted))
        self.assertIs(type(original['graph']['view']['nodes'][0]['point']['x']),int)
        self.assertIs(type(converted['graph']['view']['nodes'][0]['point']['x']),float)
        for bad in [True,2**53+1]:
            changed=copy.deepcopy(original);changed['graph']['view']['nodes'][0]['point']['x']=bad
            with self.assertRaises(AssertionError):m.native_input(changed)

if __name__=='__main__':unittest.main()
