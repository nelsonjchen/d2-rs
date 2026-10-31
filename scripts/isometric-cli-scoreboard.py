#!/usr/bin/env python3
"""Pinned-master CLI isometric source contracts; complete SVG bytes and PNG inventories.

Inputs follow d2cli/isometric{,_image,_svg}_test.go. This is a finite subprocess
source comparison, not a claim to execute all upstream in-process assertions.
No artifact is accepted from stdout/stderr success alone. Budgets are 1 GiB
for the fresh capture, 128 MiB/file and 20 GiB free, checked while writers run.
"""

from __future__ import annotations

import argparse
import collections
import hashlib
import importlib.util
import json
import os
import re
import subprocess
import time
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path

import references

ROOT = Path(__file__).resolve().parents[1]


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    obj = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(obj)
    return obj


RASTER = module("raster_gate", "check-render-parity.py")
CLI = module("cli_gif_gate", "cli-scoreboard.py")
ISO = module("isometric_score", "isometric-scoreboard.py")
DURATION = re.compile(
    r" in [0-9.]+(?:ns|µs|us|ms|s|m[0-9.]+s|h[0-9hms.]+)$", re.MULTILINE
)
SIMPLE = "a -> b\n"
ON = "vars: {d2-config: {isometric: true}}\n"
OFF = "vars: {d2-config: {isometric: false}}\n"
BOARDS = "a: {link: layers.detail}\nlayers: {detail: {b: {link: root}}}\nscenarios: {alternate: {c}}\nsteps: {next: {d}}\n"


def cases():
    rows = []

    def add(name, source=SIMPLE, args=(), env=None, suffix="svg", fail=False):
        rows.append(
            {
                "name": name,
                "source": source,
                "args": list(args),
                "env": env or {},
                "suffix": suffix,
                "fail": fail,
            }
        )

    precedence = [
        ("source", ON, [], {}),
        ("source-disabled", OFF, [], {}),
        ("flag", "", ["--isometric"], {}),
        ("flag-disables-source", ON, ["--isometric=false"], {}),
        ("flag-enables-source", OFF, ["--isometric"], {}),
        ("environment", "", [], {"D2_ISOMETRIC": "true"}),
        ("environment-disables-source", ON, [], {"D2_ISOMETRIC": "false"}),
        (
            "flag-overrides-environment",
            ON,
            ["--isometric=false"],
            {"D2_ISOMETRIC": "true"},
        ),
        (
            "flag-enables-over-environment",
            OFF,
            ["--isometric"],
            {"D2_ISOMETRIC": "false"},
        ),
    ]
    for name, config, args, env in precedence:
        add("precedence-" + name, config + SIMPLE, [*args, "--scale=.2"], env)
    for suffix in ("svg", "png"):
        for name, source, args in [
            ("file", SIMPLE, []),
            ("stdout", SIMPLE, ["--stdout-format=" + suffix, "-"]),
            ("boards", BOARDS, []),
            ("folder", "layers: {detail: {b}}", []),
            ("target", BOARDS, ["--target=layers.detail"]),
            ("root-target", BOARDS, ["--target="]),
            (
                "subtree",
                "a\nlayers: {parent: {b; layers: {child: {c}}}}",
                ["--target=parent.*"],
            ),
        ]:
            add(
                suffix + "-" + name,
                source,
                ["--isometric", "--scale=.2", *args],
                suffix=suffix,
            )
    for suffix in ("svg", "png"):
        add(suffix + "-source", ON + SIMPLE, ["--scale=.2"], suffix=suffix)
    for scale in (".001", "10"):
        add("svg-scale-" + scale, args=["--isometric", "--scale=" + scale])
        add(
            "png-scale-" + scale,
            args=["--isometric", "--scale=" + scale],
            suffix="png",
            fail=True,
        )
    for name, source, args, suffix in [
        ("flag-sketch", SIMPLE, ["--sketch", "--isometric"], "svg"),
        (
            "source-sketch",
            ON + "vars.d2-config.sketch: true\na",
            ["--isometric"],
            "svg",
        ),
        ("source-format", ON + SIMPLE, [], "txt"),
        ("source-html", ON + SIMPLE, [], "html"),
        ("flag-format", SIMPLE, ["--isometric"], "txt"),
        ("flag-html", SIMPLE, ["--isometric"], "html"),
        ("flag-zero", SIMPLE, ["--isometric", "--scale=0"], "svg"),
        ("source-zero", ON + SIMPLE, ["--scale=0"], "svg"),
        ("source-nan", ON + SIMPLE, ["--scale=NaN"], "svg"),
        ("flag-appendix", SIMPLE, ["--isometric", "--force-appendix"], "svg"),
        ("source-appendix", ON + SIMPLE, ["--force-appendix"], "svg"),
        ("flag-dark", SIMPLE, ["--isometric", "--dark-theme=200"], "svg"),
        ("source-dark", ON + "vars.d2-config.dark-theme-id: 200\na", [], "svg"),
        ("svg-slideshow", SIMPLE, ["--isometric", "--animate-interval=100"], "svg"),
        ("png-slideshow", SIMPLE, ["--isometric", "--animate-interval=100"], "png"),
        ("negative-slideshow", SIMPLE, ["--isometric", "--animate-interval=-1"], "gif"),
        (
            "svg-multiple-stdout",
            BOARDS,
            ["--isometric", "--stdout-format=svg", "-"],
            "svg",
        ),
        (
            "png-multiple-stdout",
            BOARDS,
            ["--isometric", "--stdout-format=png", "-"],
            "png",
        ),
    ]:
        add(name, source, args, suffix=suffix, fail=True)
    for name, args in [
        ("document-options", ["--pad=7", "--center", "--salt=site"]),
        ("no-xml", ["--no-xml-tag"]),
        ("omit-version", ["--omit-version"]),
    ]:
        add(name, args=["--isometric", *args])
    return rows


