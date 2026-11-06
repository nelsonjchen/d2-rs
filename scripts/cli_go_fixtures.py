#!/usr/bin/env python3
"""Go CLI oracle fixtures for crates/d2-rs-cli (tools/fixtures/langlibelk-cli.toml).

usage: cli_go_fixtures.py <family>

Builds the pinned Go `d2` command from the runner's patched upstream tree
($D2RS_UPSTREAM), stamped `v0.9.0` like every CLI scoreboard oracle
(d2-rs reports that version), runs the family's requests and writes the
family's outputs (their historical logical paths; the content is captured at
the manifest pin). Requests are the hand-composed inputs of the historical
v0.9.0-era witness scripts (.reference/codex-runs/witness-inputs/run3*-*.py,
.reference/badge-paint-order-20261006/capture-source.py,
.reference/paged-links-20261018/{capture,compare,freeze-fixtures}.py,
.reference/engine-options-20261019/, .reference/cli-version-20261019/capture.py,
.reference/raster-unavailable-20261016/, .reference/synthetic-root-raster-20261018/),
kept verbatim here. Wall-clock durations and scratch paths in captured
stderr are replaced by fixed placeholders where no test compares them.

PNG/PDF/PPTX inspection needs Pillow and pypdf: the recipes run this script
through `uv run --with pillow==... --with pypdf==...`.

Run through `uv run --no-project python scripts/regen-fixtures.py --only 'cli-*'`
(docs/FIXTURES.md); `cargo test -p d2-rs --features oracle-fixtures`
performs the native comparison.
"""
from __future__ import annotations

import hashlib
import io
import json
import os
from pathlib import Path
import posixpath
import re
import shutil
import subprocess
import sys
import xml.etree.ElementTree as ET
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
from fixture_recipe import go_env, output, workdir  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = json.loads((ROOT / "reference-manifest.json").read_text())
PIN = MANIFEST["d2"]["commit"]
FIXTURES = "crates/d2-rs-cli/tests/fixtures/"
# argv[0] of every oracle run: help text prints it, and the Rust tests compare
# it as "{program}".
PROGRAM = "D2RSPROGRAM"


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def dump(value, **kwargs) -> str:
    return json.dumps(value, indent=2, **kwargs) + "\n"


_BINARY: Path | None = None


def cli() -> Path:
    """Build the pinned Go CLI once per recipe run."""
    global _BINARY
    if _BINARY is None:
        upstream = os.environ.get("D2RS_UPSTREAM")
        if not upstream:
            sys.exit("D2RS_UPSTREAM is unset: the recipe must set upstream = true")
        binary = workdir() / "bin" / "d2"
        subprocess.run(["go", "build", "-mod=readonly",
                        "-ldflags=-X github.com/d2lang/d2/lib/version.Version=v0.9.0",
                        "-o", str(binary), "."], cwd=upstream, env=go_env(), check=True)
        _BINARY = binary
    return _BINARY


def run(args, *, cwd: Path, stdin: bytes | None = None, env=None, timeout=120):
    environment = {k: v for k, v in os.environ.items() if not k.startswith(("D2_", "D2RS_TRACE"))}
    environment.update(env or {})
    return subprocess.run([PROGRAM, *args], executable=str(cli()), cwd=cwd, input=stdin if stdin is not None else b"",
                          capture_output=True, env=environment, timeout=timeout)


def fresh(name: str) -> Path:
    path = workdir() / "cases" / name
    if path.exists():
        shutil.rmtree(path)
    path.mkdir(parents=True)
    return path


def tree(base: Path) -> dict[str, Path]:
    return {p.relative_to(base).as_posix(): p for p in sorted(base.rglob("*")) if p.is_file()}


# ---------------------------------------------------------------------------
# Subtree and stdio witnesses (run33, run38, run39, run40)

SUBTREE = 'a: excluded root\nlayers: {\n  one: { b: selected; layers: {two: {c: descendant}} }\n  sibling: {d: excluded sibling}\n}\n'
SUBTREE_QUOTED = 'a\nlayers: {\n  "x . y": { b; layers: {two: {c}}; scenarios: {next: {d}} }\n}\n'
SUBTREE_FOLDER = 'layers: {\n  one: { layers: {two: {c}} }\n}\n'
STDIO_CASES = [
    ('nested', SUBTREE, 'layers.one.*', False, False),
    ('shorthand', SUBTREE, 'one.*', False, False),
    ('quoted-mixed', SUBTREE_QUOTED, 'layers."x . y".*', False, False),
    ('folder', SUBTREE_FOLDER, 'layers.one.*', False, False),
    ('leaf', SUBTREE, 'layers.one.layers.two.*', False, False),
    ('named-stdout', SUBTREE, 'layers.one.*', False, True),
    ('root-stdout', SUBTREE, '*', False, True),
    ('stdin-file', SUBTREE, 'layers.one.*', True, False),
    ('stdin-stdout', SUBTREE, 'layers.one.*', True, True),
]


def stdio_run(directory: Path, flags, source: str, use_stdin: bool, use_stdout: bool, name: str):
    """Run with file/stdin input and file/stdout output; returns (process, output path)."""
    input_path = directory / "input.d2"
    input_path.write_text(source)
    dest = directory / "go"
    dest.mkdir()
    out = dest / name
    proc = run([*flags, "-" if use_stdin else str(input_path), "-" if use_stdout else str(out)],
               cwd=dest, stdin=source.encode() if use_stdin else None)
    if use_stdout:
        out.write_bytes(proc.stdout)
    elif proc.stdout:
        sys.exit(f"{directory.name}: unexpected stdout")
    return proc, out


def gif_subtree():
    fixtures = []
    for name, source, target, use_stdin, use_stdout in STDIO_CASES:
        proc, out = stdio_run(fresh(name), ['--pad=0', '--bundle=false', '--animate-interval=10',
                                            '--stdout-format=gif', '--target=' + target],
                              source, use_stdin, use_stdout, "out.gif")
        if proc.returncode != 0 or not out.stat().st_size:
            sys.exit(f"{name}: {proc.stderr.decode(errors='replace')}")
        fixtures.append(dict(name=name, source=source, target=target, stdin=use_stdin, stdout=use_stdout,
                             gif_hex=out.read_bytes().hex()))
    output(FIXTURES + "gif-subtree-source-v090.json").write_text(dump(fixtures))


