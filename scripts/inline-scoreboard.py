#!/usr/bin/env python3
"""Score pinned upstream tests whose expectations are inline, not goldens.

An env-gated hook in the instrumentation checkout records every subtest's
request and Go's own result beside the original assertion. A case can pass
only when that Go subtest passes and the native result equals Go's result
exactly. Suites:

- format: d2format TestPrint, scored through `d2-rs fmt` (d2cli/fmt.go's
  parse-then-format path).
- format-escape: d2format's TestEscape* string-node cases; the runner rebuilds
  each test's AST node and formats it with d2_rs_language::format.
- lsp-completion: d2lsp TestGetCompletionItems and its UTF-16 variant, scored
  through d2_rs_editor::completion_items with upstream's own contract,
  equalCompletionSets (completion_test.go:438-452): equal length and equal
  label-keyed items. Go builds style completions by ranging over a map, so
  their order is unspecified; a nil Go slice is the same empty result.
"""
from __future__ import annotations

import argparse
import collections
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess

from references import verify_source
import references

ROOT = Path(__file__).resolve().parents[1]
TRACE = references.TRACE_CHECKOUT


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


CLI = module("inline_cli", "cli-scoreboard.py")

# Per suite: Go package directory, its test package name, `go test -run`
# pattern, and (anchor line, hook line inserted before or after it).
SUITES = {
    "format": ("d2format", "d2format_test", "^TestPrint$", [
        ("\t\t\tassert.String(t, tc.exp, d2format.Format(ast))",
         '\t\t\td2rsInlineDump(t, map[string]interface{}{"in": tc.in, "exp": tc.exp}, d2format.Format(ast), nil)\n', "before"),
    ]),
    "format-escape": ("d2format", "d2format_test", "^TestEscape(SingleQuoted|DoubleQuoted|Unquoted|BlockString)$", [
        ("\t\t\tassert.String(t, tc.exp, d2format.Format(&d2ast.SingleQuotedString{",
         '\t\t\td2rsInlineDump(t, map[string]interface{}{"kind": "single", "str": tc.str}, d2format.Format(&d2ast.SingleQuotedString{Value: tc.str}), nil)\n', "before"),
        ("\t\t\t\tn = d2ast.FlatDoubleQuotedString(tc.str)\n\t\t\t}",
         '\n\t\t\td2rsInlineDump(t, map[string]interface{}{"kind": "double", "str": tc.str, "in_key": tc.inKey}, d2format.Format(n), nil)', "after"),
        ("\t\t\t\tn = d2ast.FlatUnquotedString(tc.str)\n\t\t\t}",
         '\n\t\t\td2rsInlineDump(t, map[string]interface{}{"kind": "unquoted", "str": tc.str, "in_key": tc.inKey}, d2format.Format(n), nil)', "after"),
        ("\t\t\t\tValue: tc.value,\n\t\t\t}",
         '\n\t\t\td2rsInlineDump(t, map[string]interface{}{"kind": "block", "quote": tc.quote, "tag": tc.tag, "value": tc.value}, d2format.Format(n), nil)', "after"),
    ]),
    "lsp-completion": ("d2lsp", "d2lsp", "^TestGetCompletionItems(UTF16)?$", [
        ("\t\t\tgot, err := GetCompletionItems(tt.text, tt.line, tt.column)",
         '\n\t\t\td2rsInlineDump(t, map[string]interface{}{"text": tt.text, "line": tt.line, "column": tt.column, "utf16": false}, got, err)', "after"),
        ("\t\t\tgot, err := GetCompletionItemsUTF16(tt.text, 0, tt.column)",
         '\n\t\t\td2rsInlineDump(t, map[string]interface{}{"text": tt.text, "line": 0, "column": tt.column, "utf16": true}, got, err)', "after"),
    ]),
}


