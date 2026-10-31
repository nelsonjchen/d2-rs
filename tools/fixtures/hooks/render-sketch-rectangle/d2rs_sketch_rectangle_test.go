// SPDX-License-Identifier: MPL-2.0
// Oracle hook for crates/d2-rs-render/tests/fixtures/sketch-rectangle-source-v090.json
// (docs/FIXTURES.md): rough rectangle path sets and the dots-pattern sketch
// paint order of one ordinary sketch rectangle. Overlaid into
// d2renderers/d2scenebuild; inert unless D2RS_RECIPE_OUTPUT is set. Harvested
// from the v0.9.0-era sketch_rectangle_painter_probe_test.go; the streak
// counts are that probe's recorded constants, the paint nodes are asserted.
package d2scenebuild

import (
	"context"
	"encoding/json"
	"os"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
	"github.com/d2lang/d2/d2target"
)

func TestD2RSSketchRectangle(t *testing.T) {
	path := os.Getenv("D2RS_RECIPE_OUTPUT")
	if path == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	b := newSketchShapeTestBuilder(context.Background(), d2target.NewDiagram(), largeSketchTestBudget())
	target := sketchTestShape("rectangle", d2target.ShapeRectangle, 10, 20, 140, 80)
	target.Fill = "#ffffff"
	target.FillPattern = "dots"
	nodes, err := b.buildSketchOrdinaryShape(target, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(nodes) != 5 {
		t.Fatalf("paint nodes=%d", len(nodes))
	}
	if nodes[4].Blend != d2scene.BlendDarken {
		t.Fatalf("streak blend=%v", nodes[4].Blend)
	}
	g := newSketchGenerator()
	drawable := g.Rectangle(0, 0, 140, 80, sketchShapeOptions(2, nil, 0))
	paths := []string{g.OpsToPath(drawable.Sets[0]), g.OpsToPath(drawable.Sets[1])}
	data, err := json.MarshalIndent(map[string]any{"paths": paths, "paint_order": []string{"rough", "dots-overlay", "rough", "dots-overlay", "sketch-streak-overlay"}, "streak_subpaths": 85, "streak_primitives": 84, "streak_alpha": 26, "streak_blend": "darken"}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, data, 0o644); err != nil {
		t.Fatal(err)
	}
}
