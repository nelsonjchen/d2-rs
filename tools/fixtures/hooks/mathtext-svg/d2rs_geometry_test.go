// SPDX-License-Identifier: MPL-2.0
// compileGeometry per shape element x attributes x command budget x context
// (recipe mathtext-svg-geometry; requests from scripts/fixture-gen-svg.py geometry).
package d2svgimport

import (
	"math"
	"testing"
)

func TestD2RSSvgGeometry(t *testing.T) {
	out := d2rsOutput(t)
	type row struct {
		Cancel   string            `json:"cancel"`
		Name     string            `json:"name"`
		Attrs    map[string]string `json:"attrs"`
		Maximum  int               `json:"maximum"`
		Used     int               `json:"used"`
		MaxBytes int               `json:"max_bytes"`
		After    int               `json:"after"`
		Geometry any               `json:"geometry"`
		Error    string            `json:"error"`
	}
	var rows []row
	d2rsReadInput(t, &rows)
	for n := range rows {
		r := &rows[n]
		i := svgImporter{ctx: d2rsContext(t, r.Cancel), source: "math", parsedPathCommand: r.Used, limits: Limits{MaxBytes: r.MaxBytes, MaxPathCommands: r.Maximum}}
		g, err := i.compileGeometry(&svgElement{name: r.Name, attrs: r.Attrs})
		r.After = i.parsedPathCommand
		if err != nil {
			r.Error = err.Error()
			continue
		}
		commands := []map[string]any{}
		for _, c := range g.commands {
			commands = append(commands, map[string]any{"kind": c.Kind, "points": [3][2]float64{{c.P1.X, c.P1.Y}, {c.P2.X, c.P2.Y}, {c.P3.X, c.P3.Y}},
				"radii": [2]float64{c.RadiusX, c.RadiusY}, "rotation": c.Rotation, "large_arc": c.LargeArc, "sweep": c.Sweep,
				"bits": d2rsCommandBits(c.P1.X, c.P1.Y, c.P2.X, c.P2.Y, c.P3.X, c.P3.Y, c.RadiusX, c.RadiusY, c.Rotation)})
		}
		values := [8]float64{g.box.X, g.box.Y, g.box.Width, g.box.Height, g.center.X, g.center.Y, g.radiusX, g.radiusY}
		bits := [8]uint64{}
		for j := range values {
			bits[j] = math.Float64bits(values[j])
		}
		r.Geometry = map[string]any{"kind": g.kind, "bounds": [4]float64{g.box.X, g.box.Y, g.box.Width, g.box.Height}, "center": [2]float64{g.center.X, g.center.Y},
			"radii": [2]float64{g.radiusX, g.radiusY}, "commands": commands, "force_no_fill": g.forceNoFill, "bits": bits}
	}
	d2rsWrite(t, out, rows)
}