PAGED_SOURCE = 'label: excluded root\na\nlayers: {\n one: {label: Selected; b; layers: {two: {label: Child; c}}}\n sibling: {d}\n}\n'
PAGED_QUOTED = 'a\nlayers: {\n "x . y": {label: Selected; b; layers: {two: {c}}; scenarios: {next: {d}}}\n}\n'
PAGED_FOLDER = 'layers: {one: {layers: {two: {c}}}}\n'
PAGED_NO_NAV = 'label: Excluded\na\nlayers: {one: {b; layers: {two: {c}}}}\n'
PAGED_CASES = [
    ('nested', PAGED_SOURCE, 'layers.one.*', False, False), ('shorthand', PAGED_SOURCE, 'one.*', False, False),
    ('quoted-mixed', PAGED_QUOTED, 'layers."x . y".*', False, False), ('folder', PAGED_FOLDER, 'layers.one.*', False, False),
    ('leaf', PAGED_SOURCE, 'layers.one.layers.two.*', False, False), ('named-stdout', PAGED_SOURCE, 'layers.one.*', False, True),
    ('root-stdout', PAGED_SOURCE, '*', False, True), ('stdin-file', PAGED_SOURCE, 'layers.one.*', True, False),
    ('stdin-stdout', PAGED_SOURCE, 'layers.one.*', True, True), ('no-nav', PAGED_NO_NAV, 'one.*', False, False),
    ('single', PAGED_SOURCE, 'one', False, False), ('root-only', PAGED_SOURCE, '', False, False),
    ('folder-single', PAGED_FOLDER, 'one', False, False),
]


def inspect_paged(path: Path, fmt: str):
    """run39 contract: every PPTX member, or every PDF page, drawing stream, image and annotation."""
    if fmt == 'pptx':
        with zipfile.ZipFile(path) as z:
            names = z.namelist()
            assert names and len(names) == len(set(names)) and z.testzip() is None
            return {n: sha(z.read(n)) for n in names}
    from pypdf import PdfReader
    from pypdf.generic import IndirectObject
    reader = PdfReader(path, strict=True)
    assert reader.pages

    def value(v):
        if isinstance(v, IndirectObject):
            pages = {p.indirect_reference.idnum: i for i, p in enumerate(reader.pages)}
            if v.idnum in pages:
                return {'page': pages[v.idnum]}
            return value(v.get_object())
        if isinstance(v, dict):
            return {str(k): value(v.raw_get(k)) for k in v}
        if isinstance(v, list):
            return [value(x) for x in v]
        if isinstance(v, (str, int, float, bool)) or v is None:
            return v
        return str(v)
    pages = []
    for p in reader.pages:
        ims = []
        for args, op in p.get_contents().operations:
            if op != b'Do':
                continue
            im = p['/Resources']['/XObject'][args[0]].get_object()
            mask = im.get('/SMask')
            mask = mask.get_object() if mask else None
            ims.append(dict(name=str(args[0]), width=im['/Width'], height=im['/Height'], rgb=sha(im.get_data()),
                            compressed=sha(im._data), alpha=sha(mask.get_data()) if mask else None,
                            compressed_alpha=sha(mask._data) if mask else None))
        assert len(ims) == 1
        pages.append(dict(box=list(p.mediabox), content=sha(p.get_contents().get_data()), images=ims,
                          annotations=value(p.get('/Annots', []))))
    return pages


def paged_subtree():
    fixtures = []
    for name, source, target, use_stdin, use_stdout in PAGED_CASES:
        for fmt in ['pdf', 'pptx']:
            proc, out = stdio_run(fresh(f'{name}-{fmt}'), ['--pad=0', '--bundle=false', f'--stdout-format={fmt}',
                                                         f'--target={target}'],
                                  source, use_stdin, use_stdout, f"out.{fmt}")
            row = dict(name=name, source=source, target=target, stdin=use_stdin, stdout=use_stdout, format=fmt)
            if name == 'folder-single':
                error = 'paged export requires at least one renderable board'
                if proc.returncode == 0 or error not in proc.stderr.decode() or out.exists():
                    sys.exit(f"{name} {fmt}: expected {error!r}: {proc.stderr!r}")
                fixtures.append(dict(row, expected_error=error))
                continue
            if proc.returncode != 0 or not out.is_file() or not out.stat().st_size:
                sys.exit(f"{name} {fmt}: {proc.stderr.decode(errors='replace')}")
            row['expected'] = inspect_paged(out, fmt)
            if fmt == 'pptx':
                row['raw_sha256'] = sha(out.read_bytes())
            fixtures.append(row)
    output(FIXTURES + "paged-subtree-source-v090.json").write_text(dump(fixtures))


PNG_SOURCE = 'a\nlayers: {one: {b; layers: {two: {c}}}}\n'
PNG_CASES = [('file', False, False, ['--target=']), ('stdout', False, True, ['--target=']),
             ('stdin-file', True, False, ['--target=']), ('stdin-stdout', True, True, ['--target=']),
             ('root-tree', False, False, ['--target=*']), ('named-subtree', False, False, ['--target=one.*']),
             ('omit-version', False, False, ['--target=', '--omit-version'])]


def png_outputs(out: Path) -> dict[str, Path]:
    """The complete PNG inventory: one file, or every file of its board folder."""
    folder = out.with_suffix('')
    if out.is_file() and folder.exists():
        sys.exit(f"{out}: ambiguous file and board outputs")
    files = {'': out} if out.is_file() else tree(folder) if folder.is_dir() else {}
    if not files:
        sys.exit(f"{out}: missing PNG output")
    return files


