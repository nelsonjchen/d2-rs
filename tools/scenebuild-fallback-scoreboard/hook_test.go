// SPDX-License-Identifier: MPL-2.0
package d2scenebuild

import (
	"context"
	"encoding/json"
	"fmt"
	"github.com/d2lang/d2/d2renderers/d2fonts"
	"os"
	"path/filepath"
	"sync"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
	"github.com/d2lang/d2/d2target"
)

var d2rsFallbackMu sync.Mutex
var d2rsFallbackCalls = map[string]int{}
var d2rsFallbackLimits = map[*d2fonts.BundledFallbackResolver]d2fonts.BundledFallbackLimits{}

func d2rsNewBundledFallback(t *testing.T, next d2fonts.FallbackResolver, limits d2fonts.BundledFallbackLimits) (*d2fonts.BundledFallbackResolver, error) {
	resolver, err := d2fonts.NewBundledFallbackResolver(next, limits)
	if err == nil {
		d2rsFallbackMu.Lock()
		d2rsFallbackLimits[resolver] = limits
		d2rsFallbackMu.Unlock()
	}
	return resolver, err
}

func d2rsFallbackBuild(t *testing.T, ctx context.Context, diagram *d2target.Diagram, options Options) (*d2scene.Document, error) {
	t.Helper()
	document, buildErr := Build(ctx, diagram, options)
	root := os.Getenv("D2RS_FALLBACK_CAPTURE")
	if root == "" {
		return document, buildErr
	}
	d2rsFallbackMu.Lock()
	defer d2rsFallbackMu.Unlock()
	index := d2rsFallbackCalls[t.Name()]
	d2rsFallbackCalls[t.Name()]++
	dir := filepath.Join(root, "go", t.Name(), fmt.Sprint(index))
	if err := os.MkdirAll(dir, 0755); err != nil {
		t.Fatal(err)
	}
	write := func(name string, value any) {
		data, err := json.Marshal(value)
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(dir, name), data, 0644); err != nil {
			t.Fatal(err)
		}
	}
	// Resolver identity is intentionally separate from its public interface.
	// These selected tests all construct the bundled-only native resolver.
	resolver := options.Fonts.Resolver
	if counting, ok := resolver.(*countingFontFallbackResolver); ok {
		resolver = counting.inner
	}
	bundled, ok := resolver.(*d2fonts.BundledFallbackResolver)
	if !ok {
		t.Fatal("unexpected fallback resolver type")
	}
	limits, ok := d2rsFallbackLimits[bundled]
	if !ok {
		t.Fatal("missing constructor limits")
	}
	write("input.json", object{"test": t.Name(), "diagram": diagram, "resolver_limits": limits, "fonts": object{
		"max_assets": options.Fonts.MaxAssets, "max_bytes": options.Fonts.MaxBytes,
		"max_runes_per_text": options.Fonts.MaxRunesPerText, "max_total_runes": options.Fonts.MaxTotalRunes,
		"max_coverage_checks": options.Fonts.MaxCoverageChecks, "max_font_faces_per_text": options.Fonts.MaxFontFacesPerText,
		"max_shaping_runs": options.Fonts.MaxShapingRuns, "max_shaped_glyphs": options.Fonts.MaxShapedGlyphs,
	}})
	if buildErr != nil {
		if err := os.WriteFile(filepath.Join(dir, "expected.err"), []byte(buildErr.Error()), 0644); err != nil {
			t.Fatal(err)
		}
	} else {
		write("expected.json", encDocument(document))
	}
	return document, buildErr
}
