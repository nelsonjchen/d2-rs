#!/usr/bin/env python3
"""Generators for the d2-rs-isometric oracle fixtures (tools/fixtures/renderexport-isometric.toml).

    api --script scripts/<scoreboard>.py --output <logical> --format compact|default|indent2
        Run the scoreboard's original-test capture (upstream test bodies with
        call replacements plus its hook) against $D2RS_UPSTREAM through a go
        overlay (scripts/upstream_overlay.py), verify it with the scoreboard's
        own verify(), and write {"cases": cases} (or the bare case list for
        indent2, the svg-isometric layout). --assets inlines captured asset
        files as {"assets": {sha256: base64}} (font-api). --select keeps only
        the listed tests/operations (bounded witness sets for shape and solid).
    whole-svg --output <logical> BOARD...
        Pinned upstream goldens: e2etests/testdata/<case>/<engine>/board.exp.json
        with its isometric.exp.svg (from $D2RS_UPSTREAM), for each BOARD = <case>/<engine>/isometric.
    whole-assets --output <logical> --seed <json>
        Render the seed's boards with tools/isometric-probe/main.go (built in
        $D2RS_UPSTREAM through -overlay) without and with the seed's explicit
        HTTP resources: default errors plus complete Go SVGs.

Run through scripts/regen-fixtures.py (docs/FIXTURES.md).
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import fixture_recipe  # noqa: E402
import upstream_overlay  # noqa: E402


def load(script: Path):
    spec = importlib.util.spec_from_file_location(script.stem.replace("-", "_"), script)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    if hasattr(module, "scoreboard"):
        return module.scoreboard()
    return module.base  # svg-admission family: module-level capture/verify


def api(args):
    work = fixture_recipe.workdir()
    upstream_overlay.redirect(work / "overlay")
    score = load(ROOT / args.script)
    capture = work / "capture"
    score.capture_original(capture)
    cases = score.verify(capture)
    canonical_map_order(cases)
    if args.tests:
        wanted = args.tests.split(",")
        cases = [c for c in cases if c["name"] in wanted]
        assert sorted(c["name"] for c in cases) == sorted(wanted), "--tests names a test the capture lacks"
    if args.select:
        selection = json.loads((ROOT / args.select).read_text())
        cases = [dict(c, operations=[c["operations"][i] for i in selection[c["name"]]])
                 for c in cases if c["name"] in selection]
        assert [c["name"] for c in cases] == list(selection), "selection names a test the capture lacks"
    if args.digest_keys:
        digest_keys(cases, set(args.digest_keys.split(",")))
    layout = args.layout or ("bare" if args.format == "indent2" else "cases")
    value = {"bare": cases, "cases": {"cases": cases},
             "source-cases": {"source_commit": pinned_commit(), "cases": cases}}[layout]
    if args.assets:
        value = {"assets": inline_assets(cases), "cases": cases}
    kwargs = {"compact": dict(separators=(",", ":")), "default": {}, "indent1": dict(indent=1),
              "indent2": dict(indent=2)}[args.format]
    text = canonical_temp_paths(json.dumps(value, **kwargs), capture / "tmp")
    fixture_recipe.output(args.output).write_text(text + "\n")


def probe_api(args):
    """tools/isometric-probe/api_scoreboard.py capture of one suite: the
    public Render/RenderPage calls (request, want) the original Go tests make."""
    work = fixture_recipe.workdir()
    upstream = upstream_overlay.redirect(work / "overlay")
    import references
    real_command = references.command

    def command(*argv, **kwargs):
        if argv[1:3] == ("rev-parse", "HEAD"):
            return references.SOURCE_COMMIT  # the runner's tree is the pin (no repository)
        return real_command(*argv, **kwargs)

    references.command = command
    script = ROOT / args.script
    spec = importlib.util.spec_from_file_location(script.stem.replace("-", "_"), script)
    probe = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(probe)
    tmp = work / "tmp"
    tmp.mkdir(parents=True)
    for helper in vars(probe).values():  # the loaded scripts/isometric-scoreboard.py module(s)
        if getattr(helper, "__name__", None) and hasattr(helper, "bounded_native") and hasattr(helper, "UPSTREAM"):
            helper.UPSTREAM = upstream
            helper.bounded_native = lambda command, log, guard, timeout=600: run_go(command, log, guard, timeout, tmp)
    capture = work / "capture"
    if args.suite:
        probe.capture(capture, args.suite)
    else:
        probe.capture(capture)
    calls = [json.loads(line) for line in (capture / "calls.jsonl").read_text().splitlines()]
    if args.select:
        keep = json.loads((ROOT / args.select).read_text())
        calls = [calls[i] for i in keep]
    if args.apis:
        calls = [c for c in calls if c["request"].get("api") in args.apis.split(",")]
    if args.digest:
        for call in calls:
            call["want"] = digest_want(call["want"])
    value = {"pin": pinned_commit(), **json.loads(args.meta or "{}"), "calls": calls}
    if args.no_pin:
        del value["pin"]
    kwargs = {"compact": dict(separators=(",", ":")), "indent1": dict(indent=1), "indent2": dict(indent=2)}[args.format]
    text = canonical_temp_paths(json.dumps(value, **kwargs), tmp)
    fixture_recipe.output(args.output).write_text(text + "\n")


def digest_want(want):
    """Successful encoded outputs become {sha256, bytes} of the decoded bytes
    (RenderPage keeps its Width/Height/Links as "page"; CaptureFrames PNGs each
    become {sha256, bytes}; GIFs become their decoded frame summary, see
    gif_summary); errors stay literal."""
    if want.get("error"):
        if "data" in want and not want["data"]:
            return {"error": want["error"]}  # failed GIF renders carry no data
        if "frames" in want and not isinstance(want["frames"], list):
            return {"error": want["error"], "frames": []}
        if isinstance(want.get("frames"), list):  # frames captured before the error
            return {"error": want["error"], "frames": [png_digest(f) for f in want["frames"]]}
        return want
    if "data" in want:
        return {"error": want["error"], **gif_summary(base64.b64decode(want["data"]))}
    if isinstance(want.get("frames"), list):
        return {"error": want["error"], "frames": [png_digest(f) for f in want["frames"]]}
    result = want.get("result")
    if result is None:
        return want
    if isinstance(result, str):
        data = base64.b64decode(result)
        return {"error": want["error"], "sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}
    data = base64.b64decode(result["PNG"])
    return {"error": want["error"], "sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data),
            "page": {k: v for k, v in result.items() if k != "PNG"}}


def png_digest(encoded: str) -> dict:
    data = base64.b64decode(encoded)
    return {"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}


def gif_summary(data: bytes) -> dict:
    """Decode a GIF as the Rust readers do (tests/gif_api.rs, timeline_api.rs):
    logical canvas, loop count, sha256 of the global palette, and per frame
    its rectangle, delay, disposal and the sha256 of the composited opaque
    RGBA canvas after drawing it."""
    assert data[:6] in (b"GIF87a", b"GIF89a"), "not a GIF"
    width, height, flags = int.from_bytes(data[6:8], "little"), int.from_bytes(data[8:10], "little"), data[10]
    pos = 13
    palette = b""
    if flags & 0x80:
        size = 3 << ((flags & 7) + 1)
        palette = data[pos:pos + size]
        pos += size
    canvas = bytearray(width * height * 4)
    frames, loop, delay, disposal = [], None, 0, 0
    while True:
        block = data[pos]
        pos += 1
        if block == 0x3B:
            break
        if block == 0x21:
            label = data[pos]
            pos += 1
            chunks = []
            while data[pos]:
                chunks.append(data[pos + 1:pos + 1 + data[pos]])
                pos += 1 + data[pos]
            pos += 1
            if label == 0xF9:
                body = chunks[0]
                disposal, delay = (body[0] >> 2) & 7, int.from_bytes(body[1:3], "little")
                assert not body[0] & 1, "transparent GIF frame"
            elif label == 0xFF and chunks and chunks[0] in (b"NETSCAPE2.0", b"ANIMEXTS1.0"):
                loop = int.from_bytes(chunks[1][1:3], "little")
            continue
        assert block == 0x2C, "unexpected GIF block"
        left, top, w, h = (int.from_bytes(data[pos + i:pos + i + 2], "little") for i in (0, 2, 4, 6))
        local = data[pos + 8]
        pos += 9
        colors = palette
        if local & 0x80:
            size = 3 << ((local & 7) + 1)
            colors = data[pos:pos + size]
            pos += size
        assert not local & 0x40, "interlaced GIF frame"
        minimum = data[pos]
        pos += 1
        stream = bytearray()
        while data[pos]:
            stream += data[pos + 1:pos + 1 + data[pos]]
            pos += 1 + data[pos]
        pos += 1
        pixels = lzw_decode(bytes(stream), minimum, w * h)
        for y in range(h):
            for x in range(w):
                c = pixels[y * w + x] * 3
                at = ((top + y) * width + left + x) * 4
                canvas[at:at + 3] = colors[c:c + 3]
                canvas[at + 3] = 255
        frames.append({"rect": [left, top, left + w, top + h], "delay": delay, "disposal": disposal,
                       "rgba_sha256": hashlib.sha256(canvas).hexdigest()})
    return {"canvas": [width, height], "loop": loop, "palette_sha256": hashlib.sha256(palette).hexdigest(),
            "frames": frames}


def lzw_decode(stream: bytes, minimum: int, count: int) -> bytes:
    clear, end = 1 << minimum, (1 << minimum) + 1
    table = [bytes([i]) for i in range(clear)] + [b"", b""]
    width, bits, nbits, out, prev = minimum + 1, 0, 0, bytearray(), None
    for byte in stream:
        bits |= byte << nbits
        nbits += 8
        while nbits >= width:
            code = bits & ((1 << width) - 1)
            bits >>= width
            nbits -= width
            if code == clear:
                table = table[:clear + 2]
                width, prev = minimum + 1, None
                continue
            if code == end:
                return bytes(out[:count])
            if prev is None:
                entry = table[code]
            elif code < len(table):
                entry = table[code]
                table.append(prev + entry[:1])
            else:
                entry = prev + prev[:1]
                table.append(entry)
            out += entry
            prev = entry
            if len(table) == 1 << width and width < 12:
                width += 1
    return bytes(out[:count])


def digest_keys(value, keys):
    """Replace base64 strings stored under `keys` with {length, sha256}."""
    if isinstance(value, dict):
        for key, item in value.items():
            if key in keys and isinstance(item, str):
                raw = base64.b64decode(item)
                value[key] = {"length": len(raw), "sha256": hashlib.sha256(raw).hexdigest()}
            else:
                digest_keys(item, keys)
    elif isinstance(value, list):
        for item in value:
            digest_keys(item, keys)


def run_go(command, log, guard, timeout, tmp):
    """iso.bounded_native for `env K=V... go -C <dir> test ...`: the same
    budget guard, but as a plain `go test` subprocess so the overlay applies."""
    import signal
    import time
    argv = list(command)
    env = dict(os.environ, TMPDIR=str(tmp))
    if argv[0] == "env":
        argv.pop(0)
        while "=" in argv[0]:
            key, _, value = argv.pop(0).partition("=")
            env[key] = value
    cwd = None
    if argv[1] == "-C":
        cwd = argv[2]
        del argv[1:3]
    process = subprocess.Popen(argv, env=env, cwd=cwd, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    started = time.monotonic()
    try:
        while process.poll() is None:
            failure = guard()
            if failure:
                raise RuntimeError(failure)
            if time.monotonic() - started > timeout:
                raise subprocess.TimeoutExpired(argv, timeout)
            time.sleep(0.05)
        return process.returncode
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()


ISO_PACKAGE = "d2renderers/d2isometric/d2isometricimg"


def probe_session():
    """Overlay redirection plus scripts/isometric-scoreboard.py bound to the
    runner's tree; returns (workdir, upstream, iso module)."""
    work = fixture_recipe.workdir()
    upstream = upstream_overlay.redirect(work / "overlay")
    spec = importlib.util.spec_from_file_location("iso_fixture", ROOT / "scripts/isometric-scoreboard.py")
    iso = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(iso)
    iso.UPSTREAM = upstream
    iso.TRACE = upstream
    return work, upstream, iso


