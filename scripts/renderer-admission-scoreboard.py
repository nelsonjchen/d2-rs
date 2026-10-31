#!/usr/bin/env python3
"""Observe original master renderer calls; compare complete native results.

Eight original functions; 41 calls plus the Go source-syntax ratchet. No output normalization. Original assertions
remain active. Streaming limits: 1 GiB total, 128 MiB/file, 20 GiB free;
300-second subprocess deadline, fresh capture and native paths.
"""

import argparse
import collections
import hashlib
import importlib.util
import json
import os
import re
from pathlib import Path

import references

ROOT = references.ROOT
TEMPLATE = ROOT / "tools/renderer-contract-scoreboard/observer.go.txt"
GRAPH = ROOT / "tools/renderer-contract-scoreboard/graph.go.txt"
SUITES = {
    "svg": {
        "package": "d2renderers/d2svg",
        "files": ["isometric_test.go", "serialization_ratchet_test.go"],
        "tests": {
            "TestRenderIsometricValidatesRawTargets": 18,
            "TestSVGSerializationRatchet": 0,
        },
        "imports": '"context"\n"math"\n"github.com/d2lang/d2/lib/geo"\n"github.com/d2lang/d2/d2target"\n"github.com/d2lang/d2/d2renderers/d2isometric/d2isometricimg"',
        "replacements": {
            "Render(": "d2rsRender(t, ",
            "RenderIsometric(": "d2rsIsometric(t, ",
            "RenderMultiboard(": "d2rsMultiboard(t, ",
        },
        "wrappers": 'func d2rsRender(t *testing.T, d *d2target.Diagram, opts *RenderOpts) ([]byte,error) {\n    out,err:=Render(d,opts)\n    d2rsRecord(t,map[string]any{"operation":"svg", "graph":d2rsGraph(d),"opts":opts},out,err)\n    return out,err\n}\nfunc d2rsIsometric(t *testing.T, ctx context.Context, d *d2target.Diagram, opts *RenderOpts, native *d2isometricimg.Options) ([]byte,error) {\n    out,err:=RenderIsometric(ctx,d,opts,native)\n    d2rsRecord(t,map[string]any{"operation":"isometric", "graph":d2rsGraph(d),"opts":opts},out,err)\n    return out,err\n}\nfunc d2rsMultiboard(t *testing.T, d *d2target.Diagram, opts *RenderOpts) ([][]byte,error) {\n    out,err:=RenderMultiboard(d,opts)\n    d2rsRecord(t,map[string]any{"operation":"multiboard", "graph":d2rsGraph(d),"opts":opts},out,err)\n    return out,err\n}',
        "graph": True,
        "sources": ["d2svg.go", "isometric.go", "../../d2target/render_validation.go"],
    },
    "svg_direct": {
        "package": "d2renderers/d2svg",
        "go_package": "d2svg_test",
        "files": ["render_target_validation_test.go"],
        "tests": {
            "TestRenderRejectsNonFiniteDirectTargetWithoutPanicking": 1,
            "TestRenderRejectsCyclicBoardBeforeHashing": 1,
        },
        "imports": '"math"\n"github.com/d2lang/d2/lib/geo"\n"github.com/d2lang/d2/d2target"\n"github.com/d2lang/d2/d2renderers/d2svg"',
        "replacements": {"d2svg.Render(": "d2rsRender(t, "},
        "wrappers": 'func d2rsRender(t *testing.T, d *d2target.Diagram, opts *d2svg.RenderOpts) ([]byte,error) {\n    out,err:=d2svg.Render(d,opts)\n    d2rsRecord(t,map[string]any{"operation":"svg", "graph":d2rsGraph(d),"opts":opts},out,err)\n    return out,err\n}\n',
        "graph": True,
        "sources": ["d2svg.go", "../../d2target/render_validation.go"],
    },
    "target": {
        "package": "d2target",
        "files": ["render_validation_test.go"],
        "tests": {
            "TestValidateRenderTargetBoundsBoardTree": 6,
            "TestValidateRenderTargetRejectsReusedBoard": 1,
        },
        "imports": '"math"\n"github.com/d2lang/d2/lib/geo"',
        "replacements": {
            "ValidateRenderTarget(": "d2rsValidateTarget(t, ",
            "root.HashID(": "d2rsHash(t, root, ",
        },
        "wrappers": 'func d2rsValidateTarget(t *testing.T, d *Diagram) error {\n    err:=ValidateRenderTarget(d)\n    d2rsRecord(t,map[string]any{"operation":"target_validate", "graph":d2rsGraph(d)},nil,err)\n    return err\n}\nfunc d2rsHash(t *testing.T, d *Diagram, salt *string) (string,error) {\n    out,err:=d.HashID(salt)\n    d2rsRecord(t,map[string]any{"operation":"target_hash", "graph":d2rsGraph(d)},out,err)\n    return out,err\n}',
        "graph": True,
        "sources": ["render_validation.go", "d2target.go"],
    },
    "latex": {
        "package": "d2renderers/d2latex",
        "files": ["latex_test.go"],
        "tests": {"TestValidateInputTeXEscapingAndComments": 8},
        "imports": '"encoding/base64"',
        "replacements": {"ValidateInput(": "d2rsValidate(t, "},
        "wrappers": 'func d2rsValidate(t *testing.T, input string) error {\n    err:=ValidateInput(input)\n    d2rsRecord(t,map[string]any{"operation":"latex_validate", "bytes":base64.StdEncoding.EncodeToString([]byte(input))},nil,err)\n    return err\n}',
        "graph": False,
        "sources": ["limits.go"],
    },
    "sketch": {
        "package": "d2renderers/d2sketch",
        "files": ["serialization_security_test.go"],
        "tests": {"TestConnectionRejectsConflictingAttributesWithoutPanic": 6},
        "imports": '"github.com/d2lang/d2/d2target"\n"github.com/d2lang/d2/lib/svg"\n"reflect"',
        "replacements": {
            "Connection(": "d2rsConnection(t, ",
            "ConnectionWithAttributes(": "d2rsConnectionTyped(t, ",
        },
        "wrappers": 'func d2rsConnection(t *testing.T, c d2target.Connection, path, attrs string) (string,error) {\n    out,err:=Connection(c,path,attrs)\n    d2rsRecord(t,map[string]any{"operation":"sketch", "connection":c,"path":path,"attributes":attrs},out,err)\n    return out,err\n}\nfunc d2rsConnectionTyped(t *testing.T, c d2target.Connection, path string, attrs []svg.Attribute) (string,error) {\n    encoded:=[]any{}\n    for _,a:=range attrs {name,err:=a.Name();if err!=nil {encoded=append(encoded,nil)} else {encoded=append(encoded,map[string]any{"name":name,"value":reflect.ValueOf(a).FieldByName("value").String()})}}\n    out,err:=ConnectionWithAttributes(c,path,attrs)\n    d2rsRecord(t,map[string]any{"operation":"sketch_typed", "connection":c,"path":path,"attributes":encoded},out,err)\n    return out,err\n}',
        "graph": False,
        "sources": ["sketch.go", "../../lib/svg/markup.go"],
    },
}


