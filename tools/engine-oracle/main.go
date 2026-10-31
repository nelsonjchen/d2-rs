// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// Development reference only; never shipped as part of d2-rs.
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"os"

	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/engine"
	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/graphjson"
	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/layoutgraph"
)

func run() error {
	operation := flag.String("operation", "layout", "layout, roundtrip, wire or fuzz")
	seed := flag.Int64("seed", 1, "signed 64-bit layout seed")
	flag.Parse()
	if flag.NArg() != 0 {
		return fmt.Errorf("unexpected positional arguments")
	}
	if *operation != "layout" && *operation != "roundtrip" && *operation != "wire" && *operation != "fuzz" {
		return fmt.Errorf("unknown operation %q", *operation)
	}
	data, err := io.ReadAll(os.Stdin)
	if err != nil {
		return err
	}
	if *operation == "wire" {
		var wire graphjson.SerializedGraph
		decoder := json.NewDecoder(bytes.NewReader(data))
		decoder.DisallowUnknownFields()
		if err := decoder.Decode(&wire); err != nil {
			return err
		}
		var trailing json.RawMessage
		if err := decoder.Decode(&trailing); err != io.EOF {
			if err != nil {
				return err
			}
			return fmt.Errorf("graph JSON contains trailing data")
		}
		output, err := json.Marshal(wire)
		if err != nil {
			return err
		}
		_, err = os.Stdout.Write(output)
		return err
	}
	if *operation == "fuzz" {
		graph, err := graphFromFuzzCorpus(data)
		if err != nil {
			return err
		}
		output, err := graphjson.Marshal(context.Background(), graph)
		if err != nil {
			return err
		}
		_, err = os.Stdout.Write(output)
		return err
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	var graph layoutgraph.Graph
	if err := graphjson.Unmarshal(ctx, data, &graph); err != nil {
		return err
	}
	result := &graph
	if *operation == "layout" {
		result, err = engine.Layout(ctx, &graph, engine.LayoutOptions{Seed: *seed})
		if err != nil {
			return err
		}
	}
	output, err := graphjson.Marshal(ctx, result)
	if err != nil {
		return err
	}
	_, err = os.Stdout.Write(output)
	return err
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
