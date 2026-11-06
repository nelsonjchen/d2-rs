// SPDX-License-Identifier: MPL-2.0
// Oracle hook for crates/d2-rs-export markdown_background_raster_contract
// (recipe renderexport-markdown-background-raster, docs/FIXTURES.md).
// Overlaid into d2cli; inert unless run by scripts/regen-fixtures.py.
//
// Ported from the v0.9.0-era probe ../d2-v0.9.0-trace/markdown_background_probe.go.overlay
// and .reference/markdown-background-layer-20261007/capture-final.py: each row
// compiles one filled Markdown label through bundled TALA (seed 1) and
// d2scenebuild.Build, retaining the exported shape, the Markdown group's
// transform/clip and its background node, then renders the same source with
// the CLI itself (`--layout=tala --tala-seeds=1 --theme=N in.d2 out.png`) and
// records the decoded PNG's size and RGB SHA-256.
package d2cli

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"image/color"
	"image/png"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/d2lang/util-go/xmain"
	"github.com/d2lang/util-go/xos"

	"github.com/d2lang/d2/d2graph"
	"github.com/d2lang/d2/d2layouts/d2talalayout"
	"github.com/d2lang/d2/d2lib"
	"github.com/d2lang/d2/d2renderers/d2scene"
	"github.com/d2lang/d2/d2renderers/d2scenebuild"
	"github.com/d2lang/d2/d2renderers/d2svg"
	"github.com/d2lang/d2/d2target"
	"github.com/d2lang/d2/lib/log"
	"github.com/d2lang/d2/lib/textmeasure"
)

type d2rsMDBackgroundCase struct {
	name, source string
	theme        int64
}

type d2rsMDBackgroundFrame struct {
	Width     int    `json:"width"`
	Height    int    `json:"height"`
	RGBSHA256 string `json:"rgb_sha256"`
}

type d2rsMDBackgroundRow struct {
	Background        *d2scene.Node         `json:"background"`
	Clip              any                   `json:"clip"`
	MarkdownTransform any                   `json:"markdown_transform"`
	Name              string                `json:"name"`
	Shape             d2target.Shape        `json:"shape"`
	Source            string                `json:"source"`
	Theme             int64                 `json:"theme"`
	SourceFrame       d2rsMDBackgroundFrame `json:"source_frame"`
}

// The 24 controls: rectangles over theme x fill x opacity x font size, then
// text shapes with an explicit fill (unauthored text fills are transparent).
func d2rsMDBackgroundCases() []d2rsMDBackgroundCase {
	var cases []d2rsMDBackgroundCase
	add := func(prefix string, rectangle bool, fills []string) {
		for _, theme := range []int64{0, 7} {
			for _, fill := range fills {
				for _, opacity := range []string{"1", "0.37"} {
					for _, size := range []int{40, 50} {
						style := fmt.Sprintf("opacity: %s; font-size: %d", opacity, size)
						if fill != "default" {
							style = "fill: " + fill + "; " + style
						}
						source := "a: |md\n  #### Managers\n\n  - Clock\n  - Reset\n  - Power\n|\na.style: {" + style + "}\n"
						if rectangle {
							source += "a.shape: rectangle\n"
						}
						cases = append(cases, d2rsMDBackgroundCase{
							name:   fmt.Sprintf("%st%d-%s-o%s-s%d", prefix, theme, fill, opacity, size),
							source: source,
							theme:  theme,
						})
					}
				}
			}
		}
	}
	add("", true, []string{"default", "lightpink"})
	add("text-", false, []string{"lightpink"})
	return cases
}

