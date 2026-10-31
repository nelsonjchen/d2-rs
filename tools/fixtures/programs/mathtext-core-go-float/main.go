// SPDX-License-Identifier: MPL-2.0
// strconv.ParseFloat(value, 64) over the JSON string list on stdin: validity,
// IEEE-754 bits and error per input (recipe mathtext-core-go-float). Adapted
// from the v0.9.0 opacity-float source probe.
package main

import (
	"encoding/json"
	"fmt"
	"math"
	"os"
	"strconv"
)

func main() {
	var values []string
	if err := json.NewDecoder(os.Stdin).Decode(&values); err != nil {
		panic(err)
	}
	rows := []map[string]any{}
	for _, value := range values {
		number, err := strconv.ParseFloat(value, 64)
		row := map[string]any{"input": value, "valid": err == nil, "bits": fmt.Sprintf("%016x", math.Float64bits(number))}
		if err != nil {
			row["error"] = err.Error()
		}
		rows = append(rows, row)
	}
	if err := json.NewEncoder(os.Stdout).Encode(rows); err != nil {
		panic(err)
	}
}
