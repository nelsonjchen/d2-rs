#!/usr/bin/env python3
"""ASCII caller contracts (recipe renderexport-ascii-callers, docs/FIXTURES.md).

For each hand-written source in tools/fixtures/inputs/renderexport/ascii-callers-sources.json
and each layout (dagre, elk, tala, default), ASCII mode (extended, standard) and
scale (unset, 2):

* ``output``: the ASCII text of the pinned upstream d2js public client
  (tools/d2js-capture/ascii-callers.mjs, `d2js-node-package` build);
* ``cli_kind`` / ``cli_error_template``: the pinned Go CLI writing
  ``input.d2`` to ``output.txt`` with the same flags: ``text`` (file written),
  ``no-output`` (success, no file) or ``error`` (stderr, with the input path
  as ``{input}`` and the row directory as ``{directory}``).

Readers: d2-rs-cli complete_original_go_ascii_file_caller_contracts and
d2-rs-wasm's source ASCII render contract.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
LOGICAL = "crates/d2-rs-export/tests/fixtures/ascii-callers-source-v090.json"
ENGINES = ["dagre", "elk", "tala", None]
MODES = ["extended", "standard"]
SCALES = [None, 2]


def human_paths(path: str) -> list[str]:
    """Forms the Go CLI may print for a path (ms.HumanPath abbreviates $HOME)."""
    forms = [path]
    home = os.path.expanduser("~")
    if home and path.startswith(home + "/"):
        forms.insert(0, "~" + path[len(home):])
    return forms


def main() -> None:
    work = fixture_recipe.workdir()
    upstream = Path(os.environ["D2RS_UPSTREAM"]).resolve()
    sources = json.loads((ROOT / "tools/fixtures/inputs/renderexport/ascii-callers-sources.json").read_text())
    rows = []
    for engines in (ENGINES[:3], ENGINES[3:]):
        for src in sources:
            for engine in engines:
                for mode in MODES:
                    for scale in SCALES:
                        rows.append({"name": f"{src['name']}-{engine or 'auto'}-{mode}-{scale}", "source": src["source"],
                                     "engine": engine, "mode": mode, "scale": scale})
    requests = []
    for row in rows:
        options = {"ascii": True, "asciiMode": row["mode"]}
        if row["engine"]:
            options["layout"] = row["engine"]
        if row["scale"] is not None:
            options["scale"] = row["scale"]
        requests.append({"source": row["source"], "options": options})
    (work / "requests.json").write_text(json.dumps(requests))
    subprocess.run(["node", str(ROOT / "tools/d2js-capture/ascii-callers.mjs"), str(work / "requests.json"),
                    str(work / "outputs.json")], check=True, timeout=1200)
    outputs = json.loads((work / "outputs.json").read_text())
    assert len(outputs) == len(rows)

    d2 = work / "d2"
    subprocess.run(["go", "build", "-mod=readonly", "-o", str(d2), "."], cwd=upstream,
                   env=fixture_recipe.go_env(), check=True, timeout=1200)
    cases = work / "rows"
    result = []
    for index, (row, js) in enumerate(zip(rows, outputs)):
        assert "output" in js, f"{row['name']}: d2js failed: {js}"
        directory = cases / str(index)
        directory.mkdir(parents=True)
        source, output = directory / "input.d2", directory / "output.txt"
        source.write_text(row["source"])
        argv = [str(d2), f"--ascii-mode={row['mode']}"]
        if row["engine"]:
            argv.append(f"--layout={row['engine']}")
        if row["scale"] is not None:
            argv.append(f"--scale={row['scale']}")
        env = {k: v for k, v in fixture_recipe.go_env().items() if not k.startswith("D2_")}
        cli = subprocess.run([*argv, str(source), str(output)], cwd=directory, env=env,
                             capture_output=True, text=True, timeout=600)
        record = {**row, "output": js["output"],
                  "source_sha256": hashlib.sha256(row["source"].encode()).hexdigest()}
        if cli.returncode == 0 and output.is_file():
            record["cli_kind"] = "text"
            assert output.read_text() == js["output"], f"{row['name']}: CLI and d2js ASCII differ"
        elif cli.returncode == 0:
            record["cli_kind"] = "no-output"
            tree = directory / "output"
            if tree.is_dir():  # multiboard: the CLI writes the board tree instead of output.txt
                record["cli_files"] = {p.relative_to(directory).as_posix(): p.read_text()
                                       for p in sorted(tree.rglob("*")) if p.is_file()}
        else:
            template = cli.stderr
            for form in human_paths(str(source)):
                template = template.replace(form, "{input}")
            for form in human_paths(str(directory)):
                template = template.replace(form, "{directory}")
            record["cli_kind"] = "error"
            record["cli_error_template"] = template
        result.append(record)
    shutil.rmtree(cases)
    fixture_recipe.output(LOGICAL).write_text(json.dumps(result, indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
