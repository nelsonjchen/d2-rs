#!/usr/bin/env python3
"""Generate local embedded-image inputs and compare complete CLI exports.

Run with uv run --offline --no-project --with pillow python scripts/embedded-images-sweep.py.
Generated inputs and captures stay under oracle-cache/reports; no network assets.
"""
from __future__ import annotations

import argparse
import base64
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
import hashlib
import io
import itertools
import json
from pathlib import Path
import re
import shutil
import struct
import subprocess
import tempfile
import zlib

from PIL import Image

import references
from fuzz_common import _validate_output

ROOT = Path(__file__).resolve().parents[1]
MIME = {"png": "image/png", "jpg": "image/jpeg", "gif": "image/gif",
        "webp": "image/webp", "svg": "image/svg+xml", "bin": "application/octet-stream"}


def interlaced_png(image):
    """Pillow's PNG writer ignores interlace; encode actual Adam7 scanlines."""
    def chunk(kind, payload):
        return (struct.pack(">I", len(payload)) + kind + payload
                + struct.pack(">I", zlib.crc32(kind + payload)))
    raw = bytearray()
    for x0, y0, dx, dy in [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8),
                            (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)]:
        for y in range(y0, image.height, dy):
            raw.append(0)
            for x in range(x0, image.width, dx):
                raw.extend(image.getpixel((x, y)))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", *image.size, 8, 2, 0, 0, 1))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def generate(directory):
    directory.mkdir(parents=True, exist_ok=True)
    rgb = Image.new("RGB", (19, 13))
    rgb.putdata([((x * 41 + y * 13) % 256, (x * 7 + y * 37) % 256,
                  (x * 29 + y * 11) % 256) for y in range(13) for x in range(19)])
    rgba = rgb.convert("RGBA")
    rgba.putalpha(Image.frombytes("L", rgb.size, bytes((i * 17) % 256 for i in range(247))))
    assets = {}
    def save(name, image, **options):
        path = directory / name
        image.save(path, **options)
        assets[name] = path
    save("rgb.png", rgb)
    save("rgba.png", rgba)
    save("palette.png", rgba.quantize(colors=16))
    gray16 = Image.frombytes("I;16", rgb.size, b"".join(struct.pack("<H", i * 257) for i in range(247)))
    save("gray16.png", gray16)
    path = directory / "interlaced.png"
    path.write_bytes(interlaced_png(rgb))
    assets[path.name] = path
    save("baseline.jpg", rgb, quality=87)
    save("progressive.jpg", rgb, quality=87, progressive=True)
    save("cmyk.jpg", rgb.convert("CMYK"), quality=87)
    exif = Image.Exif()
    exif[274] = 6
    save("orientation.jpg", rgb, quality=87, exif=exif)
    save("static.gif", rgba)
    save("animated.gif", rgba, save_all=True, append_images=[rgba.transpose(Image.Transpose.FLIP_LEFT_RIGHT)],
         duration=[80, 120], loop=0, disposal=2)
    save("lossy.webp", rgb, quality=80)
    save("lossless.webp", rgb, lossless=True)
    save("alpha.webp", rgba, lossless=True)
    save("lossy-alpha.webp", rgba, quality=80)
    path = directory / "vector.svg"
    path.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="19" height="13" viewBox="0 0 19 13">'
                    '<rect width="19" height="13" fill="#4169e1"/><circle cx="8" cy="6" r="4" fill="#ff8000"/></svg>')
    assets[path.name] = path
    path = directory / "corrupt.png"
    path.write_bytes(b"\x89PNG\r\n\x1a\nnot a complete image")
    assets[path.name] = path
    path = directory / "wrong.bin"
    path.write_bytes(assets["rgb.png"].read_bytes())
    assets[path.name] = path
    # Check that generated fixtures really decode, including the Adam7 file.
    for name, path in assets.items():
        if name not in ("corrupt.png", "vector.svg"):
            with Image.open(path) as image:
                image.load()
    return assets


def source_for(path, transport, shape, animated):
    content = path.read_bytes()
    uri = (f"data:{MIME[path.suffix[1:]]};base64," + base64.b64encode(content).decode()
           if transport == "data" else str(path))
    properties = 'shape: image; ' if shape == 'image' else ''
    source = ('a: {label: Picture; ' + properties + 'icon: "' + uri + '"; '
              'width: 97; height: 71; label.near: outside-bottom-center}\n'
              'b: Neighbor\na -> b\n')
    if animated:
        source += 'steps: {one: {b.label: Next}}\n'
    return source


def normalized(data, suffix):
    if suffix == ".pdf":
        # Same-width substitution keeps offsets intact; dates are the only PDF normalization.
        return re.sub(rb'/(CreationDate|ModDate) \(D:[^)]*\)',
                      lambda m: re.sub(rb'\d', b'0', m[0]), data)
    return data