def paged_cases():
    """d2cli/isometric_paged_test.go: finite public command witnesses."""
    architecture = """label: Architecture
website: Website {link: https://example.com; tooltip: Open website}
next: Detail {link: layers.detail; tooltip: Open detail}
docs: |md
  [Documentation](https://example.com/docs "Read documentation")
|
website -> next: Request {link: https://example.com/request}
layers: {detail: {label: Detail; x -> y}}
"""
    subtree = """layers: {parent: {
label: Parent
a.link: layers.child
docs: |md
  [Child](root.layers.parent.layers.child)
|
layers: {child: {label: Child; b}}
}}
"""
    rows = []
    for suffix in ("pdf", "pptx"):
        for name, source, extra in [
            ("file", architecture, []),
            ("stdout", architecture, ["-"]),
            (
                "all-kinds",
                "a\nlayers: {detail: {b}}\nscenarios: {alternate: {c}}\nsteps: {next: {d}}",
                ["--target=*", "-"],
            ),
            (
                "selected",
                "a\nlayers: {detail: {b}}\nscenarios: {alternate: {c}}",
                ["--target=layers.detail", "-"],
            ),
            (
                "folders",
                "layers: {folder: {layers: {child: {x}}}}",
                ["--target=*", "-"],
            ),
            ("subtree", subtree, ["--target=layers.parent.*", "-"]),
            ("simple", SIMPLE, []),
            ("dark-fill", "style.fill: '#123456'\n" + SIMPLE, []),
        ]:
            args = ["--isometric", "--scale=.2", *extra]
            if "-" in args:
                args.append("--stdout-format=" + suffix)
            rows.append(
                {
                    "name": suffix + "-paged-" + name,
                    "source": source,
                    "args": args,
                    "env": {},
                    "suffix": suffix,
                    "fail": False,
                }
            )
        for name, source, extra in [
            (
                "bad-asset",
                'a.link: https://example.com\nlayers: {detail: {b.icon: "data:image/png;base64,YmFk"}}',
                [],
            ),
            (
                "missing-target",
                "a.link: layers.detail\nlayers: {detail: {b}}",
                ["--target="],
            ),
        ]:
            for stdout in (False, True):
                args = ["--isometric", "--scale=.2", *extra]
                if stdout:
                    args.extend(["--stdout-format=" + suffix, "-"])
                rows.append(
                    {
                        "name": f"{suffix}-paged-{name}-{'stdout' if stdout else 'file'}",
                        "source": source,
                        "args": args,
                        "env": {},
                        "suffix": suffix,
                        "fail": True,
                        "preserve": not stdout,
                    }
                )
    return rows


