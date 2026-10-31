// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Development-only elk-go option registry oracle for crates/d2-rs-elk (recipe
// elk-option-registry). It must be built as a package under
// github.com/d2lang/elk-go/ so that it may import elk-go's internal core and
// builtins packages.
//
// No arguments: prints the JSON array of every registered option ID, in
// registration order. `resolve`: reads a JSON array of keys on stdin and
// prints {key: OptionRegistry.BySuffix(key).ID or null}.
package main

import (
	"encoding/json"
	"os"

	"github.com/d2lang/elk-go/internal/builtins"
	"github.com/d2lang/elk-go/internal/core"
)

func main() {
	registry := core.NewOptionRegistry()
	builtins.RegisterOptions(registry)
	if len(os.Args) > 1 && os.Args[1] == "resolve" {
		var keys []string
		if err := json.NewDecoder(os.Stdin).Decode(&keys); err != nil {
			panic(err)
		}
		out := map[string]*string{}
		for _, key := range keys {
			var id *string
			if option := registry.BySuffix(key); option != nil {
				value := option.ID
				id = &value
			}
			out[key] = id
		}
		if err := json.NewEncoder(os.Stdout).Encode(out); err != nil {
			panic(err)
		}
		return
	}
	ids := []string{}
	for _, option := range registry.Options() {
		ids = append(ids, option.ID)
	}
	if err := json.NewEncoder(os.Stdout).Encode(ids); err != nil {
		panic(err)
	}
}
