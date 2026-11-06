#!/usr/bin/env python3
"""Score crates/d2-rs-fontface against pinned Go fontface (D2 master 6a7b48378).

Every distinct label text in the upstream e2e goldens
(e2etests/testdata/**/board.exp.json: shape labels, connection labels and
src/dst arrowhead labels, class fields and methods, sql table columns, in
nested layers/scenarios/steps too; each line of a multi-line label is its own
text) is shaped with the bundled D2 face matching its font family and
bold/italic flags (bold wins over italic, as in d2scenebuild), at 64 ppem and
at the label's fontSize. Texts with a non-ASCII rune also get the bundled Noto
Color Emoji fallback face, as d2scenebuild appends it for missing runes.

tools/fontface-probe (Go, built in the trace checkout as
d2renderers/cmd/d2rs-fontface-probe) and the fontface-probe example of
crates/d2-rs-fontface read the same requests. Each emits a ShapedText record
(glyph ID, face, position, advance, 26.6 ink, flags, source rune and index,
advance, rune/coverage/run counts) or the error text, plus one LoadGlyph
segment record at 64 ppem for every inked (font, glyph). Records are compared
exactly: numbers by IEEE-754 bits (so -0 differs from 0), errors as text.

With --extended it also shapes a fixed list of stress texts (marks, bidi,
complex scripts, emoji sequences, ignorables, controls, missing glyphs) with
every bundled D2 face, four sizes and with/without the emoji fallback, and
compares, for every bundled face (the ten D2 fonts
and Noto Color Emoji), every glyph's LoadGlyph segments, GlyphAdvance,
GlyphRenderBounds (64 and 15 ppem), COLR0GlyphLayers and COLRv1 plan, plus
Metrics/Bounds (two hintings, three sizes), the cmap below U+3000,
SupportsRenderableRune below U+2800 and Kern for every printable ASCII pair.

usage: fontface-scoreboard.py [--capture <fresh dir>] [--output report.json]
       [--only <substring>] [--extended] [--budget-mib 1024]
"""
from __future__ import annotations

import argparse
import json
import shutil
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
D2 = ROOT.parent / "d2"
TESTDATA = D2 / "e2etests/testdata"
GO_PROBE = ROOT / ".reference/bin/d2-fontface-probe-6a7b4837"
RUST_PROBE = ROOT / "target/diagnostic/examples/fontface-probe"
PPEM_64 = 64 * 64


def font_name(family: str, bold: bool, italic: bool) -> str:
    base = "SourceCodePro" if (family or "").lower() == "mono" else "SourceSansPro"
    style = "Bold" if bold else "Italic" if italic else "Regular"
    return f"{base}-{style}"


def labels(diagram: dict):
    """(text, font, fontSize) for every plain-text label of one diagram."""
    for shape in diagram.get("shapes") or []:
        size = shape.get("fontSize") or 0
        if not shape.get("language"):
            yield shape.get("label", ""), font_name(shape.get("fontFamily"), shape.get("bold"), shape.get("italic")), size
        for row in (shape.get("fields") or []) + (shape.get("methods") or []):
            for part in ("name", "type", "return"):
                if row.get(part):
                    yield row[part], font_name("mono", False, False), size
        for column in shape.get("columns") or []:
            for part in ("name", "type"):
                label = column.get(part) or {}
                if label.get("label"):
                    yield (label["label"], font_name(label.get("fontFamily"), label.get("bold"), label.get("italic")),
                           label.get("fontSize") or size)
            if column.get("constraint"):
                for constraint in column["constraint"]:
                    yield constraint, font_name("", False, False), size
    for connection in diagram.get("connections") or []:
        size = connection.get("fontSize") or 0
        font = font_name(connection.get("fontFamily"), connection.get("bold"), connection.get("italic"))
        yield connection.get("label", ""), font, size
        for end in ("srcLabel", "dstLabel"):
            label = connection.get(end) or {}
            if label.get("label"):
                yield label["label"], font_name(label.get("fontFamily"), label.get("bold"), label.get("italic")), size
    for nested in ("layers", "scenarios", "steps"):
        for child in diagram.get(nested) or []:
            yield from labels(child)


BUNDLED = ["SourceSansPro-Regular", "SourceSansPro-Bold", "SourceSansPro-Semibold", "SourceSansPro-Italic",
           "SourceCodePro-Regular", "SourceCodePro-Bold", "SourceCodePro-Semibold", "SourceCodePro-Italic",
           "FuzzyBubbles-Regular", "FuzzyBubbles-Bold", "NotoColorEmoji"]