def gif_cases():
    """isometric_test.go:126 and isometric_image_test.go:89–165, 216–244."""
    rows = []
    boards = "a\nlayers: {detail: {b}}\nscenarios: {alternate: {c}}\nsteps: {next: {d}}"

    def add(name, source, extra=(), *, stdout=False, source_mode=False, fail=False):
        rows.append(
            {
                "name": "gif-" + name,
                "source": source,
                "args": [
                    *([] if source_mode else ["--isometric"]),
                    "--scale=.2",
                    *extra,
                    *(["--stdout-format=gif", "-"] if stdout else []),
                ],
                "env": {},
                "suffix": "gif",
                "fail": fail,
            }
        )

    traffic = "a -> b: {style.animated: true}"
    add("flag-traffic", traffic, stdout=True)
    add("source-traffic", ON + traffic, stdout=True, source_mode=True)
    add("file-traffic", traffic)
    for name, extra in [("interval", ["--animate-interval=370"]), ("default", [])]:
        add("boards-" + name, boards, extra, stdout=True)
    add("boards-file", boards, ["--animate-interval=370"])
    add("folder", "layers: {detail: {b}}")
    add("nested-folder", "layers: {folder: {layers: {child: {x}}}}", stdout=True)
    add(
        "selected-leaf",
        "a\nlayers: {detail: {x -> y}}",
        ["--target=layers.detail"],
        stdout=True,
    )
    add("leaf-interval", "a", ["--animate-interval=370"], stdout=True)
    add("selected-root", boards, ["--target=", "--animate-interval=370"])
    add(
        "subtree",
        "a\nlayers: {parent: {b; layers: {child: {c}}}}",
        ["--target=parent.*", "--animate-interval=370"],
        stdout=True,
    )
    add(
        "animated-timeline",
        traffic + "\nsteps: {next: {c}}",
        ["--animate-interval=250"],
        stdout=True,
    )
    for name, extra in [
        ("negative", ["--animate-interval=-1"]),
        ("too-long", ["--animate-interval=600001"]),
        ("scale", ["--scale=2"]),
    ]:
        add("reject-" + name, "a", extra, fail=True)
    return rows


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(binary, case, directory, source, capture, pdf_epoch=None):
    directory.mkdir()
    # The same input path is used by both source and native invocations.
    args = list(case["args"])
    stdout = "-" in args
    dest = "-" if stdout else "out." + case["suffix"]
    args = [a for a in args if a != "-"]
    env = {
        k: v
        for k, v in os.environ.items()
        if not k.startswith("D2_") and k not in ("SCALE", "OMIT_VERSION")
    }
    env.update(case["env"])
    env.pop("SOURCE_DATE_EPOCH", None)
    if pdf_epoch is not None:
        env["SOURCE_DATE_EPOCH"] = str(pdf_epoch)
    if case.get("preserve"):
        (directory / dest).write_bytes(b"previous reviewed document")
        (directory / dest).chmod(0o600)
    out = directory / ("stdout." + case["suffix"] if stdout else "stdout.log")
    err = directory / "stderr.log"
    command = [str(binary), str(source), *args, dest]
    with out.open("wb") as sink, err.open("wb") as errors:
        process = subprocess.Popen(
            command,
            cwd=directory,
            stdout=sink,
            stderr=errors,
            env=env,
            start_new_session=True,
        )
        started = time.monotonic()
        failure = None
        while process.poll() is None:
            failure = ISO.direct_guard(capture, 1 << 30)
            if failure or time.monotonic() - started > 120:
                failure = failure or "timeout"
                process.kill()
                break
            time.sleep(0.1)
        code = process.wait()
    failure = failure or ISO.direct_guard(capture, 1 << 30)
    logs = {
        key: DURATION.sub(
            " in T",
            path.read_text(errors="backslashreplace").replace(str(directory), "DIR"),
        )
        for key, path in [("stderr", err)]
    }
    if not stdout:
        logs["stdout"] = out.read_text(errors="backslashreplace")
    outputs = {
        str(p.relative_to(directory)): digest(p)
        for p in directory.rglob("*")
        if p.is_file()
        and p not in (err, directory / "stdout.log")
        and (p != out or stdout)
    }
    result = {"exit": code, "failure": failure, "logs": logs, "outputs": outputs}
    if case.get("preserve"):
        path = directory / dest
        result["preserved"] = (
            path.read_bytes() == b"previous reviewed document"
            and path.stat().st_mode & 0o777 == 0o600
        )
    return result


