// SPDX-License-Identifier: MPL-2.0
// parsePathWithCount over path data x byte/command limits
// (recipe mathtext-svg-path; requests from scripts/fixture-gen-svg.py path).
package d2svgimport

import (
	"context"
	"testing"
)

func TestD2RSSvgPath(t *testing.T) {
	out := d2rsOutput(t)
	type row struct {
		Data        string  `json:"data"`
		MaxBytes    int     `json:"max_bytes"`
		MaxCommands int     `json:"max_commands"`
		Commands    any     `json:"commands"`
		Count       *int    `json:"count"`
		Error       *string `json:"error"`
	}
	var rows []row
	d2rsReadInput(t, &rows)
	for n := range rows {
		r := &rows[n]
		commands, count, err := parsePathWithCount(context.Background(), "math", r.Data, PathLimits{r.MaxBytes, r.MaxCommands})
		if err != nil {
			s := err.Error()
			r.Error = &s
			continue
		}
		r.Count = &count
		converted := make([]map[string]any, 0, len(commands))
		for _, c := range commands {
			converted = append(converted, map[string]any{"kind": c.Kind, "points": [3][2]float64{{c.P1.X, c.P1.Y}, {c.P2.X, c.P2.Y}, {c.P3.X, c.P3.Y}},
				"radii": [2]float64{c.RadiusX, c.RadiusY}, "rotation": c.Rotation, "large_arc": c.LargeArc, "sweep": c.Sweep,
				"bits": d2rsCommandBits(c.P1.X, c.P1.Y, c.P2.X, c.P2.Y, c.P3.X, c.P3.Y, c.RadiusX, c.RadiusY, c.Rotation)})
		}
		r.Commands = converted
	}
	d2rsWrite(t, out, rows)
}