def go_hook_test(upstream, hooks, test, env, log):
    """Install tools/isometric-probe hooks (as d2rs_<name>) into the overlay and
    run one test of the isometric image package; hooks are removed afterwards."""
    package = upstream / ISO_PACKAGE
    installed = []
    try:
        for name in hooks:
            target = package / ("d2rs_" + name)
            target.write_bytes((ROOT / "tools/isometric-probe" / name).read_bytes())
            installed.append(target)
        with open(log, "w") as out:
            subprocess.run(["go", "test", "./" + ISO_PACKAGE, "-run", f"^{test}$", "-count=1", "-timeout=20m"],
                           cwd=upstream, env=dict(os.environ, **env), stdout=out, stderr=subprocess.STDOUT,
                           check=True, timeout=1500)
    finally:
        for target in installed:
            target.unlink()


def board_rows(upstream, boards, out, mode, assets):
    rows = []
    for board in boards:
        case = board.rsplit("/", 1)[0]
        rows.append({"board": str(upstream / "e2etests/testdata" / case / "board.exp.json"),
                     "out": str(out / case), "mode": mode, "assets": str(assets), "stream": False})
    return rows


def mesh_calls(args):
    """TestD2rsMeshDump in text/rich/primitives mode over pinned golden boards:
    the observed TextPainter/richLabelPainter/meshBuilder calls per board."""
    import contextlib
    work, upstream, iso = probe_session()
    out, assets = work / "mesh", work / "assets"
    assets.mkdir(parents=True)
    boards = args.boards if args.boards else sorted({w["board"] for w in json.loads((ROOT / args.witnesses).read_text())})
    requests = work / "mesh-requests.jsonl"
    requests.write_text("".join(json.dumps(r) + "\n" for r in board_rows(upstream, boards, out, args.mode, assets)))
    instrument = {"text": lambda: iso.text_instrumentation(False), "rich": lambda: iso.text_instrumentation(True),
                  "primitives": iso.primitive_instrumentation}.get(args.mode, contextlib.nullcontext)
    with instrument():
        go_hook_test(upstream, ["asset_options_test.go", "mesh_hook_test.go", "surface_json_test.go"],
                     "TestD2rsMeshDump", {"D2RS_ISOMETRIC_MESH": str(requests), "TMPDIR": str(work)},
                     work / "go.log")

    def calls(board):
        case, name = board.rsplit("/", 1)
        path = out / case / (name + ".mesh.json")
        assert path.is_file(), f"{board}: no mesh dump ({(out / case / (name + '.err'))})"
        return json.loads(path.read_text())

    if args.witnesses:
        # One retained call per witness: the first observed call with that op on its board.
        value = []
        for witness in json.loads((ROOT / args.witnesses).read_text()):
            matches = [c for c in calls(witness["board"]) if c["op"] == witness["op"]]
            assert len(matches) > witness["index"], witness
            value.append(matches[witness["index"]])
    else:
        value = [{"board": board, "calls": calls(board)} for board in boards]
    kwargs = {"compact": dict(separators=(",", ":")), "indent1": dict(indent=1)}[args.format]
    fixture_recipe.output(args.output).write_text(json.dumps(value, **kwargs) + "\n")


