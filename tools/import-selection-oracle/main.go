package main

import (
	"encoding/json"
	"github.com/d2lang/d2/d2ir"
	"os"
)

func main() {
	var requests []d2ir.SelectionRequest
	if e := json.NewDecoder(os.Stdin).Decode(&requests); e != nil {
		panic(e)
	}
	out := []any{}
	for _, r := range requests {
		out = append(out, d2ir.OracleSelection(r))
	}
	json.NewEncoder(os.Stdout).Encode(out)
}
