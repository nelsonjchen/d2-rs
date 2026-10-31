#!/usr/bin/env python3
"""Regenerate Go-oracle test artifacts into the uncommitted fixture cache.

Recipes live in ``tools/fixtures/*.toml`` (contract: ``docs/FIXTURES.md``).
Each recipe produces one or more artifacts, written atomically to
``oracle-cache/fixtures/<logical path>`` with a ``<path>.stamp.json`` sidecar
holding the cache key. Recipes whose key is unchanged are skipped.

    uv run --no-project python scripts/regen-fixtures.py            # stale only
    uv run --no-project python scripts/regen-fixtures.py --list
    uv run --no-project python scripts/regen-fixtures.py --check    # no running
    uv run --no-project python scripts/regen-fixtures.py --only 'crates/d2-rs-text/*' --jobs 4
    uv run --no-project python scripts/regen-fixtures.py --goarch amd64   # Intel-parity captures

Oracle architecture (``--goarch``, default arm64): Go fuses x*y+z into FMA
instructions on arm64 but not on amd64, so float output differs per
architecture. Every Go build gets ``GOARCH``, the architecture is part of every
cache key, and a foreign architecture must run transparently (binfmt/qemu on
Linux, Rosetta on macOS). See "Host architecture" in docs/FIXTURES.md.

Stdlib only.
"""
from __future__ import annotations

import argparse
import concurrent.futures
import datetime
import fnmatch
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import subprocess
import sys
import threading
import time
import tomllib

SCHEMA = 1
MIB = 1 << 20
GIB = 1 << 30
DEFAULT_TIMEOUT = 1800
POLL_SECONDS = 0.25
GOARCHES = ("arm64", "amd64")
DEFAULT_GOARCH = "arm64"
NAME_RE = re.compile(r"^[a-z0-9][a-z0-9._-]*$")
PLACEHOLDER_RE = re.compile(r"\{(repo|upstream|workdir|fixtures|stage|output|input|python|name)\}")


def is_repo_file(root: Path, arg: str) -> bool:
    """Whether an argv entry names a repo file (long non-path arguments are not)."""
    try:
        return (root / arg).is_file()
    except OSError:  # e.g. ENAMETOOLONG for a long comma-separated argument
        return False

COMMON_FIELDS = {"name", "kind", "description", "outputs", "inputs", "after", "env",
                 "timeout", "key_deps", "cwd", "upstream"}
KIND_FIELDS = {
    "command": ({"argv"}, {"stdout"}),
    "go-program": ({"program"}, {"module", "overlay", "go_flags", "args", "stdin", "stdout"}),
    "go-test-hook": ({"package", "hooks", "test"}, {"overlay", "go_flags", "test_args"}),
}
KEY_DEP_FIELDS = {"files", "upstream_paths", "go_modules", "go_toolchain", "extra"}


class RecipeError(Exception):
    """Invalid recipe or environment; reported without a traceback."""


class StepFailed(Exception):
    """A recipe step failed (nonzero exit, timeout, budget, missing output)."""


# --------------------------------------------------------------------------
# Paths and environment


def host_goarch() -> str:
    machine = platform.machine().lower()
    return {"x86_64": "amd64", "amd64": "amd64", "aarch64": "arm64", "arm64": "arm64"}.get(machine, machine)