def png_metadata():
    fixtures = []
    for name, use_stdin, use_stdout, flags in PNG_CASES:
        proc, out = stdio_run(fresh(name), ['--pad=0', '--stdout-format=png', *flags], PNG_SOURCE,
                              use_stdin, use_stdout, "out.png")
        if proc.returncode != 0:
            sys.exit(f"{name}: {proc.stderr.decode(errors='replace')}")
        fixtures.append(dict(name=name, source=PNG_SOURCE, stdin=use_stdin, stdout=use_stdout, flags=flags,
                             sha256={k: sha(p.read_bytes()) for k, p in png_outputs(out).items()}))
    output(FIXTURES + "png-metadata-source-v090.json").write_text(dump(fixtures))


SUBTREE_OUTPUT = [
    {'name': 'nested', 'source': 'a\nlayers: {\n  one: {\n    b\n    layers: {two: {c}}\n  }\n  sibling: {d}\n}\n', 'targets': ['layers.one.*', 'one.*']},
    {'name': 'quoted-mixed', 'source': 'a\nlayers: {\n  "x . y": {\n    b\n    layers: {two: {c}}\n    scenarios: {next: {d}}\n  }\n}\n', 'targets': ['layers."x . y".*']},
    {'name': 'folder', 'source': 'layers: {\n  one: {\n    layers: {two: {c}}\n  }\n}\n', 'targets': ['layers.one.*']},
    {'name': 'leaf', 'source': 'a\nlayers: {one: {b}}\n', 'targets': ['layers.one.*']},
]


def subtree_output():
    fixtures = []
    for case in SUBTREE_OUTPUT:
        for i, target in enumerate(case['targets']):
            name = case['name'] + str(i)
            directory = fresh(name)
            source = directory / 'input.d2'
            source.write_text(case['source'])
            base = directory / 'go-svg'
            base.mkdir()
            proc = run(['--pad=0', '--bundle=false', '--omit-version', '--no-xml-tag', '--target=' + target,
                        str(source), str(base / 'out.svg')], cwd=directory)
            if proc.returncode != 0 or proc.stdout:
                sys.exit(f"{name}: {proc.returncode} {proc.stderr!r} {proc.stdout[:200]!r}")
            files = tree(base)
            if not files:
                sys.exit(f"{name}: no outputs")
            fixtures.append(dict(name=name, source=case['source'], target=target,
                                 outputs={k: v.read_text() for k, v in files.items()}))
    output(FIXTURES + "subtree-output-source-v090.json").write_text(dump(fixtures))


# ---------------------------------------------------------------------------
# Animated SVG canvases and destination policy

EQUATION = ('formula: {\n  equation: |latex\n    \\begin{equation} \\label{eq1}\n    \\begin{split}\n'
            '    A & = \\frac{\\\\pi r^2}{2} \\\\\n     & = \\frac{1}{2} \\pi r^2\n    \\end{split}\n'
            '    \\end{equation}\n  |\n}\n\n')
# (name, input, pad, target or None, sketch)
ANIMATION_CANVAS = [
    ('ordinary', 'a -> b\nlayers: {one: {c -> d}}', 20, '*', False),
    ('folder-root', 'layers: {one: {a -> b}}', 20, '*', False),
    ('nested-folder', 'layers: {one: {layers: {two: {a -> b}}}}', 20, '*', False),
    ('empty', '', 20, '*', False),
    ('empty-folders', 'layers: {one: {layers: {two: {}}}}', 20, '*', False),
    ('wrapper-order', 'steps: {s: {step}}\nscenarios: {c: {scenario}}\nlayers: {l: {layer}}\nroot', 20, '*', False),
    ('folder-wrapper-order', 'steps: {s: {step}}\nscenarios: {c: {scenario}}\nlayers: {l: {layer}}', 20, '*', False),
    ('root-border', 'style: {stroke-width: 12; double-border: true}\na -> b\nlayers: {one: {c -> d}}', 20, '*', False),
    ('shadow', 'a: {style.shadow: true}\na -> b\nlayers: {one: {c: {width: 320; height: 180}}}', 20, '*', False),
    ('equation', EQUATION + '\nlayers: {one: {c: {width: 320; height: 180}}}', 20, '*', False),
    ('markdown', 'a: |md\n # Heading\n - first\n - second\n|\nlayers: {one: {a -> b}}', 20, '*', False),
    ('sketch', 'a -> b: message\nlayers: {one: {c -> d}}', 20, '*', True),
    ('selected-subtree', 'root\nlayers: {one: {a -> b; layers: {two: {c -> d}}}; ignored: {z: {width: 900; height: 800}}}',
     20, 'layers.one.*', False),
    ('quoted', 'layers: {"one.dot": {a -> b; layers: {"two.dot": {c -> d}}}}', 20, '*', False),
    ('quoted-selected', 'layers: {"one.dot": {a -> b; layers: {"two.dot": {c -> d}}}; ignored: {z: {width: 900}}}',
     20, 'layers."one.dot".*', False),
    ('zero-pad', 'a -> b\nlayers: {one: {c -> d}}', 0, '*', False),
    ('single-plain', 'a -> b', 20, None, False),
    ('single-root-border', 'style: {stroke-width: 12; double-border: true}\na -> b', 20, None, False),
    ('single-equation', EQUATION, 20, None, False),
    ('single-sketch', 'a -> b: message', 20, None, True),
    ('root-empty-child', 'a -> b\nlayers: {empty: {}}', 20, '*', False),
    ('selected-empty', 'layers: {empty: {}}', 20, 'layers.empty', False),
    ('selected-empty-tree', 'layers: {empty: {}}', 20, 'layers.empty.*', False),
]


def svg_canvas(svg: str, with_hash: bool):
    root = ET.fromstring(svg)
    inner = next(child for child in root if child.tag.rsplit('}', 1)[-1] == 'svg')
    keys = ['width', 'height', 'viewBox', 'preserveAspectRatio']
    expected = {'attributes': {k: root.get(k) for k in keys}, 'inner_attributes': {k: inner.get(k) for k in keys},
                'transitions': svg.count('animation: d2Transition-')}
    if with_hash:
        found = re.search(r'd2Transition-(d2-\d+)-0', svg)
        if not found:
            sys.exit('animated SVG without a d2Transition hash')
        expected['hash'] = found.group(1)
    return expected


