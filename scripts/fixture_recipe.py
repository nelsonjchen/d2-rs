"""Helpers for `kind = "command"` fixture generators run by regen-fixtures.py.

A generator reads the D2RS_RECIPE_* environment set by the runner and writes
only to its staged outputs and its workdir (docs/FIXTURES.md).
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import sys


def _env(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        sys.exit(f"{name} is unset: run this generator through "
                 "`uv run --no-project python scripts/regen-fixtures.py --only <recipe or output>`")
    return value


def output(logical: str) -> Path:
    """Staged path for one of the recipe's declared `outputs`."""
    stage = Path(_env("D2RS_RECIPE_STAGE"))
    staged = [Path(p) for p in _env("D2RS_RECIPE_OUTPUTS").split(os.pathsep)]
    path = stage / logical
    if path not in staged:
        sys.exit(f"{logical} is not a declared output of recipe {os.environ.get('D2RS_RECIPE_NAME')}")
    path.parent.mkdir(parents=True, exist_ok=True)
    return path


def workdir() -> Path:
    """Disposable per-recipe scratch directory (removed after the run)."""
    return Path(_env("D2RS_RECIPE_WORKDIR"))


def go_env() -> dict[str, str]:
    """Process environment with the runner's absolute GOMODCACHE/GOCACHE/GOTOOLCHAIN."""
    for name in ("GOMODCACHE", "GOCACHE", "GOTOOLCHAIN"):
        _env(name)
    return dict(os.environ)


def write_request_fixture(logical: str, requests, results, **extra) -> Path:
    """The `{"requests": [...], "results": [...]}` layout used by oracle tests.

    `extra` adds further top-level fields (ignored by tests that do not read them).
    """
    if len(results) != len(requests):
        sys.exit(f"{logical}: {len(results)} oracle results for {len(requests)} requests")
    path = output(logical)
    path.write_text(json.dumps({"requests": requests, "results": results, **extra}, separators=(",", ":")) + "\n")
    return path


def cached_fixture(logical: str) -> Path:
    """Path of a regenerated artifact for Python readers (oracle tier).

    Resolves ``$D2RS_FIXTURES_DIR`` (relative to the repo root) or
    ``oracle-cache/fixtures``. A missing artifact or stamp raises with the
    regeneration command; nothing is skipped (docs/FIXTURES.md).
    """
    root = Path(__file__).resolve().parents[1]
    configured = os.environ.get("D2RS_FIXTURES_DIR")
    base = (root / configured) if configured else root / "oracle-cache/fixtures"
    path = base / logical
    stamp = path.with_name(path.name + ".stamp.json")
    if not path.is_file() or not stamp.is_file():
        raise FileNotFoundError(
            f"oracle fixture `{logical}` is missing from {base}; regenerate it with:\n"
            f"    uv run --no-project python scripts/regen-fixtures.py --only {logical}")
    return path
