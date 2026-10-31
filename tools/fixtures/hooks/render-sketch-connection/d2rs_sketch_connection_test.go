// SPDX-License-Identifier: MPL-2.0
// Oracle for crates/d2-rs-render/tests/fixtures/sketch-connection-source-v090.json
// (docs/FIXTURES.md): rough.js-port sketch connection routes, arrowhead
// drawables and arrowhead placement matrices through public d2sketch APIs.
// Harvested from .reference/sketch-connections-20261018/oracle.
// Overlaid into d2renderers/d2sketch as an external test (public APIs only); inert unless
// D2RS_RECIPE_OUTPUT is set.
package d2sketch_test

import (
	"testing"
 "context"
 "encoding/json"
 "os"
 "math"
 "github.com/d2lang/d2/d2renderers/d2scene"
 "github.com/d2lang/d2/d2renderers/d2sketch"
 "github.com/d2lang/d2/d2target"
)

func TestD2RSSketchConnection(t *testing.T) {
	outputPath := os.Getenv("D2RS_RECIPE_OUTPUT")
	if outputPath == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	stdout, err := os.Create(outputPath)
	if err != nil {
		t.Fatal(err)
	}
	defer stdout.Close()

 limits := d2sketch.SceneLimits{MaxSets:32,MaxOps:100000}
 routes := []d2scene.Path{
  {Commands:[]d2scene.PathCommand{d2scene.MoveTo(70,188),d2scene.LineTo(208,188)}},
  {Commands:[]d2scene.PathCommand{d2scene.MoveTo(-3.5,12.75),d2scene.LineTo(40.25,70),d2scene.CubicTo(61,31.5,95.25,-20,105,85)}},
 }
 long := d2scene.Path{Commands:[]d2scene.PathCommand{d2scene.MoveTo(0,0)}}
 for i:=1;i<=600;i++ {long.Commands=append(long.Commands,d2scene.LineTo(float64(i),float64(i%7)))}
 routes=append(routes,long)
 out:=map[string]any{}
 routeRows:=[]any{}
 for _,path:=range routes {
  drawable,err:=d2sketch.ConnectionDrawable(context.Background(),path,limits);if err!=nil {panic(err)}
  paths,err:=d2sketch.CompileScenePaths(context.Background(),drawable,limits);if err!=nil {panic(err)}
  routeRows=append(routeRows,map[string]any{"input":path.Commands,"paths":paths})
 }
 out["routes"]=routeRows
 arrowRows:=map[string]any{}
 for _,name:=range []string{"arrow","line","triangle","unfilled-triangle","diamond","filled-diamond","circle","filled-circle","box","filled-box","cross","cf-one","cf-many","cf-one-required","cf-many-required"} {
  drawables,ok:=d2sketch.ArrowheadDrawables(d2target.Arrowhead(name),"#123456",2);if !ok {panic(name)}
  paths:=[]d2sketch.ScenePath{}
  for _,d:=range drawables {items,err:=d2sketch.CompileScenePaths(context.Background(),d,limits);if err!=nil {panic(err)};paths=append(paths,items...)}
  arrowRows[name]=paths
 }
 out["arrows"]=arrowRows
 matrices:=[]any{}
 for _,target:=range []bool{false,true} {
  angle:=math.Atan2(5,20);x,y:=1.0,2.0
  if target {x,y=21,7} else {angle+=math.Pi}
  m:=d2scene.Translate(x,y).Mul(d2scene.Rotate(angle))
  matrices=append(matrices,[]float64{m.A,m.C,m.B,m.D,m.E,m.F})
 }
 out["matrices"]=matrices
 encoder:=json.NewEncoder(stdout);if err:=encoder.Encode(out);err!=nil {panic(err)}
}
