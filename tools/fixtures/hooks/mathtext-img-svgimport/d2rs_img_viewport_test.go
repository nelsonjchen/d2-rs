// SPDX-License-Identifier: MPL-2.0
package d2svgimport

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"math"
	"os"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
)

// TestD2RSViewport: root and nested <svg> viewport admission (viewBox,
// width/height, preserveAspectRatio, x/y) and d2scene.AspectRatioMatrix with
// exact matrix bits (svg-viewport-master.json). Inputs are built here.
func TestD2RSViewport(t *testing.T) {
	out := d2rsImgEnv(t)
	rows := []map[string]any{}
	bits := func(m d2scene.Matrix) []string {
		out := []string{}
		for _, v := range []float64{m.A, m.B, m.C, m.D, m.E, m.F} {
			out = append(out, fmt.Sprintf("%016x", math.Float64bits(v)))
		}
		return out
	}
	capture := func(attrs map[string]string, nested, mathjax bool) {
		e := &svgElement{name: "svg", attrs: attrs}
		i := svgImporter{source: "math", ctx: context.Background()}
		if mathjax {
			i.mathJaxRoot = e
		}
		var viewport svgViewport
		var x, y float64
		var err error
		if nested {
			viewport, x, y, err = i.nestedViewport(e)
		} else {
			viewport, err = i.viewport(e)
		}
		message := ""
		var output, matrixBits any
		if err != nil {
			message = err.Error()
		} else {
			output = map[string]any{
				"view_box":  [4]float64{viewport.viewBox.X, viewport.viewBox.Y, viewport.viewBox.Width, viewport.viewBox.Height},
				"width":     viewport.width,
				"height":    viewport.height,
				"aspect":    [2]uint8{uint8(viewport.aspect.Align), uint8(viewport.aspect.Fit)},
				"transform": d2rsImgMatrix(viewport.transform),
				"position":  [2]float64{x, y},
			}
			matrixBits = bits(viewport.transform)
		}
		rows = append(rows, map[string]any{"operation": "viewport", "attrs": attrs, "nested": nested, "mathjax": mathjax,
			"error": message, "output": output, "matrix_bits": matrixBits})
	}
	for _, vb := range []string{"absent", "0 0 20 30", "bad", "0 0 0 2", "1e308 -1e308 1e-308 1e308", "1 2 3 4 5"} {
		for _, w := range []string{"absent", "0", "-1", "10", "2ex", "1e308", "1e-308", "bad", "50%"} {
			for _, h := range []string{"absent", "0", "-1", "10", "2ex", "1e308", "1e-308", "bad", "50%"} {
				for _, nested := range []bool{false, true} {
					for _, mathjax := range []bool{false, true} {
						attrs := map[string]string{}
						if vb != "absent" {
							attrs["viewBox"] = vb
						}
						if w != "absent" {
							attrs["width"] = w
						}
						if h != "absent" {
							attrs["height"] = h
						}
						capture(attrs, nested, mathjax)
					}
				}
			}
		}
	}
	for _, policy := range []string{"", "none", "none slice", "xMinYMin", "xMidYMin slice", "xMaxYMin", "xMinYMid",
		"xMidYMid", "xMaxYMid slice", "xMinYMax", "xMidYMax", "xMaxYMax", "bad", "defer xMidYMid", "xMidYMid bad",
		"xMidYMid meet extra", "none\u00a0slice"} {
		for _, nested := range []bool{false, true} {
			capture(map[string]string{"viewBox": "1.1 -.3 9.2 3.7", "width": "31", "height": "71", "x": "4pt", "y": "-2cm",
				"preserveAspectRatio": policy}, nested, false)
		}
	}
	for _, position := range []string{"bad", "50%", "2ex", "1e308", "-10"} {
		capture(map[string]string{"viewBox": "0 0 20 30", "width": "31", "height": "71", "x": position}, true, true)
	}
	for _, src := range []d2scene.Box{{X: 1.1, Y: -.3, Width: 9.2, Height: 3.7}, {Width: 0, Height: 1}, {X: 1e308, Width: 1e-308, Height: 1e308}} {
		for _, dst := range []d2scene.Box{{X: 2.1, Y: -4.3, Width: 31, Height: 71}, {}, {Width: -1, Height: 1}} {
			for align := 0; align <= 10; align++ {
				for fit := 0; fit <= 2; fit++ {
					matrix, err := d2scene.AspectRatioMatrix(src, dst, d2scene.AspectRatio{Align: d2scene.AspectAlign(align), Fit: d2scene.AspectFit(fit)})
					message := ""
					var matrixBits any
					if err != nil {
						message = err.Error()
					} else {
						matrixBits = bits(matrix)
					}
					rows = append(rows, map[string]any{"operation": "matrix", "source": [4]float64{src.X, src.Y, src.Width, src.Height},
						"destination": [4]float64{dst.X, dst.Y, dst.Width, dst.Height}, "aspect": [2]int{align, fit},
						"error": message, "matrix_bits": matrixBits})
				}
			}
		}
	}
	var buf bytes.Buffer
	if err := json.NewEncoder(&buf).Encode(rows); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, buf.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}