def module(name):
    spec = importlib.util.spec_from_file_location(
        name, ROOT / "scripts" / (name + ".py")
    )
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def transform(text, replacements):
    # Go comments and string/rune literals are opaque; assertion text stays exact.
    tokens = re.split(
        r'("(?:\\.|[^"\\])*"|`[^`]*`|\'(?:\\.|[^\'\\])*\'|//[^\n]*|/\*[\s\S]*?\*/)',
        text,
    )
    for i in range(0, len(tokens), 2):
        for old, new in replacements.items():
            tokens[i] = re.sub(r"\b" + re.escape(old), new, tokens[i])
    return "".join(tokens)


def source():
    pinned = references.verify_source()
    names = [
        str(Path(s["package"]) / n)
        for s in SUITES.values()
        for n in [*s["files"], *s["sources"]]
    ]
    return {
        "commit": references.SOURCE_COMMIT,
        "files": {n: sha(pinned / n) for n in names},
        "observer": sha(TEMPLATE),
        "graph_transport": sha(GRAPH),
        "script": sha(Path(__file__)),
    }


def hashes(capture):
    paths = [p for p in capture.rglob("*") if p.is_file() and p.name != "hashes.json"]
    return {str(p.relative_to(capture)): sha(p) for p in sorted(paths)}


