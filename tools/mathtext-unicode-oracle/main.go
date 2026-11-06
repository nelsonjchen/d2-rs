// SPDX-License-Identifier: MPL-2.0
// Oracle for go-text/typesetting's Unicode lookups (pinned by go.mod), which
// D2's text shaping uses. Built by scripts/fixture-gen-go-unicode.py with the
// tools/fixtures/hooks/mathtext-txt-go-unicode overlays; writes JSON to stdout.
package main

import (
	"os"

	"github.com/go-text/typesetting/harfbuzz"
)

func main() {
	if err := harfbuzz.D2RSUnicodeProperties(os.Stdout); err != nil {
		panic(err)
	}
}
