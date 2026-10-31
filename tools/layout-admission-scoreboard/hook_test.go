// SPDX-License-Identifier: MPL-2.0
// Compiled into each original test package only for the bounded capture.
package D2RS_PACKAGE

import (
 "context"
 "encoding/json"
 "os"
 "path/filepath"
 "strings"
 "testing"

 "github.com/d2lang/d2/d2graph"
)

func d2rsAdmission(t *testing.T, ctx context.Context, engine string, g *d2graph.Graph, invoke func() error) error {
 t.Helper()
 identities := map[*d2graph.Object]int{}
 for i, object := range g.Objects { identities[object] = i }
 edges := make([]any, 0, len(g.Edges))
 endpoint := func(object *d2graph.Object) any {
  if object == nil { return nil }
  if id, ok := identities[object]; ok { return id }
  id := len(identities); identities[object] = id; return id
 }
 for _, edge := range g.Edges {
  if edge == nil { edges = append(edges, nil) } else { edges = append(edges, []any{endpoint(edge.Src), endpoint(edge.Dst)}) }
 }
 canceled := ctx != nil && ctx.Err() != nil
 err := invoke()
 message := ""; if err != nil { message = err.Error() }
 value := map[string]any{"test":t.Name(), "input":map[string]any{"engine":engine, "objects":len(g.Objects), "edges":edges, "canceled":canceled}, "result":map[string]any{"error":message}}
 dir := filepath.Join(os.Getenv("D2RS_LAYOUT_ADMISSION_CAPTURE"), "D2RS_PACKAGE", strings.ReplaceAll(t.Name(), "/", "_"))
 if err := os.MkdirAll(dir, 0700); err != nil { t.Fatal(err) }
 // Multiple calls in one test remain ordered; parallel subtests own directories.
 files, errRead := filepath.Glob(filepath.Join(dir,"*.json")); if errRead != nil { t.Fatal(errRead) }
 data, errJSON := json.Marshal(value); if errJSON != nil { t.Fatal(errJSON) }
 name := filepath.Join(dir, fmtAdmissionIndex(len(files))+".json")
 if errWrite := os.WriteFile(name, data, 0600); errWrite != nil { t.Fatal(errWrite) }
 return err
}

func fmtAdmissionIndex(i int) string { return string(rune('0'+i/10))+string(rune('0'+i%10)) }
