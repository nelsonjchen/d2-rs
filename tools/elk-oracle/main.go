// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Development-only ELK wire oracle. One complete JSON graph on stdin;
// unnormalized upstream output on stdout, errors on stderr.
package main

import (
	"fmt"
	"io"
	"os"

	elk "github.com/d2lang/elk-go"
)

func main() {
	input, err := io.ReadAll(os.Stdin)
	if err == nil {
		var output []byte
		output, err = elk.LayoutJSON(input)
		if err == nil {
			_, err = os.Stdout.Write(append(output, '\n'))
		}
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
