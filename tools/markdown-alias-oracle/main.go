// SPDX-License-Identifier: MPL-2.0
package main

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"github.com/yuin/goldmark/text"
	"os"
	"strings"
)

type request struct {
	Source   string
	Size     int
	Capacity int
	Segment  text.Segment
}

// Lossless four-byte run encoding keeps large growth-boundary fixtures small.
func encodeBytes(data []byte) string {
	if len(data) <= 2048 {
		return hex.EncodeToString(data)
	}
	var out strings.Builder
	out.WriteString("rle4:")
	for i := 0; i < len(data); {
		end := min(i+4, len(data))
		chunk := string(data[i:end])
		count := 1
		j := end
		for j+4 <= len(data) && string(data[j:j+4]) == chunk {
			count++
			j += 4
		}
		fmt.Fprintf(&out, "%s*%d;", hex.EncodeToString([]byte(chunk)), count)
		i = j
	}
	return out.String()
}
func evaluate(q request) any {
	data, err := hex.DecodeString(q.Source)
	if err != nil {
		panic(err)
	}
	if q.Size > 0 {
		data = make([]byte, q.Size)
		for i := range data {
			data[i] = "abcd"[i%4]
		}
	}
	source := make([]byte, len(data), q.Capacity)
	copy(source, data)
	r := text.NewReader(source)
	cached, _ := r.PeekLine()
	segments := text.NewSegments()
	segments.Append(q.Segment)
	b := text.NewBlockReader(source, segments)
	value := q.Segment.Value(source)
	first := encodeBytes(value)
	sourceAfter := encodeBytes(source)
	backingAfter := encodeBytes(source[:cap(source)])
	if len(source) > 0 {
		source[0] = '!'
	}
	again, _ := r.PeekLine()
	block, _ := b.PeekLine()
	return map[string]any{"first": first, "value": encodeBytes(value), "capacity": cap(value), "source_after_value": sourceAfter, "backing_after_value": backingAfter, "source": encodeBytes(source), "cached": encodeBytes(cached), "again": encodeBytes(again), "block": encodeBytes(block)}
}
func main() {
	var requests []request
	if err := json.NewDecoder(os.Stdin).Decode(&requests); err != nil {
		panic(err)
	}
	results := make([]any, 0, len(requests))
	for _, q := range requests {
		results = append(results, evaluate(q))
	}
	if err := json.NewEncoder(os.Stdout).Encode(results); err != nil {
		panic(err)
	}
}
