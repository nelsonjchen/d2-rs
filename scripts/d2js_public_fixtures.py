#!/usr/bin/env python3
"""Public d2js API fixtures for d2-rs-wasm (recipe `d2js-public`).

Generates the URL-codec decode grid (scripts/url_codec_cases.py), then runs
tools/d2js-capture/public.mjs against the `d2js-node-package` build, which
replays the committed request lists in tools/d2js-capture/inputs and writes
each fixture to the stage.
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
    env = fixture_recipe.go_env()
    stage = Path(os.environ["D2RS_RECIPE_STAGE"])
    for logical in os.environ["D2RS_RECIPE_OUTPUTS"].split(os.pathsep):
        Path(logical).parent.mkdir(parents=True, exist_ok=True)
    goroot = subprocess.check_output(["go", "env", "GOROOT"], env=env, text=True).strip()
    codec = fixture_recipe.workdir() / "url-codec-cases.json"
    subprocess.run([sys.executable, str(ROOT / "scripts/url_codec_cases.py"), str(codec), goroot], check=True)
    subprocess.run(["node", str(ROOT / "tools/d2js-capture/public.mjs"), str(stage), str(ROOT / "tools/d2js-capture/inputs"), commit],
                   env=dict(env, D2RS_URL_CODEC_CASES=str(codec)), check=True)
    for logical in os.environ["D2RS_RECIPE_OUTPUTS"].split(os.pathsep):
        if not Path(logical).is_file():
            sys.exit(f"public.mjs did not write {logical}")


if __name__ == "__main__":
    main()