func d2rsMDBackgroundScene(ctx context.Context, ruler *textmeasure.Ruler, c d2rsMDBackgroundCase) (d2rsMDBackgroundRow, error) {
	row := d2rsMDBackgroundRow{Name: c.name, Source: c.source, Theme: c.theme}
	layout := "tala"
	resolver := func(string) (d2graph.LayoutGraph, error) {
		return func(ctx context.Context, g *d2graph.Graph) error {
			return d2talalayout.Layout(ctx, g, &d2talalayout.Options{Seeds: []int64{1}, MaxConcurrency: 1})
		}, nil
	}
	theme := c.theme
	diagram, _, err := d2lib.Compile(ctx, c.source, &d2lib.CompileOptions{Layout: &layout, Ruler: ruler, LayoutResolver: resolver}, &d2svg.RenderOpts{ThemeID: &theme})
	if err != nil {
		return row, err
	}
	scene, err := d2scenebuild.Build(ctx, diagram, d2scenebuild.Options{ThemeID: &theme})
	if err != nil {
		return row, err
	}
	var markdown *d2scene.Node
	var visit func(*d2scene.Node)
	visit = func(n *d2scene.Node) {
		if n == nil {
			return
		}
		if n.ID == "a:markdown" {
			markdown = n
		}
		for _, child := range n.Children {
			visit(child)
		}
	}
	visit(scene.Root)
	if markdown == nil {
		return row, fmt.Errorf("%s: missing a:markdown scene node", c.name)
	}
	for _, child := range markdown.Children {
		if child.ID == "a:markdown:background" {
			row.Background = child
		}
	}
	row.Shape = diagram.Shapes[0]
	row.MarkdownTransform = markdown.Transform
	row.Clip = markdown.Clip
	return row, nil
}

func d2rsMDBackgroundEnv() *xos.Env {
	var env []string
	for _, kv := range os.Environ() {
		if !strings.HasPrefix(kv, "D2_") {
			env = append(env, kv)
		}
	}
	return xos.NewEnv(env)
}

func d2rsMDBackgroundCLIFrame(t *testing.T, c d2rsMDBackgroundCase) d2rsMDBackgroundFrame {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "in.d2"), []byte(c.source), 0o600); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()
	var stdout, stderr bytes.Buffer
	state := &xmain.TestState{
		Run: Run, Env: d2rsMDBackgroundEnv(), PWD: dir, Stdout: &stdout, Stderr: &stderr,
		Args: []string{"d2", "--layout=tala", "--tala-seeds=1", fmt.Sprintf("--theme=%d", c.theme), "in.d2", "out.png"},
	}
	state.Start(t, ctx)
	defer state.Cleanup(t)
	if err := state.Wait(ctx); err != nil {
		t.Fatalf("%s: d2 exited with %v\n%s", c.name, err, stderr.String())
	}
	data, err := os.ReadFile(filepath.Join(dir, "out.png"))
	if err != nil {
		t.Fatal(err)
	}
	img, err := png.Decode(bytes.NewReader(data))
	if err != nil {
		t.Fatalf("%s: %v", c.name, err)
	}
	bounds := img.Bounds()
	rgb := make([]byte, 0, bounds.Dx()*bounds.Dy()*3)
	for y := bounds.Min.Y; y < bounds.Max.Y; y++ {
		for x := bounds.Min.X; x < bounds.Max.X; x++ {
			p := color.NRGBAModel.Convert(img.At(x, y)).(color.NRGBA)
			rgb = append(rgb, p.R, p.G, p.B)
		}
	}
	sum := sha256.Sum256(rgb)
	return d2rsMDBackgroundFrame{Width: bounds.Dx(), Height: bounds.Dy(), RGBSHA256: hex.EncodeToString(sum[:])}
}

func TestD2RSMarkdownBackgroundRaster(t *testing.T) {
	output := os.Getenv("D2RS_RECIPE_OUTPUT")
	if output == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	ruler, err := textmeasure.NewRuler()
	if err != nil {
		t.Fatal(err)
	}
	ctx := log.WithDefault(context.Background())
	var rows []d2rsMDBackgroundRow
	for _, c := range d2rsMDBackgroundCases() {
		row, err := d2rsMDBackgroundScene(ctx, ruler, c)
		if err != nil {
			t.Fatalf("%s: %v", c.name, err)
		}
		row.SourceFrame = d2rsMDBackgroundCLIFrame(t, c)
		rows = append(rows, row)
	}
	var out bytes.Buffer
	encoder := json.NewEncoder(&out)
	encoder.SetEscapeHTML(false)
	encoder.SetIndent("", "  ")
	if err := encoder.Encode(rows); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(output, out.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}