def animation_canvas_cases():
    """The historical 46-row grid: each input once with the default scale and once with --scale=1.25."""
    for name, source, pad, target, sketch in ANIMATION_CANVAS:
        for variant, scale in [('omitted', []), ('scaled', ['--scale=1.25'])]:
            args = [f'--pad={pad}', *([f'--target={target}'] if target is not None else []), *scale,
                    *(['--sketch'] if sketch else [])]
            yield f'{name}-{variant}', source, args


def animation_target_canvas():
    fixtures = []
    for name, source, args in animation_canvas_cases():
        directory = fresh(name)
        (directory / 'input.d2').write_text(source)
        out = directory / 'output.svg'
        proc = run(['--layout=dagre', '--animate-interval=500', *args, str(directory / 'input.d2'), str(out)],
                   cwd=directory)
        if proc.returncode != 0:
            sys.exit(f"{name}: {proc.stderr.decode(errors='replace')}")
        expected = svg_canvas(out.read_text(), True) if out.exists() else None
        fixtures.append(dict(name=name, input=source, args=args, expected=expected))
    output(FIXTURES + "animation-target-canvas-source-v090.json").write_text(dump(fixtures))


ANIMATION_IO = [
    ('stdin-multi-file', 'a -> b\nlayers: {one: {c -> d}}', '*', True, False),
    ('stdin-root-multi-stdout', 'a -> b\nlayers: {one: {c -> d}}', '*', True, True),
    ('selected-leaf-stdout', 'layers: {one: {a -> b}}', 'layers.one.*', False, True),
    ('selected-tree-stdout', 'layers: {one: {a -> b; layers: {two: {c -> d}}}}', 'layers.one.*', False, True),
    ('stdin-single-stdout', 'a -> b', None, True, True),
    ('stdin-folder-stdout', 'layers: {one: {}}', '*', True, True),
]


def animation_io():
    fixtures = []
    for name, source, target, use_stdin, use_stdout in ANIMATION_IO:
        flags = ['--layout=dagre', '--animate-interval=500', '--pad=20', *([f'--target={target}'] if target is not None else [])]
        proc, out = stdio_run(fresh(name), flags, source, use_stdin, use_stdout, "output.svg")
        row = dict(name=name, input=source, target=target, stdin=use_stdin, stdout=use_stdout,
                   expected_exit=proc.returncode, expected_error=None, expected=None)
        if proc.returncode == 0:
            row['expected'] = svg_canvas(out.read_text(), False)
        else:
            row['expected_error'] = proc.stderr.decode()
        fixtures.append(row)
    output(FIXTURES + "animation-io-source-v090.json").write_text(dump(fixtures))


# ---------------------------------------------------------------------------
# Rendered TALA/Dagre outputs

def badge_paint_order():
    """.reference/badge-paint-order-20261006/capture-source.py."""
    rows = []
    for kind in ['link', 'tooltip', 'both']:
        for nested in [False, True]:
            for opacity in [1, .35]:
                for edge in [False, True]:
                    name = f'{kind}-nested{int(nested)}-opacity{opacity}-edge{int(edge)}'
                    attrs = []
                    if kind in ['link', 'both']:
                        attrs.append('link: https://example.com')
                    if kind in ['tooltip', 'both']:
                        attrs.append('tooltip: Badge tip')
                    body = ('a: A {style.fill: "#ff0000"; style.opacity: ' + str(opacity) + '; ' + '; '.join(attrs)
                            + '}\nb: B {style.fill: "#00ff00"}\nc: C {style.fill: "#0000ff"}\n')
                    if edge:
                        body += 'a -> c: Edge {style.stroke: "#ff00ff"}\n'
                    source = 'p: {style.fill: "#ffff00";\n' + body + '}\n' if nested else body
                    directory = fresh(name)
                    path = directory / 'input.d2'
                    path.write_text(source)
                    out = directory / 'output.svg'
                    proc = run(['--layout=tala', '--tala-seeds=1', str(path), str(out)], cwd=directory)
                    if proc.returncode != 0:
                        sys.exit(f"{name}: {proc.stderr.decode(errors='replace')}")
                    elements = list(ET.parse(out).iter())
                    parents = {child: parent for parent in elements for child in parent}
                    badges = [e for e in elements if 'appendix-icon' in e.attrib.get('class', '').split()]
                    paint = [e for e in elements if e.attrib.get('fill') in ['#ff0000', '#00ff00', '#0000ff', '#ffff00']
                             or e.attrib.get('stroke') == '#ff00ff']
                    if not badges or not paint:
                        sys.exit(f"{name}: no badges or colored paint")
                    properties = {'badge_count': len(badges),
                                  'badges_after_all_colored_paint': min(elements.index(e) for e in badges)
                                  > max(elements.index(e) for e in paint),
                                  'badge_links_or_opacity_ancestors': 0}
                    for badge in badges:
                        ancestor = parents.get(badge)
                        while ancestor is not None:
                            if (ancestor.tag.endswith('}a') or 'opacity' in ancestor.attrib
                                    or 'opacity:' in ancestor.attrib.get('style', '')):
                                properties['badge_links_or_opacity_ancestors'] += 1
                            ancestor = parents.get(ancestor)
                    rows.append({'name': name, 'source': source, 'expected': properties})
    output(FIXTURES + "badge-paint-order-source-v090.json").write_text(dump(rows))


def render_png(name: str, source: str, flags) -> bytes:
    directory = fresh(name)
    (directory / 'input.d2').write_text(source)
    out = directory / 'output.png'
    proc = run([*flags, str(directory / 'input.d2'), str(out)], cwd=directory)
    if proc.returncode != 0 or not out.is_file():
        sys.exit(f"{name}: {proc.stderr.decode(errors='replace')}")
    return out.read_bytes()


BRAKET = [
    ('braket-fraction-script-source-v090.png', r"\scriptstyle \braket{\frac{a+b}{c}}"),
    ('braket-nested-root-inner-product-source-v090.png', r"x+\braket{\sqrt{\frac{x}{y}}}y"),
    ('braket-nested-root-bra-scripts-source-v090.png', r"\Bra{\sqrt{\frac{x}{y}}}_i^2"),
    ('size-empty-radical-source-v090.png', r"\sqrt\Huge x"),
]


