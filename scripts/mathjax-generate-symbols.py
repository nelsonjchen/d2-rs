#!/usr/bin/env python3
"""Generate d2-rs-math's MathJax source symbol table.

Input: the pinned mathjax-go v0.1.0 `internal/tex/source_tables_*_maps.go`
files (MathJax 3.2.2 CharacterMap/DelimiterMap data used by D2's bundled
MathJax).  Output: `crates/d2-rs-math/src/source_symbols.rs`, the resolved
`lookupMJSourceSymbol` view (tex/source_runtime.go): for every name, the
winning CharacterMap entry (later maps and later entries win), else the
winning DelimiterMap entry, with its parser kind and attribute object.

The script only reads local verified source; it runs no Go or JavaScript.

Usage: scripts/mathjax-generate-symbols.py <mathjax-go>/internal/tex <output.rs>
"""

from __future__ import annotations

import argparse
import re
from pathlib import Path

# source_tables_packages.go mjSourceMaps concatenation order.
FILES = [
    ("source_tables_base_maps.go", "mjSourceBaseMaps"),
    ("source_tables_ams_maps.go", "mjSourceAMSMaps"),
    ("source_tables_mathtools_maps.go", "mjSourceMathtoolsMaps"),
    ("source_tables_amscd_braket_cases_maps.go", "mjSourceAmsCdBraketCasesMaps"),
    ("source_tables_physics_maps.go", "mjSourcePhysicsMaps"),
    ("source_tables_empheq_newcommand_maps.go", "mjSourceEmpheqNewcommandMaps"),
    ("source_tables_selected_extensions_maps.go", "mjSourceSelectedExtensionMaps"),
]

GO_STRING = r'"(?:[^"\\]|\\.)*"'


def go_string(literal: str) -> str:
    # Go interpreted string literals used here only contain \\ and \" escapes
    # plus raw UTF-8 text.
    body = literal[1:-1]
    out = []
    i = 0
    while i < len(body):
        c = body[i]
        if c == "\\":
            n = body[i + 1]
            if n in '\\"':
                out.append(n)
                i += 2
                continue
            if n in "tnr":
                out.append({"t": "\t", "n": "\n", "r": "\r"}[n])
                i += 2
                continue
            if n in "uU":
                width = 4 if n == "u" else 8
                out.append(chr(int(body[i + 2 : i + 2 + width], 16)))
                i += 2 + width
                continue
            raise ValueError(f"unsupported escape in {literal!r}")
        out.append(c)
        i += 1
    return "".join(out)


def parse_maps(path: Path):
    text = path.read_text()
    maps = []
    # Each map literal starts with "\t{\n\t\tName:" at top level of the slice.
    for block in re.split(r"\n\t\{\n\t\tName:\s+", text)[1:]:
        name = go_string(re.match(GO_STRING, block).group(0))
        kind = re.search(r"\n\t\tKind:\s+(\w+)", block).group(1)
        parser = re.search(r"\n\t\tParser:\s+(" + GO_STRING + ")", block)
        parser = go_string(parser.group(1)) if parser else ""
        entries = []
        for match in re.finditer(r"\n\t\t\t\{Name: (" + GO_STRING + r"), Value: (.*)\},(?=\n)", block):
            entries.append((go_string(match.group(1)), match.group(2)))
        maps.append((name, kind, parser, entries))
    return maps


