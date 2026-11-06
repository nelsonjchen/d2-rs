// SPDX-License-Identifier: MPL-2.0
// Oracle program for crates/d2-rs-render/tests/fixtures/mime-registration-source-go127.json
// (docs/FIXTURES.md, recipe render-probe-mime-registration): Go standard
// library mime.AddExtensionType acceptance and TypeByExtension answers for the
// seed's registrations, applied in order to one process-wide table. Reads the
// seed on stdin. Harvested from the v0.9.0-era
// .reference/mime-database-native-20261012/probe-registration.go.
package main

import (
	"encoding/json"
	"mime"
	"os"
)

type Case struct {
	Name      string            `json:"name"`
	Extension string            `json:"extension"`
	MIME      string            `json:"mime"`
	Accepted  bool              `json:"accepted"`
	Queries   map[string]string `json:"queries"`
}

func main() {
	var cases []Case
	if err := json.NewDecoder(os.Stdin).Decode(&cases); err != nil {
		panic(err)
	}
	for i := range cases {
		c := &cases[i]
		c.Accepted = mime.AddExtensionType(c.Extension, c.MIME) == nil
		for ext := range c.Queries {
			c.Queries[ext] = mime.TypeByExtension(ext)
		}
	}
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	if err := encoder.Encode(cases); err != nil {
		panic(err)
	}
}