def braket_png():
    for filename, expression in BRAKET:
        data = render_png(filename, f"x: |latex\n  {expression}\n|\n",
                          ['--layout=tala', '--tala-seeds=1', '--theme=0', '--scale=2'])
        output(FIXTURES + filename).write_bytes(data)


def unavailable_image():
    """.reference/raster-unavailable-20261016 `missing`: an image icon that does not exist."""
    data = render_png('missing', 'a: "" {shape: image; icon: absent.png; width: 64; height: 64}\n',
                      ['--layout=dagre', '--timeout=5'])
    output(FIXTURES + "unavailable-image-v090.png").write_bytes(data)


STRUCTURED_FONT = [
    ('class-default', 'a: {shape: class; +id: int; -get(): string}\n'),
    ('sql-default', 'T: {shape: sql_table; id: int {constraint: primary_key}; name: string}\n'),
    ('structured-edge', 'a: {shape: class; id: int}\nT: {shape: sql_table; id: int}\na -> T.id: Uses\n'),
]


def tala_structured_font():
    flags = ['--layout=tala', '--timeout=30', '--tala-seeds=0']
    cases = []
    for name, source in STRUCTURED_FONT:
        data = render_png(name, source, flags)
        output(FIXTURES + f"tala-structured-font-v090/{name}.png").write_bytes(data)
        cases.append(dict(name=name, source=source, source_input_sha256=sha(source.encode()), png_sha256=sha(data),
                          command=['d2', *flags, 'input.d2', 'output.png']))
    output(FIXTURES + "tala-structured-font-v090/provenance.json").write_text(
        dump({'upstream_commit': PIN, 'cases': cases}))


PAGED_RGBA_SOURCE = 'label: Root title\na\nlayers: {"a.b": Section {b; scenarios: {"x.y": Detail {c}}}}\n'
PAGED_RGBA_TARGETS = ['', 'layers."a.b"', 'layers."a.b".scenarios."x.y"']
PML = '{http://schemas.openxmlformats.org/presentationml/2006/main}'
DML = '{http://schemas.openxmlformats.org/drawingml/2006/main}'
REL = '{http://schemas.openxmlformats.org/officeDocument/2006/relationships}'


def slide_images(z: zipfile.ZipFile):
    """Per slide, in slide order, the embedded picture members referenced by its blips."""
    slides = sorted((n for n in z.namelist() if re.fullmatch(r'ppt/slides/slide\d+\.xml', n)),
                    key=lambda n: int(n.rsplit('slide', 1)[1][:-4]))
    result = []
    for filename in slides:
        root = ET.fromstring(z.read(filename))
        rels = ET.fromstring(z.read(posixpath.join(posixpath.dirname(filename), '_rels',
                                                   posixpath.basename(filename) + '.rels')))
        relmap = {x.attrib['Id']: x.attrib for x in rels}
        members = []
        for blip in root.findall(f'.//{PML}pic/{PML}blipFill/{DML}blip'):
            target = relmap[blip.attrib[REL + 'embed']]['Target']
            members.append(posixpath.normpath(posixpath.join(posixpath.dirname(filename), target)))
        result.append(members)
    return result


def paged_rgba():
    directory = fresh('paged-rgba')
    (directory / 'input.d2').write_text(PAGED_RGBA_SOURCE)
    out = directory / 'output.pptx'
    proc = run(['--layout=dagre', '--timeout=30', str(directory / 'input.d2'), str(out)], cwd=directory)
    if proc.returncode != 0:
        sys.exit(proc.stderr.decode(errors='replace'))
    frames = []
    with zipfile.ZipFile(out) as z:
        slides = slide_images(z)
        if len(slides) != len(PAGED_RGBA_TARGETS) or any(len(s) != 1 for s in slides):
            sys.exit(f"expected one picture on each of {len(PAGED_RGBA_TARGETS)} slides: {slides}")
        for index, (target, (member,)) in enumerate(zip(PAGED_RGBA_TARGETS, slides), 1):
            data = z.read(member)
            output(FIXTURES + f"paged-rgba-v090/board-{index}.png").write_bytes(data)
            frames.append(dict(target=target, png=f"board-{index}.png", sha256=sha(data)))
    output(FIXTURES + "paged-rgba-v090/provenance.json").write_text(dump({
        'commit': PIN, 'source': PAGED_RGBA_SOURCE,
        'command': 'd2 --layout=dagre --timeout=30 input.d2 output.pptx', 'frames': frames}))


# ---------------------------------------------------------------------------
# PDF/PPTX navigation and frames (.reference/paged-links-20261018)

PAGED_LINK_SOURCES = [
    ('node-edge', 1, 'document', [], 'label: Links\na: A {link: https://example.com/a; tooltip: Node help; style.stroke-width: 3}\nb: B\na -> b: Edge {link: https://example.com/edge; tooltip: Edge help}\n'),
    ('multiboard', 2, 'document', [], 'cat: how does the cat go? {\n  link: layers.cat\n}\nlayers: {\n  cat: {\n    home: {\n      link: _\n    }\n    the cat -> meow: goes\n  }\n}\n'),
    ('quoted', 3, 'error', [], 'label: Root title\na: {link: layers."a.b"}\nlayers: {"a.b": Section {b: {link: _}; scenarios: {"x.y": Detail {c: {link: _._}}}}}\n'),
    ('markdown', 1, 'document', [], 'a: |md\n  [External](https://example.com/md "Help") and [Relative](root.html)\n|\n'),
    ('themed', 1, 'document', ['--theme=200', '--scale=1.25', '--center=true'], 'label: Themed\nstyle.fill: "#112233"\na: A {link: https://example.com/themed; style.stroke-width: 5}\n'),
    ('sequence', 1, 'document', [], 'shape: sequence_diagram\na: Alice {link: https://example.com/alice}\nb: Bob\na -> b: Message {link: https://example.com/message}\n'),
]
PML_NS = {'a': 'http://schemas.openxmlformats.org/drawingml/2006/main',
          'p': 'http://schemas.openxmlformats.org/presentationml/2006/main'}


