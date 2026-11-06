// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Development-only D2 ELK adapter wire oracle for crates/d2-rs-elk (recipes
// elk-adapter-wire-*). scripts/check-elk-adapter.py builds it as an
// overlay-only main package in the runner's patched upstream tree, together
// with an overlaid copy of d2layouts/d2elklayout/layout.go that hands the
// marshalled ELK graph to d2elklayout.D2RSWireHook just before
// elk.LayoutJSON (tools/elk-adapter-wire-oracle/hook.go.txt).
//
// Stdin: [{"name", "source"}]. Each D2 source compiles through d2lib with the
// default text ruler and d2elklayout.DefaultLayout, as the e2e tests do.
// Stdout: [{"name", "wires": [every ELK request JSON, in call order],
// "error"}].
package main

import (
	"context"
	"encoding/json"
	"io"
	"log/slog"
	"os"

	"github.com/d2lang/d2/d2graph"
	"github.com/d2lang/d2/d2layouts/d2elklayout"
	"github.com/d2lang/d2/d2lib"
	"github.com/d2lang/d2/lib/log"
	"github.com/d2lang/d2/lib/textmeasure"
)

type request struct {
	Name   string `json:"name"`
	Source string `json:"source"`
}

type result struct {
	Name  string   `json:"name"`
	Wires []string `json:"wires"`
	Error string   `json:"error,omitempty"`
}

func main() {
	var requests []request
	if err := json.NewDecoder(os.Stdin).Decode(&requests); err != nil {
		panic(err)
	}
	ruler, err := textmeasure.NewRuler()
	if err != nil {
		panic(err)
	}
	// d2's logger writes to stdout without a context logger; discard it.
	ctx := log.With(context.Background(), slog.New(slog.NewTextHandler(io.Discard, nil)))
	results := []result{}
	for _, req := range requests {
		res := result{Name: req.Name, Wires: []string{}}
		d2elklayout.D2RSWireHook = func(raw []byte) { res.Wires = append(res.Wires, string(raw)) }
		layout := "elk"
		_, _, err := d2lib.Compile(ctx, req.Source, &d2lib.CompileOptions{
			Ruler:  ruler,
			Layout: &layout,
			LayoutResolver: func(string) (d2graph.LayoutGraph, error) {
				return d2elklayout.DefaultLayout, nil
			},
		}, nil)
		if err != nil {
			res.Error = err.Error()
		}
		results = append(results, res)
	}
	if err := json.NewEncoder(os.Stdout).Encode(results); err != nil {
		panic(err)
	}
}
