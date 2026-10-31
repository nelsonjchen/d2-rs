// SPDX-License-Identifier: MPL-2.0
// Original Go D2 v0.9.0 import/raster controls, loaded only by virtual overlay.
package d2raster
import("context";"crypto/sha256";"encoding/json";"fmt";"image/color";"os";"path/filepath";"testing";"github.com/d2lang/d2/d2renderers/d2scene";"github.com/d2lang/d2/d2renderers/d2svgimport")
func TestCodexImportedPrimitives(t *testing.T){
 out:=os.Getenv("CODEX_IMPORTED_PRIMITIVES_OUTPUT");rows:=[]map[string]any{}
 bodies:=map[string]string{
 "ellipse":`<ellipse fill="#5ee1eb" cx="63.976" cy="56.766" rx="22.11" ry="12.555"/>`,
 "circle":`<circle fill="#2db357" cx="61.333333333333336" cy="53.875" r="21.125"/>`,
 "polygon":`<polygon fill="#0e834c" points="10.125,20.625 90.33333333333333,44.125 113.75,95.125 55.976,102.766"/>`,
 "polyline":`<polyline fill="#2db357" points="10.125,20.625 90.33333333333333,44.125 113.75,95.125 55.976,102.766"/>`,
 "mixed":`<polygon fill="#2db357" points="10.125,20.625 90.33333333333333,44.125 113.75,95.125 55.976,102.766"/><ellipse fill="#5ee1eb" cx="63.976" cy="56.766" rx="22.11" ry="12.555"/><circle fill="#0e834c" cx="55.976" cy="64.766" r="8.125"/>`,
 "rotated":`<g transform="matrix(0 1 -1 0 128 0)"><ellipse fill="#5ee1eb" cx="63.976" cy="56.766" rx="22.11" ry="12.555"/><polygon fill="#0e834c" points="10.125,20.625 90.33333333333333,44.125 113.75,95.125 55.976,102.766"/></g>`,
 }
 for _, mode:=range []string{"ellipse","circle","polygon","polyline","mixed","rotated"}{
  svg:=`<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128">`+bodies[mode]+`</svg>`
  limits:=d2svgimport.Limits{MaxBytes:1<<20,MaxDepth:64,MaxElements:1000,MaxAttributes:10000,MaxAttributeBytes:1<<20,MaxPathCommands:10000,MaxTransformFunctions:1000,MaxUseDepth:32,MaxResources:1000}
  imported,err:=d2svgimport.ImportNode(context.Background(),"control.svg",[]byte(svg),limits);if err!=nil{t.Fatal(err)}
  node:=d2scene.NewNode(d2scene.Image{Asset:"control",Box:d2scene.Box{X:16,Y:32,Width:128,Height:128},Aspect:d2scene.AspectRatio{Align:d2scene.AlignXMidYMid,Fit:d2scene.AspectMeet}})
  doc:=d2scene.NewDocument(d2scene.Box{Width:192,Height:192},node);doc.Assets["control"]=d2scene.VectorAsset{Root:imported.Root,ViewBox:imported.ViewBox}
  for _,scale:=range []int{1,2}{opts:=testOptions();opts.Scale=float64(scale);opts.Background=color.White;frame,err:=Render(context.Background(),doc,opts);if err!=nil{t.Fatal(err)};png,err:=EncodePNG(context.Background(),frame);if err!=nil{t.Fatal(err)};name:=fmt.Sprintf("%s-scale%d",mode,scale);if err=os.WriteFile(filepath.Join(out,name+".png"),png,0644);err!=nil{t.Fatal(err)};rows=append(rows,map[string]any{"mode":mode,"scale":scale,"asset_svg":svg,"width":frame.Bounds().Dx(),"height":frame.Bounds().Dy(),"rgba_sha256":fmt.Sprintf("%x",sha256.Sum256(frame.Pix))})}
 }
 data,err:=json.MarshalIndent(rows,"","  ");if err!=nil{t.Fatal(err)};if err=os.WriteFile(filepath.Join(out,"source-frames.json"),append(data,'\n'),0644);err!=nil{t.Fatal(err)}
}
