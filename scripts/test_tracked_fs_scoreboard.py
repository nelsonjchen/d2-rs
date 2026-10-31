#!/usr/bin/env python3
"""Original function/call inventory and complete result comparison gates."""
import copy
import importlib.util
from pathlib import Path
import unittest
spec=importlib.util.spec_from_file_location('tracked',Path(__file__).with_name('tracked-fs-scoreboard.py'))
tracked=importlib.util.module_from_spec(spec);spec.loader.exec_module(tracked)

def fixture():
    return {name:[dict(test=name,call=i,input=dict(policy='unrestricted',root='',path='/input'),result=dict(error='',denied=False,opened=['/input']),opened=['/input']) for i in range(count)] for name,count in zip(tracked.TESTS,tracked.COUNTS)}

class Gates(unittest.TestCase):
    def test_complete(self):
        value=fixture();self.assertTrue(all(r['outcome']=='match' for r in tracked.compare(value,value)))
    def test_missing_extra_native(self):
        value=fixture();del value[tracked.TESTS[0]]
        self.assertEqual(tracked.compare(fixture(),value)[0]['outcome'],'mismatch')
        value['extra']=[]
        with self.assertRaises(AssertionError):tracked.compare(fixture(),value)
    def test_missing_duplicate_original(self):
        for kind in ['function','call','duplicate']:
            value=fixture();name=tracked.TESTS[0]
            if kind=='function':del value[name]
            elif kind=='call':value[name].pop()
            else:value[name][1]['call']=0
            with self.assertRaises(AssertionError):tracked.compare(value,fixture())
    def test_changed_result_and_dependency_fail(self):
        for field in ['error','denied','opened']:
            value=fixture();value[tracked.TESTS[0]][0]['result'][field]='changed'
            self.assertEqual(tracked.compare(fixture(),value)[0]['outcome'],'mismatch')
        value=fixture();value[tracked.TESTS[0]][0]['opened'].append('/failed-open')
        self.assertEqual(tracked.compare(fixture(),value)[0]['outcome'],'mismatch')
    def test_partial_and_extra_calls_fail(self):
        for op in ['remove','extra']:
            value=fixture();calls=value[tracked.TESTS[0]]
            if op=='remove':calls.pop()
            else:calls.append(copy.deepcopy(calls[-1]))
            self.assertEqual(tracked.compare(fixture(),value)[0]['outcome'],'mismatch')

if __name__=='__main__':unittest.main()
