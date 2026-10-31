#!/usr/bin/env python3
"""Dump pinned upstream language golden inputs through env-gated test hooks.

Only the instrumentation checkout is edited. Goldens are read from pristine
../d2 by language-scoreboard.py; Go results qualify the dump against them.
Run under run-bounded.py with --output inside its scratch directory.
"""
import argparse
from pathlib import Path
import os
import subprocess
from references import verify_source
import references

ROOT = Path(__file__).resolve().parents[1]
TRACE = references.TRACE_CHECKOUT


def insert(path, old, new):
    text = path.read_text()
    if new in text:
        return
    assert text.count(old) == 1, (path, old, text.count(old))
    path.write_text(text.replace(old, new))


HELPER = '''// Env-gated d2-rs golden input capture. Test-oracle code only.
package PACKAGE_test
import (
    "crypto/sha256"
    "encoding/json"
    "fmt"
    "os"
    "path/filepath"
    "testing"
    "sync"
)
var d2rsLanguageRequests sync.Map
func d2rsLanguageDump(t testing.TB, request map[string]interface{}, got interface{}, err error) {
    dir := os.Getenv("D2RS_LANGUAGE_DUMP")
    if dir == "" { return }
    if request == nil {
        value, ok := d2rsLanguageRequests.Load(t.Name())
        if !ok { t.Fatalf("missing dump request for %s", t.Name()) }
        request = value.(map[string]interface{})
    }
    if input, ok := request["input"].(string); ok { request["input_hex"] = fmt.Sprintf("%x", []byte(input)) }
    message := ""
    if err != nil { message = err.Error() }
    row := map[string]interface{}{"package":"PACKAGE", "name":t.Name(), "request":request, "go":got, "error":message}
    data, e := json.Marshal(row)
    if e != nil { t.Fatal(e) }
    dir = filepath.Join(dir, "PACKAGE")
    if e = os.MkdirAll(dir, 0755); e != nil { t.Fatal(e) }
    path := filepath.Join(dir, fmt.Sprintf("%x.json", sha256.Sum256([]byte(t.Name()))))
    if e = os.WriteFile(path, data, 0644); e != nil { t.Fatal(e) }
}
'''


