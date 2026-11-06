// SPDX-License-Identifier: MPL-2.0
// Gradient stop CSS rules (recipe mathtext-svg-stop-rules).
package d2svgimport

import (
	"context"
	"encoding/json"
	"os"
	"testing"
)

func TestD2RSStopRules(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	rows := []map[string]any{}
	maps := []map[string]string{{}, {"fill": "bad!"}, {"fill": "red"}, {"fill": "url(#missing)"}, {"stop-color": "red", "fill": "blue"}, {"stop-opacity": ".5", "opacity": "1"}}
	for _, color := range []string{"absent", "red", "transparent", "#11223380", "inherit", "currentColor", "url(#g)", "URL(#g)", "bad!"} {
		for _, op := range []string{"absent", "0", ".5", "1", "inherit", "2", "50%", "bad!"} {
			values := map[string]string{}
			if color != "absent" {
				values["stop-color"] = color
			}
			if op != "absent" {
				values["stop-opacity"] = op
			}
			maps = append(maps, values)
		}
	}
	for _, values := range maps {
		for _, operation := range []string{"stop", "rules"} {
			i := svgImporter{ctx: context.Background(), source: "math", ids: map[string]*svgElement{}}
			var e error
			var output any
			if operation == "stop" {
				stop, err := i.stopFromDeclarations(&svgElement{name: "stop", declarations: values})
				e = err
				if e == nil {
					output = map[string]any{"offset": stop.Offset, "color": [4]uint8{stop.Color.R, stop.Color.G, stop.Color.B, stop.Color.A}}
				}
			} else {
				i.stylesheetRules = []stylesheetRule{{class: "unused", declarations: values}}
				e = i.validateStylesheetRules()
			}
			message := ""
			if e != nil {
				message = e.Error()
			}
			rows = append(rows, map[string]any{"operation": operation, "values": values, "output": output, "error": message})
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
