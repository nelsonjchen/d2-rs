// SPDX-License-Identifier: MPL-2.0
// Cumulative SVG document budget accounting (recipe mathtext-svg-document-budget).
package d2scenebuild

import (
	"encoding/json"
	"github.com/d2lang/d2/d2renderers/d2svgimport"
	"os"
	"testing"
)

type d2rsBudgetRow struct {
	Maximum         [8]int  `json:"maximum"`
	Used            [8]int  `json:"used"`
	Limits          [7]int  `json:"limits"`
	Parsed          [8]int  `json:"parsed"`
	EmittedElements int     `json:"emitted_elements"`
	EmittedPaths    int     `json:"emitted_paths"`
	Remaining       *[7]int `json:"remaining"`
	RemainingError  *string `json:"remaining_error"`
	ReserveError    *string `json:"reserve_error"`
	After           [8]int  `json:"after"`
}

func TestD2RSSVGDocumentBudget(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	data, e := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if e != nil {
		t.Fatal(e)
	}
	var rows []d2rsBudgetRow
	if e = json.Unmarshal(data, &rows); e != nil {
		t.Fatal(e)
	}
	for n := range rows {
		r := &rows[n]
		m, u, l, c := r.Maximum, r.Used, r.Limits, r.Parsed
		b := builder{options: Options{Assets: &AssetOptions{SVGImportBudget: SVGImportBudget{m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7]}, SVGImportLimits: d2svgimport.Limits{MaxBytes: l[0], MaxDepth: 100, MaxElements: l[1], MaxAttributes: l[2], MaxAttributeBytes: l[3], MaxPathCommands: l[4], MaxTransformFunctions: l[5], MaxUseDepth: 100, MaxResources: l[6]}}}, svgSourceBytes: u[0], svgElements: u[1], svgAttributes: u[2], svgAttributeBytes: u[3], svgCommands: u[4], svgTransforms: u[5], svgDeclaredResources: u[6], svgExpandedUseInstances: u[7]}
		rem, e := b.remainingSVGImportLimits("shape \"n\" latex")
		if e != nil {
			s := e.Error()
			r.RemainingError = &s
		} else {
			v := [7]int{rem.MaxBytes, rem.MaxElements, rem.MaxAttributes, rem.MaxAttributeBytes, rem.MaxPathCommands, rem.MaxTransformFunctions, rem.MaxResources}
			r.Remaining = &v
		}
		e = b.reserveSVGImport("shape \"n\" latex", d2svgimport.Metrics{SourceBytes: c[0], ParsedElements: c[1], ParsedAttributes: c[2], ParsedAttributeBytes: c[3], ParsedPathCommands: c[4], ParsedTransformFuncs: c[5], DeclaredResources: c[6], ExpandedUseInstances: c[7], EmittedElements: r.EmittedElements, EmittedPathCommands: r.EmittedPaths})
		if e != nil {
			s := e.Error()
			r.ReserveError = &s
		}
		r.After = [8]int{b.svgSourceBytes, b.svgElements, b.svgAttributes, b.svgAttributeBytes, b.svgCommands, b.svgTransforms, b.svgDeclaredResources, b.svgExpandedUseInstances}
	}
	data, e = json.MarshalIndent(rows, "", "  ")
	if e != nil {
		t.Fatal(e)
	}
	if e = os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), data, 0644); e != nil {
		t.Fatal(e)
	}
}
