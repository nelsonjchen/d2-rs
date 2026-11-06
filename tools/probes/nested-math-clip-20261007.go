// SPDX-License-Identifier: MPL-2.0
// Original Go v0.9.0 MathJax vector import and raster controls, virtual overlay only.
package d2raster
import("context";"crypto/sha256";"encoding/json";"fmt";"image/color";"os";"path/filepath";"testing";"github.com/d2lang/d2/d2renderers/d2scene";"github.com/d2lang/d2/d2renderers/d2svgimport")
func TestCodexNestedMathClipFrames(t *testing.T){
 out:=os.Getenv("CODEX_MATH_VECTOR_OUTPUT");rows:=[]map[string]any{}
 data,err:=os.ReadFile(os.Getenv("CODEX_MATH_VECTOR_INPUT"));if err!=nil{t.Fatal(err)};var requests []struct{Name string `json:"name"`;SVG string `json:"svg"`};if err=json.Unmarshal(data,&requests);err!=nil{t.Fatal(err)}
 for _,request:=range requests {
  svg:=request.SVG
  limits:=d2svgimport.Limits{MaxBytes:1<<20,MaxDepth:64,MaxElements:1000,MaxAttributes:10000,MaxAttributeBytes:1<<20,MaxPathCommands:10000,MaxTransformFunctions:1000,MaxUseDepth:32,MaxResources:1000}
  ink:=color.NRGBA{R:10,G:15,B:37,A:255};imported,err:=d2svgimport.ImportNodeWithOptions(context.Background(),"math.svg",[]byte(svg),limits,d2svgimport.ImportOptions{CurrentColor:&ink});if err!=nil{t.Fatal(err)}
  content:=d2scene.NewNode(nil);content.Transform=imported.ViewportTransform;content.Children=[]*d2scene.Node{imported.Root}
  viewport:=d2scene.Box{Width:imported.Width,Height:imported.Height};root:=d2scene.NewNode(nil);root.Children=[]*d2scene.Node{content};root.Clip=&d2scene.Clip{Path:d2scene.Path{Commands:[]d2scene.PathCommand{d2scene.MoveTo(0,0),d2scene.LineTo(viewport.Width,0),d2scene.LineTo(viewport.Width,viewport.Height),d2scene.LineTo(0,viewport.Height),d2scene.ClosePath()}},Transform:d2scene.Identity()}
  node:=d2scene.NewNode(d2scene.Image{Asset:"math",Box:d2scene.Box{X:20.25,Y:40.5,Width:imported.Width,Height:imported.Height},Aspect:d2scene.AspectRatio{Align:d2scene.AlignXMidYMid,Fit:d2scene.AspectMeet}})
  doc:=d2scene.NewDocument(d2scene.Box{Width:192,Height:160},node);doc.Assets["math"]=d2scene.VectorAsset{Root:root,ViewBox:viewport}
  for _,scale:=range []int{1,2}{opts:=testOptions();opts.Scale=float64(scale);opts.Background=color.White;frame,err:=Render(context.Background(),doc,opts);if err!=nil{t.Fatal(err)};png,err:=EncodePNG(context.Background(),frame);if err!=nil{t.Fatal(err)};name:=fmt.Sprintf("%s-scale%d",request.Name,scale);if err=os.WriteFile(filepath.Join(out,name+".png"),png,0644);err!=nil{t.Fatal(err)};rows=append(rows,map[string]any{"name":name,"asset_svg":svg,"intrinsic_width":imported.Width,"intrinsic_height":imported.Height,"scale":scale,"width":frame.Bounds().Dx(),"height":frame.Bounds().Dy(),"rgba_sha256":fmt.Sprintf("%x",sha256.Sum256(frame.Pix))})}
 }
 data,err=json.MarshalIndent(rows,"","  ");if err!=nil{t.Fatal(err)};if err=os.WriteFile(filepath.Join(out,"source-frames.json"),append(data,'\n'),0644);err!=nil{t.Fatal(err)}
}