def git(*args: str, cwd: Path, env: dict | None = None) -> str:
    return subprocess.run(["git", *args], cwd=cwd, env=env, check=True, text=True,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout.strip()


def main_checkout(root: Path) -> Path:
    """The primary checkout that owns shared caches and the sibling ../d2."""
    try:
        common = git("rev-parse", "--path-format=absolute", "--git-common-dir", cwd=root)
    except (subprocess.CalledProcessError, FileNotFoundError):
        return root
    common_path = Path(common)
    return common_path.parent if common_path.name == ".git" else root


class Paths:
    def __init__(self, root: Path, *, fixtures: Path | None = None, main: Path | None = None):
        self.root = root.resolve()
        self.main = (main or main_checkout(self.root)).resolve()
        self.manifest = json.loads((self.root / "reference-manifest.json").read_text())
        self.recipes = self.root / "tools/fixtures"
        env_dir = os.environ.get("D2RS_FIXTURES_DIR")
        self.fixtures = (fixtures or (self.root / env_dir if env_dir else self.root / "oracle-cache/fixtures")).resolve()
        # Visible top-level cache (no hidden dirs): fixtures/, upstream-<key>/, stage/, work/, logs/.
        self.regen = self.root / "oracle-cache"
        shared = self.main / ".reference"
        # AGENTS.md: absolute, shared GOMODCACHE/GOCACHE (never relative).
        self.gomodcache = Path(os.environ.get("D2RS_GOMODCACHE", shared / "go-mod/pkg/mod")).resolve()
        self.gocache = Path(os.environ.get("D2RS_GOCACHE", shared / "go-build")).resolve()
        self.upstream_repo = (self.main / self.manifest["d2"]["checkout"]).resolve()
        self.pin = self.manifest["d2"]["commit"]
        self.go_toolchain = self.manifest["d2"]["go_toolchain"]
        self.patches = [self.root / p for p in self.manifest["d2"].get("trace_patches", [])]
        self.goarch = os.environ.get("D2RS_GOARCH") or DEFAULT_GOARCH


def sha256_file(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def sha256_json(value) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def repo_rel(paths: Paths, rel: str, what: str) -> Path:
    candidate = Path(rel)
    if candidate.is_absolute() or ".." in candidate.parts:
        raise RecipeError(f"{what}: `{rel}` must be repository-relative without `..`")
    return paths.root / candidate


def hash_tree(paths: Paths, pattern: str, what: str) -> dict[str, str]:
    """Content hashes of the repo files matched by a path, directory or glob."""
    matches = sorted(paths.root.glob(pattern)) if any(c in pattern for c in "*?[") else [repo_rel(paths, pattern, what)]
    out: dict[str, str] = {}
    for match in matches:
        if match.is_dir():
            files = [p for p in sorted(match.rglob("*")) if p.is_file() and "__pycache__" not in p.parts]
        elif match.is_file():
            files = [match]
        else:
            files = []
        for f in files:
            out[f.relative_to(paths.root).as_posix()] = sha256_file(f)
    if not out:
        raise RecipeError(f"{what}: `{pattern}` matches no files")
    return out


# --------------------------------------------------------------------------
# Recipes


def merge_defaults(defaults: dict, recipe: dict) -> dict:
    merged = dict(defaults)
    for key, value in recipe.items():
        base = merged.get(key)
        if key in ("env", "key_deps") and isinstance(base, dict) and isinstance(value, dict):
            combined = dict(base)
            for k, v in value.items():
                combined[k] = (base.get(k, []) + v) if isinstance(v, list) and isinstance(base.get(k), list) else v
            merged[key] = combined
        else:
            merged[key] = value
    return merged


def validate(recipe: dict, source: Path) -> dict:
    where = f"{source.name}:{recipe.get('name', '?')}"
    name = recipe.get("name")
    if not isinstance(name, str) or not NAME_RE.match(name):
        raise RecipeError(f"{where}: `name` must match {NAME_RE.pattern}")
    kind = recipe.get("kind")
    if kind not in KIND_FIELDS:
        raise RecipeError(f"{where}: `kind` must be one of {sorted(KIND_FIELDS)}")
    required, optional = KIND_FIELDS[kind]
    unknown = set(recipe) - COMMON_FIELDS - required - optional
    if unknown:
        raise RecipeError(f"{where}: unknown fields {sorted(unknown)} for kind {kind}")
    missing = required - set(recipe)
    if missing:
        raise RecipeError(f"{where}: missing fields {sorted(missing)}")
    outputs = recipe.get("outputs")
    if not isinstance(outputs, list) or not outputs or not all(isinstance(o, str) for o in outputs):
        raise RecipeError(f"{where}: `outputs` must be a nonempty list of logical paths")
    for out in outputs:
        p = Path(out)
        if p.is_absolute() or ".." in p.parts or out.endswith(".stamp.json") or p.as_posix() != out:
            raise RecipeError(f"{where}: bad output path `{out}`")
    if len(set(outputs)) != len(outputs):
        raise RecipeError(f"{where}: duplicate outputs")
    if "stdout" in recipe and recipe["stdout"] not in outputs:
        raise RecipeError(f"{where}: `stdout` must name one of `outputs`")
    unknown_deps = set(recipe.get("key_deps", {})) - KEY_DEP_FIELDS
    if unknown_deps:
        raise RecipeError(f"{where}: unknown key_deps fields {sorted(unknown_deps)}")
    for field in ("argv", "hooks", "go_flags", "args", "test_args", "after"):
        if field in recipe and not (isinstance(recipe[field], list) and all(isinstance(x, str) for x in recipe[field])):
            raise RecipeError(f"{where}: `{field}` must be a list of strings")
    if recipe.get("cwd", "repo") not in ("repo", "workdir", "upstream"):
        raise RecipeError(f"{where}: `cwd` must be repo, workdir or upstream")
    for item in recipe.get("inputs", []):
        if not (isinstance(item, str) or (isinstance(item, dict) and set(item) <= {"path", "generate"}
                                          and isinstance(item.get("path"), str) and isinstance(item.get("generate"), list))):
            raise RecipeError(f"{where}: each input is a repo path or {{path, generate}}")
    recipe["_source"] = source
    return recipe


def uses_upstream(recipe: dict) -> bool:
    if "upstream" in recipe:
        return bool(recipe["upstream"])
    if recipe["kind"] == "go-test-hook":
        return True
    if recipe["kind"] == "go-program":
        return recipe.get("module", "upstream") == "upstream"
    return False


def load_recipes(paths: Paths) -> dict[str, dict]:
    recipes: dict[str, dict] = {}
    owners: dict[str, str] = {}
    for source in sorted(paths.recipes.glob("*.toml")):
        data = tomllib.loads(source.read_text())
        unknown = set(data) - {"defaults", "recipe"}
        if unknown:
            raise RecipeError(f"{source.name}: unknown top-level tables {sorted(unknown)}")
        for raw in data.get("recipe", []):
            recipe = validate(merge_defaults(data.get("defaults", {}), raw), source)
            if recipe["name"] in recipes:
                raise RecipeError(f"duplicate recipe name {recipe['name']}")
            for out in recipe["outputs"]:
                if out in owners:
                    raise RecipeError(f"output {out} claimed by {owners[out]} and {recipe['name']}")
                owners[out] = recipe["name"]
            recipes[recipe["name"]] = recipe
    for recipe in recipes.values():
        for dep in recipe.get("after", []):
            if dep not in recipes:
                raise RecipeError(f"{recipe['name']}: unknown `after` recipe {dep}")
    topo_order(recipes)
    return recipes


def topo_order(recipes: dict[str, dict]) -> list[str]:
    order: list[str] = []
    state: dict[str, int] = {}

    def visit(name: str, chain: list[str]):
        if state.get(name) == 2:
            return
        if state.get(name) == 1:
            raise RecipeError("recipe cycle: " + " -> ".join(chain + [name]))
        state[name] = 1
        for dep in recipes[name].get("after", []):
            visit(dep, chain + [name])
        state[name] = 2
        order.append(name)

    for name in sorted(recipes):
        visit(name, [])
    return order


# --------------------------------------------------------------------------
# Cache keys


class Keys:
    def __init__(self, paths: Paths, recipes: dict[str, dict]):
        self.paths = paths
        self.recipes = recipes
        self.cache: dict[str, tuple[str, dict]] = {}
        self.tree_cache: dict[str, str] = {}

    def upstream_series(self) -> dict:
        return {"commit": self.paths.pin,
                "patches": [{"path": p.relative_to(self.paths.root).as_posix(), "sha256": sha256_file(p)}
                            for p in self.paths.patches]}

    def tree_hash(self, rel: str) -> str:
        if rel not in self.tree_cache:
            spec = f"{self.paths.pin}^{{tree}}" if rel in (".", "") else f"{self.paths.pin}:{rel}"
            try:
                self.tree_cache[rel] = git("rev-parse", "--verify", spec, cwd=self.paths.upstream_repo)
            except (subprocess.CalledProcessError, FileNotFoundError) as error:
                raise RecipeError(f"cannot resolve upstream path `{rel}` at {self.paths.pin} in "
                                  f"{self.paths.upstream_repo}: {error}") from error
        return self.tree_cache[rel]

    def auto_files(self, recipe: dict) -> list[str]:
        files: list[str] = []
        files += [i for i in recipe.get("inputs", []) if isinstance(i, str)]
        for item in recipe.get("inputs", []):
            if isinstance(item, dict):
                files += [a for a in item["generate"] if is_repo_file(self.paths.root, a)]
        files += recipe.get("hooks", [])
        files += list(recipe.get("overlay", {}).values())
        if recipe["kind"] == "command":
            files += [a for a in recipe["argv"] if not PLACEHOLDER_RE.search(a)
                      and not Path(a).is_absolute() and is_repo_file(self.paths.root, a)]
        if recipe["kind"] == "go-program":
            files.append(recipe["program"])
            module = recipe.get("module", "upstream")
            if module == "upstream":
                files += ["tools/oracle/go.mod", "tools/oracle/go.sum"]
            else:
                files += [f"{module}/go.mod", f"{module}/go.sum"]
        return files

    def key(self, name: str) -> tuple[str, dict]:
        if name in self.cache:
            return self.cache[name]
        recipe = self.recipes[name]
        deps = recipe.get("key_deps", {})
        where = f"{name}"
        normalized = {k: v for k, v in recipe.items() if not k.startswith("_") and k != "description"}
        files: dict[str, str] = {}
        for pattern in self.auto_files(recipe) + deps.get("files", []):
            files.update(hash_tree(self.paths, pattern, where))
        # Go output differs per architecture (FMA fusion), so every key names it.
        material: dict = {"schema": SCHEMA, "recipe": normalized, "files": files, "goarch": self.paths.goarch}
        is_go = recipe["kind"] in ("go-program", "go-test-hook")
        if is_go or deps.get("go_toolchain"):
            material["go_toolchain"] = self.paths.go_toolchain
        modules = {}
        for module in deps.get("go_modules", []):
            if "@" in module:
                path, version = module.split("@", 1)
            else:
                path, version = module, self.paths.manifest.get("dependencies", {}).get(module)
                if version is None:
                    raise RecipeError(f"{name}: go module {module} has no version in reference-manifest.json")
            modules[path] = version
        if modules:
            material["go_modules"] = modules
        upstream_paths = deps.get("upstream_paths")
        if uses_upstream(recipe):
            material["upstream"] = self.upstream_series()
            upstream_paths = upstream_paths or ["."]
        if upstream_paths:
            material["upstream_trees"] = {p: self.tree_hash(p) for p in upstream_paths}
            if "upstream" not in material:
                material["upstream_commit"] = self.paths.pin
        if "extra" in deps:
            material["extra"] = deps["extra"]
        material["after"] = {dep: self.key(dep)[0] for dep in recipe.get("after", [])}
        result = (sha256_json(material), material)
        self.cache[name] = result
        return result


def stamp_path(artifact: Path) -> Path:
    return artifact.with_name(artifact.name + ".stamp.json")


def artifact_status(paths: Paths, logical: str, key: str) -> str:
    """fresh | missing | stale | corrupt for one output."""
    artifact = paths.fixtures / logical
    stamp = stamp_path(artifact)
    if not artifact.is_file():
        return "missing"
    try:
        record = json.loads(stamp.read_text())
    except (OSError, ValueError):
        return "stale"
    if record.get("key") != key or record.get("logical_path") != logical:
        return "stale"
    if record.get("size") != artifact.stat().st_size or record.get("sha256") != sha256_file(artifact):
        return "corrupt"
    return "fresh"


def recipe_status(paths: Paths, recipe: dict, key: str) -> str:
    states = [artifact_status(paths, o, key) for o in recipe["outputs"]]
    for state in ("corrupt", "missing", "stale"):
        if state in states:
            return state
    return "fresh"


# --------------------------------------------------------------------------
# Budget and process supervision


def tree_size(root: Path) -> tuple[int, Path | None, int]:
    """(total bytes, largest file, largest size) under root."""
    total, largest, largest_size = 0, None, 0
    for dirpath, _dirs, names in os.walk(root):
        for n in names:
            p = Path(dirpath) / n
            try:
                size = p.lstat().st_size
            except OSError:
                continue
            total += size
            if size > largest_size:
                largest, largest_size = p, size
    return total, largest, largest_size


class Budget:
    """Run-wide output budget: staged outputs and logs of every recipe."""

    def __init__(self, total: int, per_file: int, min_free: int, volume: Path):
        self.total, self.per_file, self.min_free, self.volume = total, per_file, min_free, volume
        self.lock = threading.Lock()
        self.committed = 0
        self.live: dict[str, int] = {}

    def check(self, name: str, stage: Path) -> str | None:
        size, largest, largest_size = tree_size(stage)
        with self.lock:
            self.live[name] = size
            used = self.committed + sum(self.live.values())
        if largest_size > self.per_file:
            return f"budget exceeded: {largest} is {largest_size} bytes > per-file limit {self.per_file}"
        if used > self.total:
            return f"budget exceeded: run total {used} bytes > limit {self.total}"
        free = shutil.disk_usage(self.volume).free
        if free < self.min_free:
            return f"free space {free} bytes below required {self.min_free}"
        return None

    def settle(self, name: str):
        with self.lock:
            self.committed += self.live.pop(name, 0)


def run_step(argv: list[str], *, cwd: Path, env: dict, stdin: Path | None, stdout: Path, log: Path,
             deadline: float, budget: Budget, name: str, stage: Path):
    """Run one process under the timeout, budget and free-space guards."""
    log.parent.mkdir(parents=True, exist_ok=True)
    stdout.parent.mkdir(parents=True, exist_ok=True)
    with log.open("ab") as log_stream:
        log_stream.write(("$ " + " ".join(argv) + f"\n  (cwd {cwd})\n").encode())
        log_stream.flush()
        out_stream = log_stream if stdout == log else stdout.open("wb")
        in_stream = stdin.open("rb") if stdin else subprocess.DEVNULL
        try:
            try:
                process = subprocess.Popen(argv, cwd=cwd, env=env, stdin=in_stream, stdout=out_stream,
                                           stderr=log_stream, start_new_session=True)
            except OSError as error:
                raise StepFailed(f"cannot start {argv[0]}: {error}") from error
            problem = None
            while True:
                try:
                    code = process.wait(timeout=POLL_SECONDS)
                    break
                except subprocess.TimeoutExpired:
                    pass
                problem = budget.check(name, stage)
                if problem is None and time.monotonic() > deadline:
                    problem = "timed out"
                if problem:
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    process.wait()
                    break
            problem = problem or budget.check(name, stage)
        finally:
            if out_stream is not log_stream:
                out_stream.close()
            if stdin:
                in_stream.close()
    if problem:
        raise StepFailed(f"{argv[0]}: {problem}")
    if code != 0:
        raise StepFailed(f"{argv[0]} exited with status {code}")


# --------------------------------------------------------------------------
# Disposable upstream checkout


class Upstream:
    """One patched upstream tree per pin+patch-series hash, shared by recipes."""

    def __init__(self, paths: Paths, keys: Keys):
        self.paths, self.keys = paths, keys
        self.lock = threading.Lock()
        self.ready: Path | None = None
        self.series_key = sha256_json(keys.upstream_series())
        self.dir = paths.regen / f"upstream-{self.series_key[:16]}"

    def marker(self) -> Path:
        return self.dir / ".d2rs-upstream.json"

    def verify(self) -> bool:
        try:
            record = json.loads(self.marker().read_text())
        except (OSError, ValueError):
            return False
        if record.get("series_key") != self.series_key:
            return False
        return all((self.dir / p).is_file() and sha256_file(self.dir / p) == h
                   for p, h in record.get("patched_files", {}).items())

    def get(self) -> Path:
        with self.lock:
            if self.ready:
                return self.ready
            if self.dir.exists() and self.verify():
                self.ready = self.dir
                return self.dir
            rmtree(self.dir)
            partial = self.dir.with_name(self.dir.name + ".partial")
            rmtree(partial)
            partial.mkdir(parents=True)
            archive = subprocess.Popen(["git", "-C", str(self.paths.upstream_repo), "archive", "--format=tar",
                                        self.paths.pin], stdout=subprocess.PIPE)
            extract = subprocess.run(["tar", "-x", "-C", str(partial)], stdin=archive.stdout)
            archive.stdout.close()
            if archive.wait() != 0 or extract.returncode != 0:
                rmtree(partial)
                raise RecipeError(f"cannot export {self.paths.pin} from {self.paths.upstream_repo}")
            # Keep `git apply` from discovering the enclosing d2-rs repository.
            env = {**os.environ, "GIT_CEILING_DIRECTORIES": str(partial.parent)}
            patched: set[str] = set()
            for patch in self.paths.patches:
                for args in (["--check"], []):
                    result = subprocess.run(["git", "apply", *args, str(patch)], cwd=partial, env=env,
                                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
                    if result.returncode != 0:
                        rmtree(partial)
                        raise RecipeError(f"patch {patch} does not apply to {self.paths.pin}:\n{result.stdout}")
                numstat = subprocess.run(["git", "apply", "--numstat", str(patch)], cwd=partial, env=env,
                                         check=True, stdout=subprocess.PIPE, text=True).stdout
                patched.update(line.split("\t", 2)[2] for line in numstat.splitlines() if line.count("\t") >= 2)
            record = {"series_key": self.series_key, **self.keys.upstream_series(),
                      "patched_files": {p: sha256_file(partial / p) for p in sorted(patched) if (partial / p).is_file()}}
            (partial / ".d2rs-upstream.json").write_text(json.dumps(record, indent=2) + "\n")
            partial.rename(self.dir)
            self.ready = self.dir
            return self.dir


def rmtree(path: Path):
    """Remove a disposable tree, including read-only Go module copies."""
    if not path.exists() and not path.is_symlink():
        return

    def make_writable(func, target, _exc):
        os.chmod(target, 0o700)
        parent = os.path.dirname(target)
        os.chmod(parent, 0o700)
        func(target)

    if path.is_dir() and not path.is_symlink():
        shutil.rmtree(path, onexc=make_writable)
    else:
        path.unlink()


# --------------------------------------------------------------------------
# Running recipes


class Runner:
    def __init__(self, paths: Paths, recipes: dict[str, dict], budget: Budget, *, keep: bool,
                 verbose: bool = False, keys: Keys | None = None):
        self.paths, self.recipes, self.budget = paths, recipes, budget
        self.keep, self.verbose = keep, verbose
        self.keys = keys or Keys(paths, recipes)
        self.upstream = Upstream(paths, self.keys)
        self.print_lock = threading.Lock()

    def say(self, message: str):
        with self.print_lock:
            print(message, flush=True)

    def expand(self, value: str, ctx: dict) -> str:
        return PLACEHOLDER_RE.sub(lambda m: str(ctx[m.group(1)]) if ctx.get(m.group(1)) is not None
                                  else self._missing(m.group(1)), value)

    @staticmethod
    def _missing(name: str) -> str:
        raise StepFailed(f"placeholder {{{name}}} is unavailable for this recipe")

    def run(self, name: str):
        recipe = self.recipes[name]
        key, material = self.keys.key(name)
        stage = self.paths.regen / "stage" / name
        work = self.paths.regen / "work" / name
        log = stage / ".logs" / f"{name}.log"
        kept_log = self.paths.regen / "logs" / f"{name}.log"
        for d in (stage, work):
            rmtree(d)
            d.mkdir(parents=True)
        deadline = time.monotonic() + float(recipe.get("timeout", DEFAULT_TIMEOUT))
        started = time.monotonic()
        try:
            upstream = self.upstream.get() if uses_upstream(recipe) else None
            staged = [stage / o for o in recipe["outputs"]]
            for s in staged:
                s.parent.mkdir(parents=True, exist_ok=True)
            ctx = {"repo": self.paths.root, "upstream": upstream, "workdir": work, "fixtures": self.paths.fixtures,
                   "stage": stage, "output": staged[0], "python": sys.executable, "name": name, "input": None}
            env = self.environment(recipe, ctx, staged, upstream)
            inputs = self.prepare_inputs(recipe, ctx, env, work, log, deadline, stage)
            ctx["input"] = inputs[0] if inputs else None
            env["D2RS_RECIPE_INPUT"] = str(inputs[0]) if inputs else ""
            env["D2RS_RECIPE_INPUTS"] = os.pathsep.join(map(str, inputs))
            for k, v in recipe.get("env", {}).items():
                env[k] = self.expand(str(v), ctx)
            cwd = {"repo": self.paths.root, "workdir": work, "upstream": upstream}[recipe.get("cwd", "repo")]
            if cwd is None:
                raise StepFailed("cwd = upstream requires an upstream recipe")
            common = dict(env=env, log=log, deadline=deadline, budget=self.budget, name=name, stage=stage)
            stdout = stage / recipe["stdout"] if recipe.get("stdout") else log
            kind = recipe["kind"]
            if kind == "command":
                run_step([self.expand(a, ctx) for a in recipe["argv"]], cwd=cwd, stdin=None, stdout=stdout, **common)
            elif kind == "go-program":
                binary = self.build_program(recipe, ctx, work, upstream, common)
                args = [self.expand(a, ctx) for a in recipe.get("args", [])]
                stdin = Path(self.expand(recipe["stdin"], ctx)) if recipe.get("stdin") else None
                run_step([str(binary), *args], cwd=cwd if recipe.get("cwd") else work, stdin=stdin,
                         stdout=stdout, **common)
            else:
                package = recipe["package"].strip("/")
                overlay = {str(upstream / package / Path(h).name): str(repo_rel(self.paths, h, name))
                           for h in recipe["hooks"]}
                overlay.update(self.overlay_entries(recipe, upstream))
                overlay_file = work / "overlay.json"
                overlay_file.write_text(json.dumps({"Replace": overlay}, indent=2))
                argv = ["go", "test", "-mod=readonly", "-count=1", "-overlay", str(overlay_file),
                        *[self.expand(a, ctx) for a in recipe.get("go_flags", [])],
                        "-run", f"^{recipe['test']}$", f"./{package}"]
                test_args = [self.expand(a, ctx) for a in recipe.get("test_args", [])]
                if test_args:
                    argv += ["-args", *test_args]
                run_step(argv, cwd=upstream, stdin=None, stdout=stdout, **common)
            self.publish(recipe, key, material, staged)
        except (StepFailed, RecipeError) as error:
            self.budget.settle(name)
            kept_log.parent.mkdir(parents=True, exist_ok=True)
            if log.exists():
                shutil.copyfile(log, kept_log)
            tail = log.read_text(errors="replace").splitlines()[-40:] if log.exists() else []
            self.say(f"FAIL {name}: {error}\n  log: {kept_log}\n" + "\n".join("  | " + t for t in tail))
            if not self.keep:
                rmtree(stage)
                rmtree(work)
            return False
        self.budget.settle(name)
        if self.keep:
            kept_log.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(log, kept_log)
        else:
            rmtree(stage)
            rmtree(work)
            if kept_log.exists():
                kept_log.unlink()
        self.say(f"ok   {name} ({time.monotonic() - started:.1f}s, key {key[:12]})")
        return True

    def environment(self, recipe: dict, ctx: dict, staged: list[Path], upstream: Path | None) -> dict:
        self.paths.gomodcache.mkdir(parents=True, exist_ok=True)
        self.paths.gocache.mkdir(parents=True, exist_ok=True)
        env = dict(os.environ)
        env.update({
            "GOMODCACHE": str(self.paths.gomodcache),
            "GOCACHE": str(self.paths.gocache),
            "GOTOOLCHAIN": self.paths.go_toolchain,
            "GOFLAGS": "-buildvcs=false",
            "GOARCH": self.paths.goarch,
            "D2RS_GOARCH": self.paths.goarch,
            "D2RS_RECIPE_NAME": recipe["name"],
            "D2RS_RECIPE_OUTPUT": str(staged[0]),
            "D2RS_RECIPE_OUTPUTS": os.pathsep.join(map(str, staged)),
            "D2RS_RECIPE_STAGE": str(ctx["stage"]),
            "D2RS_RECIPE_WORKDIR": str(ctx["workdir"]),
            "D2RS_REPO_ROOT": str(self.paths.root),
            "D2RS_FIXTURES_DIR": str(self.paths.fixtures),
            "D2RS_UPSTREAM": str(upstream) if upstream else "",
        })
        return env

    def prepare_inputs(self, recipe, ctx, env, work, log, deadline, stage) -> list[Path]:
        inputs: list[Path] = []
        for item in recipe.get("inputs", []):
            if isinstance(item, str):
                path = repo_rel(self.paths, item, recipe["name"])
                if not path.is_file():
                    raise StepFailed(f"input {item} is missing")
            else:
                path = work / "inputs" / item["path"]
                path.parent.mkdir(parents=True, exist_ok=True)
                gen_ctx = {**ctx, "output": path}
                run_step([self.expand(a, gen_ctx) for a in item["generate"]], cwd=self.paths.root,
                         env={**env, "D2RS_RECIPE_OUTPUT": str(path)}, stdin=None, stdout=log, log=log,
                         deadline=deadline, budget=self.budget, name=recipe["name"], stage=stage)
                if not path.is_file():
                    raise StepFailed(f"input generator did not write {item['path']}")
            inputs.append(path)
        return inputs

    def overlay_entries(self, recipe: dict, upstream: Path | None) -> dict[str, str]:
        entries = {}
        for target, source in recipe.get("overlay", {}).items():
            if upstream is None:
                raise StepFailed("overlay targets are upstream-relative and need an upstream recipe")
            entries[str(upstream / target)] = str(repo_rel(self.paths, source, recipe["name"]))
        return entries

    def build_program(self, recipe, ctx, work, upstream, common) -> Path:
        program = repo_rel(self.paths, recipe["program"], recipe["name"])
        module_dir = work / "module"
        module_dir.mkdir()
        sources = [p for p in program.glob("*.go") if not p.name.endswith("_test.go")]
        if not sources:
            raise StepFailed(f"{recipe['program']} has no Go sources")
        for src in sources:
            shutil.copy2(src, module_dir / src.name)
        module = recipe.get("module", "upstream")
        if module == "upstream":
            go_mod = (self.paths.root / "tools/oracle/go.mod").read_text()
            go_mod, count = re.subn(r"(?m)^replace github\.com/d2lang/d2 => .*$",
                                    f"replace github.com/d2lang/d2 => {upstream}", go_mod)
            if count != 1:
                raise StepFailed("tools/oracle/go.mod has no d2 replace line")
            (module_dir / "go.mod").write_text(go_mod)
            shutil.copy2(self.paths.root / "tools/oracle/go.sum", module_dir / "go.sum")
        else:
            for f in ("go.mod", "go.sum"):
                shutil.copy2(repo_rel(self.paths, f"{module}/{f}", recipe["name"]), module_dir / f)
        binary = work / "bin" / recipe["name"]
        argv = ["go", "build", "-mod=readonly", *[self.expand(a, ctx) for a in recipe.get("go_flags", [])]]
        overlay = self.overlay_entries(recipe, upstream)
        if overlay:
            overlay_file = work / "overlay.json"
            overlay_file.write_text(json.dumps({"Replace": overlay}, indent=2))
            argv += ["-overlay", str(overlay_file)]
        run_step([*argv, "-o", str(binary), "."], cwd=module_dir, stdin=None, stdout=common["log"], **common)
        return binary

    def publish(self, recipe: dict, key: str, material: dict, staged: list[Path]):
        for logical, path in zip(recipe["outputs"], staged):
            if not path.is_file() or path.is_symlink():
                raise StepFailed(f"output {logical} was not written")
            if path.stat().st_size == 0:
                raise StepFailed(f"output {logical} is empty")
        now = datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds")
        for logical, path in zip(recipe["outputs"], staged):
            final = self.paths.fixtures / logical
            final.parent.mkdir(parents=True, exist_ok=True)
            stamp = stamp_path(final)
            # Drop the old stamp first: a crash mid-publish reads as stale.
            if stamp.exists():
                stamp.unlink()
            record = {"schema": SCHEMA, "recipe": recipe["name"], "logical_path": logical, "key": key,
                      "size": path.stat().st_size, "sha256": sha256_file(path), "generated_at": now,
                      "key_material": material}
            tmp = final.with_name(f".{final.name}.tmp-{os.getpid()}")
            try:
                os.replace(path, tmp)
            except OSError:
                shutil.copyfile(path, tmp)
            os.replace(tmp, final)
            stamp_tmp = stamp.with_name(f".{stamp.name}.tmp-{os.getpid()}")
            stamp_tmp.write_text(json.dumps(record, indent=2, sort_keys=True) + "\n")
            os.replace(stamp_tmp, stamp)

    def cleanup(self):
        if self.keep:
            return
        for d in self.paths.regen.glob("upstream-*"):
            rmtree(d)
        for sub in ("stage", "work", "logs"):
            d = self.paths.regen / sub
            if d.is_dir() and not any(d.iterdir()):
                d.rmdir()
        if self.paths.regen.is_dir() and not any(self.paths.regen.iterdir()):
            self.paths.regen.rmdir()


def select(recipes: dict[str, dict], patterns: list[str]) -> set[str]:
    if not patterns:
        return set(recipes)
    chosen = {n for n, r in recipes.items()
              if any(fnmatch.fnmatchcase(n, p) or any(fnmatch.fnmatchcase(o, p) for o in r["outputs"])
                     for p in patterns)}
    if not chosen:
        raise RecipeError(f"--only {patterns} matches no recipe or output")
    return chosen


def with_dependencies(recipes: dict[str, dict], names: set[str]) -> set[str]:
    closure, todo = set(), list(names)
    while todo:
        n = todo.pop()
        if n not in closure:
            closure.add(n)
            todo += recipes[n].get("after", [])
    return closure


def schedule(runner: Runner, todo: list[str], jobs: int) -> list[str]:
    """Run recipes respecting `after`; returns failed (or blocked) names."""
    recipes = runner.recipes
    pending = list(todo)
    done: set[str] = set(n for n in recipes if n not in todo)
    failed: set[str] = set()
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, jobs)) as pool:
        running: dict[concurrent.futures.Future, str] = {}
        while pending or running:
            for name in list(pending):
                deps = recipes[name].get("after", [])
                if any(d in failed for d in deps):
                    pending.remove(name)
                    failed.add(name)
                    runner.say(f"FAIL {name}: dependency failed")
                elif all(d in done for d in deps) and len(running) < max(1, jobs):
                    pending.remove(name)
                    running[pool.submit(runner.run, name)] = name
            if not running:
                if pending:
                    raise RecipeError(f"unschedulable recipes: {pending}")
                break
            finished, _ = concurrent.futures.wait(running, return_when=concurrent.futures.FIRST_COMPLETED)
            for future in finished:
                name = running.pop(future)
                try:
                    ok = future.result()
                except RecipeError as error:
                    runner.say(f"FAIL {name}: {error}")
                    ok = False
                (done if ok else failed).add(name)
    return sorted(failed)


def check_goarch_runs(paths: Paths) -> None:
    """A foreign --goarch needs transparent execution: recipes run Go binaries directly."""
    if paths.goarch == host_goarch():
        return
    probe = paths.regen / "goarch-probe"
    rmtree(probe)
    probe.mkdir(parents=True)
    (probe / "go.mod").write_text("module probe\n")
    (probe / "main.go").write_text('package main\n\nimport "fmt"\n\nfunc main() { fmt.Print("ok") }\n')
    env = {**os.environ, "GOARCH": paths.goarch, "CGO_ENABLED": "0", "GOTOOLCHAIN": paths.go_toolchain,
           "GOMODCACHE": str(paths.gomodcache), "GOCACHE": str(paths.gocache)}
    try:
        subprocess.run(["go", "build", "-o", "probe", "."], cwd=probe, env=env, check=True,
                       stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        ran = subprocess.run([str(probe / "probe")], stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60)
    except (OSError, subprocess.SubprocessError) as error:
        ran = error
    finally:
        rmtree(probe)
    if not isinstance(ran, subprocess.CompletedProcess) or ran.stdout != b"ok":
        raise RecipeError(
            f"--goarch {paths.goarch} on a {host_goarch()} host needs {paths.goarch} binaries to run "
            "transparently: install qemu-user-static with binfmt (Linux) or Rosetta (macOS)")


def main(argv: list[str] | None = None, *, root: Path | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--only", action="append", default=[], metavar="GLOB",
                        help="recipe name or logical output path glob (repeatable); dependencies are included")
    parser.add_argument("--list", action="store_true", help="list recipes, outputs and cache status")
    parser.add_argument("--check", action="store_true", help="report stale/missing artifacts without running; exit 1 if any")
    parser.add_argument("--force", action="store_true", help="regenerate selected recipes even when fresh")
    parser.add_argument("--keep", action="store_true", help="keep upstream checkout, workdirs and logs")
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--budget-total-mib", type=int, default=1024, help="run output budget (default 1 GiB)")
    parser.add_argument("--budget-file-mib", type=int, default=128, help="per-file output budget (default 128 MiB)")
    parser.add_argument("--min-free-gib", type=float, default=20.0, help="required free space on the volume")
    parser.add_argument("--goarch", choices=GOARCHES, default=os.environ.get("D2RS_GOARCH") or DEFAULT_GOARCH,
                        help="architecture Go oracles are built for (default arm64, the reference platform)")
    parser.add_argument("--verbose", action="store_true")
    args = parser.parse_args(argv)
    try:
        paths = Paths(root or Path(__file__).resolve().parents[1])
        paths.goarch = args.goarch
        recipes = load_recipes(paths)
        selected = select(recipes, args.only)
        closure = with_dependencies(recipes, selected)
        keys = Keys(paths, recipes)
        order = [n for n in topo_order(recipes) if n in closure]
        if args.list or args.check:
            bad = 0
            for name in order:
                key = keys.key(name)[0]
                status = recipe_status(paths, recipes[name], key)
                bad += status != "fresh"
                if args.list or status != "fresh":
                    print(f"{status:8} {name} [{recipes[name]['kind']}]")
                    if args.list:
                        for out in recipes[name]["outputs"]:
                            print(f"           {out}")
            if args.check:
                print(f"{len(order) - bad}/{len(order)} recipes fresh")
                return 1 if bad else 0
            return 0
        todo = []
        for name in order:
            key = keys.key(name)[0]
            if (args.force and name in selected) or recipe_status(paths, recipes[name], key) != "fresh":
                todo.append(name)
            else:
                print(f"skip {name} (fresh)")
        if not todo:
            return 0
        paths.regen.mkdir(parents=True, exist_ok=True)
        paths.fixtures.mkdir(parents=True, exist_ok=True)
        budget = Budget(args.budget_total_mib * MIB, args.budget_file_mib * MIB,
                        int(args.min_free_gib * GIB), paths.regen)
        free = shutil.disk_usage(paths.regen).free
        if free < budget.min_free:
            raise RecipeError(f"only {free / GIB:.1f} GiB free; need {args.min_free_gib} GiB (AGENTS.md)")
        check_goarch_runs(paths)
        runner = Runner(paths, recipes, budget, keep=args.keep, verbose=args.verbose, keys=keys)
        try:
            failed = schedule(runner, todo, args.jobs)
        finally:
            runner.cleanup()
        print(f"{len(todo) - len(failed)}/{len(todo)} recipes regenerated; {budget.committed} bytes written")
        if failed:
            print("failed: " + ", ".join(failed))
            return 1
        return 0
    except RecipeError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
