// SPDX-License-Identifier: MPL-2.0
// Requalify retained layout-context inputs at the current source callback boundary.
package main

import (
 "context"
 "encoding/json"
 "os"
 "time"

 "github.com/d2lang/d2/d2graph"
 "github.com/d2lang/d2/d2layouts"
 "github.com/d2lang/d2/d2layouts/d2dagrelayout"
 "github.com/d2lang/d2/d2layouts/d2elklayout"
 "github.com/d2lang/d2/d2layouts/d2talalayout"
 "github.com/d2lang/d2/d2lib"
 d2log "github.com/d2lang/d2/lib/log"
 "github.com/d2lang/d2/lib/textmeasure"
)

type input struct {
 Source string `json:"source"`
 Engine string `json:"engine"`
 Seeds []int64 `json:"seeds"`
 Cancel bool `json:"cancel"`
 Reason string `json:"reason"`
}

func main() {
 var requests []json.RawMessage
 if err := json.NewDecoder(os.Stdin).Decode(&requests); err != nil { panic(err) }
 results := []any{}
 for _, raw := range requests {
  var item input; if err := json.Unmarshal(raw, &item); err != nil { panic(err) }
  base := d2log.WithDefault(context.Background())
  var layoutContext context.Context; var cancel context.CancelFunc
  switch item.Reason {
  case "deadline", "deadline-then-cancel":
   layoutContext, cancel = context.WithDeadline(base, time.Now().Add(-time.Second))
   if item.Reason == "deadline-then-cancel" { cancel() }
  case "cancel-then-deadline":
   parent, cancelParent := context.WithCancel(base); cancelParent()
   layoutContext, cancel = context.WithDeadline(parent, time.Now().Add(-time.Second))
  default:
   layoutContext, cancel = context.WithCancel(base)
   if item.Cancel || item.Reason == "cancel" { cancel() }
  }
  options := &d2talalayout.Options{Seeds:item.Seeds, MaxConcurrency:1}
  resolver := func(engine string) (d2graph.LayoutGraph, error) {
   return func(_ context.Context, g *d2graph.Graph) error {
    switch engine {
    case "tala": return d2talalayout.Layout(layoutContext,g,options)
    case "dagre": return d2dagrelayout.DefaultLayout(layoutContext,g)
    default: return d2elklayout.DefaultLayout(layoutContext,g)
    }
   }, nil
  }
  router := func(engine string) (d2graph.RouteEdges, error) {
   return func(_ context.Context, g *d2graph.Graph, edges []*d2graph.Edge) error {
    if engine == "tala" { return d2talalayout.RouteEdges(layoutContext,g,edges) }
    return d2layouts.DefaultRouter(layoutContext,g,edges)
   }, nil
  }
  ruler, err := textmeasure.NewRuler(); if err != nil { panic(err) }
  // Parsing/compilation uses an active context. This tests the native source
  // adapter's existing post-compilation layout token, not Compile cancellation.
  _, _, err = d2lib.Compile(base,item.Source,&d2lib.CompileOptions{Layout:&item.Engine,Ruler:ruler,LayoutResolver:resolver,RouterResolver:router},nil)
  cancel(); message := ""; if err != nil { message = err.Error() }
  results = append(results,map[string]any{"input":raw,"error":message})
 }
 if err := json.NewEncoder(os.Stdout).Encode(results); err != nil { panic(err) }
}
