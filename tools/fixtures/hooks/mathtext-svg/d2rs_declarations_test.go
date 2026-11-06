// SPDX-License-Identifier: MPL-2.0
// declarationsFor: presentation attributes, inline style and class rules
// (recipe mathtext-svg-declarations).
package d2svgimport

import (
	"context"
	"strings"
	"testing"
)

func TestD2RSSvgDeclarations(t *testing.T) {
	out := d2rsOutput(t)
	rows := []map[string]any{}
	inline := []string{"", ";", "fill:red", " FILL : blue ;stroke : green ", "fill:red;fill:blue", "fill", "fill:", ":red", "fill red", "fill1:red",
		"font-family:serif", "fill:red!important", "fill:!IMPORTANT", "fill:important", "transform:scale(1)", "fill:{red}", "fill:red/*x*/", "fill:red*/",
		"vertical-align:1ex", "vertical-align:1 ex", "vertical-align:1e308ex", "vertical-align:bad", "vertical-align:1ex;unknown:x",
		"enable-background:new 0 0 1 2", "enable-background:NEW -1 2 3 4", "enable-background:new 0,0 1 2", "enable-background:new 0 0 0 2",
		"enable-background:new 0 0 1", "enable-background:accumulate", "İ:x", "\u212a:x", "\u00a0fill:red", "fill:\u00a0red\u00a0",
		strings.Repeat("x", 65) + ":a", strings.Repeat(" ", 5000) + "fill:blue"}
	for _, name := range []string{"svg", "g", "path"} {
		for _, root := range []bool{false, true} {
			for _, value := range inline {
				for _, hasRule := range []bool{false, true} {
					attrs := map[string]string{"fill": " red ", "stroke": " black ", "style": value, "class": "a"}
					element := &svgElement{name: name, isRoot: root, attrs: attrs, classes: []string{"a"}, classSet: map[string]struct{}{"a": {}}}
					i := svgImporter{ctx: context.Background(), source: "math"}
					rules := []map[string]any{}
					if hasRule {
						i.stylesheetRules = []stylesheetRule{{class: "a", declarations: map[string]string{"fill": "green", "stroke": "white"}},
							{class: "b", declarations: map[string]string{"fill": "pink"}}, {class: "a", declarations: map[string]string{"stroke": "yellow"}}}
						for _, r := range i.stylesheetRules {
							rules = append(rules, map[string]any{"class": r.class, "declarations": r.declarations})
						}
					}
					d, err := i.declarationsFor(element)
					rows = append(rows, map[string]any{"name": name, "root": root, "attrs": attrs, "classes": element.classes, "rules": rules,
						"declarations": d, "mathjax": i.mathJaxRoot == element, "error": d2rsError(err)})
				}
			}
		}
	}
	d2rsWrite(t, out, rows)
}
