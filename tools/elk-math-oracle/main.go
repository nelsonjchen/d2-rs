// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Development-only scalar oracle for crates/d2-rs-elk (recipes elk-radial-trig
// and elk-radial-math). It must be built as a package under
// github.com/d2lang/elk-go/ so that it may import elk-go's internal javacompat.
//
// Default mode: a JSON array of float64 bit patterns on stdin; prints
// [[sin bits, cos bits], ...] using javacompat.JSMathSin/JSMathCos.
// D2RS_RADIAL_SCALARS=1: a JSON array of [x bits, y bits]; prints
// [[Acos(x), Atan2(y, x), Hypot(x, y)] bits, ...] using Go's math package.
// D2RS_FORCE_LOG=1: a JSON array of float64 bit patterns; prints the bits of
// Go's math.Log of each (recipe elk-scalar-log).
package main

import (
	"encoding/json"
	"math"
	"os"

	"github.com/d2lang/elk-go/internal/javacompat"
)

func main() {
	if os.Getenv("D2RS_RADIAL_SCALARS") == "1" {
		scalars()
		return
	}
	if os.Getenv("D2RS_FORCE_LOG") == "1" {
		logarithms()
		return
	}
	var bits []uint64
	if err := json.NewDecoder(os.Stdin).Decode(&bits); err != nil {
		panic(err)
	}
	out := make([][2]uint64, len(bits))
	for i, v := range bits {
		x := math.Float64frombits(v)
		out[i] = [2]uint64{math.Float64bits(javacompat.JSMathSin(x)), math.Float64bits(javacompat.JSMathCos(x))}
	}
	if err := json.NewEncoder(os.Stdout).Encode(out); err != nil {
		panic(err)
	}
}

func scalars() {
	var bits [][2]uint64
	if err := json.NewDecoder(os.Stdin).Decode(&bits); err != nil {
		panic(err)
	}
	out := make([][3]uint64, len(bits))
	for i, v := range bits {
		x, y := math.Float64frombits(v[0]), math.Float64frombits(v[1])
		out[i] = [3]uint64{math.Float64bits(math.Acos(x)), math.Float64bits(math.Atan2(y, x)), math.Float64bits(math.Hypot(x, y))}
	}
	if err := json.NewEncoder(os.Stdout).Encode(out); err != nil {
		panic(err)
	}
}

func logarithms() {
	var bits []uint64
	if err := json.NewDecoder(os.Stdin).Decode(&bits); err != nil {
		panic(err)
	}
	for i, v := range bits {
		bits[i] = math.Float64bits(math.Log(math.Float64frombits(v)))
	}
	if err := json.NewEncoder(os.Stdout).Encode(bits); err != nil {
		panic(err)
	}
}