def png_boards(args):
    """TestD2rsPNGCapture over every e2etests/png isometric fixture: the pinned
    Go native PNG of each board, as sha256 and dimensions."""
    work, upstream, iso = probe_session()
    png = upstream / "e2etests/png/testdata"
    fixtures = sorted({g.parent.relative_to(png).as_posix() for g in png.rglob("isometric*.exp.png")})
    assert fixtures, "empty PNG golden inventory"
    requests = [{"Board": str(upstream / "e2etests/testdata" / f / "board.exp.json"),
                 "Out": str(work / "go" / f), "Assets": str(work / "textures")} for f in fixtures]
    path = work / "requests.jsonl"
    path.write_text("".join(json.dumps(r) + "\n" for r in requests))
    go_hook_test(upstream, ["png_hook_test.go"], "TestD2rsPNGCapture",
                 {"D2RS_ISOMETRIC_PNG": str(path), "TMPDIR": str(work)}, work / "go.log")
    cases = []
    for fixture in fixtures:
        folder = work / "go" / fixture
        expected = {}
        for image in sorted(folder.glob("*.png")):
            data = image.read_bytes()
            assert data[12:16] == b"IHDR", image
            expected[image.name.removesuffix(".png")] = {
                "png_sha256": hashlib.sha256(data).hexdigest(),
                "width": int.from_bytes(data[16:20], "big"), "height": int.from_bytes(data[20:24], "big")}
        assert expected, f"{fixture}: no PNG output"
        cases.append({"fixture": fixture,
                      "diagram": json.loads((upstream / "e2etests/testdata" / fixture / "board.exp.json").read_text()),
                      "expected": expected})
    value = {"source_commit": pinned_commit(), "source_test": "e2etests/png/render_test.go:TestPNG", "cases": cases}
    fixture_recipe.output(args.output).write_text(json.dumps(value, separators=(",", ":")) + "\n")


