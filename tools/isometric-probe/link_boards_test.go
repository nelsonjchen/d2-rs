// SPDX-License-Identifier: MPL-2.0
// Whole-image observations of original link-destination and outside-caption inputs.
package d2isometricimg

import (
	"context"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2isometric"
	"github.com/d2lang/d2/d2renderers/d2scene"
	"github.com/d2lang/d2/d2target"
)

func d2rsObserveLinkBoard(t *testing.T, d *d2target.Diagram) {
	t.Helper()
	for _, format := range []Format{SVG, PNG} {
		data, err := Render(context.Background(), d, &Options{Format: format, Width: 320, Height: 200})
		if err != nil || len(data) == 0 {
			t.Fatalf("public link board %s: %v", format, err)
		}
	}
	p, err := RenderPage(context.Background(), d, &Options{Width: 320, Height: 200})
	if err != nil || p == nil || len(p.Links) != 1 {
		t.Fatalf("public link page: %v, %v", p, err)
	}
}

func d2rsLinkDestinationBoard(t *testing.T, link, tooltip string) d2scene.LinkRegion {
	t.Helper()
	d := d2target.NewDiagram()
	s := d2target.BaseShape()
	s.ID, s.Type, s.Width, s.Height = "destination", d2target.ShapeRectangle, 180, 100
	s.Label, s.LabelWidth, s.LabelHeight, s.FontSize = "Destination", 100, 20, 16
	s.LabelPosition, s.Link, s.Tooltip = "INSIDE_MIDDLE_CENTER", link, tooltip
	d.Shapes = []d2target.Shape{*s}
	d2rsObserveLinkBoard(t, d)
	return nativeLinkDestination(link, tooltip)
}

func d2rsOutsideLinkBoard(t *testing.T, d *d2target.Diagram, o *d2isometric.RenderOpts) (*d2isometric.Scene, error) {
	t.Helper()
	d2rsObserveLinkBoard(t, d)
	return d2isometric.BuildScene(d, o)
}
