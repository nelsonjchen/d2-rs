// SPDX-License-Identifier: MPL-2.0
// Source-backed public SVG deadline witnesses on existing golden boards.
package d2isometricimg

import (
	"context"
	"errors"
	"testing"
	"time"
)

func TestD2rsSVGDeadlineBoards(t *testing.T) {
	for _, fixture := range []string{
		"stable/all_shapes/elk/board.exp.json",
		"stable/giant_markdown_test/dagre/board.exp.json",
		"patterns/paper/dagre/board.exp.json",
	} {
		t.Run(fixture, func(t *testing.T) {
			diagram := sourcePanelFixture(t, fixture)
			data, err := Render(context.Background(), diagram, &Options{Format: SVG, Width: 1200, Height: 1200, FitContent: true, Timeout: time.Nanosecond})
			if !errors.Is(err, context.DeadlineExceeded) || data != nil {
				t.Fatalf("deadline published output: %d bytes, %v", len(data), err)
			}
		})
	}
}
