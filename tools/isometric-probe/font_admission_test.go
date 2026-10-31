// SPDX-License-Identifier: MPL-2.0
// Whole-board witnesses for text.go's configured-family face admission.
package d2isometricimg

import (
	"context"
	"strings"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2fonts"
)

func TestD2rsConfiguredFontAdmissionBoards(t *testing.T) {
	for _, engine := range []string{"dagre", "elk", "tala"} {
		for _, source := range []string{"unicode/with-style", "txtar/legend-mono"} {
			t.Run(engine+"/"+source, func(t *testing.T) {
				d := sourcePanelFixture(t, source+"/"+engine+"/board.exp.json")
				// Preserve the source's DEFAULT/mono label roles. Only the
				// caller's configured family is unavailable in this process.
				missing := d2fonts.FontFamily("UnavailableConfiguredFace")
				if source == "txtar/legend-mono" {
					d.MonoFontFamily = &missing
				} else {
					d.FontFamily = &missing
				}
				for _, format := range []Format{SVG, PNG} {
					data, err := Render(context.Background(), d, &Options{Format: format, Width: 320, Height: 200})
					if err == nil || data != nil || !strings.Contains(err.Error(), "isometric text font UnavailableConfiguredFace/bold is not loaded") {
						t.Fatalf("configured family admission %s: %v", format, err)
					}
				}
				p, err := RenderPage(context.Background(), d, &Options{Width: 320, Height: 200})
				if err == nil || p != nil || !strings.Contains(err.Error(), "isometric text font UnavailableConfiguredFace/bold is not loaded") {
					t.Fatalf("configured family page admission: %v", err)
				}
			})
		}
	}
}
