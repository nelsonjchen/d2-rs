// SPDX-License-Identifier: MPL-2.0
// Gradient stop collection x context (recipe mathtext-svg-gradient-stops).
package d2svgimport

import (
	"context"
	"encoding/json"
	"github.com/d2lang/d2/d2renderers/d2scene"
	"image/color"
	"os"
	"testing"
	"time"
)

func TestD2RSGradientStops(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	rows := []map[string]any{}
	sets := [][]*d2scene.GradientStop{{}, {nil}, {{Offset: 0, Color: color.NRGBA{R: 1, A: 255}}}, {{Offset: 0}, {Offset: 1}}, {{Offset: .5}, {Offset: .5}}, {{Offset: .8}, {Offset: .2}}, {{Offset: 0}, nil}}
	for _, stops := range sets {
		for _, mode := range []string{"", "cancelled", "deadline"} {
			ctx := context.Background()
			if mode == "cancelled" {
				var c context.CancelFunc
				ctx, c = context.WithCancel(ctx)
				c()
			} else if mode == "deadline" {
				var c context.CancelFunc
				ctx, c = context.WithDeadline(ctx, time.Unix(0, 0))
				defer c()
			}
			g := &d2scene.LinearGradient{Transform: d2scene.Identity(), Stops: []d2scene.GradientStop{{Offset: .5, Color: color.NRGBA{G: 2, A: 200}}}}
			root := &svgElement{name: "linearGradient", gradient: g}
			input := []any{}
			for _, s := range stops {
				root.children = append(root.children, &svgElement{name: "stop", gradientStop: s})
				if s == nil {
					input = append(input, nil)
				} else {
					input = append(input, map[string]any{"offset": s.Offset, "color": [4]uint8{s.Color.R, s.Color.G, s.Color.B, s.Color.A}})
				}
			}
			i := svgImporter{ctx: ctx, source: "math"}
			e := i.compileGradientResources(root)
			message := ""
			if e != nil {
				message = e.Error()
			}
			rows = append(rows, map[string]any{"stops": input, "cancel": mode, "after": d2rsPaint(*g), "error": message})
		}
	}
	data, e := json.MarshalIndent(rows, "", "  ")
	if e != nil {
		t.Fatal(e)
	}
	if e = os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), data, 0644); e != nil {
		t.Fatal(e)
	}
}
