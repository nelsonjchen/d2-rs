// SPDX-License-Identifier: MPL-2.0
// Gradient resource trees and <stop> element admission
// (recipes mathtext-svg-gradient-tree, mathtext-svg-stop-element).
package d2svgimport

import (
	"context"
	"encoding/json"
	"os"
	"testing"
)

func TestD2RSGradientTree(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	rows := []map[string]any{}
	for _, parent := range []string{"svg", "defs", "g", "linearGradient"} {
		for _, id := range []string{"", "g"} {
			for _, child := range []string{"absent", "stop", "path"} {
				root := &svgElement{name: parent}
				grad := &svgElement{name: "linearGradient", id: id}
				root.children = []*svgElement{grad}
				if child != "absent" {
					grad.children = []*svgElement{{name: child}}
				}
				i := svgImporter{ctx: context.Background(), source: "math"}
				e := i.validateGradientTree(root)
				message := ""
				if e != nil {
					message = e.Error()
				}
				rows = append(rows, map[string]any{"parent": parent, "id": id, "child": child, "error": message})
			}
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
func TestD2RSGradientStopElement(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	rows := []map[string]any{}
	maps := []map[string]string{{}, {"offset": "0"}, {"offset": ".5", "stop-color": "red", "stop-opacity": ".5"}, {"offset": "2", "stop-color": "bad!"}, {"offset": "bad", "style": "fill:red"}, {"offset": ".5", "style": "stop-color:blue;stop-opacity:.25"}, {"offset": ".5", "style": "stroke:red"}, {"offset": ".5", "id": "x"}, {"offset": ".5", "class": "a a b", "style": "stop-color:green"}, {"offset": "1%"}, {"offset": "1", "stop-color": "inherit"}}
	for _, attrs := range maps {
		for _, ruleMode := range []bool{false, true} {
			order := []string{}
			for _, name := range []string{"offset", "id", "class", "style", "stop-color", "stop-opacity"} {
				if _, ok := attrs[name]; ok {
					order = append(order, name)
				}
			}
			el := &svgElement{name: "stop", attrs: attrs, attrOrder: order}
			i := svgImporter{ctx: context.Background(), source: "math"}
			rules := []map[string]any{}
			if ruleMode {
				i.stylesheetRules = []stylesheetRule{{class: "a", declarations: map[string]string{"stop-color": "red"}}}
				rules = append(rules, map[string]any{"class": "a", "declarations": map[string]string{"stop-color": "red"}})
			}
			e := i.compileGradientElement(el)
			message := ""
			if e != nil {
				message = e.Error()
			}
			var stop any
			if el.gradientStop != nil {
				s := el.gradientStop
				stop = map[string]any{"offset": s.Offset, "color": [4]uint8{s.Color.R, s.Color.G, s.Color.B, s.Color.A}}
			}
			classes := append([]string{}, el.classes...)
			rows = append(rows, map[string]any{"attrs": attrs, "order": order, "rules": rules, "classes": classes, "declarations": el.declarations, "stop": stop, "error": message})
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
