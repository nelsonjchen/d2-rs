#!/usr/bin/env python3
"""Build upstream d2js at the pin as a Node ESM package (recipe `d2js-node-package`).

Mirrors the Node-ESM half of upstream ``d2js/js/build.js`` without Bun:
``index.js`` and ``worker.node.js`` are copied byte-exact, ``platform.node.js``
gets the same module-directory banner build.js prepends, ``d2.wasm`` is
``GOOS=js GOARCH=wasm go build -trimpath -ldflags='-s -w' ./d2js`` (unstamped,
as the v0.9.0-era captures were) and ``wasm_exec.js`` is the manifest
toolchain's own ``lib/wasm/wasm_exec.js``. Consumers import
``$D2RS_FIXTURES_DIR/tools/d2js-node/package/index.js`` (see
``tools/d2js-capture/d2js.mjs``).
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

PREFIX = "tools/d2js-node/package/"
BANNER = ('import {fileURLToPath} from "node:url";\n'
          'import {dirname} from "node:path";\n'
          'const __D2_NODE_MODULE_DIR__=dirname(fileURLToPath(import.meta.url));\n')


def main() -> None:
    upstream = Path(os.environ["D2RS_UPSTREAM"])
    env = fixture_recipe.go_env()
    src = upstream / "d2js/js/src"
    fixture_recipe.output(PREFIX + "index.js").write_bytes((src / "index.js").read_bytes())
    fixture_recipe.output(PREFIX + "worker.js").write_bytes((src / "worker.node.js").read_bytes())
    fixture_recipe.output(PREFIX + "platform.js").write_bytes(BANNER.encode() + (src / "platform.node.js").read_bytes())
    fixture_recipe.output(PREFIX + "package.json").write_text(json.dumps({"type": "module"}) + "\n")
    goroot = subprocess.check_output(["go", "env", "GOROOT"], cwd=upstream, env=env, text=True).strip()
    fixture_recipe.output(PREFIX + "wasm_exec.js").write_bytes((Path(goroot) / "lib/wasm/wasm_exec.js").read_bytes())
    wasm = fixture_recipe.output(PREFIX + "d2.wasm")
    subprocess.run(["go", "build", "-mod=readonly", "-trimpath", "-ldflags=-s -w", "-o", str(wasm), "./d2js"],
                   cwd=upstream, env=dict(env, GOOS="js", GOARCH="wasm"), check=True)


if __name__ == "__main__":
    main()
