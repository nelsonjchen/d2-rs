// SPDX-License-Identifier: MPL-2.0
// Authored diagnostic test overlay for the pristine D2 v0.9.0 source.
package d2scenebuild

import (
	"context"
	"encoding/json"
	"fmt"
	"math"
	"os"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
	"github.com/d2lang/d2/d2target"
	"github.com/d2lang/d2/lib/geo"
)

func TestFrozenMultilinePrecisionOrigins(t *testing.T) {
	diagram := validDiagram()
	connection := &diagram.Connections[0]
	connection.Text = d2target.Text{Label: "identical\nidentical\nidentical", FontSize: 16, FontFamily: "default", Italic: true, LabelWidth: 56, LabelHeight: 53}
	connection.LabelPosition = "INSIDE_MIDDLE_CENTER"
	connection.Route = geo.Route{geo.NewPoint(0, 100), geo.NewPoint(200, 100)}
	topLeft := connection.GetLabelTopLeft()
	if topLeft.X != 72 || topLeft.Y != 74 {
		t.Fatalf("GetLabelTopLeft=%v, want (72,74)", topLeft)
	}
	pad := int64(0)
	document, err := Build(context.Background(), diagram, Options{Pad: &pad})
	if err != nil {
		t.Fatal(err)
	}
	lines := []map[string]any{}
	for index := 0; index < 3; index++ {
		run := findSceneNode(t, document.Root, fmt.Sprintf("a-b:label:%d", index)).Primitive.(d2scene.TextRun)
		expectedY := float64(74+16) + float64(index)*(float64(53)/3)
		if math.Float64bits(run.Origin.Y) != math.Float64bits(expectedY) {
			t.Fatalf("line%d: originY%016x want%016x", index, math.Float64bits(run.Origin.Y), math.Float64bits(expectedY))
		}
		lines = append(lines, map[string]any{"index": index, "text": run.Text, "origin": run.Origin,
			"x_bits": fmt.Sprintf("%016x", math.Float64bits(run.Origin.X)), "y_bits": fmt.Sprintf("%016x", math.Float64bits(run.Origin.Y))})
	}
	report := map[string]any{"scope": "Original GetLabelTopLeft plus actual scene text origins for the Rust precision regression route", "top_left": topLeft, "lines": lines}
	encoded, err := json.MarshalIndent(report, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if output := os.Getenv("D2_TEXT_ORIGIN_WITNESS_OUTPUT"); output != "" {
		if err := os.WriteFile(output, append(encoded, '\n'), 0644); err != nil {
			t.Fatal(err)
		}
	}
	t.Log(string(encoded))
}