def capture(binary, directory, source, engine, fmt, iso, timeout):
    directory.mkdir()
    (directory / "input.d2").write_text(source)
    flags = [f"--layout={engine}", "--tala-seeds=1", "--omit-version"]
    if iso:
        flags.append("--isometric")
    if fmt == "gif":
        flags.append("--animate-interval=100")
    try:
        result = subprocess.run([str(binary), *flags, "input.d2", f"output.{fmt}"],
                                cwd=directory, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return {"exit": "timeout", "files": {}, "invalid": []}
    files, invalid = {}, []
    for path in sorted(directory.rglob("*")):
        if not path.is_file() or path.name == "input.d2":
            continue
        if path.stat().st_size > 128 * 1024**2:
            raise RuntimeError("per-file output budget exceeded")
        data = path.read_bytes()
        try:
            _validate_output(path, data)
            if path.suffix in (".png", ".gif"):
                with Image.open(io.BytesIO(data)) as image:
                    for frame in range(image.n_frames):
                        image.seek(frame)
                        image.load()
        except (ValueError, OSError) as error:
            invalid.append(f"{path.name}: {error}")
        files[str(path.relative_to(directory))] = hashlib.sha256(normalized(data, path.suffix)).hexdigest()
    if result.returncode == 0 and not any(p.endswith('.' + fmt) for p in files):
        invalid.append("missing output inventory")
    stderr = result.stderr.decode(errors="backslashreplace").replace(str(directory), "DIR")
    # CLI success logs include measured duration, as in differential-scoreboard.py.
    stderr = re.sub(r" in [0-9.]+(?:ns|µs|us|ms|s|m[0-9.]+s|h[0-9hms.]+)$", " in T", stderr, flags=re.M)
    return {"exit": result.returncode, "stdout": result.stdout.decode(errors="backslashreplace"),
            "stderr": stderr, "files": files, "invalid": invalid}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust", type=Path, default=ROOT / "target/diagnostic/d2-rs")
    parser.add_argument("--go", type=Path, default=references.GO_ORACLE)
    parser.add_argument("--engines", default="dagre,elk,tala")
    parser.add_argument("--formats", default="svg,png,pdf,pptx,gif")
    parser.add_argument("--modes", default="normal,isometric")
    parser.add_argument("--assets", help="comma separated asset filenames; default all")
    parser.add_argument("--timeout", type=float, default=30)
    parser.add_argument("--jobs", type=int, choices=range(1, 7), default=3)
    parser.add_argument("--out", type=Path, default=references.REPORTS / "embedded-images")
    args = parser.parse_args()
    for field, allowed in [("engines", {"dagre", "elk", "tala"}),
                           ("formats", {"svg", "png", "pdf", "pptx", "gif"}),
                           ("modes", {"normal", "isometric"})]:
        values = getattr(args, field).split(',')
        if not set(values) <= allowed or len(values) != len(set(values)):
            parser.error(f"--{field} must select distinct values from {', '.join(sorted(allowed))}")
    args.out.mkdir(parents=True, exist_ok=True)
    rows = []
    with tempfile.TemporaryDirectory(prefix="inputs-", dir=args.out) as temporary:
        root = Path(temporary)
        assets = generate(root / "assets")
        names = args.assets.split(',') if args.assets else assets
        matrix = itertools.product(names, ("local", "data"), ("icon", "image"),
                                   args.engines.split(','), args.formats.split(','), args.modes.split(','))
        def run_case(item):
            index, (name, transport, shape, engine, fmt, mode) = item
            source = source_for(assets[name], transport, shape, fmt == "gif")
            directory = root / f"case-{index:04}"
            directory.mkdir()
            got = {label: capture(binary.resolve(), directory / label, source, engine, fmt,
                                  mode == "isometric", args.timeout)
                   for label, binary in [("go", args.go), ("rust", args.rust)]}
            failures = [key for key in got['go'] if got['go'].get(key) != got['rust'].get(key)]
            if any(r["invalid"] or r["exit"] == "timeout" for r in got.values()):
                failures.append("invalid-or-timeout")
            row = dict(asset=name, transport=transport, shape=shape, engine=engine, format=fmt,
                       mode=mode, outcome="mismatch" if failures else "match", differences=failures)
            if failures:
                row.update(got)
            return index, row, source, directory

        retained_bytes = 0
        with ThreadPoolExecutor(max_workers=args.jobs) as pool:
            for index, row, source, directory in pool.map(run_case, enumerate(matrix)):
                if row['outcome'] != 'match':
                    witness = args.out / f"case-{index:04}"
                    if witness.exists():
                        raise RuntimeError(f"capture exists: {witness}; use a fresh --out")
                    shutil.copytree(directory, witness)
                    # Keep witnesses reproducible after the scratch directory is removed.
                    shutil.copytree(root / 'assets', witness / 'assets')
                    for label in ('go', 'rust'):
                        (witness / label / 'input.d2').write_text(source.replace(str(root / 'assets'), str(witness.resolve() / 'assets')))
                    retained_bytes += sum(p.stat().st_size for p in witness.rglob('*') if p.is_file())
                rows.append(row)
                shutil.rmtree(directory)
                if retained_bytes > 1024**3:
                    raise RuntimeError("total capture budget exceeded")
                if (index + 1) % 50 == 0:
                    print(index + 1, dict(Counter(r['outcome'] for r in rows)), flush=True)
    report = {"source_commit": references.SOURCE_COMMIT,
              "binaries": {k: references.digest(v) for k, v in [("go", args.go), ("rust", args.rust)]},
              "counts": dict(Counter(r['outcome'] for r in rows)), "rows": rows}
    (args.out / "summary.json").write_text(json.dumps(report, indent=2) + '\n')
    print(report['counts'])
    return int(any(r['outcome'] != 'match' for r in rows))


if __name__ == "__main__":
    raise SystemExit(main())
