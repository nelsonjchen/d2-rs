// SPDX-License-Identifier: MPL-2.0
// Public whole-image observations of the original source composition fixtures.
package d2isometricimg

import (
	"context"
	"testing"

	"github.com/d2lang/d2/d2target"
)

// The original source_panels_test.go helper is renamed only during capture.
// Its assertions and native scene remain intact; each original input also
// traverses the three public still-image boundaries at the helper's resolution.
func nativeFixtureScene(t *testing.T, d *d2target.Diagram) *nativeScene {
	t.Helper()
	for _, format := range []Format{SVG, PNG} {
		data, err := Render(context.Background(), d, &Options{Format: format, Width: 640, Height: 480})
		if err != nil || len(data) == 0 {
			t.Fatalf("public composition %s: %v", format, err)
		}
	}
	p, err := RenderPage(context.Background(), d, &Options{Width: 640, Height: 480})
	if err != nil || p == nil {
		t.Fatalf("public composition page: %v", err)
	}
	return d2rsOriginalnativeFixtureScene(t, d)
}
