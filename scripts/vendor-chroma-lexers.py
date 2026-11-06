#!/usr/bin/env python3
"""Vendor manifest-pinned Chroma lexer rules and configuration tables.

These are MIT-licensed source inputs, not generated Go oracle output. Build
and test d2-rs without Go after vendoring. Samples are maintained separately.
"""
from __future__ import annotations

import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import xml.etree.ElementTree as ET

import references


def custom_definitions(source, assets):
    """Serialize declarative Go rules, never lexer output, as native source inputs.

    Copy only the named rule constructors from the pin. HTTP's three fixed
    emitters are equivalent to Token/ByGroups; rstCodeBlock remains a named
    Rust emitter because it concatenates captures before delegation. Haxe's
    preprocessor mutator is named for native shared-stack execution. Raku's
    closure factories retain their operands for native delimiter, rule and
    named-group execution.
    """
    constructors = {
        "caddyfile.go": ["caddyfileCommonRules", "caddyfileRules", "caddyfileDirectivesRules"],
        "gemtext.go": ["gemtextRules"],
        "genshi.go": ["genshiTextRules", "genshiMarkupRules"],
        "haxe.go": ["haxeRules"],
        "http.go": ["httpRules"],
        "markless.go": ["marklessRules"],
        "raku.go": ["rakuRules"],
        "rst.go": ["restructuredtextRules"],
        "svelte.go": ["svelteRules"],
    }
    lexers = [
        ("Caddyfile", "caddyfile", "caddyfileRules"),
        ("Caddyfile Directives", "caddyfile-directives", "caddyfileDirectivesRules"),
        ("Gemtext", "gemtext", "gemtextRules"),
        ("Genshi Text", "genshi-text", "genshiTextRules"),
        ("Genshi HTML", "genshi-html", "genshiMarkupRules"),
        ("Genshi", "genshi", "genshiMarkupRules"),
        ("Haxe", "haxe", "haxeRules"),
        ("HTTP", "http", "httpRules"),
        ("Markless", "markless", "marklessRules"),
        ("Raku", "raku", "rakuRules"),
        ("reStructuredText", "rst", "restructuredtextRules"),
        ("Svelte", "svelte", "svelteRules"),
    ]
    functions = []
    for filename, names in constructors.items():
        text = (source / filename).read_text()
        for name in names:
            match = re.search(rf"(?ms)^func {name}\(\) Rules \{{.*?^\}}", text)
            if not match:
                raise RuntimeError(f"missing pinned rule constructor: {filename}:{name}")
            constructor = match[0]
            if filename == "raku.go":
                # Preserve the declarative rules and bracket table. Runtime
                # closures become named native operations, never token captures.
                constructor = re.sub(
                    r"(?ms)\tfindBrackets := func.*?(?=\t// Raku rules)",
                    "\tfindBrackets := func(t RakuToken) Mutator { return rakuBrackets{int(t)} }\n\n",
                    constructor,
                )
                constructor = constructor.replace("ByGroupNames(", "rakuGroups(")
                constructor = constructor.replace("colonPair(", "rakuColonPair(")
                for emitter in ("quote", "podConfig", "podCode"):
                    constructor = constructor.replace(f"EmitterFunc({emitter})", f'rakuEmitter{{"{emitter}"}}')
                pairs = re.findall(r"'\\u([0-9a-f]{4})': '\\u([0-9a-f]{4})'", constructor)
                (assets / "raku-brackets.json").write_text(json.dumps(
                    {chr(int(a, 16)): chr(int(b, 16)) for a, b in pairs},
                    ensure_ascii=False, indent=2, sort_keys=True) + "\n")
            functions.append(constructor)
        if filename == "caddyfile.go":
            functions.extend(re.findall(r"(?m)^var caddyfile\w+Regexp = .*", text))
    rules = "\n".join(functions)
    for before, after in {
        "EmitterFunc(httpContentBlock)": "Generic",
        "EmitterFunc(httpHeaderBlock)": "ByGroups(Name, Text, Operator, Text, Literal, Text)",
        "EmitterFunc(httpContinuousHeaderBlock)": "ByGroups(Text, Literal, Text)",
        "EmitterFunc(rstCodeBlock)": "rstCodeEmitter{}",
        "MutatorFunc(haxePreProcMutator)": "haxePreProcMutator{}",
        # Chroma's Marshal drops nil group slots. Preserve the slot as an
        # Ignore emitter, which the native iterator discards before collection.
        "ByGroups(StringHeredoc, nil, String, String, String, StringHeredoc)":
            "ByGroups(StringHeredoc, Ignore, String, String, String, StringHeredoc)",
    }.items():
        if rules.count(before) != 1:
            raise RuntimeError(f"pinned custom emitter changed: {before}")
        rules = rules.replace(before, after)
    program = '''package main
import (
    "encoding/json"
    "encoding/xml"
    "os"
    "regexp"
    "sort"
    . "github.com/alecthomas/chroma/v2"
    "github.com/alecthomas/chroma/v2/lexers"
)
type rstCodeEmitter struct{}
func (rstCodeEmitter) EmitterKind() string { return "rst-code-block" }
func (rstCodeEmitter) Emit([]string, *LexerState) Iterator { panic("source serialization only") }
type haxePreProcMutator struct{}
func (haxePreProcMutator) MutatorKind() string { return "haxe-preproc" }
func (haxePreProcMutator) Mutate(*LexerState) error { panic("source serialization only") }
type rakuBrackets struct { Class int `xml:"class,attr"` }
func (rakuBrackets) MutatorKind() string { return "raku-brackets" }
func (rakuBrackets) Mutate(*LexerState) error { panic("source serialization only") }
type rakuEmitter struct { Name string `xml:"name,attr"` }
func (rakuEmitter) EmitterKind() string { return "raku-emitter" }
func (rakuEmitter) Emit([]string, *LexerState) Iterator { panic("source serialization only") }
func rakuColonPair(t TokenType) Emitter { return rakuEmitter{"colonPair-" + t.String()} }
type rakuNamedGroups map[string]Emitter
func (rakuNamedGroups) EmitterKind() string { return "raku-groups" }
func (rakuNamedGroups) Emit([]string, *LexerState) Iterator { panic("source serialization only") }
func rakuGroups(m map[string]Emitter) Emitter { return rakuNamedGroups(m) }
func (m rakuNamedGroups) MarshalXML(e *xml.Encoder, start xml.StartElement) error {
    if err := e.EncodeToken(start); err != nil { return err }
    names := []string{}; for name := range m { names = append(names, name) }; sort.Strings(names)
    for _, name := range names {
        group := xml.StartElement{Name: xml.Name{Local:"group"}, Attr: []xml.Attr{{Name:xml.Name{Local:"name"}, Value:name}}}
        if err := e.EncodeToken(group); err != nil { return err }
        value := m[name]; if value == nil { value = Ignore }
        if err := e.EncodeElement(value, xml.StartElement{Name:xml.Name{Local:value.(SerialisableEmitter).EmitterKind()}}); err != nil { return err }
        if err := e.EncodeToken(group.End()); err != nil { return err }
    }
    return e.EncodeToken(start.End())
}
type ruleReplacingConfig struct { delimiter []rune; tokenType Emitter; stateName string; popState, pushState bool }
type rakuReplace struct {
    Delimiter string `xml:"delimiter,attr"`
    Token string `xml:"token,attr"`
    State string `xml:"state,attr"`
    Pop bool `xml:"pop,attr"`
    Push bool `xml:"push,attr"`
}
func (rakuReplace) MutatorKind() string { return "raku-replace" }
func (rakuReplace) Mutate(*LexerState) error { panic("source serialization only") }
func replaceRule(c ruleReplacingConfig) Mutator { return rakuReplace{string(c.delimiter), c.tokenType.(TokenType).String(), c.stateName, c.popState, c.pushState} }
func joinRuneMap(m map[rune]rune) string {
    keys := []rune{}; for k := range m { keys = append(keys, k) }; sort.Slice(keys, func(i,j int) bool { return keys[i]<keys[j] }); return string(keys)
}
''' + rules + '''
func main() {
    result := map[string]string{}
    mimeTypes := map[string]string{}
    for _, lexer := range lexers.GlobalLexerRegistry.Lexers {
        for _, mime := range lexer.Config().MimeTypes {
            mimeTypes[mime] = lexers.MatchMimeType(mime).Config().Name
        }
    }
    mimeJSON, err := json.Marshal(mimeTypes); if err != nil { panic(err) }
    result["mime-types"] = string(mimeJSON)
''' + "\n".join(
        f'    data{i}, err := Marshal(MustNewLexer(lexers.Get({json.dumps(name)}).Config(), {constructor})); '
        f'if err != nil {{ panic(err) }}; result[{json.dumps(filename)}] = string(data{i})'
        for i, (name, filename, constructor) in enumerate(lexers)
    ) + '''
    if err := json.NewEncoder(os.Stdout).Encode(result); err != nil { panic(err) }
}
'''
    with tempfile.TemporaryDirectory(prefix="chroma-source-") as directory:
        work = Path(directory)
        for filename in ("go.mod", "go.sum"):
            shutil.copyfile(references.ROOT / "tools/chroma-code-oracle" / filename, work / filename)
        version = subprocess.check_output(
            ["go", "list", "-m", "-f", "{{.Version}}", "github.com/alecthomas/chroma/v2"],
            cwd=work, text=True,
        ).strip()
        if version != references.MANIFEST["dependencies"]["github.com/alecthomas/chroma/v2"]:
            raise RuntimeError(f"source serializer uses unpinned Chroma {version}")
        (work / "main.go").write_text(program)
        definitions = json.loads(subprocess.check_output(["go", "run", "."], cwd=work, text=True))
    destination = assets / "custom"
    destination.mkdir(exist_ok=True)
    rows = []
    (assets / "mime-types.json").write_text(json.dumps(json.loads(definitions["mime-types"]), indent=2, sort_keys=True) + "\n")
    for name, filename, _ in lexers:
        # Marshal iterates a Go map of independent states. Sort the state
        # blocks for reproducible source files; preserve every rule and byte
        # inside each state, including repeated push attributes.
        text = definitions[filename]
        start = text.index("  <rules>\n") + len("  <rules>\n")
        end = text.index("\n  </rules>")
        states = re.findall(r'(?ms)^    <state name="[^"]*">.*?^    </state>', text[start:end])
        if not states or re.sub(r'(?ms)^    <state name="[^"]*">.*?^    </state>', '', text[start:end]).strip():
            raise RuntimeError(f"unexpected serialized state layout: {name}")
        text = text[:start] + "\n".join(sorted(states)) + text[end:]
        (destination / f"{filename}.xml").write_text(text + "\n")
        rows.append(f'    ({json.dumps(name)}, include_str!("../../assets/chroma/custom/{filename}.xml")),')
    return rows