def install():
    for package in ["d2parser", "d2ir", "d2compiler", "d2oracle"]:
        (TRACE / package / "d2rs_scoreboard_test.go").write_text(HELPER.replace("PACKAGE", package))
    path = TRACE / "d2parser/parse_test.go"
    old = '\t\t\terr = diff.TestdataJSON(filepath.Join("..", "testdata", "d2parser", t.Name()), got)'
    insert(path, old, '\t\t\td2rsLanguageDump(t, map[string]interface{}{"input":tc.text, "path":d2Path}, got, err)\n' + old)
    path = TRACE / "d2ir/compile_test.go"
    old = '\terr = diff.TestdataJSON(filepath.Join("..", "testdata", "d2ir", t.Name()), m)'
    insert(path, old, '\td2rsLanguageDump(t, map[string]interface{}{"input":mfs[path], "path":path, "files":mfs}, m, nil)\n' + old)
    path = TRACE / "d2compiler/compile_test.go"
    old = '\t\t\tg, _, err := d2compiler.Compile(d2Path, strings.NewReader(tc.text), opts)'
    new = old + '''
            d2rsFiles := map[string]string{}
            for file, content := range tc.files { d2rsFiles[fmt.Sprintf("d2/testdata/d2compiler/TestCompile/%v", file)] = content }
            d2rsLanguageDump(t, map[string]interface{}{"input":tc.text, "path":d2Path, "files":d2rsFiles}, struct { Graph *d2graph.Graph `json:"graph"`; Err error `json:"err"` }{g, err}, err)'''
    insert(path, old, new)
    old = '\tg, config, err := d2compiler.Compile(d2Path, strings.NewReader(text), nil)'
    insert(path, old, old + '\n\td2rsLanguageDump(t, map[string]interface{}{"input":text, "path":d2Path}, struct { Graph *d2graph.Graph `json:"graph"`; Err error `json:"err"` }{g, err}, err)')
    path = TRACE / "d2oracle/edit_test.go"
    text = path.read_text()
    operations = {
        "Create": '"operation":"create", "key":tc.key, "board_path":tc.boardPath, "files":tc.fsTexts',
        "Set": '"operation":"set", "key":tc.key, "tag":tc.tag, "value":tc.value, "board_path":tc.boardPath, "files":tc.fsTexts',
        "ReconnectEdge": '"operation":"reconnect", "key":tc.edgeKey, "src":tc.newSrc, "dst":tc.newDst, "board_path":tc.boardPath',
        "Rename": '"operation":"rename", "key":tc.key, "new_key":tc.newName, "board_path":tc.boardPath, "files":tc.fsTexts',
        "Move": '"operation":"move", "key":tc.key, "new_key":tc.newKey, "include_descendants":tc.includeDescendants, "board_path":tc.boardPath, "files":tc.fsTexts',
        "Delete": '"operation":"delete", "key":tc.key, "board_path":tc.boardPath, "files":tc.fsTexts',
    }
    for operation, fields in operations.items():
        start = text.index(f"func Test{operation}(t *testing.T)")
        end = text.index("\nfunc ", start + 1)
        section = text[start:end]
        marker = '\t\t\tet.run(t)'
        hook = '\t\t\td2rsLanguageRequests.Store(t.Name(), map[string]interface{}{"input":tc.text, ' + fields + '})\n'
        if hook not in section:
            assert section.count(marker) == 1
            section = section.replace(marker, hook + marker)
            text = text[:start] + section + text[end:]
    path.write_text(text)
    old = '\terr = diff.TestdataJSON(filepath.Join("..", "testdata", "d2oracle", t.Name()), got)'
    # Save the initial parse path and exact public formatted result too. The
    # complete graph golden remains the primary tier; text is a separate tier.
    hook = '''
    if value, ok := d2rsLanguageRequests.Load(t.Name()); ok {
        request := value.(map[string]interface{})
        request["path"] = d2Path
        request["expected_text"] = tc.exp
        d2rsLanguageDump(t, request, got, err)
    }
'''
    insert(path, old, hook + old)
    old = '\t\t\tgot, err := d2oracle.UpdateImport(tc.text, tc.path, tc.newPath)'
    insert(path, old, old + '''
            d2rsLanguageDump(t, map[string]interface{}{
                "result_kind":"text", "operation":"update_import", "input":tc.text,
                "path":"", "old_path":tc.path, "new_path":tc.newPath,
                "expected_text":tc.exp, "expected_error":tc.expErr,
            }, got, err)''')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--packages", default="d2parser,d2ir,d2compiler,d2oracle")
    parser.add_argument("--run", default="TestParse|TestCompile|TestCreate$|TestSet$|TestReconnectEdge$|TestRename$|TestMove$|TestDelete$|TestUpdateImport$")
    parser.add_argument("--json", action="store_true", help="emit Go test events for input-binding audits")
    args = parser.parse_args()
    verify_source()
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=TRACE, text=True).strip()
    assert revision == references.SOURCE_COMMIT, revision
    output = args.output.resolve()
    assert not output.exists() or not any(output.iterdir()), "dump must use a fresh output directory"
    install()
    environment = dict(os.environ, D2RS_LANGUAGE_DUMP=str(output),
                       GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
                       GOCACHE=str(ROOT / ".reference/go-build"),
                       GOFLAGS="-mod=mod", GOPROXY="off")
    packages = ["./" + package for package in args.packages.split(",")]
    return subprocess.call(["go", "test", *packages, "-run", args.run, "-count=1", *(["-json"] if args.json else [])], cwd=TRACE, env=environment)


if __name__ == "__main__":
    raise SystemExit(main())
