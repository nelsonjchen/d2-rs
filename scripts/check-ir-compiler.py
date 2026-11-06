#!/usr/bin/env python3
"""Capture AST-to-IR compilation with key-level enter/exit traces.

Fixture generator for recipe `lang-ir-compiler` (tools/fixtures/langlibelk-language.toml):
builds tools/field-oracle with the compileKey trace overlay at the manifest
pin, compiles the constructed corpus below plus every literal compiler call
mined from the pinned d2ir/d2compiler tests, and writes ir-compiler-trace.json
to the staged output. The plain fixture is not cached separately: it is this
capture without the `trace` key, which is asserted here against an
uninstrumented build. `cargo test -p d2-rs-language --features oracle-fixtures`
does the native comparison (docs/FIXTURES.md).
"""
from language_oracle import call, field_oracle, fixture, upstream, write
source = upstream()
go = field_oracle(trace=True)
texts=['','a','a: hello','a.b: 2; a.c: true','a: { b: 1; c: two }; a.b: 3',
 'a: old; a: null; a: new','a: {b}; a: null','vars: { a: null; b: true }',
 'a: [1; true; null; "hi"; [2;3]; {x:y}]','a: hello { b: hi }; a: 3',
 'a: {b: 1}; a: [2]; a.c: 4','a: [1]; *.x: y','a: {_.b: 3}',
 '_: x','_: {b}','a: {_.x: 1; _: 2}','a.width.x: y','a.classes.x: 2',
 'a: suspend; b: unsuspend','a: {b}; a: suspend','a: suspend; a: unsuspend',
 'a: {style.fill: red}; a.style: suspend','a.link: layers.one','a.link: _.layers.one',
 'a.link: https://example.com','a.link: "\'layers\'.one"','a.link: ſteps.one',
 'a; b; *.label: yes','*.label: yes; a; b','**.style.fill: red; a.b; a.c; d',
 '**.style.fill: red; a: {b; c}; a.b.style.fill: blue; d',
 '**.child; a; b; c','a; *.x: {y: 2}; b','a; b; *.x: null',
 'a: [ { b: 2 }; { c: 3 } ]; **.style.fill: red','a: {b: null}; **.x: yes',
 '**.style.fill: red; table: {shape: sql_table; a: b}',
 'x: { **.style.fill: green; a }; y: {b}','a; *.shape: circle; a.shape: null; b',
 'a; *.style.fill: yellow; *.style.fill: red; b','*.label: first; a: explicit; b',
 '# comment\na: 1\n# trailing','a: [# comment\n1; 2]',
 'a -> b','layers.x: {a}','a: ${x}','*: {&shape: circle}',
 'vars.d2-config.layout-engine: tala']