def install(suite):
    verify_source()
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=TRACE, text=True).strip()
    assert revision == references.SOURCE_COMMIT, revision
    relative, package, _, hooks = SUITES[suite]
    directory = TRACE / relative
    helper = (ROOT / "tools/inline-scoreboard/hook_test.go").read_text().replace("PACKAGE", package)
    (directory / "d2rs_inline_scoreboard_test.go").write_text(helper)
    for test_file in sorted(directory.glob("*_test.go")):
        text = test_file.read_text()
        for anchor, hook, where in hooks:
            new = hook + anchor if where == "before" else anchor + hook
            if anchor in text and new not in text:
                assert text.count(anchor) == 1, (test_file, anchor)
                text = text.replace(anchor, new)
        test_file.write_text(text)
    return directory


def native(suite, case, work, rust, editor, capture):
    """Return (result, failure) for one captured request."""
    work.mkdir(parents=True)
    if suite == "format":
        source = work / "in.d2"
        source.write_text(case["request"]["in"])
        run = CLI.bounded_run([str(rust), "fmt", str(source)], os.environ.copy(), capture,
                              work / "process.log", 60)
        failure = run["failure"] or ("rust-failed" if run["exit"] else None)
        return (source.read_text() if source.is_file() else None), failure
    request = work / "request.json"
    request.write_text(json.dumps(case["request"]))
    output = work / "result.json"
    with output.open("wb") as stream:
        process = subprocess.run([str(editor), str(request)], stdout=stream, timeout=60)
    if process.returncode:
        return None, "rust-failed"
    return json.loads(output.read_text()), None


def equal(suite, case, actual):
    if suite != "lsp-completion":
        return actual == case["got"]
    expected = case["got"] or []
    return (len(actual) == len(expected)
            and {item["Label"]: item for item in actual} == {item["Label"]: item for item in expected})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", choices=sorted(SUITES), required=True)
    parser.add_argument("--rust", type=Path, default=ROOT / "target/diagnostic/d2-rs")
    parser.add_argument("--editor", type=Path, default=ROOT / "target/oracle/examples/inline-scoreboard")
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.suite != "format" and not args.editor.is_file():
        parser.error("build first: cargo build --profile oracle -p d2-rs-editor --example inline-scoreboard")
    capture = args.capture.resolve()
    assert not capture.exists(), "use a fresh capture path"
    directory = install(args.suite)
    (capture / "go").mkdir(parents=True)
    environment = dict(os.environ, D2RS_INLINE_DUMP=str(capture / "go"),
                       GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
                       GOCACHE=str(ROOT / ".reference/go-build"), GOFLAGS="-mod=mod", GOPROXY="off")
    command = ["go", "test", "./" + str(directory.relative_to(TRACE)), "-run", SUITES[args.suite][2],
               "-count=1", "-json"]
    result = CLI.bounded_run(command, environment, capture, capture / "go.log", 600)
    if result["failure"]:
        raise RuntimeError(result)
    events = CLI.events(capture / "go.log")
    rows = []
    for path in sorted((capture / "go").glob("*.json")):
        case = json.loads(path.read_text())
        go_test = events.get(case["name"], "missing-event")
        actual, failure = native(args.suite, case, capture / "rust" / path.stem,
                                 args.rust.resolve(), args.editor.resolve(), capture)
        if go_test != "pass":
            outcome = "go-" + go_test
        elif failure:
            outcome = failure
        else:
            outcome = "match" if equal(args.suite, case, actual) else "mismatch"
        rows.append({"name": case["name"], "go_test": go_test, "outcome": outcome})
    # Every live subtest of the selected tests must be inventoried.
    captured = {row["name"] for row in rows}
    selected = re.compile(SUITES[args.suite][2])
    for name, status in sorted(events.items()):
        if "/" in name and selected.match(name.split("/")[0]) and name not in captured:
            rows.append({"name": name, "go_test": status, "outcome": "go-missing-capture"})
    report = {"suite": args.suite, "source_commit": references.SOURCE_COMMIT,
              "go_run": result, "summary": dict(collections.Counter(row["outcome"] for row in rows)),
              "rows": rows}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(args.suite, report["summary"], "cases", len(rows))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