def pdf_links(path: Path):
    from pypdf import PdfReader
    reader = PdfReader(path)
    result = []
    for page in reader.pages:
        annotations = []
        for ref in page.get('/Annots', []):
            a = ref.get_object()
            action = a.get('/A', {})
            dest = a.get('/Dest') or action.get('/D')
            dest = dest.get_object() if hasattr(dest, 'get_object') else dest
            page_index = next((i for i, x in enumerate(reader.pages) if x.indirect_reference == dest[0]), None) if dest else None
            annotations.append(dict(rect=[float(v) for v in a['/Rect']], page=page_index, uri=action.get('/URI')))
        result.append(dict(size=[float(v) for v in page.mediabox], text=page.extract_text().rstrip('\n'),
                           annotations=annotations))
    return result


def pptx_links(path: Path):
    from PIL import Image
    z = zipfile.ZipFile(path)
    slides = sorted((n for n in z.namelist() if n.startswith('ppt/slides/slide') and n.endswith('.xml')),
                    key=lambda n: int(n.rsplit('slide', 1)[1][:-4]))
    result, images = [], []
    for filename in slides:
        root = ET.fromstring(z.read(filename))
        rels = ET.fromstring(z.read(posixpath.join(posixpath.dirname(filename), '_rels', posixpath.basename(filename) + '.rels')))
        relmap = {x.attrib['Id']: x.attrib for x in rels}
        links = []
        for shape in root.findall('.//p:sp', PML_NS):
            click = shape.find('p:nvSpPr/p:cNvPr/a:hlinkClick', PML_NS)
            if click is None:
                continue
            rel = relmap[click.attrib[REL + 'id']]
            xf = shape.find('p:spPr/a:xfrm', PML_NS)
            off, ext = xf.find('a:off', PML_NS), xf.find('a:ext', PML_NS)
            links.append(dict(destination=rel['Target'], external=rel.get('TargetMode') == 'External',
                              tooltip=click.get('tooltip', ''), action=click.get('action', ''),
                              rect=[int(off.get('x')), int(off.get('y')), int(ext.get('cx')), int(ext.get('cy'))]))
        nav = [relmap[c.attrib[REL + 'id']]['Target'] for c in root.findall('.//a:r/a:rPr/a:hlinkClick', PML_NS)]
        result.append(dict(text=[x.text for x in root.findall('.//a:t', PML_NS)], links=links, navigation=nav,
                           pictures=[dict(x.attrib) for x in root.findall('.//p:pic/p:spPr/a:xfrm/a:off', PML_NS)
                                     + root.findall('.//p:pic/p:spPr/a:xfrm/a:ext', PML_NS)]))
        frame = []
        for pic in root.findall('.//p:pic/p:blipFill/a:blip', PML_NS):
            rel = relmap[pic.attrib[REL + 'embed']]
            im = Image.open(io.BytesIO(z.read(posixpath.normpath(posixpath.join(posixpath.dirname(filename), rel['Target'])))))
            im = im.convert('RGBA')
            frame.append(dict(width=im.width, height=im.height, rgba_sha256=sha(im.tobytes())))
        if not frame:
            sys.exit(f"{path}: slide without a picture")
        images.append(frame)
    return result, images


def paged_links():
    rows = []
    for key, pages, kind, flags, source in PAGED_LINK_SOURCES:
        for engine in ['dagre', 'elk', 'tala']:
            name = f'{engine}-{key}'
            directory = fresh(name)
            (directory / 'input.d2').write_text(source)
            row = dict(name=name, engine=engine, expected_pages=pages, expected_kind=kind, flags=flags, source=source)
            results = {}
            for fmt in ['pdf', 'pptx']:
                out = directory / f'output.{fmt}'
                proc = run([f'--layout={engine}', '--timeout=30', *(['--tala-seeds=0'] if engine == 'tala' else []),
                            *flags, str(directory / 'input.d2'), str(out)], cwd=directory)
                results[fmt] = (proc, out)
            if kind == 'error':
                row['errors'] = {}
                for fmt, (proc, out) in results.items():
                    if proc.returncode != 1 or out.exists():
                        sys.exit(f"{name} {fmt}: expected a failure without output")
                    row['errors'][fmt] = proc.stderr.decode().split(': ', 2)[-1].rstrip('\n')
            else:
                for fmt, (proc, out) in results.items():
                    if proc.returncode != 0:
                        sys.exit(f"{name} {fmt}: {proc.stderr.decode(errors='replace')}")
                row['pdf'] = pdf_links(results['pdf'][1])
                row['pptx'], row['images'] = pptx_links(results['pptx'][1])
                if len(row['pdf']) != pages or len(row['pptx']) != pages:
                    sys.exit(f"{name}: expected {pages} pages")
            rows.append(row)
    output(FIXTURES + "paged-links-v090.json").write_text(dump({'commit': PIN, 'rows': rows}))


# ---------------------------------------------------------------------------
# Engine option hydration (.reference/engine-options-20261019)

ENGINE_SOURCES = {
    'ordinary': 'a -> c: first\nb -> c: second\na -> c: parallel\nc -> c: loop\n',
    'container': 'box: {a -> c: first; b -> c: second}\nx -> box\n',
    'grid': 'g: {grid-columns: 2; cell: {a -> c; b -> c}; x}\nx -> g\n',
    'sequence': 'seq: {shape: sequence_diagram; a -> b: request; b -> a: response}\nx; y; x -> seq; y -> seq\n',
    'boards': 'a -> b\nlayers: {detail: {x -> z; y -> z}}\n',
}
ENGINE_ARGS = {
    'dagre': [('default', []), ('node20', ['--dagre-nodesep=20']), ('node140', ['--dagre-nodesep', '140']),
              ('edge80', ['--dagre-edgesep=80']), ('both', ['--dagre-nodesep=120', '--dagre-edgesep=60'])],
    'elk': [('default', []), ('node30', ['--elk-nodeNodeBetweenLayers=30']),
            ('edge90', ['--elk-edgeNodeBetweenLayers', '90']), ('self100', ['--elk-nodeSelfLoop=100']),
            ('padding', ['--elk-padding=[top=20,left=70,bottom=40,right=90]']), ('layered', ['--elk-algorithm=layered'])],
}
BOARD_MANIFEST = '.d2-board-output-manifest-v1.json'


