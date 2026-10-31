// SPDX-License-Identifier: MPL-2.0
// compileClipPathResource over clip templates x use depth x resource and
// command limits (recipe mathtext-svg-clip-compile).
package d2svgimport

import (
	"context"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
)

func TestD2RSSvgClipCompile(t *testing.T) {
	out := d2rsOutput(t)
	rows := []map[string]any{}
	modes := []string{"direct", "use", "chain", "cycle", "unknown", "unsupported", "root-property", "child-property", "bad-rule",
		"overflow-transform", "overflow-geometry", "empty"}
	for _, mode := range modes {
		for _, depth := range []int{0, 1, 4} {
			for _, resources := range []int{0, 1, 4} {
				for _, commands := range []int{0, 1, 20} {
					root := &svgElement{name: "clipPath", id: "c", transform: d2scene.Identity(), declarations: map[string]string{"clip-rule": "evenodd"}}
					child := &svgElement{name: "rect", id: "p", transform: d2scene.Translate(2, 3), geometry: svgGeometry{kind: geometryRect, box: d2scene.Box{X: 1, Y: 2, Width: 10, Height: 20}}}
					use := &svgElement{name: "use", id: "u", href: "p", useX: 5, useY: 7, transform: d2scene.Scale(2, 3), declarations: map[string]string{"clip-rule": "inherit"}}
					use2 := &svgElement{name: "use", id: "u2", href: "u", useX: 11, useY: 13, transform: d2scene.Translate(17, 19), declarations: map[string]string{"clip-rule": "nonzero"}}
					root.children = []*svgElement{child}
					elements := []*svgElement{root, child, use, use2}
					ids := map[string]*svgElement{"c": root, "p": child, "u": use, "u2": use2}
					switch mode {
					case "use":
						root.children = []*svgElement{use}
					case "chain":
						root.children = []*svgElement{use2}
					case "cycle":
						root.children = []*svgElement{use}
						use.href = "u"
					case "unknown":
						root.children = []*svgElement{use}
						use.href = "missing"
					case "unsupported":
						root.children = []*svgElement{use}
						child.name = "g"
					case "root-property":
						root.declarations = map[string]string{"fill": "bad"}
					case "child-property":
						child.declarations = map[string]string{"opacity": "bad"}
					case "bad-rule":
						child.declarations = map[string]string{"clip-rule": "bad"}
					case "overflow-transform":
						root.transform = d2scene.Scale(1e308, 1e308)
						child.transform = d2scene.Scale(1e308, 1e308)
					case "overflow-geometry":
						child.geometry.box = d2scene.Box{X: 1e308, Y: 1e308, Width: 1e308, Height: 1e308}
					case "empty":
						child.geometry.box.Width = 0
					}
					inputs := []map[string]any{}
					for _, e := range elements {
						children := []int{}
						for _, c := range e.children {
							for idx, v := range elements {
								if v == c {
									children = append(children, idx)
								}
							}
						}
						decl := e.declarations
						if decl == nil {
							decl = map[string]string{}
						}
						inputs = append(inputs, map[string]any{"name": e.name, "id": e.id, "children": children, "declarations": decl,
							"transform": d2rsMatrix(e.transform), "geometry": d2rsGeometry(e.geometry), "href": e.href, "use_position": [2]float64{e.useX, e.useY}})
					}
					i := svgImporter{ctx: context.Background(), source: "math", ids: ids, limits: Limits{MaxUseDepth: depth, MaxResources: resources, MaxPathCommands: commands}}
					compiled, err := i.compileClipPathResource(root)
					var output any
					if compiled != nil {
						output = map[string]any{"commands": d2rsCommands(compiled.path.Commands), "fill_rule": compiled.path.FillRule,
							"transform": d2rsMatrix(compiled.transform), "uses": compiled.useInstances}
					}
					// Overflow geometry stays accepted (clip paths have no paint, so
					// the bounds stage does not inspect generated coordinates), but
					// JSON cannot encode the infinities: keep acceptance evidence only.
					if mode == "overflow-geometry" && output != nil {
						output = nil
					}
					rows = append(rows, map[string]any{"mode": mode, "elements": inputs, "depth": depth, "resources": resources,
						"maximum_commands": commands, "output": output, "error": d2rsError(err)})
				}
			}
		}
	}
	d2rsWrite(t, out, rows)
}
