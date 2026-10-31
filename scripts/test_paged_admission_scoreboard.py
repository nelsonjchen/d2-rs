#!/usr/bin/env python3
"""Complete original call inventory, exact budget/error/output failure gates."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('paged',Path(__file__).with_name('paged-admission-scoreboard.py'))
paged = importlib.util.module_from_spec(spec); spec.loader.exec_module(paged)

def fixture():
    calls = []
    for i,name in enumerate(paged.NAMES):
        if i < 7:
            request = dict(kind='page',budget=[16000,14,2,100],width=100,height=100,canceled=False,page={})
            result = dict(error='',budget=[8000,7,1,81])
        else:
            request = dict(kind='write',remaining=2,canceled=i==8,data='eA==')
            result = dict(error='context canceled' if i==8 else 'limit',remaining=2,n=0,output='')
        calls.append(dict(test=name,call=i,input=request,result=result))
    return calls

class Gates(unittest.TestCase):
    def test_complete(self):
        calls=fixture()
        self.assertEqual(paged.compare(calls,[c['result'] for c in calls])[0]['outcome'],'match')

    def test_original_inventory_rejects_missing_duplicate_extra_reordered(self):
        for change in ['missing','duplicate','extra','reordered','subtest']:
            calls=fixture()
            if change=='missing':calls.pop()
            elif change=='duplicate':calls[1]['call']=0
            elif change=='extra':calls.append(copy.deepcopy(calls[-1]))
            elif change=='reordered':calls[0],calls[1]=calls[1],calls[0]
            else:calls[3]['test']=paged.TEST
            with self.assertRaises(AssertionError):paged.compare(calls,[])

    def test_native_missing_extra_reordered_fail(self):
        calls=fixture(); original=[c['result'] for c in calls]
        for results in [[],original[:-1],original+[original[-1]],list(reversed(original))]:
            self.assertEqual(paged.compare(calls,results)[0]['outcome'],'mismatch')

    def test_budget_full_error_byte_and_count_changes_fail(self):
        calls=fixture()
        for index,field,value in [(0,'budget',[0,0,0,0]),(7,'error','changed'),(8,'output','eA=='),(7,'n',1),(8,'remaining',1)]:
            results=copy.deepcopy([c['result'] for c in calls]);results[index][field]=value
            self.assertEqual(paged.compare(calls,results)[0]['outcome'],'mismatch')

    def test_expected_cannot_enter_native_request(self):
        calls=fixture();calls[0]['input']['expected']=calls[0]['result']
        with self.assertRaises(AssertionError):paged.compare(calls,[])

if __name__=='__main__':unittest.main()
