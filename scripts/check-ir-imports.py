#!/usr/bin/env python3
"""Capture imported IR compilation, cache state and key execution traces.

Fixture generator for recipe `lang-ir-import` (tools/fixtures/langlibelk-language.toml):
builds tools/field-oracle with the compileKey trace overlay at the manifest
pin, compiles the constructed import corpus below plus the import cases mined
from the pinned d2ir tests, and writes ir-import-trace.json to the staged
output. The plain fixture is this capture without the `trace` key (asserted
here against an uninstrumented build). `cargo test -p d2-rs-language
--features oracle-fixtures` does the native comparison (docs/FIXTURES.md).

Rows left out of the trace fixture, all by Go-side criteria:
- upstream panics (reviewed nil-import outcomes), written with their corpus
  index to ir-import-panics.json,
- compilations whose AST cannot be projected, written (plain view) to
  ir-import-projection.json,
- diagnostics that print Go pointer addresses (`%!q(*d2ast.…)`), which differ
  between runs and cannot be an expected value.
"""
import json
from language_oracle import call, field_oracle, fixture, upstream, write
source = upstream()
go = field_oracle(trace=True)
files={'lib.d2':'a: one; b: {c: two}; a -> b','values.d2':'scalar: hi; array: [1; 2]; map: {inner: yes}', 'vars.d2':'vars: {local: here}; a: ${local}; b: ${outer}', 'boards.d2':'base; layers.one: {a}; scenarios.one: {b}; steps.one: {c}', 'nested.d2':'x: @lib; ...@values.map', 'bad.d2':'a: {', 'cycle.d2':'...@cycle', 'dir/lib.d2':'x: @other', 'dir/other.d2':'a: okay'}
texts=['x: @lib','...@lib','x: @lib; y: @lib','...@lib; ...@lib','x: @values.scalar; y: @values.scalar','x: @values.map; y: @values.map','x: @values.array; y: @values.array','x: old; x: @values.scalar','x: {old}; x: @values.map','x: {a -> b}; x: @lib','x: @lib; x.a: changed; y: @lib','x: [@values.scalar; @values.array; @values.map; @lib]','x: [0; ...@values.array; 3]','x: [...@values.scalar]','...@values.scalar','...@values.map','vars: {outer: there}; x: @vars; y: @vars','...@nested','x: @nested; y: @nested','...@boards','layers.one: @lib; layers.two: @lib','scenarios.one: @lib; scenarios.two: @lib','steps.one: @lib; steps.two: @lib','x: @missing','x: @bad; y: @bad','...@cycle','x: @dir/lib; y: @dir/lib','x: { ...@lib }; y: { ...@lib }','*.style.fill: red; x: @lib','x: @lib; *.style.fill: red','x: {a -> b}; x: @lib; y: @lib','x: { ...@values.map; inner -> other }; x -> y']
requests=[{'input':t,'path':'main.d2','files':files} for t in texts]
for length in [0,1,2,3,4,5,7,8,9,15,16,17,31,32,33,65]:
    for prefix in [0,1,5]:
        imported=';'.join(str(i) for i in range(length))
        items=';'.join([*map(str,range(prefix)),'...@bulk.values','...${items}','99'])
        requests.append({'input':'vars: {items: [7;8;9]}; a: ['+items+']','path':'main.d2','files':{'bulk.d2':'values: ['+imported+']'},'origin':'constructed bulk-import/substitution capacity boundary'})
link_values=['image.svg','./image.svg','../image.svg','../../a/../image.svg','/absolute.svg','//host','//host/image.svg','///host','%2fabsolute.svg','https://example.com/x','http://[::1]/x','http://[bad]/x','http://a:1,b:2/x','postgres://a:1,b:2/x','data:image/svg+xml,abc','http&#58;//example.com/x','&sol;image.svg','&amp;name','%ZZ','root.layers.one','root._.layers.one','root._._.layers.one','root."_".layers.one','layers.one','#fragment','?query','http://host/x#%ZZ','http://host/x?%ZZ','a:b','a:/%ZZ','雪.svg','./a//b/../../image.svg','']
for value in link_values:
    library='item: {icon: '+json.dumps(value,ensure_ascii=False)+'; link: '+json.dumps(value,ensure_ascii=False)+'; child.icon: "../child.svg"}'
    for text in ['x: @dir/library; y: @dir/library','...@dir/library','x: @dir/library.item','x: [@dir/library]','x: @dir/wrapper']:
        requests.append({'input':text,'path':'main.d2','files':{'dir/library.d2':library,'dir/wrapper.d2':'nested: @library'},'origin':'constructed imported icon/link relocation'})
for text,library in [
    ('layers.one: @lib','link: layers.two; x'),
    ('...@lib','link: layers.two; x'),
    ('x: @lib; y: @lib','a: {link: root._.layers.one; icon: ./a.svg}; layers.one: {b.link: root._.layers.two}'),
    ('layers.outer: {layers.inner: @lib}','a.link: root._.layers.one; b.link: root.layers.two'),
    ('x: @lib','"link": layers.one; "icon": ./a.svg; a.icon: ${outside}'),
    ('vars: {outside: https://example.com/a.svg}; x: @lib','a.icon: ${outside}'),
    ('x: {old.icon: before.svg}; x: @lib','new.icon: after.svg'),
]:
    requests.append({'input':text,'path':'main.d2','files':{'lib.d2':library},'origin':'constructed board/error/quoted/substitution relocation'})
