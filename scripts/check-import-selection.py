#!/usr/bin/env python3
"""selectImport, nilScopeMap and importTemplateSafeAt in the unchanged Go d2ir.

Fixture generator for recipe `lang-import-selection` (tools/fixtures/langlibelk-language.toml):
captures tools/import-selection-oracle at the manifest pin and writes
import-selection.json to the staged output. `cargo test -p d2-rs-language
--features oracle-fixtures` does the native comparison (docs/FIXTURES.md).

Missing-key diagnostics are formatted by Go fmt from raw AST pointers
(`%!q(*d2ast.…)`), so their text changes between runs. Those rows keep the
whole selection result except `errors` and are marked `unstable_errors`; the
test compares the rest. The oracle runs twice and the stable part must agree.
"""
from language_oracle import call, fixture, import_selection_oracle, write

go = import_selection_oracle()
inputs=['a: one; b: {c: two}; a -> b','a: [{inside: 1}; [{deep: 2}]]; b: {c}','layers: {one: {x}; two: {y}}; scenarios.one: {a}; steps.one: {b}','a: {b: {c}}; a.b -> a.b.c','vars: {x: {inner: yes}}; a: ${x}','a: null; "shape": yes; "two words": value','a: {x}; *.style.fill: red','a: [{m -> n; m: yes}]']
imports=['@file','@file.a','@file.b.c','@file.missing','@file.a.missing','@file.layers.one','@file."two words"','@file."shape"','@file."line\\nfeed"','@file."雪"']
requests=[{'input':i,'import':s} for i in inputs for s in imports]
first = call(go, requests)
second = call(go, requests)
assert len(first) == len(second) == len(requests)
cases = []
unstable = 0
for request, result, repeat in zip(requests, first, second):
    if any('%!q(*d2ast.' in error['errmsg'] for error in result.get('errors') or []):
        stable = {k: v for k, v in result.items() if k != 'errors'}
        assert stable == {k: v for k, v in repeat.items() if k != 'errors'}, request
        cases.append({'request': request, 'reference': stable, 'unstable_errors': True})
        unstable += 1
    else:
        assert result == repeat, request
        cases.append({'request': request, 'reference': result})
write(fixture('import-selection.json'), cases)
print(f'{len(cases)} import-selection requests captured; {unstable} with pointer-formatted diagnostics')