def verify(capture):
    assert json.loads((capture / "source.json").read_text()) == source()
    assert json.loads((capture / "hashes.json").read_text()) == hashes(capture)
    cases = []
    for key, suite in SUITES.items():
        root = capture / key
        assert json.loads((root / "run.json").read_text()) == {
            "exit": 0,
            "failure": None,
        }
        events = module("cli-scoreboard").events(root / "events.jsonl")
        assert all(events.get(t) == "pass" for t in suite["tests"]), events
        rows = [
            json.loads(p.read_text()) for p in sorted((root / "calls").glob("*.json"))
        ]
        assert collections.Counter(r["test"].split("/")[0] for r in rows) == {
            t: n for t, n in suite["tests"].items() if n
        }
        assert all(
            r["package"] == suite["package"] and "request" in r and "got" in r
            for r in rows
        )
        assert len({r["name"] for r in rows}) == len(rows)
        assert all(set(r["got"]) == {"output", "error"} for r in rows)
        counts = collections.Counter(r["test"] for r in rows)
        assert {r["name"] for r in rows} == {
            f"{t}#{i}" for t, n in counts.items() for i in range(n)
        }
        cases.extend(rows)
    sizes = [p.stat().st_size for p in capture.rglob("*") if p.is_file()]
    assert sizes and max(sizes) <= 128 << 20 and sum(sizes) <= 1 << 30
    return cases


def capture_original(capture):
    capture.mkdir(exist_ok=False)
    pinned = references.verify_source()
    (capture / "source.json").write_text(json.dumps(source(), indent=2) + "\n")
    for key, suite in SUITES.items():
        root = capture / key
        (root / "calls").mkdir(parents=True)
        tests = [
            references.TRACE_CHECKOUT / suite["package"] / f for f in suite["files"]
        ]
        originals = [
            (pinned / suite["package"] / f).read_bytes() for f in suite["files"]
        ]
        helper = tests[0].parent / "d2rs_renderer_contract_test.go"
        assert (
            all(t.read_bytes() == original for t, original in zip(tests, originals))
            and not helper.exists()
        )
        hook = TEMPLATE.read_text().replace("PACKAGE_PATH", suite["package"])
        hook = hook.replace(
            "PACKAGE", suite.get("go_package", Path(suite["package"]).name)
        )
        wrappers = suite["wrappers"]
        if suite["graph"]:
            wrappers += GRAPH.read_text().replace(
                "TARGET.", "" if key == "target" else "d2target."
            )
        hook = hook.replace("EXTRA_IMPORTS", suite["imports"]).replace(
            "WRAPPERS", wrappers
        )
        env = dict(
            os.environ,
            D2RS_RENDERER_CONTRACT_DUMP=str(root / "calls"),
            GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
            GOCACHE=str(ROOT / ".reference/go-build"),
            GOFLAGS="-mod=mod",
            GOPROXY="off",
        )
        env.pop("TA", None)
        env.pop("TESTDATA_ACCEPT", None)
        try:
            for test, original in zip(tests, originals):
                test.write_text(transform(original.decode(), suite["replacements"]))
            helper.write_text(hook)
            run = module("cli-scoreboard").bounded_run(
                [
                    "go",
                    "test",
                    "./" + suite["package"],
                    "-json",
                    "-count=1",
                    "-parallel=1",
                    "-ldflags=-X github.com/d2lang/d2/lib/version.Version=v0.9.0",
                    "-run",
                    "^(" + "|".join(suite["tests"]) + ")$",
                ],
                env,
                capture,
                root / "events.jsonl",
                300,
            )
        finally:
            for test, original in zip(tests, originals):
                test.write_bytes(original)
            helper.unlink(missing_ok=True)
        (root / "run.json").write_text(json.dumps(run) + "\n")
        assert run == {"exit": 0, "failure": None}, (key, run)
    (capture / "hashes.json").write_text(json.dumps(hashes(capture), indent=2) + "\n")
    verify(capture)


def unsupported(request):
    if request["operation"] == "latex_validate":
        import base64

        try:
            base64.b64decode(request["bytes"]).decode("utf-8")
        except UnicodeError:
            return True
    graph = request.get("graph")
    if not graph:
        return False
    seen = {0}
    for node in graph:
        for children in node["edges"].values():
            for child in children or []:
                if child is not None:
                    if child in seen:
                        return True
                    seen.add(child)
    return False


