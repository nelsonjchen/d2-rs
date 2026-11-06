// SPDX-License-Identifier: MPL-2.0
// <use> x/y/width/height placement (recipe mathtext-svg-use-placement).
package d2svgimport

import (
	"context"
	"encoding/json"
	"math"
	"os"
	"strings"
	"testing"
)

func TestD2RSUsePlacement(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	type row struct {
		Attrs map[string]string `json:"attrs"`
		Bits  *[2]uint64        `json:"bits"`
		Error string            `json:"error"`
	}
	rows := []row{}
	for _, href := range []string{"#a", "", "http://example.com", "absent"} {
		for _, x := range []string{"absent", "1cm", "-0", "3ex", "100%", "1e999", strings.Repeat("1", 513), ""} {
			for _, y := range []string{"absent", "2mm", "bad"} {
				for _, overflow := range []string{"absent", "visible", " visible\t", "\u00a0visible", "Visible", "hidden", " \fvisible "} {
					attrs := map[string]string{}
					for name, value := range map[string]string{"href": href, "x": x, "y": y, "overflow": overflow} {
						if value != "absent" {
							attrs[name] = value
						}
					}
					i := svgImporter{ctx: context.Background(), source: "math", ids: map[string]*svgElement{"a": {name: "path"}}}
					element := &svgElement{name: "use", attrs: attrs}
					e := i.compileUse(element)
					r := row{Attrs: attrs}
					if e != nil {
						r.Error = e.Error()
					} else {
						v := [2]uint64{math.Float64bits(element.useX), math.Float64bits(element.useY)}
						r.Bits = &v
					}
					rows = append(rows, r)
				}
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
