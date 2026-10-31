#!/usr/bin/env python3
"""Original isometric private-network test, six paired Go/native CLI calls.

The original assertions run against native returns; Go is checked independently.
Both calls share the server URL, source path and complete input inventory.
SVG/PDF compare full bytes under explicit equal version/UTC clock controls.
PNG uses the existing complete decoded-pixel gate. No outputs are normalized.
"""
import argparse
import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import xml.etree.ElementTree as ET

import references

ROOT = references.ROOT
TEST = "TestIsometricAssetsHonorPrivateNetworkPolicy"


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


CLI = module("policy_cli", "cli-scoreboard.py")


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def decode(value):
    return base64.b64decode(value, validate=True)


def compare(calls, execution, events, directory):
    assert len(calls) == 6, "missing/extra original calls"
    rows = []
    for i, call in enumerate(calls):
        fmt = ["svg", "png", "pdf"][i // 2]
        allowed = bool(i % 2)
        assert call["test"] == TEST + "/" + fmt
        assert type(call["allowed"]) is bool and call["allowed"] == allowed
        args = (["--allow-private-network"] if allowed else []) + ["--isometric", "--scale=.1", "--stdout-format=" + fmt, "-"]
        assert call["args"] == args, "changed original arguments"
        assert call["source"].startswith("a: {icon: http://127.0.0.1:")
        assert call["source"].endswith("/" + fmt + "/icon.svg}"), "changed original source"
        assert set(call["results"]) == {"go", "native"}
        left, right = call["results"]["go"], call["results"]["native"]
        for side, result in [("go", left), ("native", right)]:
            valid = (type(result["requests"]) is int and set(result["files"]) == {"input.d2"}
                     and type(result["files"]["input.d2"]["mode"]) is int
                     and decode(result["files"]["input.d2"]["data"]) == call["source"].encode())
            data = decode(result["data"]) if result["data"] is not None else b""
            valid = valid and ((result["error"] == "" and data and result["requests"] > 0) if allowed
                               else (result["error"] != "" and not data and result["requests"] == 0))
            if side == "go":
                assert valid, "Go did not reproduce original assertion"
            else:
                native_valid = bool(valid)
        matched = native_valid and left["error"] == right["error"] and left["files"] == right["files"] and left["requests"] == right["requests"]
        if allowed:
            paths = []
            for side, result in [("go", left), ("native", right)]:
                path = directory / f"{i:02d}-{side}.{fmt}"
                assert not path.exists(), "fresh artifact path required"
                path.write_bytes(decode(result["data"]))
                paths.append(path)
            if fmt == "png":
                # This helper validates nonempty, complete inventories on both
                # sides, then every PNG and decoded pixel through the shared gate.
                matched = matched and CLI.RASTER.compare_png_outputs(*paths)["matched"]
            elif fmt == "svg":
                for path in paths:
                    assert ET.fromstring(path.read_bytes()).tag == "{http://www.w3.org/2000/svg}svg"
                matched = matched and paths[0].read_bytes() == paths[1].read_bytes()
            else:
                for path in paths:
                    data = path.read_bytes()
                    assert data.startswith(b"%PDF-") and data.rstrip().endswith(b"%%EOF"), "incomplete PDF"
                    assert b"/Type /Page" in data and b"/Subtype /Image" in data, "missing PDF page/image"
                matched = matched and paths[0].read_bytes() == paths[1].read_bytes()
        rows.append(dict(name=call["test"] + ("/allowed" if allowed else "/denied"),
                         outcome="match" if matched else "mismatch", format=fmt, allowed=allowed))
    if execution != {"exit": 0, "failure": None} or events.get(TEST) != "pass" or any(events.get(TEST + "/" + fmt) != "pass" for fmt in ["svg", "png", "pdf"]):
        for row in rows:
            row["outcome"] = "execution-failed"
    return rows


def instrument(original):
    text = original.decode()
    start = text.index("func " + TEST + "(")
    end = text.index("\nfunc ", start + 1)
    body = text[start:end]
    assert body.count("imageCLI(t,") == 2
    return text[:start] + body.replace("imageCLI(t,", "d2rsPolicyCLI(t, &requests,") + text[end:]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust", required=True, type=Path)
    parser.add_argument("--capture", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--pdf-epoch", required=True, type=int)
    args = parser.parse_args()
    assert 0 <= args.pdf_epoch <= 253402300799
    source = references.verify_source()
    runner, capture, output = args.rust.resolve(), args.capture.resolve(), args.output.resolve()
    assert not output.exists()
    capture.mkdir(exist_ok=False)
    tmp = capture / "tmp"
    tmp.mkdir()
    trace = references.TRACE_CHECKOUT
    original = (source / "d2cli/isometric_integration_test.go").read_bytes()
    image_original = (source / "d2cli/isometric_image_test.go").read_bytes()
    pdf_original = (source / "lib/pdf/pdf.go").read_bytes()
    target, pdf = trace / "d2cli/isometric_integration_test.go", trace / "lib/pdf/pdf.go"
    image_target = trace / "d2cli/isometric_image_test.go"
    helper, clock = trace / "d2cli/d2rs_policy_test.go", trace / "lib/pdf/d2rs_clock.go"
    assert target.read_bytes() == original and image_target.read_bytes() == image_original and not helper.exists(), "concurrent trace mutation"
    hook = ROOT / "tools/private-asset-scoreboard/hook_test.go"
    clock_source = ROOT / "tools/cli-scoreboard/pdf_clock.go"
    old = "newGofPDF.SetMargins(0, 0, 0)"
    assert pdf_original.decode().count(old) == 1
    instrumented_pdf = pdf_original.decode().replace(old, old + "\n\td2rsPDFClock(newGofPDF)").encode()
    pdf_before = pdf.read_bytes()
    clock_before = clock.read_bytes() if clock.exists() else None
    assert (pdf_before == pdf_original and clock_before is None) or (pdf_before == instrumented_pdf and clock_before == clock_source.read_bytes()), "unknown PDF instrumentation"
    digest = sha(runner)
    try:
        target.write_text(instrument(original))
        image_text = image_original.decode()
        assert image_text.count("var stdout bytes.Buffer") == 1
        image_target.write_text(image_text.replace("var stdout bytes.Buffer", "var stdout d2rsPolicyBuffer"))
        helper.write_bytes(hook.read_bytes())
        pdf.write_bytes(instrumented_pdf)
        clock.write_bytes(clock_source.read_bytes())
        env = {k: v for k, v in os.environ.items() if not k.startswith(("D2_", "D2RS_")) and k not in ["TESTDATA_ACCEPT", "TA"]}
        env.update(GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"), GOCACHE=str(ROOT / ".reference/go-build"),
                   GOFLAGS="-mod=mod", GOPROXY="off", TMPDIR=str(tmp), GOTMPDIR=str(tmp),
                   D2RS_POLICY_CAPTURE=str(capture), D2RS_POLICY_BINARY=str(runner), SOURCE_DATE_EPOCH=str(args.pdf_epoch))
        execution = CLI.bounded_run(["go", "test", "./d2cli", "-run", "^" + TEST + "$", "-json", "-count=1", "-parallel=1",
                                     "-ldflags=-X github.com/d2lang/d2/lib/version.Version=v0.9.0"], env, capture, capture / "go.jsonl", 600)
    finally:
        target.write_bytes(original)
        image_target.write_bytes(image_original)
        pdf.write_bytes(pdf_before)
        helper.unlink(missing_ok=True)
        if clock_before is None:
            clock.unlink(missing_ok=True)
        else:
            clock.write_bytes(clock_before)
    assert sha(runner) == digest
    calls = json.loads((capture / "calls.json").read_text()) if (capture / "calls.json").is_file() else []
    events = CLI.events(capture / "go.jsonl")
    artifacts = capture / "artifacts"
    artifacts.mkdir()
    rows = compare(calls, execution, events, artifacts) if len(calls) == 6 else [dict(name=TEST, outcome="execution-failed", calls=len(calls))]
    report = dict(source_commit=references.SOURCE_COMMIT, package="d2cli", rust=str(runner), rust_sha256=digest,
                  capture=str(capture), hook_sha256=sha(hook), clock_hook_sha256=sha(clock_source),
                  original_sha256=hashlib.sha256(original).hexdigest(), pdf_source_sha256=hashlib.sha256(pdf_original).hexdigest(),
                  image_helper_sha256=hashlib.sha256(image_original).hexdigest(),
                  pdf_epoch=args.pdf_epoch, version="v0.9.0", execution=execution, events=events, calls=calls,
                  budget=dict(total_bytes=1024**3, per_file_bytes=128*1024**2, min_free_bytes=20*1024**3, timeout_seconds=600),
                  rows=[dict(test=TEST, calls=6, matched_calls=sum(r["outcome"] == "match" for r in rows),
                             outcome="match" if len(rows) == 6 and all(r["outcome"] == "match" for r in rows) else "mismatch")], call_rows=rows,
                  captured_sha256={str(p.relative_to(capture)): sha(p) for p in capture.rglob("*") if p.is_file()})
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(dict(rows=report["rows"], call_rows=rows, execution=execution), indent=2))


if __name__ == "__main__":
    main()
