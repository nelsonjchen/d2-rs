#!/usr/bin/env python3
"""Pinned-master watch source contracts: real file reloads and preview navigation.

The finite subprocess corpus follows d2cli/isometric_watch_test.go. Full SVG
previews and complete file inventories compare; PNGs use the shared raster gate.
This does not claim to execute all original in-process watch assertions.
"""

from __future__ import annotations

import argparse
import collections
import hashlib
import http.client
import importlib.util
import json
import os
import re
import shutil
import signal
import subprocess
import tempfile
import time
import xml.etree.ElementTree as ET
from pathlib import Path
from urllib.parse import urlsplit

import references

ROOT = Path(__file__).resolve().parents[1]


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    obj = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(obj)
    return obj


ISO = module("isometric_score", "isometric-scoreboard.py")
RASTER = module("raster_gate", "check-render-parity.py")
ON = "vars: {d2-config: {isometric: true}}\n"
OFF = "vars: {d2-config: {isometric: false}}\n"
BOARDS = (
    "a: {link: layers.first}\nlayers: {first: {b: {link: root}; "
    "sibling: {link: root.layers.second}; external: {link: https://example.test/docs?q=a&b=c}}; "
    "second: {c: {link: root.layers.first}}}\n"
)


def cases():
    rows = [
        {
            "name": "source-reload",
            "suffix": "svg",
            "source": ON + "a -> b\n",
            "args": [],
            "steps": [
                {"name": "initial", "isometric": True},
                {"name": "changed", "source": ON + "a -> b -> c\n", "isometric": True},
                {
                    "name": "disabled",
                    "source": OFF + "a -> b -> c\n",
                    "isometric": False,
                },
                {"name": "restored", "source": ON + "a -> b -> c\n", "isometric": True},
            ],
        }
    ]
    for suffix in ("svg", "png"):
        for folder in (False, True):
            rows.append(
                {
                    "name": f"{suffix}-{'folder' if folder else 'root'}-navigation",
                    "suffix": suffix,
                    "source": BOARDS.split("\n", 1)[1] if folder else BOARDS,
                    "args": ["--isometric"],
                    "steps": [
                        {"name": "initial", "isometric": True},
                        {
                            "name": "first",
                            "path": "/layers/first.svg",
                            "isometric": True,
                        },
                        {
                            "name": "second",
                            "path": "/layers/second.svg",
                            "isometric": True,
                        },
                        {"name": "root", "path": "/index.svg", "isometric": True},
                    ],
                }
            )
    return rows


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def check_guard(capture):
    failure = ISO.direct_guard(capture, 1 << 30)
    if failure:
        raise RuntimeError(failure)


def wait_for(test, process, capture, timeout=30):
    end = time.monotonic() + timeout
    while True:
        check_guard(capture)
        value = test()
        if value is not None:
            return value
        if process.poll() is not None:
            raise RuntimeError(f"watch exited early: {process.returncode}")
        if time.monotonic() >= end:
            raise TimeoutError("watch update timeout")
        time.sleep(0.05)


def snapshot(directory, target):
    target.mkdir()
    outputs = {}
    for path in sorted(directory.rglob("*")):
        if path.is_file():
            rel = path.relative_to(directory)
            dest = target / rel
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, dest)
            outputs[str(rel)] = digest(dest)
    return outputs


def run(binary, case, source, directory, capture):
    import websocket

    directory.mkdir()
    work = directory / "work"
    work.mkdir()
    source.write_text(case["source"])
    env = {k: v for k, v in os.environ.items() if not k.startswith("D2_")}
    env.update(BROWSER="0", D2_BROWSER="0")
    command = [
        str(binary),
        "--watch",
        "--host=127.0.0.1",
        "--port=0",
        "--scale=.2",
        *case["args"],
        str(source),
        "out." + case["suffix"],
    ]
    rows, failure, ws = [], None, None
    with (
        (directory / "stdout.log").open("wb") as out,
        (directory / "stderr.log").open("wb") as err,
    ):
        process = subprocess.Popen(
            command, cwd=work, env=env, stdout=out, stderr=err, start_new_session=True
        )
        try:

            def address():
                log = (directory / "stderr.log").read_text(errors="replace")
                match = re.search(
                    r"http://127\.0\.0\.1:\d+/\?token=[A-Za-z0-9_-]+", log
                )
                return match.group(0) if match else None

            url = urlsplit(wait_for(address, process, capture))
            token = url.query.removeprefix("token=")
            cookie = f"d2-watch-{url.port}={token}"
            ws = websocket.create_connection(
                f"ws://{url.netloc}/watch",
                cookie=cookie,
                timeout=0.25,
                http_no_proxy=["127.0.0.1"],
            )
            previous = None
            for step in case["steps"]:
                if "source" in step:
                    source.write_text(step["source"])
                if "path" in step:
                    conn = http.client.HTTPConnection(url.netloc, timeout=5)
                    try:
                        conn.request("GET", step["path"], headers={"Cookie": cookie})
                        response = conn.getresponse()
                        response.read()
                        if response.status != 200:
                            raise RuntimeError(f"navigation HTTP {response.status}")
                    finally:
                        conn.close()

                def receive(previous=previous, navigating="path" in step):
                    try:
                        message = ws.recv()
                    except websocket.WebSocketTimeoutException:
                        return None
                    if not message:
                        raise RuntimeError("empty/closed watch update")
                    if len(message) > 128 << 20:
                        raise RuntimeError("watch update exceeds file budget")
                    value = json.loads(message)
                    return value if value != previous or navigating else None

                value = wait_for(receive, process, capture)
                previous = value
                stage = directory / step["name"]
                stage.mkdir()
                (stage / "update.json").write_text(json.dumps(value, indent=2) + "\n")
                svg = value.get("svg", "")
                (stage / "preview.svg").write_text(svg)
                inventory = snapshot(work, stage / "files")
                rows.append(
                    {"name": step["name"], "message": value, "outputs": inventory}
                )
                # Keep a negative result concrete without waiting for all later stages.
                if value.get("err"):
                    break
        except (
            OSError,
            ValueError,
            RuntimeError,
            TimeoutError,
            websocket.WebSocketException,
        ) as error:
            failure = str(error)
        finally:
            if ws is not None:
                ws.close(timeout=1)
            if process.poll() is None:
                process.send_signal(signal.SIGINT)
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
                failure = failure or "watch shutdown timeout"
    check_guard(capture)
    return {
        "command": command,
        "failure": failure,
        "exit": process.returncode,
        "rows": rows,
    }


