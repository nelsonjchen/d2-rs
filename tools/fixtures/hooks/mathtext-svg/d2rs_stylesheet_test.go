// SPDX-License-Identifier: MPL-2.0
// <style> stylesheet parsing x context (recipe mathtext-svg-stylesheet).
package d2svgimport

import (
	"context"
	"encoding/json"
	"os"
	"strings"
	"testing"
	"time"
)

func TestD2RSStylesheet(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	rows := []map[string]any{}
	values := []string{"text/css", " TEXT/CSS\t", "text/css\u00a0", " a a b\t a \u00a0c\u00a0", "", " ", ".a{}", ".a {fill:red}", ".a{fill:red;fill:blue}.a{stroke:black}", ".a{} .b{}", "@import x", "@x{}", "a{}", "#a{}", ".{}", ".1a{}", "._a-2{}", ".a.b{}", ".a,.b{}", ".a b{}", ".🙂{}", ".a", "}", ".a}{}", ".a{{}}", ".a{fill:red", ".a{fill}", ".a{:red}", ".a{fill:}", ".a{font-family:serif}", ".a{transform:none}", ".a{fill:red!important}", ".a{fill:r!ed}", ".a{fill:red}/*comment*/", ".a{x:x}*/", ".a{ FILL : blue ;}", ".a{İ:x}", ".a{K:x}", ".a{vertical-align:1ex}", ".a{enable-background:new 0 0 1 1}", ".a{\u00a0fill:red}", ".a{fill:\u00a0red}", ".a{" + strings.Repeat("x", 65) + ":red}", strings.Repeat(" ", 5000) + ".a{fill:red}"}
	for _, operation := range []string{"parse", "classes", "element"} {
		for _, mode := range []string{"", "cancelled", "deadline"} {
			for _, value := range values {
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
				i := svgImporter{ctx: ctx, source: "math"}
				var e error
				var output any
				if operation == "parse" {
					rules, err := i.parseStylesheet(value)
					e = err
					if e == nil {
						projected := []map[string]any{}
						for _, r := range rules {
							projected = append(projected, map[string]any{"class": r.class, "declarations": r.declarations})
						}
						output = projected
					}
				}
				if operation == "classes" {
					classes, _, err := i.classTokens(&svgElement{attrs: map[string]string{"class": value}})
					e = err
					if e == nil {
						output = append([]string{}, classes...)
					}
				}
				if operation == "element" {
					e = i.validateStyleElement(&svgElement{attrs: map[string]string{"type": value}, attrOrder: []string{"type"}})
				}
				message := ""
				if e != nil {
					message = e.Error()
				}
				rows = append(rows, map[string]any{"operation": operation, "cancel": mode, "value": value, "output": output, "error": message})
			}
		}
	}
	for _, order := range [][]string{{}, {"id"}, {"type", "id"}, {strings.Repeat("x", 65)}} {
		for _, children := range []int{0, 1} {
			element := &svgElement{attrs: map[string]string{"type": "text/css"}, attrOrder: order}
			if children == 1 {
				element.children = []*svgElement{{name: "g"}}
			}
			i := svgImporter{ctx: context.Background(), source: "math"}
			e := i.validateStyleElement(element)
			message := ""
			if e != nil {
				message = e.Error()
			}
			rows = append(rows, map[string]any{"operation": "order", "order": order, "children": children, "error": message})
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