def png_contracts(args):
    """TestD2rsSurfacePNGContracts: static-PNG adapter witnesses (input, Go re-encoding)."""
    work, upstream, iso = probe_session()
    out = work / "contracts.json"
    go_hook_test(upstream, ["png_contract_test.go"], "TestD2rsSurfacePNGContracts",
                 {"D2RS_SURFACE_PNG_CONTRACTS": str(out), "TMPDIR": str(work)}, work / "go.log")
    fixture_recipe.output(args.output).write_text(json.dumps(json.loads(out.read_text()), indent=1) + "\n")


def pinned_commit() -> str:
    return json.loads((ROOT / "reference-manifest.json").read_text())["d2"]["commit"]


TEMP_PLACEHOLDER = "@D2RS_TMP@"


def canonical_temp_paths(text: str, tmp: Path) -> str:
    """Go t.TempDir() roots live under the capture's TMPDIR with a random
    suffix (<TMPDIR>/<TestName><digits>/<NNN>). Replace the TMPDIR prefix with
    TEMP_PLACEHOLDER and drop the random digits, so two runs give the same
    bytes; replays substitute a fresh directory for the placeholder."""
    for prefix in {str(tmp), str(tmp.resolve())}:
        text = text.replace(prefix, TEMP_PLACEHOLDER)
    return re.sub(re.escape(TEMP_PLACEHOLDER) + r"/([^/\"\\]*?)\d+/(\d{3})", TEMP_PLACEHOLDER + r"/\1/\2", text)


