// SPDX-License-Identifier: MPL-2.0
package d2scenebuild

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
	"github.com/d2lang/d2/d2target"
	"github.com/d2lang/d2/lib/imageasset"
)

// Serialize only the observed Build calls. Original assertions, including
// resolver I/O counts, remain in the upstream tests and are not replaced.
var d2rsAssetMu sync.Mutex
var d2rsAssetCalls = map[string]int{}
var d2rsAssetResources map[string]any

func d2rsAssetResolve(ctx context.Context, resolver *imageasset.Resolver, source string) (*imageasset.Resource, error) {
	resource, err := resolver.Resolve(ctx, source)
	if d2rsAssetResources != nil {
		value := object{}
		if err != nil {
			value["error"] = err.Error()
			value["unavailable"] = errors.Is(err, imageasset.ErrUnavailable)
		} else if resource != nil {
			data, copyErr := resource.BytesContext(ctx)
			if copyErr != nil {
				panic(copyErr)
			}
			value = object{"kind": resource.Kind(), "mime_type": resource.MIMEType(), "data": data,
				"pixel_width": resource.PixelWidth(), "pixel_height": resource.PixelHeight(), "decoded_bytes": resource.DecodedBytes()}
		}
		d2rsAssetResources[source] = value
	}
	return resource, err
}

func d2rsAssetBuild(t *testing.T, ctx context.Context, diagram *d2target.Diagram, options Options) (*d2scene.Document, error) {
	t.Helper()
	if os.Getenv("D2RS_ASSET_CAPTURE") == "" {
		return Build(ctx, diagram, options)
	}
	d2rsAssetMu.Lock()
	defer d2rsAssetMu.Unlock()
	d2rsAssetResources = map[string]any{}
	defer func() { d2rsAssetResources = nil }()
	document, buildErr := Build(ctx, diagram, options)
	index := d2rsAssetCalls[t.Name()]
	d2rsAssetCalls[t.Name()]++
	dir := filepath.Join(os.Getenv("D2RS_ASSET_CAPTURE"), "go", t.Name(), fmt.Sprint(index))
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
	write("input.json", object{"test": t.Name(), "diagram": diagram, "pad": options.Pad,
		"configured": options.Assets != nil && options.Assets.Resolver != nil, "assets": options.Assets,
		"resources": d2rsAssetResources})
	if buildErr != nil {
		if err := os.WriteFile(filepath.Join(dir, "expected.err"), []byte(buildErr.Error()), 0644); err != nil {
			t.Fatal(err)
		}
	} else {
		write("expected.json", encDocument(document))
	}
	return document, buildErr
}
