#!/usr/bin/env python3
import copy
import importlib.util
from pathlib import Path
import unittest
spec=importlib.util.spec_from_file_location('score',Path(__file__).with_name('elk-options-scoreboard.py'))
score=importlib.util.module_from_spec(spec);spec.loader.exec_module(score)

class OriginalInventory(unittest.TestCase):
    def setUp(self):
        self.calls=[dict(test=test,input=dict(kind=kind,options=None),result={'value':i})
                    for i,(test,kind) in enumerate((test,kind) for test,count in score.TESTS.items()
                    for kind in (['edge-label','leaf','port'] if count==3 else ['container','root']))]
        self.results=[c['result'].copy() for c in self.calls]
    def test_exact(self):
        self.assertTrue(all(r['outcome']=='match' for r in score.compare(self.calls,self.results)))
    def test_missing_extra_duplicate(self):
        for calls in [self.calls[:-1],self.calls+[self.calls[0]],self.calls[:-1]+[self.calls[0]]]:
            with self.assertRaises(AssertionError):score.compare(calls,self.results)
    def test_wrong_kind(self):
        calls=copy.deepcopy(self.calls);calls[0]['input']['kind']='port'
        with self.assertRaises(AssertionError):score.compare(calls,self.results)
    def test_missing_or_corrupt_native(self):
        self.assertTrue(all(r['outcome']=='mismatch' for r in score.compare(self.calls,[])))
        self.results[0]['value']=-1
        self.assertEqual(score.compare(self.calls,self.results)[0]['outcome'],'mismatch')
    def test_expected_data_rejected(self):
        self.calls[0]['input']['result']=self.results[0]
        with self.assertRaises(AssertionError):score.compare(self.calls,self.results)

if __name__=='__main__':unittest.main()