def map_ordered_tests() -> set:
    """Original test functions that iterate a Go map literal (`range map[`):
    their operations come in random order, so they are sorted canonically."""
    names = set()
    upstream = pinned_upstream()
    for folder in ("d2renderers/d2isometric", "d2renderers/d2svg"):
        for path in sorted((upstream / folder).rglob("*_test.go")):
            text = path.read_text()
            for match in re.finditer(r"(?ms)^func (Test\w+)\(.*?(?=^func |\Z)", text):
                if "range map[" in match[0]:
                    names.add(match[1])
    return names


def canonical_map_order(cases) -> None:
    ordered = map_ordered_tests()
    for case in cases:
        if isinstance(case, dict) and case.get("name") in ordered and isinstance(case.get("operations"), list):
            case["operations"].sort(key=lambda op: json.dumps(op.get("request"), sort_keys=True))


def inline_assets(cases):
    """font-api layout: request font bytes (DataFile paths or inline Data) become
    sha256 DataRefs into "assets" (base64), and Go-side "pixels"/"Data" in results are reduced to
    {length, sha256}, as the Rust replay compacts its own results."""
    assets = {}

    def refs(value, inline):
        if isinstance(value, dict):
            data = None
            if isinstance(value.get("DataFile"), str):
                data = Path(value.pop("DataFile")).read_bytes()
            elif inline and isinstance(value.get("Data"), str):
                data = base64.b64decode(value.pop("Data"))
            if data is not None:
                digest = hashlib.sha256(data).hexdigest()
                assets.setdefault(digest, base64.b64encode(data).decode())
                value["DataRef"] = digest
            for item in value.values():
                refs(item, inline)
        elif isinstance(value, list):
            for item in value:
                refs(item, inline)

    def compact(value):
        if isinstance(value, dict):
            for key, item in value.items():
                if key in ("pixels", "Data") and isinstance(item, str):
                    raw = base64.b64decode(item)
                    value[key] = {"length": len(raw), "sha256": hashlib.sha256(raw).hexdigest()}
                else:
                    compact(item)
        elif isinstance(value, list):
            for item in value:
                compact(item)

    for case in cases:
        for operation in case["operations"]:
            refs(operation["request"], True)  # inline request font bytes too
            refs(operation["got"], False)
            compact(operation["got"])
    return assets


