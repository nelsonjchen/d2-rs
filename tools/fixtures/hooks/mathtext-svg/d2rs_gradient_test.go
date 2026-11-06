// SPDX-License-Identifier: MPL-2.0
// compileLinearGradient over single-attribute edits x transform budget
// (recipe mathtext-svg-gradient).
package d2svgimport

import (
	"context"
	"testing"
)

func TestD2RSSvgGradient(t *testing.T) {
	out := d2rsOutput(t)
	rows := []map[string]any{}
	attrs := []map[string]string{{}, {"x1": "0"}, {"x1": "0", "y1": "0", "x2": "1", "y2": "0"}}
	base := map[string]string{"x1": "0", "y1": "0", "x2": "1", "y2": "0", "gradientUnits": "userSpaceOnUse"}
	attrs = append(attrs, base)
	for _, name := range []string{"x1", "y1", "x2", "y2", "gradientUnits", "spreadMethod", "gradientTransform"} {
		for _, value := range []string{"", "bad", "1mm", "-1", "50%", "pad", "reflect", "userSpaceOnUse", " userSpaceOnUse ", "translate(2 3)", "rotate(30)",
			"scale(0)", "scale(1e300,1e-300)", "scale(1e-300,1e-300)", "scale(1e300,1e300)", "matrix(1 1 1 1 0 0)", "scale(1) scale(2)"} {
			a := map[string]string{}
			for k, v := range base {
				a[k] = v
			}
			a[name] = value
			attrs = append(attrs, a)
		}
	}
	for _, a := range attrs {
		for _, maximum := range []int{0, 1, 10} {
			for _, used := range []int{0, 1} {
				i := svgImporter{ctx: context.Background(), source: "math", parsedTransforms: used, limits: Limits{MaxAttributeBytes: 10000, MaxTransformFunctions: maximum}}
				el := &svgElement{name: "linearGradient", attrs: a}
				err := i.compileLinearGradient(el)
				rows = append(rows, map[string]any{"attrs": a, "maximum": maximum, "used": used, "after": i.parsedTransforms,
					"paint": d2rsGradient(el.gradient), "error": d2rsError(err)})
			}
		}
	}
	d2rsWrite(t, out, rows)
}
