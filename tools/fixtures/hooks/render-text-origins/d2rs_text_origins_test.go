// SPDX-License-Identifier: MPL-2.0
// Oracle hook for crates/d2-rs-render/tests/fixtures/sql-text-origins-source-v090.json
// and code-line-origins-source-v090.json (docs/FIXTURES.md): d2scenebuild
// TextRun origins (float bits) of SQL tables (column count x height x y x font
// size) and code shapes (source x font size x y). Overlaid into
// d2renderers/d2scenebuild; inert unless D2RS_RECIPE_OUTPUTS is set.
// Harvested from the v0.9.0-era sql_origin_probe_test.go and
// code_baseline_probe_test.go, with their input grids inlined.
package d2scenebuild

import (
	"encoding/json"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
	"github.com/d2lang/d2/d2target"
)

func d2rsWalkTextRuns(n *d2scene.Node, visit func(d2scene.TextRun)) {
	if v, ok := n.Primitive.(d2scene.TextRun); ok {
		visit(v)
	}
	for _, c := range n.Children {
		d2rsWalkTextRuns(c, visit)
	}
}

func TestD2RSTextOrigins(t *testing.T) {
	outputs := filepath.SplitList(os.Getenv("D2RS_RECIPE_OUTPUTS"))
	if len(outputs) != 2 {
		t.Skip("run via scripts/regen-fixtures.py")
	}

	type sqlRun struct {
		Text  string `json:"text"`
		XBits uint64 `json:"x_bits"`
		YBits uint64 `json:"y_bits"`
	}
	type sqlRow struct {
		Count  int      `json:"count"`
		Height int      `json:"height"`
		Y      int      `json:"y"`
		Size   int      `json:"size"`
		Runs   []sqlRun `json:"runs"`
	}
	sqlRows := []sqlRow{}
	for _, count := range []int{0, 1, 2, 5} {
		for _, height := range []int{99, 101, 512} {
			for _, y := range []int{0, 21} {
				for _, size := range []int{16, 20} {
					r := sqlRow{Count: count, Height: height, Y: y, Size: size}
					s := structuredTestShape("table", d2target.ShapeSQLTable, 10, r.Y, 240, r.Height)
					s.FontSize = r.Size
					s.Label = "users"
					s.LabelWidth = 50
					s.LabelHeight = 20
					for j := 0; j < r.Count; j++ {
						typ := "varchar"
						if j == 1 {
							typ = "var  char"
						}
						s.Columns = append(s.Columns, d2target.SQLColumn{Name: d2target.Text{Label: fmt.Sprintf("name%d", j), LabelWidth: 35 + j*7}, Type: d2target.Text{Label: typ, LabelWidth: 60}, Constraint: []string{"primary_key"}})
					}
					doc := buildStructuredDocument(t, s)
					d2rsWalkTextRuns(doc.Root, func(v d2scene.TextRun) {
						r.Runs = append(r.Runs, sqlRun{v.Text, math.Float64bits(v.Origin.X), math.Float64bits(v.Origin.Y)})
					})
					sqlRows = append(sqlRows, r)
				}
			}
		}
	}

	type codeToken struct {
		Text  string  `json:"text"`
		X     float64 `json:"x"`
		Y     float64 `json:"y"`
		XBits uint64  `json:"x_bits"`
		YBits uint64  `json:"y_bits"`
	}
	type codeRow struct {
		Label  string      `json:"label"`
		Size   int         `json:"size"`
		PosY   int         `json:"pos_y"`
		Tokens []codeToken `json:"tokens"`
	}
	codeRows := []codeRow{}
	prototype := loadCodeDiagram(t, "regression/code_leading_trailing_newlines/dagre/board.exp.json")
	for _, label := range []string{"\n\n# comment\ndef hello():\n\n  print \"world\"\n\n", "\n\ndef hello():\n  pass", "def hello():\n\n  pass\n\n"} {
		for _, size := range []int{12, 13, 16, 17} {
			for _, posY := range []int{0, 21, 127} {
				r := codeRow{Label: label, Size: size, PosY: posY}
				d := *prototype
				s := prototype.Shapes[0]
				s.Label = r.Label
				s.FontSize = r.Size
				s.Pos.Y = r.PosY
				d.Shapes = []d2target.Shape{s}
				d.Connections = nil
				doc := buildCodeDocument(t, &d, nil)
				d2rsWalkTextRuns(doc.Root, func(v d2scene.TextRun) {
					r.Tokens = append(r.Tokens, codeToken{Text: v.Text, X: v.Origin.X, Y: v.Origin.Y, XBits: math.Float64bits(v.Origin.X), YBits: math.Float64bits(v.Origin.Y)})
				})
				codeRows = append(codeRows, r)
			}
		}
	}

	for i, rows := range []any{sqlRows, codeRows} {
		data, err := json.MarshalIndent(rows, "", "  ")
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(outputs[i], data, 0o644); err != nil {
			t.Fatal(err)
		}
	}
}
