// SPDX-License-Identifier: MPL-2.0
// Oracle hook for crates/d2-rs-render/tests/fixtures/sketch-svg-source-v090.json
// and sketch-svg-rectangle-source-v090.json (docs/FIXTURES.md): public
// d2sketch.Connection/Arrowheads SVG and rough-go path operation sets for
// hand-written paths x arrowheads x widths x animation, plus one dots-pattern
// d2sketch.Rect with its fill-pattern definitions. Overlaid into
// d2renderers/d2sketch; inert unless D2RS_RECIPE_OUTPUTS is set. Harvested
// from the v0.9.0-era .reference/sketch-svg-20261018/oracle/main.go and its
// cases.json, generated inline here.
package d2sketch

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"github.com/d2lang/d2/d2target"
	"github.com/d2lang/d2/lib/geo"
	rough "github.com/d2lang/rough-go"
)

type d2rsSketchRequest struct {
	Name     string `json:"name"`
	Rect     bool   `json:"rect,omitempty"`
	Path     string `json:"path"`
	Source   string `json:"source"`
	Target   string `json:"target"`
	Width    int    `json:"width"`
	Animated bool   `json:"animated"`
}

func d2rsSketchRequests() []d2rsSketchRequest {
	requests := []d2rsSketchRequest{}
	for i, path := range []string{"M0 0 L20 5", "M-10.5 3.75 L18.25 -6.5 L40 22", "M0 0 C10 40 80 -20 100 25",
		"M1 2 C8 18 12 22 20 30 C28 50 60 10 72 32", "M0 0 L20 0 C25 0 25 0 25 5 L25 40", "M0 0 L10 10 M30 40 L50 60",
		"M0 0 L20 0 L20 20 Z", "M0.00000001 -0.00000003 C0.00000002 0.00000004 1 2 3 4"} {
		requests = append(requests, d2rsSketchRequest{Name: fmt.Sprintf("path-%d", i), Path: path, Width: 2})
	}
	for _, arrow := range []string{"arrow", "line", "triangle", "unfilled-triangle", "diamond", "filled-diamond", "circle",
		"filled-circle", "box", "filled-box", "cross", "cf-one", "cf-many", "cf-one-required", "cf-many-required"} {
		for _, width := range []int{0, 2, 5} {
			requests = append(requests, d2rsSketchRequest{Name: fmt.Sprintf("arrow-%s-%d", arrow, width), Path: "M0 0 L20 5", Source: arrow, Target: arrow, Width: width})
		}
	}
	for _, pair := range [][2]string{{"filled-circle", "triangle"}, {"triangle", "filled-circle"}, {"line", "arrow"}} {
		requests = append(requests, d2rsSketchRequest{Name: "mixed-" + pair[0] + "-" + pair[1], Path: "M0 0 L20 5", Source: pair[0], Target: pair[1], Width: 2})
	}
	for _, pair := range [][2]string{{"", ""}, {"triangle", ""}, {"", "triangle"}, {"triangle", "triangle"}} {
		requests = append(requests, d2rsSketchRequest{Name: "animated-" + pair[0] + "-" + pair[1], Path: "M 0 0 L 20 5", Source: pair[0], Target: pair[1], Width: 2, Animated: true})
	}
	requests = append(requests,
		d2rsSketchRequest{Name: "smooth-first", Path: "M0 0 L20 0 S25 0 25 5 L25 40", Width: 2},
		d2rsSketchRequest{Name: "smooth-chain", Path: "M0 0 C10 30 20 -10 30 5 S45 30 60 5 S80 -10 90 20", Width: 2},
		d2rsSketchRequest{Name: "horizontal-vertical", Path: "M0 0 H30 V40 L50 60", Width: 2})
	for _, c := range [][2]string{{"polyline", "M 0 0 L 10 0 L 10 30 L 40 30"}, {"cubic", "M 0 0 C 10 40 80 -20 100 25"}, {"smooth", "M 0 0 L 10 0 S 20 0 20 10 L 20 40"}} {
		for _, arrow := range []string{"", "triangle"} {
			requests = append(requests, d2rsSketchRequest{Name: "animated-" + c[0] + "-" + arrow, Path: c[1], Source: arrow, Target: arrow, Width: 2, Animated: true})
		}
	}
	return requests
}

func TestD2RSSketchSVG(t *testing.T) {
	outputs := filepath.SplitList(os.Getenv("D2RS_RECIPE_OUTPUTS"))
	if len(outputs) != 2 {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	rows := []any{}
	for _, request := range d2rsSketchRequests() {
		c := d2target.Connection{Stroke: "#123456", StrokeWidth: request.Width, Opacity: 1, SrcArrow: d2target.NoArrowhead, DstArrow: d2target.NoArrowhead, Route: []*geo.Point{geo.NewPoint(0, 0), geo.NewPoint(20, 5)}, Animated: request.Animated}
		if request.Source != "" {
			c.SrcArrow = d2target.Arrowhead(request.Source)
		}
		if request.Target != "" {
			c.DstArrow = d2target.Arrowhead(request.Target)
		}
		connection, err := Connection(c, request.Path, `mask="url(#diagram)"`)
		if err != nil {
			t.Fatal(err)
		}
		arrows, err := Arrowheads(c, geo.NewPoint(1, 2), geo.NewPoint(1, 2))
		if err != nil {
			t.Fatal(err)
		}
		g := rough.NewGenerator(nil)
		drawable := g.Path(request.Path, &rough.Options{Seed: rough.Float64(1), Roughness: rough.Float64(.5)})
		rows = append(rows, map[string]any{"input": request, "connection": connection, "arrows": arrows, "operations": drawable.Sets})
	}
	request := d2rsSketchRequest{Name: "rectangle", Rect: true}
	shape := d2target.Shape{Pos: d2target.Point{X: 10, Y: 20}, Width: 140, Height: 80, Fill: "#ffffff", Stroke: "#112233", StrokeWidth: 2, FillPattern: "dots", Opacity: 1}
	rendered, err := Rect(shape, "diagram")
	if err != nil {
		t.Fatal(err)
	}
	var buffer bytes.Buffer
	buffer.WriteString(rendered)
	buffer.WriteString(`<!-- url(#streaks-bright-diagram) -->`)
	DefineFillPatterns(&buffer, "diagram")
	rect := []any{map[string]any{"input": request, "shape": shape, "svg": buffer.String()}}
	for i, value := range []any{rows, rect} {
		data, err := json.Marshal(value)
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(outputs[i], append(data, '\n'), 0o644); err != nil {
			t.Fatal(err)
		}
	}
}
