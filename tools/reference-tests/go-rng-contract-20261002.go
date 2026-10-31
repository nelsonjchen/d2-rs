// SPDX-License-Identifier: MPL-2.0
// Development oracle: execute with the pinned Go1.27.0 toolchain.
package main

import (
 "encoding/json"
 "fmt"
 "math"
 "math/rand"
 "os"
)

type countedSource struct { rand.Source; draws uint64 }
func (s *countedSource) Int63() int64 { s.draws++; return s.Source.Int63() }

type record struct {
 Seed int64 `json:"seed"`
 Bound int64 `json:"bound"`
 Result int64 `json:"result"`
 Draws uint64 `json:"draws"`
 NextFloatBits string `json:"next_float_bits"`
 DrawsAfterFloat uint64 `json:"draws_after_float"`
}

func main() {
 var records []record
 for _, seed := range []int64{0,1,2,17,42,2147483647,-1,-9223372036854775808} {
  for _, bound := range []int64{1,2,2147483647,2147483648,2147483649,4611686018427387904,4611686018427387907,9223372036854775807} {
   source := &countedSource{Source:rand.NewSource(seed)}
   random := rand.New(source)
   value := random.Int63n(bound)
   draws := source.draws
   next := random.Float64()
   records=append(records,record{seed,bound,value,draws,fmt.Sprintf("%016x",math.Float64bits(next)),source.draws})
  }
 }
 if err:=json.NewEncoder(os.Stdout).Encode(records);err!=nil { panic(err) }
}
