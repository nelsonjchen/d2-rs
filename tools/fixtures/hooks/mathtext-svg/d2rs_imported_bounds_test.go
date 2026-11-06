// SPDX-License-Identifier: MPL-2.0
// Imported scene bounds x scale x context (recipe mathtext-svg-imported-bounds).
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

func TestD2RSImportedBounds(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	rows := []map[string]any{}
	for _, mode := range []string{"nil", "group", "rect", "transparent-rect", "overflow-rect", "child-overflow", "clip", "clip-empty", "clip-rule", "path", "path-overflow"} {
		for _, scale := range []float64{1, 1e308, 1e-308} {
			for _, clipScale := range []float64{1, 1e308} {
				for _, cancel := range []string{"", "cancelled", "deadline"} {
					root := d2scene.NewNode(nil)
					root.Transform = d2scene.Scale(scale, scale)
					root.Opacity = 0
					switch mode {
					case "nil":
						root = nil
					case "rect", "transparent-rect", "overflow-rect":
						alpha := uint8(255)
						if mode == "transparent-rect" {
							alpha = 0
						}
						x, w := 1., 2.
						if mode == "overflow-rect" {
							x, w = 1e308, 1e308
						}
						root.Primitive = d2scene.Rect{Box: d2scene.Box{X: x, Y: 2, Width: w, Height: 3}, Fill: d2scene.SolidPaint{Color: color.NRGBA{A: alpha}}}
					case "child-overflow":
						child := d2scene.NewNode(nil)
						child.Transform = d2scene.Scale(1e308, 1e308)
						root.Children = []*d2scene.Node{child}
					case "clip", "clip-empty", "clip-rule":
						commands := []d2scene.PathCommand{d2scene.MoveTo(0, 0), d2scene.LineTo(2, 3)}
						if mode == "clip-empty" {
							commands = nil
						}
						rule := d2scene.NonZero
						if mode == "clip-rule" {
							rule = d2scene.FillRule(2)
						}
						root.Clip = &d2scene.Clip{Path: d2scene.Path{Commands: commands, FillRule: rule}, Transform: d2scene.Scale(clipScale, clipScale)}
					case "path", "path-overflow":
						x := 2.
						if mode == "path-overflow" {
							x = 1e308
						}
						root.Primitive = d2scene.Path{Commands: []d2scene.PathCommand{d2scene.MoveTo(0, 0), d2scene.LineTo(x, 3)}, Fill: d2scene.SolidPaint{Color: color.NRGBA{A: 255}}}
					}
					ctx := context.Background()
					if cancel == "cancelled" {
						var c context.CancelFunc
						ctx, c = context.WithCancel(ctx)
						c()
					} else if cancel == "deadline" {
						var c context.CancelFunc
						ctx, c = context.WithDeadline(ctx, time.Unix(0, 0))
						defer c()
					}
					err := validateImportedBounds(ctx, root, d2scene.Identity())
					message := ""
					if err != nil {
						message = err.Error()
					}
					rows = append(rows, map[string]any{"mode": mode, "scale": scale, "clip_scale": clipScale, "cancel": cancel, "error": message})
				}
			}
		}
	}
	data, err := json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), data, 0644); err != nil {
		t.Fatal(err)
	}
}
