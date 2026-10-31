#!/usr/bin/env python3
"""Contract tests for scripts/regen-fixtures.py using fake recipes (no Go).

    uv run --no-project python scripts/test_regen_fixtures.py
"""
from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import textwrap
import unittest

SPEC = importlib.util.spec_from_file_location("regen_fixtures", Path(__file__).with_name("regen-fixtures.py"))
regen = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(regen)

WRITE = "import os,sys; open(os.environ['D2RS_RECIPE_OUTPUT'],'w').write(sys.argv[1])"


class Repo:
    def __init__(self, base: Path, *, patches: list[str] | None = None):
        self.root = base / "repo"
        (self.root / "tools/fixtures").mkdir(parents=True)
        self.upstream = self.root / "d2"
        self.upstream.mkdir()
        env = {**os.environ, "GIT_CEILING_DIRECTORIES": str(base)}
        run = lambda *a: subprocess.run(["git", *a], cwd=self.upstream, env=env, check=True, capture_output=True)
        run("init", "-q")
        (self.upstream / "lib").mkdir()
        (self.upstream / "lib/a.txt").write_text("one\n")
        run("add", ".")
        run("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "pin")
        pin = subprocess.run(["git", "rev-parse", "HEAD"], cwd=self.upstream, env=env, check=True,
                             capture_output=True, text=True).stdout.strip()
        manifest = {"d2": {"commit": pin, "checkout": "d2", "go_toolchain": "go1.27.0",
                           "trace_patches": patches or []},
                    "dependencies": {"example.com/mod": "v1.2.3"}}
        (self.root / "reference-manifest.json").write_text(json.dumps(manifest))
        os.environ["D2RS_GOMODCACHE"] = str(base / "gomod")
        os.environ["D2RS_GOCACHE"] = str(base / "gocache")

    def recipes(self, text: str):
        (self.root / "tools/fixtures/test.toml").write_text(textwrap.dedent(text))

    def run(self, *args: str) -> tuple[int, str]:
        out = io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
            code = regen.main(["--min-free-gib", "0", *args], root=self.root)
        return code, out.getvalue()

    def artifact(self, logical: str) -> Path:
        return self.root / "oracle-cache/fixtures" / logical


class RegenTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.base = Path(self.tmp.name).resolve()
        os.environ.pop("D2RS_FIXTURES_DIR", None)

    def tearDown(self):
        self.tmp.cleanup()

    def simple(self, repo: Repo, body: str = "hello", extra: str = ""):
        (repo.root / "seed.txt").write_text("seed\n")
        repo.recipes(f"""
            [[recipe]]
            name = "simple"
            kind = "command"
            outputs = ["crates/x/tests/fixtures/a.json"]
            inputs = ["seed.txt"]
            argv = ["{{python}}", "-c", "{WRITE}", "{body}"]
            {extra}
        """)

    def test_publish_stamp_skip_and_staleness(self):
        repo = Repo(self.base)
        self.simple(repo)
        code, out = repo.run()
        self.assertEqual(code, 0, out)
        artifact = repo.artifact("crates/x/tests/fixtures/a.json")
        self.assertEqual(artifact.read_text(), "hello")
        stamp = json.loads(artifact.with_name("a.json.stamp.json").read_text())
        self.assertEqual(stamp["logical_path"], "crates/x/tests/fixtures/a.json")
        self.assertEqual(stamp["size"], 5)
        self.assertIn("seed.txt", stamp["key_material"]["files"])
        self.assertEqual(repo.run("--check")[0], 0)
        code, out = repo.run()
        self.assertIn("skip simple (fresh)", out)
        (repo.root / "seed.txt").write_text("changed\n")
        code, out = repo.run("--check")
        self.assertEqual(code, 1)
        self.assertIn("stale", out)
        self.assertEqual(repo.run()[0], 0)
        artifact.write_text("tampered")
        self.assertIn("corrupt", repo.run("--check")[1])
        self.assertFalse((repo.root / "oracle-cache/work").exists())
        self.assertFalse((repo.root / "oracle-cache/stage").exists())

    def test_force_and_only(self):
        repo = Repo(self.base)
        self.simple(repo)
        repo.run()
        code, out = repo.run("--force", "--only", "crates/x/*")
        self.assertEqual(code, 0)
        self.assertIn("ok   simple", out)
        self.assertEqual(repo.run("--only", "nomatch")[0], 2)

    def test_failures_publish_nothing(self):
        cases = {
            "nonzero": ('argv = ["{python}", "-c", "raise SystemExit(3)"]', "exited with status 3"),
            "missing": ('argv = ["{python}", "-c", "pass"]', "was not written"),
            "timeout": ('argv = ["{python}", "-c", "import time; time.sleep(30)"]\ntimeout = 1', "timed out"),
            "budget": ('argv = ["{python}", "-c", "import os; open(os.environ[\'D2RS_RECIPE_OUTPUT\'],\'wb\').write(bytes(3<<20)); import time; time.sleep(5)"]',
                       "budget exceeded"),
        }
        for name, (argv, message) in cases.items():
            with self.subTest(name):
                base = self.base / name
                base.mkdir()
                repo = Repo(base)
                repo.recipes(f"""
                    [[recipe]]
                    name = "{name}"
                    kind = "command"
                    outputs = ["out/{name}.bin"]
                    {argv}
                """)
                code, out = repo.run("--budget-file-mib", "1")
                self.assertEqual(code, 1, out)
                self.assertIn(message, out)
                self.assertFalse(repo.artifact(f"out/{name}.bin").exists())
                self.assertTrue((repo.root / f"oracle-cache/logs/{name}.log").exists())

    def test_after_dependencies_and_key_propagation(self):
        repo = Repo(self.base)
        (repo.root / "v.txt").write_text("1")
        repo.recipes(f"""
            [[recipe]]
            name = "base"
            kind = "command"
            outputs = ["out/base.txt"]
            argv = ["{{python}}", "-c", "{WRITE}", "B"]
            key_deps.files = ["v.txt"]

            [[recipe]]
            name = "derived"
            kind = "command"
            after = ["base"]
            outputs = ["out/derived.txt"]
            argv = ["{{python}}", "-c", "import os; open(os.environ['D2RS_RECIPE_OUTPUT'],'w').write(open(os.environ['D2RS_FIXTURES_DIR']+'/out/base.txt').read()+'D')"]
        """)
        code, out = repo.run("--jobs", "2")
        self.assertEqual(code, 0, out)
        self.assertEqual(repo.artifact("out/derived.txt").read_text(), "BD")
        (repo.root / "v.txt").write_text("2")
        code, out = repo.run("--check")
        self.assertIn("stale    base", out)
        self.assertIn("stale    derived", out)

    def test_upstream_checkout_patched_and_removed(self):
        patch = textwrap.dedent("""\
            diff --git a/lib/a.txt b/lib/a.txt
            --- a/lib/a.txt
            +++ b/lib/a.txt
            @@ -1 +1 @@
            -one
            +patched
        """)
        base = self.base
        (base / "repo/tools/patches").mkdir(parents=True)
        (base / "repo/tools/patches/p.patch").write_text(patch)
        repo = Repo(base, patches=["tools/patches/p.patch"])
        repo.recipes("""
            [[recipe]]
            name = "up"
            kind = "command"
            upstream = true
            outputs = ["out/up.txt"]
            argv = ["{python}", "-c", "import os,shutil; shutil.copy(os.environ['D2RS_UPSTREAM']+'/lib/a.txt', os.environ['D2RS_RECIPE_OUTPUT'])"]
            key_deps.upstream_paths = ["lib"]
        """)
        code, out = repo.run()
        self.assertEqual(code, 0, out)
        self.assertEqual(repo.artifact("out/up.txt").read_text(), "patched\n")
        stamp = json.loads(repo.artifact("out/up.txt").with_name("up.txt.stamp.json").read_text())
        self.assertEqual(stamp["key_material"]["upstream"]["patches"][0]["path"], "tools/patches/p.patch")
        self.assertIn("lib", stamp["key_material"]["upstream_trees"])
        self.assertEqual(list((repo.root / "oracle-cache").glob("upstream-*")), [])
        (base / "repo/tools/patches/p.patch").write_text(patch.replace("+patched", "+other"))
        self.assertIn("stale", repo.run("--check")[1])

    def test_rejects_bad_recipes(self):
        repo = Repo(self.base)
        bad = {
            "unknown field": 'outputs = ["a"]\nargv = ["x"]\nbogus = 1',
            "escaping output": 'outputs = ["../a"]\nargv = ["x"]',
            "missing argv": 'outputs = ["a"]',
        }
        for label, body in bad.items():
            with self.subTest(label):
                repo.recipes(f'[[recipe]]\nname = "r"\nkind = "command"\n{body}\n')
                self.assertEqual(repo.run("--list")[0], 2)

    def test_go_modules_key_from_manifest(self):
        repo = Repo(self.base)
        self.simple(repo, extra='key_deps.go_modules = ["example.com/mod"]')
        self.assertEqual(repo.run()[0], 0)
        stamp = json.loads(repo.artifact("crates/x/tests/fixtures/a.json").with_name("a.json.stamp.json").read_text())
        self.assertEqual(stamp["key_material"]["go_modules"], {"example.com/mod": "v1.2.3"})

    def test_goarch_in_key_and_environment(self):
        repo = Repo(self.base)
        repo.recipes(f"""
            [[recipe]]
            name = "arch"
            kind = "command"
            outputs = ["crates/x/tests/fixtures/arch.txt"]
            argv = ["{{python}}", "-c", "import os; open(os.environ['D2RS_RECIPE_OUTPUT'],'w').write(os.environ['GOARCH'] + ' ' + os.environ['D2RS_GOARCH'])"]
        """)
        os.environ.pop("D2RS_GOARCH", None)
        self.assertEqual(repo.run()[0], 0)
        artifact = repo.artifact("crates/x/tests/fixtures/arch.txt")
        self.assertEqual(artifact.read_text(), "arm64 arm64")
        stamp = json.loads(artifact.with_name("arch.txt.stamp.json").read_text())
        self.assertEqual(stamp["key_material"]["goarch"], "arm64")
        self.assertEqual(repo.run("--check")[0], 0)
        code, out = repo.run("--check", "--goarch", "amd64")
        self.assertEqual(code, 1)
        self.assertIn("stale", out)


if __name__ == "__main__":
    sys.exit(unittest.main())