texts += [
 'a: base; label: root label; scenarios: {one: {b}; two: {c}}',
 'a: base; steps: {one: {b}; two: {c}; three: {a: changed}}',
 'a; steps: {one: caption; two: {b}; three: {c}}',
 'a; steps: {one: {b}; two: null; three: {c}}',
 'a; layers: {one: {b}; two: {c}}',
 '*.shape: circle; a; steps: {one: {b}; two: {c}; three: {d}}',
 '**.shape: circle; a; scenarios: {one: {b}; two: {c}; three: {d}}',
 '***.shape: circle; a; scenarios: {one: {b}; two: {c}; three: {d}}',
 '**.shape: circle; ***.style.fill: red; a; layers: {one: {b}; two: {c}}',
 '**.x; a; steps: {one: {b}; two: {c}}; scenarios: {one: {d}; two: {e}}',
 '***.style.fill: red; scenarios: {one: {**.style.fill: blue; x}; two: {y}}',
 '***.style.fill: red; steps: {one: {**.style.fill: blue; x}; two: {y}}',
 'classes: {one: {style.fill: red}; two: {shape: circle}}; layers: {alpha: {a}; beta: labeled {b}; gamma: {classes.one.style.stroke: blue; c}}',
 'classes.one.style.fill: red; layers.one.layers.two: {a}',
 'classes.one.style.fill: red; layers: {one: {classes: simple; a}; two: {a}}',
 'classes: label; layers: {one: {a}; two: {b}}',
 'classes.one.style.fill: red; scenarios: {one: {layers.child: {a}}}',
 'a; scenarios.one: {steps: {a: {x}; b: {y}}}; scenarios.two: {z}',
 'a; layers.one: {x; scenarios: {a: {y}; b: {z}}}',
 '***.label: glob; a: explicit; layers.one: {b}; scenarios.one: {c}; steps.one: {d}',
 'layers.one: {x.link: _.layers.two}; layers.two: {y.link: _.scenarios.three}',
 'a; Layers.one: {b}; "layers".two: {c}',
 'a; layers: {one: {b}}; layers.one: {c}',
 'a; scenarios: {one: {b}}; scenarios.one: {c}',
 'a; steps: {one: {b}}; steps.one: {c}',
]
texts += [
 'a -> b -> c: chain',
 'a -> b; a -> b; (a -> b)[0]: first; (a -> b)[1]: second',
 'a <-> b; b <-> a; (a <-> b)[*].style.fill: red',
 'a -- b; b -- a; (a -- b)[1]: next',
 'a -> a: loop; b; * -> *',
 'a -> b: [1;2]',
 'a -> b: { c -> d }',
 '(a -> b)[0]: missing',
 'a -> b; (a -> b)[2].style.fill: red',
 'a -> b; a -> b; (a -> b)[0]: null; a -> b',
 'a -> b; b -> c; (* -> *)[*]: null',
 'a -> b: {style.stroke: red}; (a -> b)[0].style: null',
 'a -> b; (a -> b)[0]: suspend',
 'a -> b; a: suspend; (a -> b)[0]: unsuspend',
 'x: {a -> b}; x: suspend; x.(a -> b)[0]: unsuspend',
 'x: {a -> b}; x.a: suspend; x.(a -> b)[0]: unsuspend',
 '(* -> *)[*]: suspend; a -> b',
 'a -> b: explicit; (* -> *)[*]: glob; c -> d',
 '(* -> *)[*]: glob; a -> b: explicit; c -> d',
 'a.b -> a.c; a.(b -> c)[0]: changed',
 'a.a.b -> a.a.c; a.a.(b -> c)[0].style.opacity: 0.5',
 'a: [1]; a.b -> a.c',
 'a: [1]; a.(b -> c)[0]: missing',
 'a: {b; _.c -> b}',
 '_.a -> b',
 'a -> shape; "shape" -> a',
 'a -> style.fill; a -> "style".fill',
 'a -> layers',
 'layers.one: {a}; layers.two: {b}; layers.one.a -> layers.two.b',
 'layers.one: {a}; layers.one -> a',
 'a -> b; steps: {one: {(a -> b)[0]: suspend}; two: {(a -> b)[0]: unsuspend}}',
 '*** -> ***; a; b; layers.one: {c; d}; scenarios.one: {e}',
 '** -> sink; c: {x}; d: {shape: circle}; "e": {shape: circle}',
 'a.b; a.c; a.* -> a.*',
 '(*** -> ***)[*].style.opacity: 0.3; a -> b; layers.one: {c -> d}',
]
# Exercise computed/default predicates as well as upstream's literal filter cases.
for prop, default in [('shape','rectangle'),('style.border-radius','0'),('style.stroke-dash','0'),('style.opacity','1'),('style.stroke-width','2'),('icon','""'),('tooltip','""'),('link','""'),('style.shadow','false'),('style.multiple','false'),('style.3d','false'),('style.animated','false'),('style.filled','false')]:
 for value in [default, '*', 'unmatched']:
  for negate in ['', '!']:
   texts.append(f'a; b; *: {{ {negate}&{prop}: {value}; style.fill: red }}; c')
for prop in ['level','leaf','connected','label']:
 for value in ['0','1','2','-1','+0','01','9223372036854775808','true','false','True','FALSE','t','F','yes','""','" 1"','"é"']:
  texts.append(f'a: {{b; c -> d}}; a -> e; **: {{ &{prop}: {value}; style.fill: red }}')
for endpoint in ['src','dst']:
 for prop, value in [('', 'a'),('', 'box.a'),('shape','circle'),('label','hi'),('level','0'),('leaf','true'),('connected','true'),('style.fill','red'),('missing','x')]:
  predicate=endpoint + ('.'+prop if prop else '')
  for prefix in ['', 'box: {']:
   body=f'a: hi {{shape: circle; style.fill: red}}; b: {{shape: rectangle}}; a -> b; (* -> *)[*]: {{ &{predicate}: {value}; style.stroke: blue }}'
   texts.append(prefix+body+('}' if prefix else ''))
