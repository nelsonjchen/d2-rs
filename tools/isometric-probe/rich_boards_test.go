// SPDX-License-Identifier: MPL-2.0
// Whole-board observations of the exact targets built by original rich tests.
package d2isometricimg

import (
	"context"
	"fmt"

	"github.com/d2lang/d2/d2renderers/d2scene"
	"github.com/d2lang/d2/d2renderers/d2scenebuild"
	"github.com/d2lang/d2/d2target"
)

type d2rsRichBoardKey struct{}

// Only the outer original helper call adds observations. Render's recursive
// use of the same native helper still executes the complete original body.
// The original Build call, result and test assertions are preserved unchanged.
func d2rsRichBoardBuild(ctx context.Context, d *d2target.Diagram, o d2scenebuild.Options) (*d2scene.Document, error) {
	doc, err := d2scenebuild.Build(ctx, d, o)
	if err != nil || ctx.Value(d2rsRichBoardKey{}) != nil {
		return doc, err
	}
	ctx = context.WithValue(ctx, d2rsRichBoardKey{}, true)
	opts := &Options{Width: 640, Height: 480}
	opts.Render.ThemeID, opts.Render.ThemeOverrides = o.ThemeID, o.ThemeOverrides
	for _, format := range []Format{SVG, PNG} {
		opts.Format = format
		data, e := Render(ctx, d, opts)
		if e != nil || len(data) == 0 {
			return nil, fmt.Errorf("public rich board %s: %v", format, e)
		}
	}
	opts.Format = PNG
	p, e := RenderPage(ctx, d, opts)
	if e != nil || p == nil {
		return nil, fmt.Errorf("public rich board page: %v", e)
	}
	return doc, nil
}
