// SPDX-License-Identifier: MPL-2.0
// compileElement admission (attributes, boilerplate, classes, declarations,
// transforms, nested viewports) stopped before geometry
// (recipe mathtext-svg-element; needs tools/patches/mathtext-svgimport-admission.patch).
package d2svgimport

import (
	"context"
	"sort"
	"testing"
)

func TestD2RSSvgElement(t *testing.T) {
	out := d2rsOutput(t)
	names := []string{"svg", "g", "defs", "style", "title", "metadata", "rdf:RDF", "cc:Work", "dc:format", "dc:type", "sodipodi:namedview", "path", "rect",
		"circle", "ellipse", "line", "polyline", "polygon", "image", "text", "use", "clipPath", "linearGradient", "stop", "unknown"}
	attrs := []map[string]string{{}, {"id": "p"}, {"unknown": "x"}, {"class": "a a b"}, {"class": "a", "style": "fill:blue"}, {"style": "fill:red; bad:x"},
		{"transform": "translate(2,3)"}, {"transform": "translate(2,3) scale(4)"}, {"transform": "bad("}, {"transform": "none"}, {"stop-color": "red"},
		{"clip-path": "none"}, {"version": "bad", "x": "1"}, {"version": "1.1", "x": "0px", "y": "-0"}, {"x": "1ex"}, {"enable-background": "new 0 0 2 3"},
		{"enable-background": "bad"}, {"style": "vertical-align:-0.5ex; fill:red"}, {"style": "vertical-align:-0.5ex; bad:x"},
		{"viewBox": "0 0 20 30", "width": "10", "height": "20", "x": "2pt", "y": "3"}, {"viewBox": "bad", "width": "bad"},
		{"offset": "0.5", "stop-color": "blue", "stop-opacity": "0.5"},
		{"id": "g", "x1": "0", "y1": "0", "x2": "10", "y2": "0", "gradientUnits": "userSpaceOnUse", "gradientTransform": "translate(2,3)"}}
	rows := []map[string]any{}
	for _, name := range names {
		for _, a := range attrs {
			roots := []bool{false}
			if name == "svg" {
				roots = append(roots, true)
			}
			for _, root := range roots {
				for _, child := range []string{"absent", "title", "path"} {
					for _, used := range []int{0, 2} {
						order := []string{}
						for key := range a {
							order = append(order, key)
						}
						sort.Strings(order)
						e := &svgElement{name: name, isRoot: root, attrs: a, attrOrder: order}
						if child != "absent" {
							e.children = []*svgElement{{name: child}}
						}
						rules := []stylesheetRule{{class: "a", declarations: map[string]string{"fill": "red"}}}
						i := svgImporter{ctx: context.Background(), source: "math", d2rsElementAdmissionOnly: true, parsedTransforms: used,
							stylesheetRules: rules, limits: Limits{MaxAttributeBytes: 10000, MaxTransformFunctions: 2}}
						err := i.compileElement(e)
						classes := e.classes
						if classes == nil {
							classes = []string{}
						}
						declarations := e.declarations
						if declarations == nil {
							declarations = map[string]string{}
						}
						continuation := ""
						if err == nil {
							continuation = "complete"
							if i.d2rsElementAdmissionReached {
								continuation = "geometry"
							}
						}
						rows = append(rows, map[string]any{"name": name, "attrs": a, "order": order, "root": root, "child": child, "used": used,
							"error": d2rsError(err), "classes": classes, "declarations": declarations,
							"state":    map[string]any{"transform": d2rsMatrix(e.transform), "viewport": d2rsViewport(e)},
							"gradient": d2rsGradient(e.gradient), "stop": d2rsGradientStop(e.gradientStop), "after_transforms": i.parsedTransforms,
							"mathjax": i.mathJaxRoot == e, "continuation": continuation})
					}
				}
			}
		}
	}
	d2rsWrite(t, out, rows)
}