texts += [
 'a -> b; (* -> *)[*]: {&label: [x]; style.fill: red}',
 'a -> b: named; (* -> *)[*]: {&label: [x]; style.fill: red}',
 'a.class: [foo; bar]; b.class: [food]; *: {&class: foo*; style.fill: red}',
 'a: hello; b: help; *: {&label: he*; style.fill: red}',
 'a: hello; b: help; *: {&label: "he*"; style.fill: red}',
 'a.style.fill: red; *: {&style.fill: [red]; label: changed}',
 'a.style.fill: red; *: {!&style.fill: [red]; label: changed}',
 'a: [1]; *: {&shape: rectangle; label: changed}',
 'a; b; *: null { &label: a }',
 'a; b; *: suspend { &label: a }; *: unsuspend { &label: a }',
 'a -> b: first; a -> b: second; (* -> *)[*]: suspend { &label: first }',
 'a -> b: first; a: suspend; (* -> *)[*]: unsuspend { &label: first }',
 '***: {&level: 0; style.fill: red}; a; layers.one: {b}; scenarios.one: {c}; steps.one: {d}',
 '**: {&leaf: true; style.fill: red}; a: {b}; c',
 '**: {&connected: true; style.fill: red}; a; b; a -> b',
 '*: {&shape: circle; style.fill: red}; a; a.shape: circle',
 '!&shape: circle; a',
]
# Substitution ownership, source ordering and spread replay witnesses.
texts += [
 'vars: {x: 3}; a: ${x}; b: "${x}"; c: pre${x}post',
 'vars: {x: null}; a: ${x}; b: "${x}"',
 'vars: {x: true}; a -> b: ${x}; c: [${x}; "${x}"]',
 'vars: {x: one; X: two}; a: ${X}',
 'vars: {x: {y: nested}}; a: ${x.y}; b: ${x.missing}',
 'vars: {x: empty}; a: ${x.y}',
 'vars: {x: one}; a: {vars: {x: ${x}-two}; b: ${x}}',
 'a: ${x}; vars: {x: late}',
 'vars: {x: ${y}; y: later}; a: ${x}',
 'vars: {x: ${x}}; a: ${x}',
 'vars: {x: value}; a: ${missing}-${x}',
 'vars: {x: value}; a: ${x}-${missing}',
 'vars: {x: value}; a: "${x}-${missing}"',
 'vars: {x: {a: 1; b: 2}}; a: ${x}; b: ${x}',
 'vars: {x: label {a: 1}}; a: pre${x}post; a.extra: y',
 'vars: {x: {a: 1}}; a: pre${x}post',
 'vars: {x: [1;2]}; a -> b: ${x}',
 'vars: {x: {style.fill: red}}; a -> b: ${x}',
 'vars: {x: [1;2]}; a: [0; ...${x}; 3]',
 'vars: {x: [1;2]; y: yes}; a: [...${x}; ${y}; ${y}]',
 'vars: {x: []; y: yes}; a: [...${x}; ${y}; ${y}]',
 'vars: {x: [1;2]; y: [3;4]}; a: [...${x}; ...${y}; ...${x}]',
 'vars: {x: {a: 1}}; a: [...${x}]',
 'vars: {x: value}; a: [...${x}]',
 'vars: {x: value}; a: [[${x}]; {b: ${x}}]',
 'vars: {x: {a: 1; b: 2}}; before; ...${x}; after',
 'vars: {x: {a: 1; b: 2}}; a: old; ...${x}; a: last; after',
 'vars: {x: {a: 1; b: 2}}; ...${x}; ...${x}',
 'vars: {x: {a -> b}}; ...${x}; (* -> *)[*].style.fill: red',
 '*.style.fill: red; vars: {x: {a; b}}; ...${x}; c',
 '**: {&leaf: true; style.fill: red}; vars: {x: {a; b}}; ...${x}',
 '***.style.fill: red; vars: {x: {a; b}}; layers.one: {...${x}}; scenarios.one: {...${x}}',
 'vars: {x: [1;2]}; a: {...${x}; b}',
 'vars: {x: value}; a: {...${x}; b}',
 'vars: {x: null}; a: {...${x}; b}',
 'vars: {x: {a}}; box: {...${missing}; b}',
 'vars: {x: value}; "vars": {x: quoted}; a: ${x}',
 'Vars: {x: uppercase}; a: ${x}',
 'vars: {x: value}; layers.one: {a: ${x}}; scenarios.one: {b: ${x}}; steps.one: {c: ${x}}',
 'vars: {x: value}; a: |md hello ${x} |',
]
for length in [3,4,7,8,31,32,33,64,255,256,257,511,512]:
 tail=';'.join(['${y}']*(length-2))
 texts.append('vars: {x: [9;8;7]; y: done}; a: [0; ...${x}; '+tail+']')
# Config validation runs after substitutions within each unquoted vars map.
for key in ['sketch','center','pad','theme-id','dark-theme-id','layout-engine','theme-overrides','dark-theme-overrides','data','unknown','Theme-id']:
 for value in ['', 'true', 'False', 't', '1', '0', '-1', '01', '"+1"', '" 1"', '2', '9223372036854775808', 'null', '[1]', '{}', 'caption {x: 1}', '"line\\nvalue"']:
  texts.append('vars: {d2-config: {'+key+(':'+value if value else '')+'}}')
