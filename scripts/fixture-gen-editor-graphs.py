"""Generator for recipe mathtext-txt-editor-graphs (tools/fixtures/mathtext-txt.toml).

Writes crates/d2-rs-editor/tests/fixtures/editor-graphs-v090.json: every live
d2oracle edit request of the pinned upstream TestCreate, TestSet,
TestReconnectEdge, TestRename, TestMove and TestDelete tables, each with the
upstream graph golden testdata/d2oracle/<test>.exp.json it is checked against.

The requests are recorded by running those upstream tests with an env-gated
recording hook: the runner's patched upstream tree is left untouched, and the
instrumented copy of d2oracle/edit_test.go plus a helper are supplied through
`go test -overlay`. The goldens are upstream files, not regenerated output.
"""
from __future__ import annotations

import hashlib
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

LOGICAL = "crates/d2-rs-editor/tests/fixtures/editor-graphs-v090.json"
TESTS = ["Create", "Set", "ReconnectEdge", "Rename", "Move", "Delete"]
FIELDS = {
    "Create": '"operation":"create", "key":tc.key, "board_path":tc.boardPath, "files":tc.fsTexts',
    "Set": '"operation":"set", "key":tc.key, "tag":tc.tag, "value":tc.value, "board_path":tc.boardPath, "files":tc.fsTexts',
    "ReconnectEdge": '"operation":"reconnect", "key":tc.edgeKey, "src":tc.newSrc, "dst":tc.newDst, "board_path":tc.boardPath',
    "Rename": '"operation":"rename", "key":tc.key, "new_key":tc.newName, "board_path":tc.boardPath, "files":tc.fsTexts',
    "Move": '"operation":"move", "key":tc.key, "new_key":tc.newKey, "include_descendants":tc.includeDescendants, "board_path":tc.boardPath, "files":tc.fsTexts',
    "Delete": '"operation":"delete", "key":tc.key, "board_path":tc.boardPath, "files":tc.fsTexts',
}

HELPER = '''// Env-gated d2-rs request recorder (tools/fixtures/mathtext-txt.toml).
package d2oracle_test

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"testing"
)

var d2rsEditRequests sync.Map

func d2rsEditDump(t testing.TB, request map[string]interface{}, err error) {
	dir := os.Getenv("D2RS_EDIT_DUMP")
	if dir == "" {
		return
	}
	if input, ok := request["input"].(string); ok {
		request["input_hex"] = fmt.Sprintf("%x", []byte(input))
	}
	message := ""
	if err != nil {
		message = err.Error()
	}
	data, e := json.Marshal(map[string]interface{}{"name": t.Name(), "request": request, "error": message})
	if e != nil {
		t.Fatal(e)
	}
	path := filepath.Join(dir, fmt.Sprintf("%x.json", sha256.Sum256([]byte(t.Name()))))
	if e = os.WriteFile(path, data, 0644); e != nil {
		t.Fatal(e)
	}
}
'''


def instrument(text: str) -> str:
    for test, fields in FIELDS.items():
        start = text.index(f"func Test{test}(t *testing.T)")
        end = text.index("\nfunc ", start + 1)
        section = text[start:end]
        marker = "\t\t\tet.run(t)"
        if section.count(marker) != 1:
            sys.exit(f"Test{test}: expected one et.run(t)")
        hook = '\t\t\td2rsEditRequests.Store(t.Name(), map[string]interface{}{"input":tc.text, ' + fields + "})\n"
        text = text[:start] + section.replace(marker, hook + marker) + text[end:]
    old = '\terr = diff.TestdataJSON(filepath.Join("..", "testdata", "d2oracle", t.Name()), got)'
    if text.count(old) != 1:
        sys.exit("edit_test.go: expected one d2oracle TestdataJSON call")
    hook = '''	if value, ok := d2rsEditRequests.Load(t.Name()); ok {
		request := value.(map[string]interface{})
		request["path"] = d2Path
		request["expected_text"] = tc.exp
		d2rsEditDump(t, request, err)
	}
'''
    return text.replace(old, hook + old)


UPDATE_IMPORT = "crates/d2-rs-editor/tests/fixtures/update-import-v090.json"


