"""elk-go's own curated layout inputs (testdata/curated/*.json in the pinned module).

Several qualification matrices start from these graphs. They are read from
the pinned module, so they key on its version like the oracle output does.
$D2RS_ELK_GO_DIR overrides the module directory; otherwise it is located with
`go mod download -json` under the runner's GOMODCACHE.
"""
import functools
import json
import os
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from references import MANIFEST  # noqa: E402

MODULE = "github.com/d2lang/elk-go"


@functools.cache
def module_dir() -> Path:
    if os.environ.get("D2RS_ELK_GO_DIR"):
        return Path(os.environ["D2RS_ELK_GO_DIR"])
    version = MANIFEST["dependencies"][MODULE]
    result = subprocess.run(["go", "mod", "download", "-json", f"{MODULE}@{version}"], stdout=subprocess.PIPE,
                            check=True, cwd=os.environ.get("D2RS_RECIPE_WORKDIR"))
    return Path(json.loads(result.stdout)["Dir"])


def load(name: str):
    """A fresh copy of testdata/curated/<name>.json, e.g. load('09_compound')."""
    return json.loads((module_dir() / "testdata/curated" / f"{name}.json").read_text())


def _single(fixture_name: str, curated_name: str):
    return [dict(name=fixture_name, input=load(curated_name))]


# Single-row complete-raw-output controls (model_order_contract.rs): one curated
# graph each, named as recorded by the 2026-10-17 probes.
def owned_edge_compound():
    return _single("original09_compound", "09_compound")


def curated_cross_hierarchy():
    return _single("original11_cross_hierarchy", "11_cross_hierarchy")


def curated_component_direction():
    return _single("original12_force_model_order", "12_force_model_order")


def curated_isomorphic():
    return _single("original13_moon_issue8_isomorphic", "13_moon_issue8_isomorphic")


def curated_single_container():
    return _single("original14_moon_issue9_single_container", "14_moon_issue9_single_container")


def curated_coffman():
    return _single("15_coffman_graham", "15_coffman_graham")