def rgba_frame(path: Path):
    """Opaque RGB8 PNG expanded to RGBA (engine_option_tests.rs `frame`)."""
    from PIL import Image
    image = Image.open(path)
    image.load()
    if image.mode != 'RGB' or image.info.get('interlace'):
        sys.exit(f"{path}: expected non-interlaced opaque RGB8 PNG, got {image.mode}")
    return {'width': image.width, 'height': image.height, 'rgba_sha256': sha(image.convert('RGBA').tobytes())}


def engine_options():
    rows = []
    for kind, source in ENGINE_SOURCES.items():
        for layout, variants in ENGINE_ARGS.items():
            for suffix, args in variants:
                name = f'{kind}-{layout}-{suffix}'
                directory = fresh(name)
                (directory / 'input.d2').write_text(source)
                out = directory / 'output.png'
                proc = run([f'--layout={layout}', '--timeout=30', *args, str(directory / 'input.d2'), str(out)],
                           cwd=directory)
                if proc.returncode != 0:
                    sys.exit(f"{name}: {proc.stderr.decode(errors='replace')}")
                files = {k: p for k, p in png_outputs(out).items() if Path(k).name.lower() != BOARD_MANIFEST}
                rows.append(dict(name=name, source=source, layout=layout, args=args,
                                 expected={k: rgba_frame(p) for k, p in files.items()}))
    output(FIXTURES + "engine-options-source-v090.json").write_text(dump({
        'provenance': {'source': 'Go d2 at the manifest pin: d2cli layout resolver, in-process Dagre and ELK option hydration',
                       'comparison': 'Complete PNG inventories (board output manifests excluded); opaque RGB pixels expanded with alpha 255.'},
        'rows': rows}))


# ---------------------------------------------------------------------------
# Version and help dispatch (.reference/cli-version-20261019/capture.py)

VERSION_ARGS = [['--version'], ['-v'], ['version'], ['version', 'extra'], ['version', 'a', 'b'], ['--version=true'],
                ['--version=false'], ['-v=true'], ['-v=false'], ['--version=bogus'], ['--version', '--version=false'],
                ['--version=false', '--version'], ['--pad=20', 'version'], ['version', '--pad=20'], ['--version', 'version'],
                ['version', '--version=false'], ['--bogus', '--version'], ['--version', '--bogus'], ['version', '--bogus'],
                ['--version', '--pad=bad'], ['--pad=bad', 'version'], ['version', '--'], ['--', 'version'],
                ['--version=false', 'version'], ['--version', 'false'],
                ['--version', '--stdout-format=ascii', 'input.d2', '-'], ['--stdout-format=ascii', 'input.d2', '-', '--version'],
                ['--version=false', '--stdout-format=ascii', 'input.d2', '-'],
                ['--version=true', '--stdout-format=ascii', 'input.d2', '-'], ['--salt', '--version'],
                ['--scale=1.5', '--version'], ['--version', '--theme=999999']]


def version_flags():
    directory = fresh('version')
    (directory / 'input.d2').write_text('a -> b: version control\n')
    rows = []
    for index, args in enumerate(VERSION_ARGS):
        proc = run(args, cwd=directory, timeout=30)
        stdout = proc.stdout.decode().replace(PROGRAM, '{program}')
        stderr = proc.stderr.decode().replace(PROGRAM, '{program}')
        # Document rendering reached the compiler: its timing and scratch
        # paths vary per run, and the tests only check the dispatch.
        document = 'compiled' in stderr or 'failed to compile' in stderr
        if document:
            stderr = re.sub(r' in [0-9.]+(?:ns|µs|ms|s|m[0-9.]+s|h[0-9hm.]+s)\n', ' in {duration}\n', stderr)
            stderr = stderr.replace(str(directory), '{cwd}')
        rows.append(dict(name=f'version-{index:02}', args=args, exit=proc.returncode, stdout=stdout, stderr=stderr,
                         dispatch='document' if document else 'output'))
    help_rows = [r for r in rows if 'Usage:\n' in r['stdout']]
    if not help_rows or len({r['stdout'] for r in help_rows}) != 1:
        sys.exit("help output differs between help dispatches")
    output(FIXTURES + "version-flags-source-v090.json").write_text(dump({
        'provenance': {'source': 'Go d2 at the manifest pin: d2cli/main.go, util-go/xmain/opts.go, pflag',
                       'comparison': 'The executable name in help becomes {program}; compiler timings and the '
                                     'scratch directory in document-dispatch stderr become {duration} and {cwd}.'},
        'rows': rows}))
    output(FIXTURES + "help-master-6a7b4837.txt").write_text(help_rows[0]['stdout'])


# ---------------------------------------------------------------------------
# Upstream golden copy

def sanity_svg():
    """Upstream e2etests/testdata/sanity/basic/dagre/sketch.exp.svg, stamped v0.9.0 like the d2-rs CLI."""
    upstream = Path(os.environ["D2RS_UPSTREAM"])
    text = (upstream / "e2etests/testdata/sanity/basic/dagre/sketch.exp.svg").read_text()
    text, count = re.subn(r'data-d2-version="[^"]*"', 'data-d2-version="v0.9.0"', text)
    if count != 1:
        sys.exit("golden has no single data-d2-version attribute")
    output(FIXTURES + "sanity-basic-dagre-v090.svg").write_text(text + "\n")


# ---------------------------------------------------------------------------
# Isometric board hash inputs

ISOMETRIC_SOURCE = "a: {link: layers.detail}\nlayers: {detail: {b: {link: root}}}\nscenarios: {alternate: {c}}\nsteps: {next: {d}}\n"
HASH_ID = ("func (diagram Diagram) HashID(salt *string) (string, error) {\n\tbytes, err := diagram.Bytes()\n"
           "\tif err != nil {\n\t\treturn \"\", err\n\t}\n")