# Hand-picked shaping stress texts (not fixture outputs): combining marks,
# bidi with digits and paired delimiters, complex scripts, emoji sequences,
# default-ignorables, controls and missing-glyph placeholders.
SYNTHETIC_TEXTS = [
    "e\u0301 a\u0308\u0301 n\u0303 \u1e17 o\u0328\u0304",
    "fi fl ffi ffl \ufb01",
    'AVAWAYTaToVaWaYa LT P. F, \'A\' "V"',
    "Hello \u05e9\u05dc\u05d5\u05dd (123) world",
    "\u0645\u0631\u062d\u0628\u0627 [\u0628\u0643] 42",
    "abc \u05d0\u05d1\u05d2 \u0661\u0662\u0663 def",
    "\u0928\u092e\u0938\u094d\u0924\u0947 \u0915\u094d\u0937",
    "\u0e2a\u0e27\u0e31\u0e2a\u0e14\u0e35",
    "\u4f60\u597d\u4e16\u754c \u3053\u3093\u306b\u3061\u306f \uc548\ub155",
    "\U0001f600\U0001f44d\U0001f3fd \U0001f468\u200d\U0001f469\u200d\U0001f467 \U0001f1fa\U0001f1f8 1\ufe0f\u20e3 \u2764\ufe0f",
    "\u2705 \u2601 \u2614\ufe0e \U0001f3f3\ufe0f\u200d\U0001f308 \U0001f9d1\U0001f3ff\u200d\U0001f4bb",
    "soft\xadhyphen zero\u200bwidth joiner\u200dx bom\ufeffy lrm\u200erlm\u200f",
    "tab\there \x07bell \x7fdel \u2028ls",
    "\u0378 \U000e0001 \U0010fffd \ue000 private",
    "\u0391\u03b8\u03ae\u03bd\u03b1 \u041c\u043e\u0441\u043a\u0432\u0430 \u0130stanbul",
    "x\u0336y\u0336 q\u0323\u0307 \u1ea0\u0300",
    "\xc5ngstr\xf6m \u2126 \u212b \u2153 \xbd",
    "\u2192 \u21d2 \u2200x\u2208\u211d \u221a\u03c0 \u2264\u2265 \u2610\u2611\u2612 \u25a1 \ufffd",
    "\u0627\u0644\u0639\u0631\u0628\u064a\u0629 (\u0645\u062b\u0627\u0644) \u05e2\u05d1\u05e8\u05d9\u05ea",
    "a\u0300\u0301\u0302\u0303\u0304\u0305\u0306\u0307\u0308\u0309\u030a",
    "   leading and trailing   ",
    "\u1019\u1004\u103a\u1039\u1002\u101c\u102c\u1015\u102b \u0915\u094d\u200d\u0937 \u0d15\u0d4d\u0d15",
    "\u202bembedded RTL\u202c \u202eoverride\u202c \u2067isolate \u05d0\u2069 \u2068first strong\u2069",
    "a (b [c] d) e \u05d0 (\u05d1) [x] \xab\u05d2\xbb \u201cq\u201d",
    "\u0628\u0650\u0633\u0652\u0645\u0650 \u0627\u0644\u0644\u0651\u064e\u0647\u0650",
    "\u0661\u0662\u0663 abc (\u0664\u0665) $12.50 +3% -4 \u05d0 1/2 \u05d1",
    "x\u0301\u05b0\u0651 \u05e9\u05c1\u05b8 \u200c\u200d\u034f",
]


def synthetic_requests() -> list[dict]:
    rows = []
    for text in SYNTHETIC_TEXTS:
        for font in BUNDLED[:-1]:
            for size in (640, 864, 1024, 4096):
                for fallback in (False, True):
                    rows.append({"key": f"synthetic:{len(rows)}", "text": text, "font": font,
                                 "fallback": fallback, "size": size})
    return rows


def extended_requests() -> list[dict]:
    """Every glyph and font-level sfnt/fontface query of every bundled face."""
    rows = synthetic_requests()
    for font in BUNDLED:
        rows.append({"key": f"font:{font}", "kind": "font", "font": font})
        rows.append({"key": f"glyphs:{font}", "kind": "glyphs", "font": font})
    return rows


def build_requests(only: str | None) -> list[dict]:
    seen = set()
    rows = []
    for board in sorted(TESTDATA.rglob("board.exp.json")):
        if only and only not in str(board):
            continue
        for text, font, size in labels(json.loads(board.read_text())):
            for line in text.split("\n"):
                if not line:
                    continue
                fallback = any(ord(c) > 0x7F for c in line)
                for ppem in sorted({PPEM_64, int(size) * 64} - {0}):
                    identity = (line, font, fallback, ppem)
                    if identity in seen:
                        continue
                    seen.add(identity)
                    rows.append({"key": f"{len(rows)}", "text": line, "font": font, "fallback": fallback, "size": ppem})
    return rows


def run(command: list[str], requests: Path, output: Path, budget: int) -> str | None:
    with requests.open("rb") as stdin, output.open("wb") as stdout:
        process = subprocess.Popen(command, stdin=stdin, stdout=stdout, stderr=subprocess.PIPE)
        try:
            _, stderr = process.communicate(timeout=3600)
        except subprocess.TimeoutExpired:
            process.kill()
            return f"{command[0]}: timeout"
    if output.stat().st_size > budget:
        return f"{command[0]}: output exceeds budget {budget} bytes"
    if process.returncode != 0:
        return f"{command[0]}: exit {process.returncode}: {stderr.decode(errors='replace')[-2000:]}"
    return None


