// SPDX-License-Identifier: MPL-2.0
package d2isometricimg

import (
	"context"
	"github.com/d2lang/d2/d2target"
	"strconv"
	"testing"
)

func frameTime(t float64) string { return strconv.FormatFloat(t, 'g', -1, 64) }

func TestD2rsGIFFrameBoards(t *testing.T) {
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
			if err := CaptureFrames(context.Background(), d, &Options{Width: 160, Height: 100}, []float64{0, .25, .5, 1, CycleSeconds}, true, func(_ int, p []byte) error {
				if len(p) == 0 {
					t.Fatal("empty frame")
				}
				return nil
			}); err != nil {
				t.Fatal(err)
			}
		})
	}
}
