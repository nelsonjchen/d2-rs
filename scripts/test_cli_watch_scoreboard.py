#!/usr/bin/env python3
import copy
import importlib.util
from pathlib import Path
import unittest

spec=importlib.util.spec_from_file_location('watch_score',Path(__file__).with_name('cli-watch-scoreboard.py'))
score=importlib.util.module_from_spec(spec);spec.loader.exec_module(score)

class WatchContract(unittest.TestCase):
    def setUp(self):
        self.calls=[]
        for name in score.NAMES:
            token=name==score.TESTS[0]
            request={'operation':'token'} if token else dict(operation='authorize',token='correct-token',cookie_name='d2-watch-12345',target='/',cookie='')
            result=dict(error='',length=43,url_safe=True,distinct=True) if token else dict(allowed=False,code=403,headers={'X-Content-Type-Options':['nosniff']},body='watch access denied\n')
            self.calls.append(dict(test=name,call=len(self.calls),input=request,result=result))
        self.results=[copy.deepcopy(c['result']) for c in self.calls]
    def test_complete_inventory(self):
        self.assertTrue(all(r['outcome']=='match' for r in score.compare(self.calls,self.results)))
        self.assertEqual([r['calls'] for r in score.compare(self.calls,self.results)],[2,5,12])
    def test_missing_duplicate_and_extra_fail(self):
        for calls in [self.calls[:-1],self.calls+[self.calls[0]],self.calls[:-1]+[self.calls[0]]]:
            with self.assertRaises(AssertionError):score.compare(calls,self.results)
        for results in [[],self.results[:-1],self.results+[self.results[0]]]:
            self.assertTrue(all(r['outcome']=='mismatch' for r in score.compare(self.calls,results)))
    def test_every_response_field_compares(self):
        for index in [0,2,7]:
            for field in self.results[index]:
                results=copy.deepcopy(self.results);results[index][field]=None
                rows=score.compare(self.calls,results)
                self.assertEqual(rows[0 if index==0 else 1 if index==2 else 2]['outcome'],'mismatch')
    def test_expected_output_rejected_in_input(self):
        self.calls[2]['input']['expected']=self.results[2]
        with self.assertRaises(AssertionError):score.compare(self.calls,self.results)
    def test_witness_cannot_credit_original(self):
        results=copy.deepcopy(self.results);results[2]['allowed']=True
        rows=score.compare(self.calls,results)
        self.assertEqual(rows[1]['outcome'],'mismatch')
        self.assertEqual(rows[2]['outcome'],'match')

if __name__=='__main__':unittest.main()