HASH_DUMP = ("\tif d2rsPath := os.Getenv(\"D2RS_HASH_DUMP\"); d2rsPath != \"\" {\n"
             "\t\tif _, err := os.Stat(d2rsPath); os.IsNotExist(err) {\n"
             "\t\t\t_ = os.WriteFile(d2rsPath, bytes, 0o644)\n\t\t}\n\t}\n")


def isometric_hash():
    """Diagram.Bytes() of the first HashID call (the SVG renderer's diagram hash)
    of `d2 --isometric` for the root board alone and for the whole board tree.
    The CLI is rebuilt with an env-gated dump in d2target.HashID, supplied as a
    go -overlay; the dump is inert unless D2RS_HASH_DUMP is set."""
    upstream = Path(os.environ["D2RS_UPSTREAM"])
    work = workdir() / "isometric-hash"
    work.mkdir(parents=True, exist_ok=True)
    text = (upstream / "d2target/d2target.go").read_text()
    if text.count(HASH_ID) != 1:
        sys.exit("d2target.HashID changed shape")
    text = text.replace(HASH_ID, HASH_ID + HASH_DUMP)
    if '\t"os"\n' not in text:
        text = text.replace("import (\n", "import (\n\t\"os\"\n", 1)
    (work / "d2target.go").write_text(text)
    (work / "overlay.json").write_text(json.dumps({"Replace": {str(upstream / "d2target/d2target.go"): str(work / "d2target.go")}}))
    binary = work / "d2"
    subprocess.run(["go", "build", "-mod=readonly", "-overlay", str(work / "overlay.json"),
                    "-ldflags=-X github.com/d2lang/d2/lib/version.Version=v0.9.0", "-o", str(binary), "."],
                   cwd=upstream, env=go_env(), check=True)
    for name, args in [("root", ["--target=", "--scale=.2"]), ("tree", ["--target=*"])]:
        directory = fresh(f"isometric-{name}")
        (directory / "input.d2").write_text(ISOMETRIC_SOURCE)
        dump = directory / "hash-input.bytes"
        proc = subprocess.run([PROGRAM, "--isometric", "--layout=dagre", *args, str(directory / "input.d2"),
                               str(directory / "out.svg")], executable=str(binary), cwd=directory, capture_output=True,
                              env=dict(os.environ, D2RS_HASH_DUMP=str(dump)), timeout=300)
        if proc.returncode != 0 or not dump.is_file():
            sys.exit(f"isometric {name}: {proc.stderr.decode(errors='replace')}")
        output(FIXTURES + f"isometric-{name}-hash-master.bytes").write_bytes(dump.read_bytes())


# ---------------------------------------------------------------------------
# Layout and sketch probes (tools/fixtures/programs/cli-layout-probes)

PROBES = {
    'label-positions': "crates/d2-rs-cli/tests/fixtures/authored-label-position-source-v090.json",
    'raw-labeled': "crates/d2-rs-lib/tests/fixtures/raw-labeled-opposing-source-v090.json",
    'dagre-flat': "crates/d2-rs-dagre/tests/fixtures/flat-long-label-source-v090.json",
    'solid-paths': "crates/d2-rs-sketch/tests/fixtures/solid-paths-source-v090.tsv",
}


def probe(name: str):
    """Build the probe in a scratch module with upstream's own go.mod/go.sum
    (upstream replaced by the patched tree) and capture one probe's stdout."""
    upstream = Path(os.environ["D2RS_UPSTREAM"])
    module = workdir() / "probe-module"
    module.mkdir(parents=True, exist_ok=True)
    shutil.copy2(ROOT / "tools/fixtures/programs/cli-layout-probes/main.go", module / "main.go")
    go_mod, count = re.subn(r'(?m)^module github\.com/d2lang/d2$', 'module d2rs.local/cli-layout-probes',
                            (upstream / "go.mod").read_text())
    if count != 1:
        sys.exit("upstream go.mod has no module github.com/d2lang/d2 line")
    go_mod += ('\nrequire github.com/d2lang/d2 v0.0.0-00010101000000-000000000000\n'
               f'\nreplace github.com/d2lang/d2 => {upstream}\n')
    (module / "go.mod").write_text(go_mod)
    shutil.copy2(upstream / "go.sum", module / "go.sum")
    binary = workdir() / "bin" / "cli-layout-probes"
    subprocess.run(["go", "build", "-mod=readonly", "-o", str(binary), "."], cwd=module, env=go_env(), check=True)
    env = dict(os.environ, D2RS_SIMPLE_GRAPH=str(ROOT / "crates/d2-rs-lib/tests/fixtures/simple.json"))
    result = subprocess.run([str(binary), name], capture_output=True, env=env, timeout=1200)
    if result.returncode != 0:
        sys.exit(f"probe {name}: {result.stderr.decode(errors='replace')}")
    output(PROBES[name]).write_bytes(result.stdout)


FAMILIES = {
    **{f'probe-{name}': (lambda name=name: probe(name)) for name in PROBES},
    'gif-subtree': gif_subtree, 'paged-subtree': paged_subtree, 'png-metadata': png_metadata,
    'subtree-output': subtree_output, 'animation-target-canvas': animation_target_canvas,
    'animation-io': animation_io, 'badge-paint-order': badge_paint_order, 'braket-png': braket_png,
    'unavailable-image': unavailable_image, 'tala-structured-font': tala_structured_font, 'paged-rgba': paged_rgba,
    'engine-options': engine_options, 'version-flags': version_flags, 'sanity-svg': sanity_svg,
    'paged-links': paged_links, 'isometric-hash': isometric_hash,
}

if __name__ == "__main__":
    if len(sys.argv) != 2 or sys.argv[1] not in FAMILIES:
        sys.exit(f"usage: {sys.argv[0]} {{{'|'.join(FAMILIES)}}}")
    FAMILIES[sys.argv[1]]()
