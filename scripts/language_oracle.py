"""Build and run the d2-rs-language Go oracles for fixture recipes.

Used by the `kind = "command"` generators of tools/fixtures/langlibelk-language.toml
(docs/FIXTURES.md). Every harness builds in the recipe workdir against the
runner's patched upstream checkout ($D2RS_UPSTREAM, the manifest pin), with
upstream's own go.mod/go.sum so the module graph is exactly the pinned one:

- parser-oracle: tools/parser-oracle/main.go compiled together with a copy of
  upstream d2parser/parse.go whose package clause is renamed to `main` (the
  only source change); private d2ir functions are reached by go:linkname.
- import-selection-oracle: tools/import-selection-oracle/main.go plus its
  bridge.go overlaid into d2ir as d2ir/oracle_selection.go.
- field-oracle: tools/field-oracle/main.go plus tools/field-oracle/bridge.go
  added to d2ir as d2ir/oracle_fields.go through `go build -overlay`; the
  trace build also overlays d2ir/compile.go with key enter/exit events.
  The checkout itself is never modified.

Generators write only the staged outputs; `cargo test -p d2-rs-language
--features oracle-fixtures` does the native comparison.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

from fixture_recipe import go_env, output, workdir
from references import ROOT


def upstream() -> Path:
    value = os.environ.get("D2RS_UPSTREAM")
    if not value:
        sys.exit("D2RS_UPSTREAM is unset: the recipe must set upstream = true")
    return Path(value)


def _module(name: str) -> Path:
    """Scratch module requiring exactly upstream's module graph."""
    source = upstream()
    module = workdir() / name
    if module.exists():
        shutil.rmtree(module)
    module.mkdir(parents=True)
    go_mod, count = re.subn(r"(?m)^module github\.com/d2lang/d2$", f"module d2rs.local/{name}",
                            (source / "go.mod").read_text())
    assert count == 1, "upstream go.mod has no `module github.com/d2lang/d2` line"
    go_mod += ("\nrequire github.com/d2lang/d2 v0.0.0-00010101000000-000000000000\n"
               f"\nreplace github.com/d2lang/d2 => {source}\n")
    (module / "go.mod").write_text(go_mod)
    shutil.copy2(source / "go.sum", module / "go.sum")
    return module


def _build(module: Path, overlay: dict[str, str] | None = None) -> Path:
    binary = workdir() / "bin" / module.name
    argv = ["go", "build", "-mod=readonly"]
    if overlay:
        (module / "overlay.json").write_text(json.dumps({"Replace": overlay}))
        argv += ["-overlay", str(module / "overlay.json")]
    subprocess.run([*argv, "-o", str(binary), "."], cwd=module, env=go_env(), check=True)
    return binary


_built: dict[str, Path] = {}


def parser_oracle() -> Path:
    if "parser" not in _built:
        module = _module("parser-oracle")
        text = (upstream() / "d2parser/parse.go").read_text()
        assert text.startswith("package d2parser\n"), "d2parser/parse.go package clause moved"
        (module / "parse.go").write_text(text.replace("package d2parser\n", "package main\n", 1))
        shutil.copy2(ROOT / "tools/parser-oracle/main.go", module / "main.go")
        _built["parser"] = _build(module)
    return _built["parser"]


def field_oracle(trace: bool = False) -> Path:
    key = "field-trace" if trace else "field"
    if key not in _built:
        source = upstream()
        module = _module("field-oracle-trace" if trace else "field-oracle")
        shutil.copy2(ROOT / "tools/field-oracle/main.go", module / "main.go")
        overlay = {str(source / "d2ir/oracle_fields.go"): str(ROOT / "tools/field-oracle/bridge.go")}
        if trace:
            original = (source / "d2ir/compile.go").read_text()
            marker = "func (c *compiler) compileKey(refctx *RefContext) {"
            assert original.count(marker) == 1, "d2ir compileKey signature changed"
            instrumented = original.replace(marker, marker + '\n oracleTraceEnabled=true\n oracleCompilerEvent(c,"enter",refctx)\n defer oracleCompilerEvent(c,"exit",refctx)')
            # The initializer also identifies an instrumented empty compilation.
            instrumented += "\nfunc init(){oracleTraceEnabled=true}\n"
            (module / "compile-trace.go.txt").write_text(instrumented)
            overlay[str(source / "d2ir/compile.go")] = str(module / "compile-trace.go.txt")
        _built[key] = _build(module, overlay)
    return _built[key]


def import_selection_oracle() -> Path:
    """tools/import-selection-oracle: main.go plus bridge.go overlaid as d2ir/oracle_selection.go."""
    if "import-selection" not in _built:
        module = _module("import-selection-oracle")
        shutil.copy2(ROOT / "tools/import-selection-oracle/main.go", module / "main.go")
        overlay = {str(upstream() / "d2ir/oracle_selection.go"): str(ROOT / "tools/import-selection-oracle/bridge.go")}
        _built["import-selection"] = _build(module, overlay)
    return _built["import-selection"]


def call(binary: Path, requests=None, *args: str, timeout: float = 1500, cwd: Path | None = None):
    """Run an oracle with JSON requests on stdin; decode its JSON stdout.

    Corpus-mining modes take upstream paths relative to `cwd` (the upstream
    tree), so recorded origins are host-independent.
    """
    stdin = None if requests is None else json.dumps(requests).encode()
    result = subprocess.run([str(binary), *args], input=stdin, stdout=subprocess.PIPE, check=True, timeout=timeout,
                            cwd=cwd)
    return json.loads(result.stdout)


def write(logical: str, value, *, compact: bool = False) -> Path:
    """Write one staged output in the historical fixture layout."""
    path = output(logical)
    text = json.dumps(value, separators=(",", ":")) if compact else json.dumps(value, indent=2)
    path.write_text(text + "\n")
    return path


def fixture(name: str) -> str:
    return f"crates/d2-rs-language/tests/fixtures/{name}"

