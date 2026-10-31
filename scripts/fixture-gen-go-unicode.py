"""Generator for recipe mathtext-txt-go-unicode (tools/fixtures/mathtext-txt.toml).

Writes vendor/rustybuzz-go-v090/tests/fixtures/go-unicode-v090.json: go-text/
typesetting's script, general category, combining class, mirror and modified
combining class lookups over every Unicode scalar value (run-length ranges),
its canonical decompositions and composition probes.

The lookups are unexported, so tools/mathtext-unicode-oracle is built with two
files from tools/fixtures/hooks/mathtext-txt-go-unicode added to a disposable
copy of the pinned typesetting module (Go forbids overlays in the module
cache, which is never modified).
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402
import mathtext_gen  # noqa: E402

LOGICAL = "vendor/rustybuzz-go-v090/tests/fixtures/go-unicode-v090.json"
MODULE = "github.com/go-text/typesetting"
PROGRAM = mathtext_gen.ROOT / "tools/mathtext-unicode-oracle"
HOOKS = mathtext_gen.ROOT / "tools/fixtures/hooks/mathtext-txt-go-unicode"


def main() -> None:
    work = fixture_recipe.workdir()
    module = work / "module"
    module.mkdir()
    for name in ("main.go", "go.mod", "go.sum"):
        shutil.copy2(PROGRAM / name, module / name)
    # Go refuses overlays beneath GOMODCACHE, so build against a disposable
    # copy of the pinned module (a `replace`) with the hook files added.
    source = mathtext_gen.module_dir(MODULE, mathtext_gen.required_version(PROGRAM / "go.mod", MODULE))
    copy = work / "typesetting"
    shutil.copytree(source, copy, copy_function=shutil.copyfile)
    for directory, _, _ in os.walk(copy):
        os.chmod(directory, 0o755)  # copytree keeps the cache's read-only directory modes
    shutil.copyfile(HOOKS / "d2rs_unicode_properties.go", copy / "harfbuzz/d2rs_unicode_properties.go")
    shutil.copyfile(HOOKS / "d2rs_composition.go", copy / "internal/unicodedata/d2rs_composition.go")
    with (module / "go.mod").open("a") as go_mod:
        go_mod.write(f"\nreplace {MODULE} => {copy}\n")
    binary = work / "mathtext-unicode-oracle"
    env = fixture_recipe.go_env()
    subprocess.run(["go", "build", "-mod=readonly", "-o", str(binary), "."],
                   cwd=module, env=env, check=True, stdout=sys.stderr)
    with fixture_recipe.output(LOGICAL).open("wb") as out:
        subprocess.run([str(binary)], env=env, check=True, stdout=out)


if __name__ == "__main__":
    main()
