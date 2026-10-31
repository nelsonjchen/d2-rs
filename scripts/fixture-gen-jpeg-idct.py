"""Generator for recipe mathtext-img-jpeg-idct (tools/fixtures/mathtext-img.toml).

Captures Go's unexported image/jpeg idct (src/image/jpeg/dct.go of the
manifest toolchain, copied at run time into a scratch main package, never
committed) on raw 8x8 coefficient blocks: the zero block, every basis
position at ten signed magnitudes up to the int32 limits, and 16 seeded random
blocks at each of four magnitude ranges. Writes [{"input", "output"}] to
$D2RS_RECIPE_OUTPUT.
"""
from __future__ import annotations

import json
import random
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

MAGNITUDES = (-2**31, -65536, -4096, -255, -1, 1, 255, 4096, 65536, 2**31 - 1)
RANDOM_RANGES = ((-255, 255), (-2047, 2047), (-65535, 65535), (-2**31, 2**31 - 1))

MAIN = """package main

import (
\t"encoding/json"
\t"os"
)

func main() {
\tvar blocks []block
\tif err := json.NewDecoder(os.Stdin).Decode(&blocks); err != nil {
\t\tpanic(err)
\t}
\tfor i := range blocks {
\t\tidct(&blocks[i])
\t}
\tif err := json.NewEncoder(os.Stdout).Encode(blocks); err != nil {
\t\tpanic(err)
\t}
}
"""


def blocks() -> list[list[int]]:
    out = [[0] * 64]
    for position in range(64):
        for value in MAGNITUDES:
            block = [0] * 64
            block[position] = value
            out.append(block)
    rng = random.Random(759)
    for low, high in RANDOM_RANGES:
        out += [[rng.randint(low, high) for _ in range(64)] for _ in range(16)]
    return out


def main() -> None:
    env = fixture_recipe.go_env()
    work = fixture_recipe.workdir() / "jpeg-idct"
    work.mkdir(parents=True, exist_ok=True)
    goroot = subprocess.run(["go", "env", "GOROOT"], env=env, check=True, capture_output=True, text=True).stdout.strip()
    dct = (Path(goroot) / "src/image/jpeg/dct.go").read_text()
    if "\npackage jpeg\n" not in dct or "func idct(b *block)" not in dct:
        sys.exit("unexpected image/jpeg dct.go layout")
    (work / "dct.go").write_text(dct.replace("\npackage jpeg\n", "\npackage main\n", 1))
    (work / "main.go").write_text(MAIN)
    (work / "go.mod").write_text("module d2rs.local/jpegidct\n\ngo 1.27\n")
    subprocess.run(["go", "build", "-o", "idct", "."], cwd=work, env=env, check=True)
    inputs = blocks()
    result = subprocess.run([str(work / "idct")], input=json.dumps(inputs), env=env, check=True,
                            capture_output=True, text=True)
    outputs = json.loads(result.stdout)
    if len(outputs) != len(inputs):
        sys.exit(f"{len(outputs)} outputs for {len(inputs)} blocks")
    rows = [{"input": i, "output": o} for i, o in zip(inputs, outputs)]
    Path(fixture_recipe._env("D2RS_RECIPE_OUTPUT")).write_text(json.dumps(rows, separators=(",", ":")) + "\n")


if __name__ == "__main__":
    main()