def compare(case, go, rust, go_dir, rust_dir):
    if go["failure"] or rust["failure"]:
        return "capture-failed"
    if case["fail"]:
        if case.get("preserve") and not (go.get("preserved") and rust.get("preserved")):
            return "publication-mismatch"
        return "error-match" if go["exit"] != 0 and go == rust else "error-mismatch"
    if go["exit"] or rust["exit"] or not go["outputs"] or not rust["outputs"]:
        return "render-failed"
    if go["logs"] != rust["logs"] or set(go["outputs"]) != set(rust["outputs"]):
        return "source-mismatch"
    if case["suffix"] == "gif":
        # Encoded bytes are evidence; the contract covers every composited
        # frame, delay, canvas and loop, using the established CLI GIF gate.
        for name in go["outputs"]:
            if not name.endswith(".gif"):
                raise ValueError("unexpected GIF output")
            if CLI.gif_frames(go_dir / name) != CLI.gif_frames(rust_dir / name):
                return "frame-mismatch"
        return "source-match"
    if go["outputs"] != rust["outputs"]:
        return "source-mismatch"
    if case["suffix"] == "png":
        # Both the complete inventory and every decoded board must pass.
        RASTER.collect_png_outputs(
            go_dir / ("stdout.png" if "-" in case["args"] else "out.png")
        )
        RASTER.collect_png_outputs(
            rust_dir / ("stdout.png" if "-" in case["args"] else "out.png")
        )
        result = RASTER.compare_png_outputs(
            go_dir / ("stdout.png" if "-" in case["args"] else "out.png"),
            rust_dir / ("stdout.png" if "-" in case["args"] else "out.png"),
        )
        if not result["matched"]:
            return "pixel-mismatch"
    if case["suffix"] in ("pdf", "pptx"):
        for directory in (go_dir, rust_dir):
            for name in go["outputs"]:
                path = directory / name
                if case["suffix"] == "pdf":
                    data = path.read_bytes()
                    if not data.startswith(b"%PDF-") or not data.rstrip().endswith(
                        b"%%EOF"
                    ):
                        raise ValueError("invalid PDF envelope")
                    if b"/Type /Page\n" not in data or b"/Subtype /Image" not in data:
                        raise ValueError("missing PDF page/image")
                else:
                    with zipfile.ZipFile(path) as archive:
                        if archive.testzip() is not None:
                            raise ValueError("corrupt PPTX member")
                        names = archive.namelist()
                        if (
                            "ppt/presentation.xml" not in names
                            or "ppt/slides/slide1.xml" not in names
                            or "ppt/media/slide1Image.png" not in names
                        ):
                            raise ValueError("missing PPTX page/image")
                        for member in names:
                            if member.endswith((".xml", ".rels")):
                                ET.fromstring(archive.read(member))
    return "source-match"


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("--rust", type=Path, default=ROOT / "target/diagnostic/d2-rs")
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--go", type=Path, default=references.GO_ORACLE)
    parser.add_argument(
        "--suite", choices=("static", "paged", "gif", "all"), default="static"
    )
    parser.add_argument("--pdf-epoch", type=int)
    args = parser.parse_args()
    capture = args.capture.resolve()
    capture.mkdir(parents=True, exist_ok=False)
    rows = []
    selected_cases = (
        (cases() if args.suite in ("static", "all") else [])
        + (paged_cases() if args.suite in ("paged", "all") else [])
        + (gif_cases() if args.suite in ("gif", "all") else [])
    )
    for case in selected_cases:
        root = capture / case["name"]
        root.mkdir()
        source = root / "source.d2"
        source.write_text(case["source"])
        go_dir, rust_dir = root / "go", root / "rust"
        go = run(args.go.resolve(), case, go_dir, source, capture, args.pdf_epoch)
        rust = run(args.rust.resolve(), case, rust_dir, source, capture, args.pdf_epoch)
        try:
            outcome = compare(case, go, rust, go_dir, rust_dir)
        except (
            ValueError,
            KeyError,
            OSError,
            zipfile.BadZipFile,
            ET.ParseError,
        ) as error:
            outcome = "invalid-artifact"
            rust["gate_error"] = str(error)
        rows.append(dict(**case, outcome=outcome, go=go, rust=rust))
    report = {
        "pin": references.SOURCE_COMMIT,
        "go": str(args.go.resolve()),
        "go_sha256": digest(args.go),
        "suite": args.suite,
        "pdf_epoch": args.pdf_epoch,
        "rust": str(args.rust.resolve()),
        "rust_sha256": digest(args.rust),
        "capture": str(capture),
        "budgets": {"total": 1 << 30, "file": 128 << 20, "free": 20 << 30},
        "summary": dict(collections.Counter(r["outcome"] for r in rows)),
        "rows": rows,
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(report["summary"])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