def compare_step(step, go, rust, go_dir, rust_dir, suffix):
    for row in (go, rust):
        message = row["message"]
        if message.get("err") or not message.get("svg") or not row["outputs"]:
            return "render-failed"
        svg = ET.fromstring(message["svg"])
        if svg.tag != "{http://www.w3.org/2000/svg}svg":
            raise ValueError("invalid preview SVG")
        if ("D2 isometric diagram" in message["svg"]) != step["isometric"]:
            return "mode-mismatch"
    if go["message"] != rust["message"]:
        return "preview-mismatch"
    if go["outputs"] != rust["outputs"]:
        return "artifact-mismatch"
    for directory in (go_dir, rust_dir):
        for name in go["outputs"]:
            path = directory / "files" / name
            if path.suffix == ".svg":
                ET.fromstring(path.read_bytes())
    if suffix == "png":
        # After leaf navigation the single file coexists with the earlier
        # directory export. Validate both inventories and every retained board.
        for directory in (go_dir, rust_dir):
            if (directory / "files/out").is_dir():
                RASTER.collect_png_outputs(directory / "files/out")
        # Watch intentionally retains an earlier board directory beside the
        # selected leaf's single file. Give each file an isolated gate path
        # rather than relaxing collect_png_outputs' ambiguity rejection.
        with tempfile.TemporaryDirectory(prefix="png-gate-", dir=rust_dir) as temp:
            temp = Path(temp)
            for name in go["outputs"]:
                if name.endswith(".png"):
                    shutil.copyfile(go_dir / "files" / name, temp / "go.png")
                    shutil.copyfile(rust_dir / "files" / name, temp / "rust.png")
                    RASTER.collect_png_outputs(temp / "go.png")
                    RASTER.collect_png_outputs(temp / "rust.png")
                    result = RASTER.compare_png_outputs(
                        temp / "go.png", temp / "rust.png"
                    )
                    if not result["matched"]:
                        return "pixel-mismatch"
    return "source-match"


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("--rust", type=Path, required=True)
    parser.add_argument("--go", type=Path, default=references.GO_ORACLE)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    capture = args.capture.resolve()
    capture.mkdir(parents=True, exist_ok=False)
    rows, runs = [], []
    for case in cases():
        root = capture / case["name"]
        root.mkdir()
        source = root / "source.d2"
        go = run(args.go.resolve(), case, source, root / "go", capture)
        rust = run(args.rust.resolve(), case, source, root / "rust", capture)
        runs.append({"case": case, "go": go, "rust": rust})
        for index, step in enumerate(case["steps"]):
            outcome, error = "capture-failed", None
            if not go["failure"] and not rust["failure"]:
                outcome = "missing-update"
                if index < len(go["rows"]) and index < len(rust["rows"]):
                    try:
                        outcome = compare_step(
                            step,
                            go["rows"][index],
                            rust["rows"][index],
                            root / "go" / step["name"],
                            root / "rust" / step["name"],
                            case["suffix"],
                        )
                    except (ValueError, KeyError, OSError, ET.ParseError) as exc:
                        outcome, error = "invalid-artifact", str(exc)
            rows.append(
                {
                    "name": case["name"] + "/" + step["name"],
                    "outcome": outcome,
                    "error": error,
                }
            )
    report = {
        "pin": references.SOURCE_COMMIT,
        "go": str(args.go.resolve()),
        "go_sha256": digest(args.go),
        "rust": str(args.rust.resolve()),
        "rust_sha256": digest(args.rust),
        "capture": str(capture),
        "budgets": {"total": 1 << 30, "file": 128 << 20, "free": 20 << 30},
        "summary": dict(collections.Counter(r["outcome"] for r in rows)),
        "rows": rows,
        "runs": runs,
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(report["summary"])


if __name__ == "__main__":
    main()
