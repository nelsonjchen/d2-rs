// SPDX-License-Identifier: MPL-2.0
// validateUseGraph over local <use> graphs x maximum use depth
// (recipe mathtext-svg-use-graph; graphs from scripts/fixture-gen-svg.py use-graph).
package d2svgimport

import (
	"context"
	"fmt"
	"testing"
)

func TestD2RSSvgUseGraph(t *testing.T) {
	out := d2rsOutput(t)
	type element struct {
		Children []int `json:"children"`
		IsUse    bool  `json:"is_use"`
		Target   *int  `json:"target"`
	}
	type row struct {
		Elements []element `json:"elements"`
		Maximum  int       `json:"maximum_depth"`
		Error    string    `json:"error"`
	}
	var rows []row
	d2rsReadInput(t, &rows)
	for n := range rows {
		r := &rows[n]
		nodes := make([]*svgElement, len(r.Elements))
		ids := map[string]*svgElement{}
		for j := range nodes {
			nodes[j] = &svgElement{name: "g", id: fmt.Sprint(j)}
			ids[fmt.Sprint(j)] = nodes[j]
		}
		for j, v := range r.Elements {
			if v.IsUse {
				nodes[j].name = "use"
				if v.Target != nil {
					nodes[j].href = fmt.Sprint(*v.Target)
				} else {
					nodes[j].href = "missing"
				}
			}
			for _, child := range v.Children {
				nodes[j].children = append(nodes[j].children, nodes[child])
			}
		}
		i := svgImporter{ctx: context.Background(), source: "math", ids: ids, parsedElements: len(nodes), limits: Limits{MaxUseDepth: r.Maximum}}
		r.Error = d2rsError(i.validateUseGraph(nodes[0]))
	}
	d2rsWrite(t, out, rows)
}
