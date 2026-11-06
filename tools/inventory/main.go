// SPDX-License-Identifier: MPL-2.0
// Inventory source declarations and test entrypoints without executing upstream.
package main

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
)

type File struct {
	Path        string   `json:"path"`
	SHA256      string   `json:"sha256"`
	Package     string   `json:"package"`
	Symbols     []string `json:"symbols"`
	RustPackage string   `json:"rust_package"`
	Status      string   `json:"status"`
}
type Test struct {
	ID           string `json:"id"`
	Line         int    `json:"line"`
	Kind         string `json:"kind"`
	Status       string `json:"status"`
	Reason       string `json:"reason"`
	HasSkipCalls bool   `json:"has_skip_calls"`
}
type Inventory struct {
	Commit string `json:"commit"`
	Note   string `json:"note"`
	Files  []File `json:"files"`
	Tests  []Test `json:"tests"`
}

func destination(path string) string {
	switch {
	case strings.HasPrefix(path, "d2layouts/d2talalayout/"):
		return "d2-rs-tala"
	case strings.HasPrefix(path, "d2layouts/d2dagrelayout/"):
		return "d2-rs-dagre"
	case strings.HasPrefix(path, "d2layouts/d2elklayout/"):
		return "d2-rs-elk"
	case strings.HasPrefix(path, "d2layouts/"), strings.HasPrefix(path, "d2plugin/"):
		return "d2-rs-layout"
	case strings.HasPrefix(path, "d2renderers/d2latex/"):
		return "d2-rs-math"
	case strings.HasPrefix(path, "d2renderers/d2sketch/"):
		return "d2-rs-sketch"
	case strings.HasPrefix(path, "d2renderers/d2fonts/"), strings.HasPrefix(path, "lib/font/"), strings.HasPrefix(path, "lib/textmeasure/"):
		return "d2-rs-text"
	case strings.HasPrefix(path, "d2renderers/"):
		return "d2-rs-render"
	case strings.HasPrefix(path, "d2parser/"), strings.HasPrefix(path, "d2compiler/"), strings.HasPrefix(path, "d2format/"):
		return "d2-rs-language"
	case strings.HasPrefix(path, "d2lsp/"), strings.HasPrefix(path, "d2oracle/"):
		return "d2-rs-editor"
	case strings.HasPrefix(path, "d2js/"):
		return "d2-rs-wasm"
	case strings.HasPrefix(path, "d2cli/"), path == "main.go":
		return "d2-rs-cli"
	case strings.HasPrefix(path, "d2lib/"):
		return "d2-rs-lib"
	case strings.HasPrefix(path, "lib/png/"), strings.HasPrefix(path, "lib/pdf/"), strings.HasPrefix(path, "lib/pptx/"), strings.HasPrefix(path, "lib/xgif/"):
		return "d2-rs-export"
	case strings.HasPrefix(path, "d2ast/"), strings.HasPrefix(path, "d2ir/"), strings.HasPrefix(path, "d2graph/"), strings.HasPrefix(path, "d2target/"), strings.HasPrefix(path, "lib/geo/"):
		return "d2-rs-core"
	default:
		return "inventory-review"
	}
}
func run() error {
	source := flag.String("source", "../d2", "pristine checkout")
	output := flag.String("out", "oracle-cache/reports/upstream-inventory.json", "output file")
	flag.Parse()
	git := func(args ...string) (string, error) {
		cmd := exec.Command("git", args...)
		cmd.Dir = *source
		b, e := cmd.Output()
		return strings.TrimSpace(string(b)), e
	}
	commit, err := git("rev-parse", "HEAD")
	if err != nil {
		return err
	}
	if commit != "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda" {
		return fmt.Errorf("unexpected source commit %s", commit)
	}
	dirty, err := git("status", "--porcelain")
	if err != nil {
		return err
	}
	if dirty != "" {
		return fmt.Errorf("source is dirty")
	}
	listing, err := git("ls-files")
	if err != nil {
		return err
	}
	inventory := Inventory{Commit: commit, Note: "Static function inventory; table cases and runtime subtests require execution discovery. No inherited test is counted as upstream-passing. A skip call is not proof a test is always skipped."}
	for _, path := range strings.Split(listing, "\n") {
		if !strings.HasSuffix(path, ".go") {
			continue
		}
		content, err := os.ReadFile(filepath.Join(*source, path))
		if err != nil {
			return err
		}
		sum := sha256.Sum256(content)
		positions := token.NewFileSet()
		tree, err := parser.ParseFile(positions, path, content, 0)
		if err != nil {
			return err
		}
		entry := File{Path: path, SHA256: hex.EncodeToString(sum[:]), Package: tree.Name.Name, Symbols: []string{}, RustPackage: destination(path), Status: "unmapped"}
		for _, decl := range tree.Decls {
			fn, ok := decl.(*ast.FuncDecl)
			if !ok {
				continue
			}
			symbol := fn.Name.Name
			if fn.Recv != nil {
				field := fn.Recv.List[0].Type
				if ptr, ok := field.(*ast.StarExpr); ok {
					field = ptr.X
				}
				if id, ok := field.(*ast.Ident); ok {
					symbol = id.Name + "." + symbol
				}
			}
			entry.Symbols = append(entry.Symbols, symbol)
			if !strings.HasSuffix(path, "_test.go") || fn.Recv != nil {
				continue
			}
			kind := ""
			for _, prefix := range []string{"Test", "Fuzz", "Benchmark", "Example"} {
				if strings.HasPrefix(fn.Name.Name, prefix) {
					kind = prefix
					break
				}
			}
			if kind == "" {
				continue
			}
			test := Test{ID: path + "::" + fn.Name.Name, Line: positions.Position(fn.Pos()).Line, Kind: kind, Status: "failing", Reason: "not_ported_or_executed_against_rust"}
			ast.Inspect(fn, func(n ast.Node) bool {
				if call, ok := n.(*ast.CallExpr); ok {
					if sel, ok := call.Fun.(*ast.SelectorExpr); ok {
						if sel.Sel.Name == "Skip" || sel.Sel.Name == "Skipf" || sel.Sel.Name == "SkipNow" {
							test.HasSkipCalls = true
						}
					}
				}
				return true
			})
			inventory.Tests = append(inventory.Tests, test)
		}
		sort.Strings(entry.Symbols)
		inventory.Files = append(inventory.Files, entry)
	}
	bytes, err := json.MarshalIndent(inventory, "", "  ")
	if err != nil {
		return err
	}
	if err = os.MkdirAll(filepath.Dir(*output), 0o755); err != nil {
		return err
	}
	if err = os.WriteFile(*output, append(bytes, '\n'), 0644); err != nil {
		return err
	}
	fmt.Printf("Inventoried %d source/test files and %d test/fuzz/benchmark/example functions\n", len(inventory.Files), len(inventory.Tests))
	return nil
}
func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
