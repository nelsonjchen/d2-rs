// SPDX-License-Identifier: MPL-2.0
// Whole-image admission witnesses derived from the original container fixture.
package d2isometricimg

import (
	"context"
	"strings"
	"testing"
)

func TestD2rsContainerGradientAdmissionBoard(t *testing.T) {
	for _, tc := range []struct{ name, fill, problem string }{
		{"extra field", "linear-gradient(to right, #ff000080 25% extra, #0000ffff 100%)", "invalid color stop 0"},
		{"nonfinite position", "linear-gradient(to right, #ff000080 NaN, #0000ffff 100%)", "must be a finite number or percentage"},
		{"canonical positions", "linear-gradient(to right, #ff000080 +.5%, #0000ffff 1e2)", ""},
	} {
		t.Run(tc.name, func(t *testing.T) {
			d := sourcePanelFixture(t, "regression/cylinder_grid_label/elk/board.exp.json")
			found := false
			for i := range d.Shapes {
				if d.Shapes[i].ID == "container title is hidden" {
					d.Shapes[i].Fill = tc.fill
					// An authored dashed container is an organizational wash,
					// so it follows nativeGradientOpacity rather than a slab.
					d.Shapes[i].StrokeDash = 5
					found = true
				}
			}
			if !found {
				t.Fatal("missing original container")
			}
			for _, format := range []Format{SVG, PNG} {
				o := &Options{Format: format, Width: 320, Height: 200}
				data, err := Render(context.Background(), d, o)
				if tc.problem == "" {
					if err != nil || len(data) == 0 {
						t.Fatalf("valid gradient: %v", err)
					}
				} else if err == nil || data != nil || !strings.Contains(err.Error(), tc.problem) {
					t.Fatalf("malformed gradient published image: %d, %v", len(data), err)
				}
			}
			p, err := RenderPage(context.Background(), d, &Options{Width: 320, Height: 200})
			if tc.problem == "" {
				if err != nil || p == nil {
					t.Fatalf("valid gradient page: %v", err)
				}
			} else if err == nil || p != nil || !strings.Contains(err.Error(), tc.problem) {
				t.Fatalf("malformed gradient published page: %v", err)
			}
		})
	}
}
