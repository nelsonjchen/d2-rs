// SPDX-License-Identifier: MPL-2.0
// Test-only observer, installed only in the manifest's trace checkout.
package png_test

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2isometric/d2isometricimg"
	"github.com/d2lang/d2/d2target"
)

func d2rsPNGWrite(t *testing.T, path string, data []byte) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, data, 0600); err != nil {
		t.Fatal(err)
	}
}

// Run the unmodified snapshot assertions against private copies. Never write
// .got files or accept snapshots in the pinned source or shared trace fixtures.
func d2rsPNGSnapshotDir(t *testing.T, fixture string) string {
	t.Helper()
	root := os.Getenv("D2RS_PNG_CAPTURE")
	if root == "" {
		return filepath.Join("testdata", fixture)
	}
	data, err := os.ReadFile(filepath.Join("..", "testdata", fixture, "board.exp.json"))
	if err != nil {
		t.Fatal(err)
	}
	d2rsPNGWrite(t, filepath.Join(root, "inputs", fixture, "board.json"), data)
	paths, err := filepath.Glob(filepath.Join("testdata", fixture, "isometric*.exp.png"))
	if err != nil {
		t.Fatal(err)
	}
	dir := filepath.Join(root, "snapshots", fixture)
	for _, path := range paths {
		data, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		d2rsPNGWrite(t, filepath.Join(dir, filepath.Base(path)), data)
	}
	return dir
}

func d2rsPNGRender(t *testing.T, ctx context.Context, board *d2target.Diagram, opts *d2isometricimg.Options) ([]byte, error) {
	t.Helper()
	data, err := d2isometricimg.Render(ctx, board, opts)
	root := os.Getenv("D2RS_PNG_CAPTURE")
	if root == "" {
		return data, err
	}
	if opts.Format != d2isometricimg.PNG || opts.Width != 1200 || opts.Height != 1200 || !opts.FitContent {
		t.Fatal("original PNG options changed")
	}
	name := strings.TrimPrefix(t.Name(), "TestPNG/")
	if err != nil {
		d2rsPNGWrite(t, filepath.Join(root, "go", name+".err"), []byte(err.Error()))
	} else {
		d2rsPNGWrite(t, filepath.Join(root, "go", name+".png"), data)
	}
	options, marshalErr := json.Marshal(opts)
	if marshalErr != nil {
		t.Fatal(marshalErr)
	}
	d2rsPNGWrite(t, filepath.Join(root, "options", name+".json"), options)
	return data, err
}