def main():
    version = references.MANIFEST["dependencies"]["github.com/alecthomas/chroma/v2"]
    source = references.REFERENCE / f"go-mod/pkg/mod/github.com/alecthomas/chroma/v2@{version}/lexers"
    assets = references.ROOT / "crates/d2-rs-text/assets/chroma"
    files = sorted((source / "embedded").glob("*.xml"))
    if not files:
        raise RuntimeError(f"missing pinned Chroma sources: {source}")
    destination = assets / "lexers"
    destination.mkdir(exist_ok=True)
    rows = []
    for path in files:
        xml = path.read_text()
        config = ET.fromstring(xml[xml.index("<config>"):xml.index("</config>") + 9])
        name = json.dumps(config.findtext("name"), ensure_ascii=False)
        shutil.copyfile(path, destination / path.name)
        rows.append(f'    ({name}, include_str!("../../assets/chroma/lexers/{path.name}")),')
    rows.extend(custom_definitions(source, assets))
    index = references.ROOT / "crates/d2-rs-text/src/code/xml_lexers.rs"
    index.write_text(
        "// SPDX-License-Identifier: MIT\n"
        f"// Chroma {version} embedded and custom lexer rules, indexed by config.name.\n"
        "// Generated by scripts/vendor-chroma-lexers.py; see assets/chroma/COPYING.\n"
        "pub(super) const SOURCES: &[(&str, &str)] = &[\n" + "\n".join(rows) + "\n];\n"
    )
    subprocess.run(["rustfmt", "--edition", "2024", str(index)], check=True)
    mappings = {}
    specifications = [
        ("cl.go", "Common Lisp", [
            ("clBuiltinFunctions", "NameFunction"), ("clSpecialForms", "Keyword"),
            ("clMacros", "NameBuiltin"), ("clLambdaListKeywords", "Keyword"),
            ("clDeclarations", "Keyword"), ("clBuiltinTypes", "KeywordType"),
            ("clBuiltinClasses", "NameClass"),
        ]),
        ("emacs.go", "EmacsLisp", [
            ("emacsBuiltinFunction", "NameFunction"), ("emacsSpecialForms", "NameBuiltin"),
            ("emacsErrorKeywords", "NameException"), ("emacsBuiltinFunctionHighlighted", "NameBuiltin"),
            ("emacsMacros", "NameBuiltin"), ("emacsLambdaListKeywords", "KeywordPseudo"),
        ]),
    ]
    for filename, lexer, groups in specifications:
        text = (source / filename).read_text()
        arrays = {m[1]: re.findall(r'"([^"\n]*)"', m[2])
                  for m in re.finditer(r"(?ms)^\s+(\w+) = \[\]string\{(.*?)^\s*\}", text)}
        # remap.go TypeRemappingLexer: last entry for the same word wins.
        mappings[lexer] = {word: kind for name, kind in groups for word in arrays[name]}
    (assets / "lisp-mappings.json").write_text(json.dumps(mappings, indent=2, sort_keys=True) + "\n")
    print(f"Vendored {len(files)} XML lexers, {len(rows) - len(files)} custom rule sets "
          f"and {len(mappings)} Lisp tables from Chroma {version}")


if __name__ == "__main__":
    main()
