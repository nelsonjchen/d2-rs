// SPDX-License-Identifier: MPL-2.0
// computeStyle over single and paired presentation declarations x parent
// style (recipe mathtext-svg-style). Properties are visited in a fixed order
// (the v0.9.0 capture iterated a Go map, so its row order was random).
package d2svgimport

import (
	"context"
	"image/color"
	"strings"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
)

func TestD2RSSvgStyle(t *testing.T) {
	out := d2rsOutput(t)
	declarations := []map[string]string{{}}
	props := map[string][]string{"color": {"red", "none", "inherit", "currentColor", "bad!"},
		"fill":              {"red", "none", "currentColor", "inherit", "url(#g)", "URL(#g)", "url(#missing)", "url(#c)", "bad!"},
		"stroke":            {"blue", "inherit", "none", "currentColor", "url(#g)", "bad!"},
		"fill-opacity":      {"0", ".5", "1", "inherit", "-1", "2", "10%"},
		"stroke-opacity":    {"0", ".5", "1", "inherit", "-1", "2"},
		"stroke-width":      {"0", "-0", "2mm", "inherit", "-1", "bad", "1%"},
		"stroke-linecap":    {"inherit", "butt", "ROUND", "square", " round ", "bad", strings.Repeat("x", 65)},
		"stroke-linejoin":   {"inherit", "miter", "round", "bevel", "bad"},
		"stroke-miterlimit": {"inherit", "1", "4", "0", "-1", "1px", "bad"},
		"stroke-dasharray":  {"inherit", "none", "1 2 3", "0 0", "0 1", "bad"},
		"stroke-dashoffset": {"inherit", "-2mm", "0", "10%", "bad"},
		"fill-rule":         {"inherit", "nonzero", "evenodd", "bad"},
		"clip-rule":         {"inherit", "nonzero", "evenodd", "bad"},
		"clip-path":         {"none", "inherit", "url(#c)", "url(#g)", "url(#missing)", "url(#🙂)", "bad"},
		"opacity":           {"inherit", "0", ".5", "1", "2", "10%", "bad"},
		"display":           {"inherit", "none", "inline", "block", "bad"},
		"visibility":        {"inherit", "visible", "hidden", "collapse", "bad"}}
	order := []string{"color", "fill", "stroke", "fill-opacity", "stroke-opacity", "stroke-width", "stroke-linecap", "stroke-linejoin", "stroke-miterlimit",
		"stroke-dasharray", "stroke-dashoffset", "fill-rule", "clip-rule", "clip-path", "opacity", "display", "visibility"}
	if len(order) != len(props) {
		t.Fatal("property order does not cover the property table")
	}
	for _, property := range order {
		for _, value := range props[property] {
			declarations = append(declarations, map[string]string{property: value})
		}
	}
	for n := 0; n < len(order); n++ {
		for j := n + 1; j < len(order); j++ {
			declarations = append(declarations, map[string]string{order[n]: "bad!", order[j]: "bad!"})
		}
	}
	rows := []map[string]any{}
	for parentKind := 0; parentKind < 2; parentKind++ {
		parent := defaultSVGStyle()
		if parentKind == 1 {
			parent.color = color.NRGBA{R: 1, G: 2, B: 3, A: 4}
			parent.fillCurrent, parent.strokeCurrent = true, true
			parent.fillOpacity, parent.strokeOpacity, parent.strokeWidth = .25, .75, 3
			parent.lineCap, parent.lineJoin = d2scene.CapSquare, d2scene.JoinBevel
			parent.dashes, parent.dashOffset = []float64{2, 3}, -1
			parent.fillRule, parent.clipRule, parent.clipPathID = d2scene.EvenOdd, d2scene.EvenOdd, "parent"
			parent.opacity, parent.display, parent.visible = .3, false, false
		}
		for _, values := range declarations {
			i := svgImporter{ctx: context.Background(), source: "math", ids: map[string]*svgElement{"g": {name: "linearGradient",
				gradient: &d2scene.LinearGradient{Start: d2scene.Point{}, End: d2scene.Point{X: 1}, Transform: d2scene.Identity(),
					Stops: []d2scene.GradientStop{{Offset: 0, Color: color.NRGBA{R: 255, A: 255}}, {Offset: 1, Color: color.NRGBA{B: 255, A: 255}}}}},
				"c": {name: "clipPath"}, "🙂": {name: "clipPath"}}}
			s, err := i.computeStyle(parent, &svgElement{name: "path", declarations: values})
			var output any
			if err == nil {
				output = d2rsStyle(s)
			}
			rows = append(rows, map[string]any{"parent": d2rsStyle(parent), "values": values, "style": output, "error": d2rsError(err)})
		}
	}
	d2rsWrite(t, out, rows)
}
