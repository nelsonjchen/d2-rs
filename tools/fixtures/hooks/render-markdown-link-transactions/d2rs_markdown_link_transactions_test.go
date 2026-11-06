// SPDX-License-Identifier: MPL-2.0
// Oracle hook for crates/d2-rs-render/tests/fixtures/markdown-link-transactions-source-v090.json
// (docs/FIXTURES.md): d2scenebuild addMarkdownLinkRegion transactions and
// link/appendix budget admission over region/byte budgets x appendix x
// pre-seeded regions x URL x title, plus degenerate boxes and multibyte
// title budgets. Overlaid into d2renderers/d2scenebuild; inert unless
// D2RS_RECIPE_OUTPUT is set. Harvested from the v0.9.0-era
// tools/probes/markdown-link-transaction-source-20261008.go; its request
// list is generated inline.
package d2scenebuild

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
)

type d2rsLinkRequest struct {
	Name, URL, Title            string
	MaxRegions, MaxBytes, Seed int
	Appendix                    bool
	Box                         d2scene.Box
}

func TestD2RSMarkdownLinkTransactions(t *testing.T) {
	output := os.Getenv("D2RS_RECIPE_OUTPUT")
	if output == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	box := d2scene.Box{Width: 4, Height: 3}
	requests := []d2rsLinkRequest{}
	for _, maxRegions := range []int{1, 2, 4} {
		for _, maxBytes := range []int{4, 16, 64} {
			for _, appendix := range []bool{false, true} {
				for _, seed := range []int{0, 1, 2} {
					for _, url := range []string{"../x", "https://example.com"} {
						for _, title := range []string{"", "ok", "甲", "bad\x01"} {
							requests = append(requests, d2rsLinkRequest{URL: url, Title: title, MaxRegions: maxRegions, MaxBytes: maxBytes, Seed: seed, Appendix: appendix, Box: box})
						}
					}
				}
			}
		}
	}
	for _, size := range [][2]float64{{0, 0}, {0, 1}, {-1, 0}, {-1, 1}} {
		for _, url := range []string{"", "x"} {
			for _, title := range []string{"", "t"} {
				for _, seed := range []int{0, 1} {
					requests = append(requests, d2rsLinkRequest{URL: url, Title: title, MaxRegions: 1, MaxBytes: 64, Seed: seed, Appendix: true, Box: d2scene.Box{Width: size[0], Height: size[1]}})
				}
			}
		}
	}
	for _, maxBytes := range []int{4, 8} {
		for _, seed := range []int{1, 2} {
			for _, title := range []string{"x", "甲"} {
				requests = append(requests, d2rsLinkRequest{URL: "u", Title: title, MaxRegions: 4, MaxBytes: maxBytes, Seed: seed, Appendix: true, Box: box})
			}
		}
	}
	rows := []map[string]any{}
	for i, r := range requests {
		r.Name = fmt.Sprintf("transaction-%d", i)
		b := builder{ctx: context.Background(), options: Options{Appendix: r.Appendix, LinkBudget: LinkBudget{MaxRegions: r.MaxRegions, MaxStringBytes: r.MaxBytes}}}
		for j := 0; j < r.Seed; j++ {
			b.links = append(b.links, d2scene.LinkRegion{Box: d2scene.Box{Width: 1, Height: 1}, URL: "s"})
			b.linkBytes++
			b.appendixItems = append(b.appendixItems, appendixItem{object: "seed", field: "tooltip", text: "seed"})
			b.appendixStringBytes += 4
		}
		err := b.addMarkdownLinkRegion("node", 3, r.URL, r.Title, r.Box)
		titles := []string{}
		for _, item := range b.appendixItems {
			titles = append(titles, item.text)
		}
		errorText := ""
		if err != nil {
			errorText = err.Error()
		}
		rows = append(rows, map[string]any{"request": r, "error": errorText, "links": b.links, "link_bytes": b.linkBytes, "appendix": titles, "appendix_bytes": b.appendixStringBytes})
	}
	data, err := json.Marshal(rows)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(output, append(data, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}
