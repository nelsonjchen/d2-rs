#!/usr/bin/env python3
"""Failure gates for the original residual contract batch."""
import copy
import importlib.util
import tempfile
import unittest
from pathlib import Path
spec=importlib.util.spec_from_file_location('residual',Path(__file__).with_name('isometric-residual-scoreboard.py'))
suite=importlib.util.module_from_spec(spec);spec.loader.exec_module(suite)
class ResidualGates(unittest.TestCase):
 def setUp(self):
  self.score=suite.scoreboard()
  self.cases=[{'name':name,'operations':[{'request':{'op':op},'got':{}} for op,n in ops.items() for _ in range(n)]} for name,ops in suite.TESTS.items()]
 def test_exact_original_inventory(self):
  self.assertEqual(len(self.cases),20)
  self.score.validate_cases(self.cases)
  for cases in [[],self.cases[:-1],self.cases+self.cases]:
   with self.assertRaises(AssertionError):self.score.validate_cases(cases)
  for action in ['missing','extra','unknown']:
   cases=copy.deepcopy(self.cases);ops=cases[0]['operations']
   if action=='missing':ops.pop()
   elif action=='extra':ops.append(ops[0])
   else:ops[0]['request']['op']='unknown'
   with self.assertRaises(AssertionError):self.score.validate_cases(cases)
 def test_every_pixel_operation_in_inventory_gate(self):
  self.assertEqual(self.score.PIXEL_OPS,{'render','native-frame','draw'})
 def test_changed_shadow_packet_mesh_or_admission_fails(self):
  value={'error':'','frame':{'sha256':'original'},'shadow':{'depth':'a','opacity':'b'},'packet':{'points':[1.]},'mesh':{'owner':1}}
  self.assertEqual(self.score.outcome(value,value),'match')
  for keys in [('error',),('frame','sha256'),('shadow','depth'),('shadow','opacity'),('packet','points',0),('mesh','owner')]:
   changed=copy.deepcopy(value);current=changed
   for key in keys[:-1]:current=current[key]
   current[keys[-1]]='changed'
   self.assertEqual(self.score.outcome(value,changed),'mismatch')
if __name__=='__main__':unittest.main()
