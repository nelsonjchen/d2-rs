// SPDX-License-Identifier: MPL-2.0
package d2isometricimg

import (
	"context"
	"github.com/d2lang/d2/d2target"
	"testing"
)

func TestD2rsGIFBoards(t *testing.T) {
	for _, name := range []string{"traffic", "bidirectional", "node", "static-route", "legend"} {
		t.Run(name, func(t *testing.T) {
			d := captureDiagram()
			switch name {
			case "bidirectional":
				d.Connections[0].SrcArrow = d2target.ArrowArrowhead
			case "node":
				d.Shapes[0].Animated = true
			case "static-route":
				d.Connections[0].Animated = false
			case "legend":
				d = sourcePanelFixture(t, "txtar/legend-mono/dagre/board.exp.json")
				c := sourcePanelFixture(t, "stable/sequence_diagram_groups/dagre/board.exp.json").Connections[0]
				c.Animated = true
				c.Label = "Message"
				d.Legend.Connections = []d2target.Connection{c}
			}
			data, err := Render(context.Background(), d, &Options{Format: GIF, Width: 160, Height: 100})
			if err != nil || len(data) == 0 {
				t.Fatalf("GIF %s: %v", name, err)
			}
		})
	}
}