def pinned_upstream() -> Path:
    """The runner's tree of the pin (recipes declare upstream = true)."""
    return Path(os.environ["D2RS_UPSTREAM"]).resolve()


def whole_svg(args):
    import references
    testdata = pinned_upstream() / "e2etests/testdata"
    cases = []
    for board in args.boards:
        folder = testdata / board.rsplit("/", 1)[0]
        cases.append({
            "source_commit": references.SOURCE_COMMIT,
            "board": board,
            "diagram": json.loads((folder / "board.exp.json").read_text()),
            "themeID": None,
            "svg": (folder / "isometric.exp.svg").read_text(),
        })
    fixture_recipe.output(args.output).write_text(json.dumps(cases, indent=1) + "\n")


def whole_assets(args):
    import references
    work = fixture_recipe.workdir()
    upstream = Path(os.environ["D2RS_UPSTREAM"]).resolve()
    seed = json.loads((ROOT / args.seed).read_text())
    resources, sources = {}, {}
    (work / "assets").mkdir(parents=True)
    for url, resource in seed["resources"].items():
        data = base64.b64decode(resource["data"])
        digest = hashlib.sha256(data).hexdigest()
        assert digest == resource["sha256"], url
        path = work / "assets" / digest
        path.write_bytes(data)
        sources[url] = {"path": str(path), "mime": resource["mime"]}
        resources[url] = {"mime": resource["mime"], "sha256": digest, "data": resource["data"]}
    overlay = work / "overlay.json"
    overlay.write_text(json.dumps({"Replace": {
        str(upstream / "cmd/d2rs-isometric-probe/main.go"): str(ROOT / "tools/isometric-probe/main.go")}}))
    probe = work / "isometric-probe"
    env = fixture_recipe.go_env()
    subprocess.run(["go", "build", "-mod=readonly", f"-overlay={overlay}", "-o", str(probe),
                    "./cmd/d2rs-isometric-probe"], cwd=upstream, env=env, check=True)
    testdata = upstream / "e2etests/testdata"
    outputs = {}
    for mode, assets in [("default", None), ("assets", sources)]:
        rows = []
        for case in seed["boards"]:
            folder = case["board"].rsplit("/", 1)[0]
            row = {"board": str(testdata / folder / "board.exp.json"), "themeID": case["themeID"],
                   "out": str(work / mode / folder), "mode": "svg"}
            if assets is not None:
                row["assetSources"] = assets
            rows.append(row)
        subprocess.run([str(probe)], input="".join(json.dumps(r) + "\n" for r in rows), text=True,
                       check=True, timeout=1200)
        outputs[mode] = work / mode
    cases = []
    for case in seed["boards"]:
        folder, name = case["board"].rsplit("/", 1)
        board = testdata / folder / "board.exp.json"
        error = outputs["default"] / folder / (name + ".err")
        svg = outputs["assets"] / folder / (name + ".svg")
        assert error.is_file() and not (outputs["default"] / folder / (name + ".svg")).exists(), case
        assert svg.is_file() and svg.stat().st_size, case
        text = svg.read_bytes().decode()
        cases.append({
            "board": case["board"],
            "diagram": json.loads(board.read_text()),
            "diagram_sha256": hashlib.sha256(board.read_bytes()).hexdigest(),
            "themeID": case["themeID"],
            "default_error": error.read_text(),
            "svg": text,
            "svg_sha256": hashlib.sha256(svg.read_bytes()).hexdigest(),
        })
    value = {"source_commit": references.SOURCE_COMMIT, "profile": seed["profile"],
             "resources": resources, "cases": cases}
    fixture_recipe.output(args.output).write_text(json.dumps(value, indent=1) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("api")
    p.add_argument("--script", required=True)
    p.add_argument("--output", required=True)
    p.add_argument("--format", choices=["compact", "default", "indent1", "indent2"], default="compact")
    p.add_argument("--layout", choices=["bare", "cases", "source-cases"],
                   help="default: bare for --format indent2 (svg-isometric), cases otherwise")
    p.add_argument("--tests", help="comma-separated original tests to keep whole")
    p.add_argument("--digest-keys", help="comma-separated keys whose base64 values become {length, sha256}")
    p.add_argument("--assets", action="store_true")
    p.add_argument("--select", help="seed JSON {test: [operation indices]}: the retained witnesses")
    p = sub.add_parser("probe-api")
    p.add_argument("--script", default="tools/isometric-probe/api_scoreboard.py",
                   help="capture script with capture(path[, suite]) writing calls.jsonl")
    p.add_argument("--suite")
    p.add_argument("--output", required=True)
    p.add_argument("--format", choices=["compact", "indent1", "indent2"], default="compact")
    p.add_argument("--meta", help="JSON object of fixed descriptive fields written before calls")
    p.add_argument("--select", help="seed JSON [call indices]: the retained witnesses")
    p.add_argument("--apis", help="comma-separated request apis to keep (render, page)")
    p.add_argument("--digest", action="store_true", help="store successful outputs as sha256 and size")
    p.add_argument("--no-pin", action="store_true", help="omit the pin field")
    p = sub.add_parser("mesh-calls")
    p.add_argument("--mode", choices=["text", "rich", "primitives"], required=True)
    p.add_argument("--output", required=True)
    p.add_argument("--format", choices=["compact", "indent1"], default="compact")
    p.add_argument("--witnesses", help="seed JSON [{board, op, index}]: one retained call each")
    p.add_argument("boards", nargs="*", help="<case>/<engine>/isometric")
    p = sub.add_parser("png-boards")
    p.add_argument("--output", required=True)
    p = sub.add_parser("png-contracts")
    p.add_argument("--output", required=True)
    p = sub.add_parser("whole-svg")
    p.add_argument("--output", required=True)
    p.add_argument("boards", nargs="+")
    p = sub.add_parser("whole-assets")
    p.add_argument("--output", required=True)
    p.add_argument("--seed", required=True)
    args = parser.parse_args()
    {"api": api, "probe-api": probe_api, "mesh-calls": mesh_calls, "png-boards": png_boards,
     "png-contracts": png_contracts, "whole-svg": whole_svg, "whole-assets": whole_assets}[args.command](args)


if __name__ == "__main__":
    main()
