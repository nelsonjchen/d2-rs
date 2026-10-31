// Development oracle: pinned Go os.Open/File.Read, no D2 instrumentation.
package main

import (
	"encoding/json"
	"os"
)

func main() {
	var paths []string
	if err := json.NewDecoder(os.Stdin).Decode(&paths); err != nil {
		panic(err)
	}
	results := []any{}
	for _, path := range paths {
		f, err := os.Open(path)
		if err != nil {
			results = append(results, map[string]any{"open_error": err.Error()})
			continue
		}
		b := make([]byte, 4096)
		n, err := f.Read(b)
		message := ""
		if err != nil {
			message = err.Error()
		}
		f.Close()
		results = append(results, map[string]any{"bytes": byteNumbers(b[:n]), "read_error": message})
	}
	json.NewEncoder(os.Stdout).Encode(results)
}

func byteNumbers(b []byte) []int {
	a := make([]int, len(b))
	for i, v := range b {
		a[i] = int(v)
	}
	return a
}
