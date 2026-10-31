#!/usr/bin/env python3
"""Source compile/render and raw-argument d2js fixtures (recipe `d2js-sources`).

Runs tools/d2js-capture/sources.mjs against the `d2js-node-package` build. It
replays the committed request lists in tools/d2js-capture/inputs and derives
two families from the `d2js-target` and `d2lib-sequence-targets` artifacts in
$D2RS_FIXTURES_DIR, writing each d2-rs-wasm fixture to the stage.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]


def main() -> None:
    commit = os.environ.get("D2RS_CAPTURE_COMMIT") or json.loads((ROOT / "reference-manifest.json").read_text())["d2"]["commit"]
    stage = Path(os.environ["D2RS_RECIPE_STAGE"])
    subprocess.run(["node", str(ROOT / "tools/d2js-capture/sources.mjs"), str(stage), str(ROOT / "tools/d2js-capture/inputs"),
                    os.environ["D2RS_FIXTURES_DIR"], commit], env=fixture_recipe.go_env(), check=True)
    for logical in os.environ["D2RS_RECIPE_OUTPUTS"].split(os.pathsep):
        if not Path(logical).is_file():
            sys.exit(f"sources.mjs did not write {logical}")


if __name__ == "__main__":
    main()
