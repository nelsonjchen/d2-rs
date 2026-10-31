// SPDX-License-Identifier: MPL-2.0
// XML token/attribute budgets over sample documents x limits (recipe mathtext-svg-xml-budget).
package d2svgimport

import (
	"context"
	"encoding/json"
	"os"
	"strings"
	"testing"
)

func TestD2RSXMLBudget(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	type row struct {
		XML    string  `json:"xml"`
		Limits [9]int  `json:"limits"`
		Counts *[4]int `json:"counts"`
		Error  *string `json:"error"`
	}
	data, e := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if e != nil {
		t.Fatal(e)
	}
	var rows []row
	if e = json.Unmarshal(data, &rows); e != nil {
		t.Fatal(e)
	}
	for n := range rows {
		r := &rows[n]
		l := r.Limits
		limits := Limits{l[0], l[1], l[2], l[3], l[4], l[5], l[6], l[7], l[8]}
		e := validateImportLimits(limits)
		if e == nil && len(r.XML) > l[0] {
			_, e = ImportNode(context.Background(), "math", []byte(r.XML), limits)
		}
		if e == nil {
			i := svgImporter{ctx: context.Background(), source: "math", limits: limits, ids: make(map[string]*svgElement)}
			_, e = i.parse([]byte(strings.TrimPrefix(r.XML, "\ufeff")))
			if e == nil {
				v := [4]int{len(strings.TrimPrefix(r.XML, "\ufeff")), i.parsedElements, i.parsedAttributes, i.parsedAttrBytes}
				r.Counts = &v
			}
		}
		if e != nil {
			s := e.Error()
			r.Error = &s
		}
	}
	data, e = json.MarshalIndent(rows, "", "  ")
	if e != nil {
		t.Fatal(e)
	}
	if e = os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), data, 0644); e != nil {
		t.Fatal(e)
	}
}