for value in [-1,*range(10),99,*range(100,107),199,200,201,202,299,300,301,302,303,304]:
 texts.append(f'vars: {{d2-config: {{theme-id: {value}; dark-theme-id: {value}}}}}')
texts += [
 'vars: {x: true; d2-config: {sketch: ${x}; center: false; pad: -10; layout-engine: anything}}',
 'vars: {d2-config: {sketch: ${x}}; x: true}',
 'vars: {d2-config: {theme-overrides: {B1: red}; data: {unknown: [1;2]}}}',
 'vars: {d2-config: {sketch: true}}; layers.one: {a}; scenarios.one: {b}; steps.one: {c}',
 'a.vars: {d2-config: {sketch: true}}; b.vars: {d2-config: {center: false}}',
 'layers.one: {vars: {d2-config: {sketch: true}}}; scenarios.one: {vars: {d2-config: {pad: 3}}}; steps.one: {vars: {d2-config: {center: 1}}}',
 'a -> b: {vars: {d2-config: {sketch: true}}}',
 'vars: {d2-config: label}; a.vars: {d2-config: label}',
 'vars: {d2-config: [1;2]}',
 'Vars: {d2-config: {unknown: true}}',
 '"vars": {d2-config: {unknown: true}}',
 'vars: {"d2-config": {unknown: true}}',
 'vars: {D2-CONFIG: {unknown: true}}',
 'd2-config: {unknown: true}',
 'vars: {template: {pad: 2}; d2-config: ${template}}',
 'vars: {template: {pad: 2}; d2-config: {...${template}}}',
]
requests=[dict(input=t,path='fixture.d2',origin='constructed') for t in texts]
# Import literal compiler calls from upstream tables. Unsupported cases remain
# recorded as untranslated; they are never counted as matching/pass results.
for file in ['d2ir/filter_test.go','d2ir/pattern_test.go','d2ir/compile_test.go','d2ir/lazy_glob_scheduler_test.go']:
 for row in call(go,None,'literals',file,cwd=source):
  requests.append(dict(row,path='fixture.d2'))
# D2 graph-compiler variable cases also exercise the underlying IR compiler.
for row in call(go,None,'literals','d2compiler/compile_test.go','assertCompile',cwd=source):
 if '${' in row['input'] or 'd2-config' in row['input']:
  requests.append(dict(row,path='fixture.d2'))
left=call(go,requests,'compile',timeout=600)
assert len(left)==len(requests)
# Reviewed upstream panics at the pin (a matching panic is an upstream
# limitation, never a passing compilation). Each is reproduced through public
# d2compiler.Compile on an uninstrumented build.
known_source_panics={
 'a; steps: {one: caption; two: {b}; three: {c}}': 'master d2ir/compile.go overlay -> expansion.go reserveVariableCopy dereferences the nil map of a scalar previous step (v0.9.0 compiled this without a panic)',
}
unreviewed=[r for a,r in zip(left,requests) if a.get('panic') and r['input'] not in known_source_panics]
assert not unreviewed, 'Unreviewed upstream panics must be investigated: '+repr(unreviewed)
# Ensure config cases reach validation instead of passing on earlier syntax errors.
config_witnesses={
 'vars: {d2-config: {theme-id: 0; dark-theme-id: 0}}': None,
 'vars: {d2-config: {layout-engine:true}}': None,
 'vars: {d2-config: {sketch:2}}': 'expected a boolean for "sketch"',
 'vars: {d2-config: {theme-id: 2; dark-theme-id: 2}}': '2 is not a valid theme ID',
 'vars: {d2-config: {theme-overrides:true}}': '"theme-overrides" needs a map',
 'a.vars: {d2-config: {sketch: true}}; b.vars: {d2-config: {center: false}}': 'can only appear at root vars',
}
observations={r['input']:a for a,r in zip(left,requests)}
for source_text,message in config_witnesses.items():
 observation=observations[source_text]
 assert 'errors' in observation, 'Config witness failed before compilation'
 if message is None:
  assert observation['errors']==[], (source_text,observation['errors'])
 else:
  assert any(message in err['errmsg'] for err in observation['errors']), (source_text,observation['errors'])
plain=call(field_oracle(),requests,'compile',timeout=600)
assert [{k:x for k,x in v.items() if k!='trace'} for v in left]==plain, 'Go instrumentation changed ordinary output'
write(fixture('ir-compiler-trace.json'),dict(requests=requests,reference=left),compact=True)
print(f"{len(requests)} compilations captured; {sum(bool(v.get('panic')) for v in left)} upstream panics; {sum(len(v.get('trace',[])) for v in left)} trace events")