def canonical(value):
    """Exact comparison form: floats by bit pattern, ints by value."""
    if isinstance(value, bool) or value is None or isinstance(value, str):
        return value
    if isinstance(value, float):
        if value.is_integer() and not (value == 0 and str(value).startswith("-")):
            return ("num", int(value))
        return ("bits", struct.pack(">d", value).hex())
    if isinstance(value, int):
        return ("num", value)
    if isinstance(value, list):
        return [canonical(v) for v in value]
    if isinstance(value, dict):
        return {k: canonical(v) for k, v in value.items()}
    raise TypeError(type(value))


def first_difference(go, rust, path="") -> str:
    if type(go) is not type(rust):
        return f"{path}: go={go!r} rust={rust!r}"
    if isinstance(go, dict):
        for key in sorted(set(go) | set(rust)):
            if key not in go or key not in rust:
                return f"{path}/{key}: go={go.get(key)!r} rust={rust.get(key)!r}"
            if go[key] != rust[key]:
                return first_difference(go[key], rust[key], f"{path}/{key}")
    if isinstance(go, list):
        if len(go) != len(rust):
            return f"{path}: len go={len(go)} rust={len(rust)}"
        for index, (a, b) in enumerate(zip(go, rust)):
            if a != b:
                return first_difference(a, b, f"{path}[{index}]")
    return f"{path}: go={go!r} rust={rust!r}"


def load_records(path: Path) -> dict:
    records = {}
    for line in path.read_text().splitlines():
        # Go prints a float64 negative zero as "-0", which json would read
        # as the integer 0; keep its sign.
        record = json.loads(line, parse_int=lambda text: -0.0 if text == "-0" else int(text))
        records[record.pop("key")] = canonical(record)
    return records


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--capture", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--only")
    parser.add_argument("--budget-mib", type=int, default=1024)
    parser.add_argument("--show", type=int, default=20)
    parser.add_argument("--extended", action="store_true",
                        help="also compare every glyph and font-level query of each bundled face")
    args = parser.parse_args()
    capture = args.capture or Path(tempfile.mkdtemp(prefix="fontface-scoreboard-"))
    capture.mkdir(parents=True, exist_ok=True)
    if any(capture.iterdir()):
        sys.exit(f"capture directory {capture} is not empty")
    for binary in (GO_PROBE, RUST_PROBE):
        if not binary.exists():
            sys.exit(f"missing {binary}")
    requests = build_requests(args.only) + (extended_requests() if args.extended else [])
    request_path = capture / "requests.jsonl"
    request_path.write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in requests))
    budget = args.budget_mib * 1024 * 1024 // 2
    failures = []
    for name, command in (("go", [str(GO_PROBE), str(D2)]), ("rust", [str(RUST_PROBE)])):
        failure = run(command, request_path, capture / f"{name}.jsonl", budget)
        if failure:
            failures.append(failure)
    if failures:
        print("\n".join(failures))
        return 1
    go = load_records(capture / "go.jsonl")
    rust = load_records(capture / "rust.jsonl")
    if len(go) < len(requests) - (len(BUNDLED) if args.extended else 0):
        print(f"Go probe emitted {len(go)} records for {len(requests)} requests")
        return 1
    by_text = {r["key"]: r for r in requests}
    mismatches = []
    for key, expected in go.items():
        actual = rust.get(key)
        if actual is None:
            mismatches.append((key, "missing Rust record"))
        elif actual != expected:
            mismatches.append((key, first_difference(expected, actual)))
    extra = sorted(set(rust) - set(go))
    groups = {"shaping": [], "outlines": [], "synthetic": [], "glyphs": [], "fonts": []}
    for k in go:
        prefix = k.split(":", 1)[0]
        groups[{"outline": "outlines", "glyphs": "glyphs", "font": "fonts", "synthetic": "synthetic"}.get(prefix, "shaping")].append(k)
    bad = {k for k, _ in mismatches}
    report = {
        "requests": len(requests),
        "records": len(go),
        "matched": len(go) - len(mismatches),
        **{name: {"total": len(keys), "matched": sum(k not in bad for k in keys)} for name, keys in groups.items()},
        "go_errors": sum(1 for v in go.values() if isinstance(v, dict) and "error" in v),
        "extra_rust_records": len(extra),
        "mismatches": [
            {"key": k, "request": by_text.get(k), "difference": d} for k, d in mismatches
        ],
    }
    parts = ", ".join(f"{name} {report[name]['matched']}/{report[name]['total']}" for name in groups if report[name]["total"])
    print(f"fontface scoreboard: {report['matched']}/{report['records']} records match "
          f"({parts}, Go errors {report['go_errors']}, extra Rust records {len(extra)})")
    for row in report["mismatches"][: args.show]:
        print(f"  {row['key']} {json.dumps(row['request'], ensure_ascii=False) if row['request'] else ''}\n    {row['difference']}")
    if args.output:
        args.output.write_text(json.dumps(report, indent=1, ensure_ascii=False))
    if not args.capture:
        shutil.rmtree(capture)
    return 0 if not mismatches and not extra else 1


if __name__ == "__main__":
    sys.exit(main())
