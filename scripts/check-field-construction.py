#!/usr/bin/env python3
"""Compare the recursive field primitive, not the complete compiler scheduler.

Fixture generator for recipe `lang-field-construction` (tools/fixtures/langlibelk-language.toml):
captures tools/field-oracle at the manifest pin and writes field-construction.json to the staged
output. `cargo test -p d2-rs-language --features oracle-fixtures` does the native
comparison (docs/FIXTURES.md).
"""
from language_oracle import call, field_oracle, fixture, write
go = field_oracle()
def ensure(key='',**kw): return dict(op='ensure',key=key,create=True,**kw)
def query(key='',**kw): return dict(op='ensure',key=key,create=False,**kw)
def glob(key='',**kw): return ensure(key,glob=True,enter=True,**kw)
scenarios=[]
for key in ['a','a.b','a.b.c','A.b','style.fill','Style.fill','shape.x','a._.b','_',
            'a.classes.x','a.layers.one','layers.one.x','scenarios.one.x','steps.one.x',
            'a."classes".x','a."layers".one','a.width.x','a.STYLE.fill','a.lİnk.x']:
 scenarios.append([ensure(key),ensure(key),query(key)])
scenarios += [
 [query('a.b')],
 [ensure('a'),query('a.b')],
 [ensure('a.b'),ensure('c',map='a'),ensure('a.d',map='a',start=1)],
 [ensure('a'),ensure('A'),ensure('"a"'),ensure('a',no_context=True)],
 [ensure('a.b'),dict(op='set',path='a',array=True),ensure('a.c')],
 [ensure('a.b'),dict(op='set',path='a',array=True),glob('*.c')],
 [ensure('a.b'),dict(op='set',path='a',array=True),glob('**.c')],
 [ensure('a.b'),dict(op='set',path='a',array=True),glob('***.c')],
 [ensure('a'),ensure('a',context=0,lazy=True),ensure('a',context=0,lazy=True)],
 [glob('**.x'),ensure('a.b',stack=[0]),ensure('a.c',stack=[0])],
 [ensure('a'),glob('a -> *',path='src'),glob(context=1),glob(context=1,lazy=True)],
 [ensure('a'),ensure('b'),glob('* -> b',path='src'),glob(context=2),glob(context=2,lazy=True)],
 [ensure('a'),ensure('b'),glob('a -> *',path='dst'),glob(context=2)],
 [ensure('a'),ensure('b'),glob('(* -> b)[*].*.fill',path='edge'),glob(context=2)],
 [ensure('a'),glob('**.width.x'),glob(context=1)],
 [ensure('a'),glob('**.style.fill'),glob('**.style.fill')],
]
for pattern in ['*','a*','**','***','*.style.fill','**.style.fill','***.style.fill','**.child']:
 scenarios.append([ensure('a.b'),ensure('a.c'),ensure('d'),ensure('vars.x'),
                   ensure('classes.x'),ensure('layers.one.x'),glob(pattern),glob(context=6),
                   ensure('a.e'),glob(context=6,lazy=True,target='a.e')])
for shape in ['class','sql_table','rectangle','CLASS']:
 scenarios.append([ensure('a.shape'),dict(op='set',path='a.shape',value=shape),ensure('b'),
                   glob('**.x'),glob('**.width')])
# Same operation sequence is qualified with direct lookup and compiler indexes.
requests=[[dict(op,indexed=indexed) for op in ops] for ops in scenarios for indexed in [False,True]]
reference=call(go,requests)
write(fixture('field-construction.json'),dict(requests=requests,reference=reference),compact=True)
print(f'{len(requests)} field-construction scenarios captured')
