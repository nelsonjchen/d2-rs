// SPDX-License-Identifier: MPL-2.0
// Public PNG and page deadline witnesses reuse existing golden-board inputs.
package d2isometricimg

import (
	"context"
	"errors"
	"testing"
	"time"
)

func TestD2rsPNGDeadlineBoards(t *testing.T) {
	for _, fixture := range []string{
		"stable/all_shapes/elk/board.exp.json",
		"stable/giant_markdown_test/dagre/board.exp.json",
		"patterns/paper/dagre/board.exp.json",
	} {
		t.Run(fixture, func(t *testing.T) {
			diagram := sourcePanelFixture(t, fixture)
			o := &Options{Format: PNG, Width: 320, Height: 200, FitContent: true, Timeout: time.Nanosecond}
			data, err := Render(context.Background(), diagram, o)
			if !errors.Is(err, context.DeadlineExceeded) || data != nil {
				t.Fatalf("deadline published PNG: %d bytes, %v", len(data), err)
			}
			page, err := RenderPage(context.Background(), diagram, o)
			if !errors.Is(err, context.DeadlineExceeded) || page != nil {
				t.Fatalf("deadline published page: %v, %v", page, err)
			}
		})
	}
}
