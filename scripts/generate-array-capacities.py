#!/usr/bin/env python3
"""Pinned Go []Value append-growth capacities for native regression tests.

Fixture generator for recipe `lang-array-capacities` (tools/fixtures/langlibelk-language.toml):
captures tools/field-oracle `array-capacities` (64-bit Go runtime growslice
with size-class rounding, 16-byte interface elements) at the manifest pin and
writes array-capacities.json as [old, new, capacity] triples to the staged
output. `cargo test -p d2-rs-language --features oracle-fixtures` does the
native comparison (docs/FIXTURES.md).
"""
from language_oracle import call, field_oracle, fixture, write

requests = sorted({(old, new) for old in [*range(257), 511, 512, 513, 1023, 1024, 1025, 2047, 2048, 2049, 4095, 4096]
                   for new in [old + 1, old + 2, old + 17, old * 2 + 1, old + 1025]})
values = call(field_oracle(), requests, 'array-capacities')
assert len(values) == len(requests)
write(fixture('array-capacities.json'), [(*request, value) for request, value in zip(requests, values)], compact=True)
print(f'{len(requests)} pinned Go capacity observations recorded')
