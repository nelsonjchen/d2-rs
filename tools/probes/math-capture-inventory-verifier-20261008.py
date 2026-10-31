from pathlib import Path
import json,hashlib,runpy,subprocess,re,xml.etree.ElementTree as ET
root=Path.cwd();base=root/'.reference/general-math-native-20261008';gate=runpy.run_path('scripts/check-render-parity.py');bindings={}
# Decode each immutable output inventory once per case. Reuse the strict shared
# collector and comparator; discard all cached pixels before the next case.
original_collector=gate['collect_png_outputs'];case_frames={}
def collect_once(path):
 key=str(Path(path).resolve())
 if key not in case_frames:case_frames[key]=original_collector(path)
 return case_frames[key]
gate['collect_png_outputs']=collect_once
gate['compare_png_outputs'].__globals__['collect_png_outputs']=collect_once
def bind(p):
 p=Path(p).resolve();assert p.is_file(),p
 digest=hashlib.sha256(p.read_bytes()).hexdigest();bindings[str(p)]=digest;return digest
def read(p):bind(p);return json.loads(Path(p).read_text())
def qualify_report(p):
 d=read(p);assert d['complete'] and d['total']==len(d['checks']) and d['total']>0
 print('verifying',Path(p).name,flush=True)
 for info in d['binaries'].values():assert bind(info['path'])==info['sha256']
 for c in d['checks']:
  case_frames.clear()
  assert bind(c['input'])==c['input_sha256']
  for run in c['runs'].values():
   output=Path(run['command'][-1]);assert run['exit']!=124
   assert (bind(output) if output.exists() else None)==run['output_sha256']
   assert output.with_suffix('.stderr').read_text()==run['stderr'];bind(output.with_suffix('.stderr'));bind(output.with_suffix('.stdout'))
   if run['exit']==0:
    frames=gate['collect_png_outputs'](output);assert frames and set(frames)==set(run['frames'])
    for k,(w,h,rgb) in frames.items():assert run['frames'][k]=={'width':w,'height':h,'rgb_sha256':hashlib.sha256(rgb).hexdigest()}
    if output.with_suffix('').is_dir():
     for image in output.with_suffix('').rglob('*.png'):bind(image)
  s=c['runs']['source'];o=c['runs']['official'];n=c['runs']['native'];assert s['exit']==o['exit']
  if s['exit']==0:
   assert gate['compare_png_outputs'](Path(s['command'][-1]),Path(o['command'][-1]))['matched']
   comparison=gate['compare_png_outputs'](Path(s['command'][-1]),Path(n['command'][-1])) if n['exit']==0 else {'matched':False}
   matched=n['exit']==0 and comparison['matched']
   if not matched:diffs.append({'report':str(p),'name':c['name'],'comparison':comparison})
  else:
   assert s['stderr']==o['stderr'];matched=s['exit']==n['exit'] and s['stderr']==n['stderr']
  assert matched==c['passed']
 assert d['passed']==sum(c['passed'] for c in d['checks'])
 assert d['expected_errors']==sum(c['runs']['source']['exit']!=0 for c in d['checks'])
 return d

import sys
scope=sys.argv[1]
counts={};diffs=[]
bind(__file__);bind(root/'scripts/check-render-parity.py')
if scope=='543':
 progress=read(base/'prior-replay-progress.json');assert progress['complete'] and progress['completed_datasets']==23
 reports={name:info['path'] for name,info in progress['reports'].items()}
 for info in progress['reports'].values():assert bind(info['path'])==info['sha256']
 assert len(reports)==23 and sum(info['total'] for info in progress['reports'].values())==18580
 reports['current-new']=str(base/'authored-qualified-report.json')
 snapshot=read(base/'candidate-cli-source-manifest.json')
 for entry in snapshot['files'].values():assert bind(entry['snapshot'])==entry['sha256']
 destination=base/'capture-inventory-verification-attempt2.json'
else:
 other=root/'.reference/line-precision-native-20261008'
 reports={'new':str(other/'authored-qualified-report.json'),'units-final':str(other/'units-final-line-regression-report.json')}
 destination=other/'capture-inventory-verification.json'
for name,p in reports.items():
 d=qualify_report(p);counts[name]={'passed':d['passed'],'total':d['total'],'expected_errors':d['expected_errors']}
if scope=='543':assert len(counts)==24 and sum(x['total'] for x in counts.values())==40204
certificate={'verified_capture_inventories':True,'scope':scope,'counts':counts,'passed':sum(x['passed'] for x in counts.values()),'total':sum(x['total'] for x in counts.values()),'retained_image_differences':diffs,'file_bindings':bindings,'limits':'Strict independent output/inventory/error verification only. Not aggregate API/build/broad qualification or overall D2 parity. Failures remain failures; goal active.'}
destination.write_text(json.dumps(certificate,indent=2)+'\n')
print(json.dumps({k:v for k,v in certificate.items() if k not in ('file_bindings','retained_image_differences')},indent=2),flush=True)
