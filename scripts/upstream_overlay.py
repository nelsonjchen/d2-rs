"""Run a trace-checkout capture against the runner's upstream tree, read-only.

Historical scoreboard captures (``capture_original`` in
scripts/isometric-*-scoreboard.py, svg-admission-scoreboard.py, ...) rewrite
upstream test files in place in ``references.TRACE_CHECKOUT``, drop hook files
next to them, run ``go test``, and restore everything afterwards. The fixture
runner's patched upstream tree is shared by all recipes of a run and must not
be written (docs/FIXTURES.md), so :func:`redirect` makes those captures write
a ``go -overlay`` instead:

* ``references.TRACE_CHECKOUT`` and ``references.verify_source()`` both point
  at ``$D2RS_UPSTREAM`` (the pinned tree plus the manifest patch series);
* ``Path.write_text``/``write_bytes``/``unlink`` and ``shutil.copyfile`` on a
  path inside that tree record an overlay entry backed by a file in the
  recipe workdir (writing the file's original bytes back drops the entry);
  reads of an overlaid path see the overlay contents;
* every ``go build|test|run|vet|list`` subprocess gets ``-overlay=<json>`` for
  the entries live at that moment, and Go environment overrides that the
  historical captures hard-code (offline proxy, ``-mod=mod``, repo-relative
  caches) are replaced by the runner's absolute shared caches.

Nothing is written into the upstream tree by this module. Tests that write
their own files (e.g. ``.got`` outputs) still go to the disposable tree.
"""
from __future__ import annotations

import json
import os
import pathlib
import shutil
import subprocess
import sys

_GO_SUBCOMMANDS = {"build", "test", "run", "vet", "list"}
_ENV_FROM_RUNNER = ("GOMODCACHE", "GOCACHE", "GOTOOLCHAIN")


class Overlay:
    def __init__(self, upstream: pathlib.Path, store: pathlib.Path):
        self.upstream = upstream.resolve()
        self.store = store
        self.store.mkdir(parents=True, exist_ok=True)
        self.entries: dict[pathlib.Path, pathlib.Path] = {}
        self.serial = 0

    def owns(self, path) -> pathlib.Path | None:
        candidate = pathlib.Path(os.path.abspath(os.fspath(path)))
        try:
            candidate.relative_to(self.upstream)
        except ValueError:
            return None
        return candidate

    def write(self, target: pathlib.Path, data: bytes):
        if target.is_file() and _REAL["read_bytes"](target) == data:
            self.entries.pop(target, None)  # restoring the original
            return
        backing = self.store / target.relative_to(self.upstream)
        backing.parent.mkdir(parents=True, exist_ok=True)
        _REAL["write_bytes"](backing, data)
        self.entries[target] = backing

    def overlay_file(self) -> pathlib.Path:
        self.serial += 1
        path = self.store.parent / f"overlay-{self.serial}.json"
        _REAL["write_text"](path, json.dumps(
            {"Replace": {str(k): str(v) for k, v in sorted(self.entries.items())}}, indent=1))
        return path


_REAL = {
    "write_text": pathlib.Path.write_text,
    "write_bytes": pathlib.Path.write_bytes,
    "read_bytes": pathlib.Path.read_bytes,
    "read_text": pathlib.Path.read_text,
    "unlink": pathlib.Path.unlink,
    "exists": pathlib.Path.exists,
    "is_file": pathlib.Path.is_file,
    "copyfile": shutil.copyfile,
    "Popen": subprocess.Popen,
}


def redirect(store: pathlib.Path) -> pathlib.Path:
    """Install the redirection; returns the upstream tree. Call before loading scoreboards."""
    upstream_env = os.environ.get("D2RS_UPSTREAM")
    if not upstream_env:
        sys.exit("D2RS_UPSTREAM is unset: run through scripts/regen-fixtures.py (recipe with upstream = true)")
    upstream = pathlib.Path(upstream_env).resolve()
    overlay = Overlay(upstream, store)

    sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
    import references

    references.TRACE_CHECKOUT = upstream
    references.verify_source = lambda: upstream

    def write_text(self, data, encoding=None, errors=None, newline=None):
        target = overlay.owns(self)
        if target is None:
            return _REAL["write_text"](self, data, encoding=encoding, errors=errors, newline=newline)
        overlay.write(target, data.encode(encoding or "utf-8", errors or "strict"))
        return len(data)

    def write_bytes(self, data):
        target = overlay.owns(self)
        if target is None:
            return _REAL["write_bytes"](self, data)
        overlay.write(target, bytes(data))
        return len(data)

    def read_bytes(self):
        target = overlay.owns(self)
        if target is not None and target in overlay.entries:
            return _REAL["read_bytes"](overlay.entries[target])
        return _REAL["read_bytes"](self)

    def read_text(self, encoding=None, errors=None, **kwargs):
        target = overlay.owns(self)
        if target is not None and target in overlay.entries:
            return _REAL["read_text"](overlay.entries[target], encoding=encoding, errors=errors)
        return _REAL["read_text"](self, encoding=encoding, errors=errors, **kwargs)

    def unlink(self, missing_ok=False):
        target = overlay.owns(self)
        if target is None:
            return _REAL["unlink"](self, missing_ok=missing_ok)
        if target in overlay.entries:
            del overlay.entries[target]
            return None
        if missing_ok and not _REAL["exists"](target):
            return None
        raise PermissionError(f"refusing to delete {target} from the shared upstream tree")

    def exists(self, **kwargs):
        target = overlay.owns(self)
        if target is not None and target in overlay.entries:
            return True
        return _REAL["exists"](self, **kwargs)

    def is_file(self, **kwargs):
        target = overlay.owns(self)
        if target is not None and target in overlay.entries:
            return True
        return _REAL["is_file"](self, **kwargs)

    def copyfile(src, dst, *, follow_symlinks=True):
        target = overlay.owns(dst)
        if target is None:
            return _REAL["copyfile"](src, dst, follow_symlinks=follow_symlinks)
        overlay.write(target, pathlib.Path(src).read_bytes())
        return dst

    class Popen(_REAL["Popen"]):
        def __init__(self, args, *rest, **kwargs):
            argv = list(args) if isinstance(args, (list, tuple)) else args
            if isinstance(argv, list) and len(argv) > 1 and os.path.basename(str(argv[0])) == "go" \
                    and argv[1] in _GO_SUBCOMMANDS:
                argv = [argv[0], argv[1], f"-overlay={overlay.overlay_file()}", *argv[2:]]
                env = dict(kwargs.get("env") or os.environ)
                for name in _ENV_FROM_RUNNER:
                    if os.environ.get(name):
                        env[name] = os.environ[name]
                env.pop("GOPROXY", None)
                if os.environ.get("GOPROXY"):
                    env["GOPROXY"] = os.environ["GOPROXY"]
                env["GOFLAGS"] = "-mod=readonly -buildvcs=false"
                kwargs["env"] = env
                if kwargs.get("cwd") is not None and overlay.owns(kwargs["cwd"]) is None:
                    kwargs["cwd"] = str(upstream)
            super().__init__(argv, *rest, **kwargs)

    pathlib.Path.write_text = write_text
    pathlib.Path.write_bytes = write_bytes
    pathlib.Path.read_bytes = read_bytes
    pathlib.Path.read_text = read_text
    pathlib.Path.unlink = unlink
    pathlib.Path.exists = exists
    pathlib.Path.is_file = is_file
    shutil.copyfile = copyfile
    subprocess.Popen = Popen
    return upstream