def instrument_update_import(text: str) -> str:
    """Record every TestUpdateImport UpdateImport call with its table row."""
    call = "\t\t\tgot, err := d2oracle.UpdateImport(tc.text, tc.path, tc.newPath)\n"
    if text.count(call) != 1:
        sys.exit("edit_test.go: expected one TestUpdateImport UpdateImport call")
    hook = ('\t\t\td2rsEditDump(t, map[string]interface{}{"result_kind": "text", "operation": "update_import", '
            '"input": tc.text, "path": "", "old_path": tc.path, "new_path": tc.newPath, '
            '"expected_text": tc.exp, "expected_error": tc.expErr}, err)\n')
    return text.replace(call, call + hook)


def run_instrumented(upstream: Path, work: Path, text: str, run: str) -> list[dict]:
    dump = work / "dump"
    dump.mkdir()
    edit_test = work / "edit_test.go"
    edit_test.write_text(text)
    helper = work / "d2rs_edit_dump_test.go"
    helper.write_text(HELPER)
    overlay = work / "overlay.json"
    overlay.write_text(json.dumps({"Replace": {
        str(upstream / "d2oracle/edit_test.go"): str(edit_test),
        str(upstream / "d2oracle/d2rs_edit_dump_test.go"): str(helper),
    }}))
    env = dict(fixture_recipe.go_env(), D2RS_EDIT_DUMP=str(dump))
    subprocess.run(["go", "test", "-mod=readonly", "-count=1", "-overlay", str(overlay), "-run", run, "./d2oracle"],
                   cwd=upstream, env=env, check=True, stdout=sys.stderr)
    return sorted((json.loads(p.read_text()) for p in dump.iterdir()), key=lambda row: row["name"])


def update_import() -> None:
    """`update-import` mode: crates/d2-rs-editor/tests/fixtures/update-import-v090.json,
    every TestUpdateImport call (request, expected text and error)."""
    upstream = Path(fixture_recipe._env("D2RS_UPSTREAM"))
    source = upstream / "d2oracle/edit_test.go"
    rows = run_instrumented(upstream, fixture_recipe.workdir(), instrument_update_import(source.read_text()),
                            "^TestUpdateImport$")
    manifest = json.loads((Path(fixture_recipe._env("D2RS_REPO_ROOT")) / "reference-manifest.json").read_text())
    fixture_recipe.output(UPDATE_IMPORT).write_text(json.dumps(
        {"upstream_commit": manifest["d2"]["commit"], "source": "d2oracle/edit_test.go TestUpdateImport",
         "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(), "cases": rows}, indent=2) + "\n")


def main() -> None:
    if sys.argv[1:] == ["update-import"]:
        update_import()
        return
    upstream = Path(fixture_recipe._env("D2RS_UPSTREAM"))
    work = fixture_recipe.workdir()
    dump = work / "dump"
    dump.mkdir()
    edit_test = work / "edit_test.go"
    edit_test.write_text(instrument((upstream / "d2oracle/edit_test.go").read_text()))
    helper = work / "d2rs_edit_dump_test.go"
    helper.write_text(HELPER)
    overlay = work / "overlay.json"
    overlay.write_text(json.dumps({"Replace": {
        str(upstream / "d2oracle/edit_test.go"): str(edit_test),
        str(upstream / "d2oracle/d2rs_edit_dump_test.go"): str(helper),
    }}))
    env = dict(fixture_recipe.go_env(), D2RS_EDIT_DUMP=str(dump))
    run = "^Test(" + "|".join(TESTS) + ")$"
    subprocess.run(["go", "test", "-mod=readonly", "-count=1", "-overlay", str(overlay), "-run", run, "./d2oracle"],
                   cwd=upstream, env=env, check=True, stdout=sys.stderr)
    manifest = json.loads((Path(fixture_recipe._env("D2RS_REPO_ROOT")) / "reference-manifest.json").read_text())
    cases = []
    for path in dump.iterdir():
        row = json.loads(path.read_text())
        golden = f"testdata/d2oracle/{row['name']}.exp.json"
        data = (upstream / golden).read_bytes()
        cases.append({"name": row["name"], "request": row["request"], "error": row["error"], "golden": golden,
                      "golden_sha256": hashlib.sha256(data).hexdigest(), "expected": json.loads(data)})
    cases.sort(key=lambda c: c["name"])
    fixture_recipe.output(LOGICAL).write_text(
        json.dumps({"upstream_commit": manifest["d2"]["commit"], "cases": cases}, separators=(",", ":")) + "\n")


if __name__ == "__main__":
    main()