def parse_value(raw: str):
    raw = raw.strip()
    if re.fullmatch(GO_STRING, raw):
        return go_string(raw), None
    match = re.fullmatch(r"mjSourceList\{(" + GO_STRING + r")(?:, mjSourceObject\{(.*)\})?\}", raw)
    if not match:
        return None, None
    character = go_string(match.group(1))
    attributes = []
    if match.group(2):
        for prop in re.finditer(r"\{Name: (" + GO_STRING + r"), Value: ([^}]*)\}", match.group(2)):
            value = prop.group(2).strip()
            if re.fullmatch(GO_STRING, value):
                value = go_string(value)
            attributes.append((go_string(prop.group(1)), value))
    return character, attributes


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path, help="mathjax-go internal/tex directory")
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    maps = []
    for filename, _ in FILES:
        maps.extend(parse_maps(args.source / filename))
    resolved = {}
    # lookupMJSourceSymbol: CharacterMap first, then DelimiterMap; within a
    # kind the last map and last entry win.
    for wanted in ("mjSourceCharacterMap", "mjSourceDelimiterMap"):
        for map_name, kind, map_parser, entries in maps:
            if kind != wanted:
                continue
            for entry_name, raw in entries:
                if kind == "mjSourceDelimiterMap" and entry_name.startswith("\\"):
                    entry_name = entry_name[1:]
                character, attributes = parse_value(raw)
                if wanted == "mjSourceDelimiterMap" and entry_name in resolved and resolved[entry_name][0] == "mjSourceCharacterMap":
                    continue
                resolved[entry_name] = (kind, map_name, map_parser, character, attributes)
    # tex/commands.go command(): package handlers (ams tags, mathtools,
    # physics, amscd, base AMS) run before lookupMJSourceSymbol, so a name
    # one of them owns never reaches the symbol maps; omit any such name.
    # Their case labels (tex/{ams_tags,mathtools_handlers,physics_handlers,
    # amscd_handlers,base_ams_handlers}.go) are the names they own.
    commands = set()
    for path in sorted(args.source.glob("*.go")):
        if path.name.endswith("_test.go"):
            continue
        text = path.read_text()
        for function in ("amsTagCommand", "mathtoolsCommand", "physicsCommand", "amscdCommand", "baseAMSCommand"):
            body = re.search(r"func \(p \*parser\) " + function + r"\(.*?\n}\n", text, re.S)
            if body:
                for case in re.findall(r"case ([^:]*):", body.group(0)):
                    commands.update(go_string(m) for m in re.findall(GO_STRING, case))
    shadowed = sorted(name for name in resolved if name in commands)
    for name in shadowed:
        del resolved[name]
    lines = [
        "// Copyright 2009-2022 The MathJax Consortium",
        "// SPDX-License-Identifier: Apache-2.0",
        "// Source: pinned mathjax-go v0.1.0 tex/source_tables_*_maps.go, resolved as",
        "// tex/source_runtime.go lookupMJSourceSymbol (MathJax 3.2.2 SymbolMap data).",
        "// Generated by scripts/mathjax-generate-symbols.py; do not edit.",
        f"// Omitted (owned by an earlier package command handler): {', '.join(shadowed)}.",
        "",
        "#[derive(Clone, Copy, Debug, PartialEq, Eq)]",
        "pub(super) enum Parser {",
        "    Mathchar0mi,",
        "    Mathchar0mo,",
        "    Mathchar7,",
        "    Delimiter,",
        "    /// A CharacterMap whose parser is neither of the above; the source",
        "    /// still creates an mo token.",
        "    Other,",
        "}",
        "",
        "#[derive(Debug)]",
        "pub(super) struct Symbol {",
        "    pub name: &'static str,",
        "    pub map: &'static str,",
        "    pub parser: Parser,",
        "    /// None: the source entry value is not a character (unusable).",
        "    pub text: Option<&'static str>,",
        "    /// The entry's attribute object in source order (values as Go literals).",
        "    pub attributes: &'static [(&'static str, &'static str)],",
        "}",
        "",
        "pub(super) fn lookup(name: &str) -> Option<&'static Symbol> {",
        "    SYMBOLS",
        "        .binary_search_by(|s| s.name.cmp(name))",
        "        .ok()",
        "        .map(|i| &SYMBOLS[i])",
        "}",
        "",
        "pub(super) static SYMBOLS: &[Symbol] = &[",
    ]

    def rust_str(value: str) -> str:
        return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'

    for entry_name in sorted(resolved, key=lambda s: s.encode()):
        kind, map_name, map_parser, character, attributes = resolved[entry_name]
        if kind == "mjSourceDelimiterMap":
            parser_kind = "Delimiter"
        elif "mathchar0mi" in map_parser:
            parser_kind = "Mathchar0mi"
        elif "mathchar7" in map_parser:
            parser_kind = "Mathchar7"
        elif "mathchar0mo" in map_parser:
            parser_kind = "Mathchar0mo"
        else:
            parser_kind = "Other"
        attrs = ", ".join(f"({rust_str(n)}, {rust_str(v)})" for n, v in (attributes or []))
        text = "None" if character is None else f"Some({rust_str(character)})"
        lines.append(
            f"    Symbol {{ name: {rust_str(entry_name)}, map: {rust_str(map_name)}, "
            f"parser: Parser::{parser_kind}, text: {text}, attributes: &[{attrs}] }},"
        )
    lines.append("];")
    args.output.write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