def score(capture, runner, output, native_test, missing_typed_api=False):
    assert not output.exists()
    cases = verify(capture)
    work = output.with_suffix("")
    work.mkdir(exist_ok=False)
    responses = {}
    runners = {"core": native_test, "public": runner}
    runner_hashes = {k: sha(p) for k, p in runners.items()}
    for key, binary in runners.items():
        subset = [
            (i, c)
            for i, c in enumerate(cases)
            if not unsupported(c["request"])
            and not (missing_typed_api and c["request"]["operation"] == "sketch_typed")
            and (
                c["request"]["operation"]
                in {"isometric", "multiboard", "latex_validate"}
                or c["request"]["operation"] == "svg"
                and c["request"].get("opts") is not None
            )
            == (key == "public")
        ]
        path = work / (key + "-requests.json")
        path.write_text(json.dumps([c["request"] for i, c in subset]))
        log = work / (key + "-log.txt")
        env = os.environ.copy()
        if key == "core":
            response = work / (key + "-responses.json")
            env.update(
                D2RS_RENDER_ADMISSION_INPUT=str(path),
                D2RS_RENDER_ADMISSION_OUTPUT=str(response),
            )
            command = [
                str(binary),
                "target::admission_api_tests::original_calls",
                "--exact",
                "--nocapture",
            ]
        else:
            response = log
            command = [str(binary), str(path)]
        run = module("cli-scoreboard").bounded_run(command, env, work, log, 300)
        try:
            actual = (
                json.loads(response.read_text())
                if run == {"exit": 0, "failure": None}
                else None
            )
        except (ValueError, UnicodeError, FileNotFoundError):
            actual = {}
        assert sha(binary) == runner_hashes[key]
        for j, (i, c) in enumerate(subset):
            if actual is None:
                responses[i] = {"outcome": run["failure"] or "rust-failed"}
            elif not isinstance(actual, list) or len(actual) != len(subset):
                responses[i] = {"outcome": "invalid-inventory"}
            elif module("language-scoreboard").same_json(actual[j], c["got"]):
                responses[i] = {"outcome": "match"}
            else:
                responses[i] = {
                    "outcome": "mismatch",
                    "expected": c["got"],
                    "actual": actual[j],
                }
    rows = []
    for i, c in enumerate(cases):
        row = {k: c[k] for k in ["name", "test", "package"]}
        if missing_typed_api and c["request"]["operation"] == "sketch_typed":
            row.update(
                outcome="unsupported-native-api",
                expected=c["got"],
                reason="Negative baseline has no ConnectionWithAttributes entry point",
            )
            rows.append(row)
            continue
        row.update(
            {
                "outcome": "unsupported",
                "reason": "Go repeated board pointer or invalid UTF-8 string cannot be represented by the corresponding Rust type",
                "expected": c["got"],
            }
            if unsupported(c["request"])
            else responses[i]
        )
        rows.append(row)
    rows.append(
        {
            "test": "TestSVGSerializationRatchet",
            "package": "d2renderers/d2svg",
            "name": "TestSVGSerializationRatchet",
            "outcome": "not-applicable",
            "reason": "Go AST/source-format allowlist maintenance test; original Go assertion passed, no native API call",
        }
    )
    report = {
        "source_commit": references.SOURCE_COMMIT,
        "capture": str(capture),
        "capture_hashes": hashes(capture),
        "runners": {
            k: {"path": str(p), "sha256": runner_hashes[k]} for k, p in runners.items()
        },
        "summary": dict(collections.Counter(r["outcome"] for r in rows)),
        "rows": rows,
    }
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(report["summary"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True, type=Path)
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument("--rust", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--native-test", type=Path)
    parser.add_argument(
        "--missing-typed-api",
        action="store_true",
        help="Explicit negative-baseline classification before ConnectionWithAttributes exists",
    )
    args = parser.parse_args()
    if not args.reuse:
        capture_original(args.capture.resolve())
    if args.output:
        assert args.rust and args.native_test
        score(
            args.capture.resolve(),
            args.rust.resolve(),
            args.output.resolve(),
            args.native_test.resolve(),
            args.missing_typed_api,
        )


if __name__ == "__main__":
    main()