unicode_sources=['😀: one; 雪: two; 😀 -> 雪','entry: {😀: one; 雪: two}; entry.label: café','😀: { child: value; _ .other: yes }','😀: {','😀 ->','entry: {label: "é 😀"; values: [😀; 雪]}','vars: {😀: world}; x: ${😀}','😀.width.x: invalid','a: "😀\n雪"']
for text in unicode_sources:
    for encoding in ['utf8','utf16le']:
        content=text.encode() if encoding=='utf8' else b'\xff\xfe'+text.encode('utf-16le')
        for utf16 in [False,True]:
            requests.append({'input':'😀: @unicode; again: @unicode','path':'main.d2','files':{},'files_hex':{'unicode.d2':content.hex()},'utf16':utf16,'origin':'constructed '+encoding+' import source positions'})
for content in [b'\xff\xfe',b'\xff\xfea',b'\xff\xfea\x00:0',b'\xff\xfe\x00\xd8',b'\xff\xfe\x00\xdc',b'\xff\xfe\x00\xd8a\x00',b'\xef\xbb\xbf'+ '😀: one'.encode(),b'\xff',b'a: \xff']:
    for utf16 in [False,True]:
        requests.append({'input':'😀: @unicode; again: @unicode','path':'main.d2','files':{},'files_hex':{'unicode.d2':content.hex()},'utf16':utf16,'origin':'constructed malformed/BOM import source positions'})
# Reviewed against the unchanged Go compiler. Panic outcomes are recorded
# separately from successful compilations; projection failure is distinct too.
nil_cases=[
 ('a: [@lib.empty]','substitutions'),
 ('a: [@lib.empty]; a: null','success'),
 ('a: [@lib.empty]; a: []','success'),
 ('a: [@lib.empty]; a: {}','success'),
 ('a: [@lib.empty]; a: [1]','success'),
 ('a: [@lib.empty]; a: replaced','substitutions'),
 ('a: [[@lib.empty]]','projection'),
 ('a: [{b: [@lib.empty]}]','projection'),
 ('a: [[@lib.empty]]; a: null','success'),
 ('a: [@lib.empty]; a: suspend','substitutions'),
 ('a: [@lib.empty]; a: null; a: new','success'),
 ('a: [@lib.empty; @lib.empty]; a: null','success'),
 ('a: [@lib.empty]; b: @lib.empty; a: null','success'),
 ('a: [@lib.empty]; scenarios.one: {}','compile-map'),
 ('a: [[@lib.empty]]; scenarios.one: {}','compile-map'),
 ('a: [@lib.empty]; a: null; scenarios.one: {}','success'),
]
for text,outcome in nil_cases:
    requests.append({'input':text,'path':'main.d2','files':{'lib.d2':'empty'},'origin':'reviewed nil imported scalar','expected_nil_outcome':outcome})
for file in ['import_test.go','import_selection_test.go','pattern_test.go']:
    requests+=call(go,None,'import-corpus','d2ir/'+file,cwd=source)
# This regression exercises a lazy glob contributed by an imported map spread,
# including both imported descendants and class application in the importer.
# It lives in a separate scheduler test rather than import_test.go, so include
# the same source/files pair explicitly instead of silently omitting it from
# the compiler import corpus.
requests.append({
    'input':'before\n...@defs\nafter: { class: paint }\n',
    'path':'index.d2',
    'files':{
        'index.d2':'before\n...@defs\nafter: { class: paint }\n',
        'defs.d2':'***.style.fill: green\nimported: { child }\nclasses: { paint: { style.stroke: blue } }\n',
    },
    'origin':'d2ir/lazy_glob_scheduler_test.go:TestLazyGlobsIntroducedByImportPreserveClasses',
})
# This index regression checks that a leading recursive glob is replayed on
# the newly materialized subtree after a selected-field import.
requests.append({
    'input':'**: default\nx\nx: @nested\n',
    'path':'index.d2',
    'files':{
        'index.d2':'**: default\nx\nx: @nested\n',
        'nested.d2':'a: { b }\n',
    },
    'origin':'d2ir/index_test.go:TestLeadingGlobReplaysAfterSelectedFieldImport',
})
reference=call(go,requests,'compile-imports',timeout=600)
plain=call(field_oracle(),requests,'compile-imports',timeout=600)
assert len(reference)==len(plain)==len(requests)
kept=[];panics=[];projection=[];pointer=0
for index,(r,g,p) in enumerate(zip(requests,reference,plain)):
    if g.get('panic'):
        assert r.get('expected_nil_outcome')==g['phase'] and g['phase'] in ['substitutions','compile-map'], (r,g)
        panics.append({'index':index,'request':r,'reference':g});continue
    if g.get('ast')=={'panic':True}:
        assert r.get('expected_nil_outcome')=='projection', (r,g)
        projection.append((r,p));continue
    assert r.get('expected_nil_outcome','success')=='success', (r,g)
    if any('%!q(*d2ast.' in e['errmsg'] for e in g.get('errors') or []):
        pointer+=1;continue
    assert {k:v for k,v in g.items() if k!='trace'}==p, ('Go instrumentation changed ordinary output', r)
    kept.append((r,g))
write(fixture('ir-import-trace.json'),{'requests':[r for r,g in kept],'reference':[g for r,g in kept]})
write(fixture('ir-import-panics.json'),panics)
write(fixture('ir-import-projection.json'),{'requests':[r for r,p in projection],'reference':[p for r,p in projection]})
print(f'{len(kept)}/{len(requests)} imported compilations captured; {len(panics)} upstream panics, {len(projection)} projection failures, {pointer} pointer-address diagnostics left out')
