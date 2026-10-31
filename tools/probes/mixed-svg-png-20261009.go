// SPDX-License-Identifier: MPL-2.0
// Original import/placement/clip/opacity/gradient/raster public API controls.
package d2svgimport
import("context";"crypto/sha256";"encoding/hex";"encoding/json";"image/color";"os";"testing";"github.com/d2lang/d2/d2renderers/d2scene";"github.com/d2lang/d2/d2renderers/d2raster")
func TestCodexMixedPNG(t *testing.T) {
 data,err:=os.ReadFile(os.Getenv("CODEX_MIXED_PNG_INPUT"));if err!=nil{t.Fatal(err)}
 var requests []struct{Name,SVG,Aspect string;Box,Canvas [4]float64;Scale float64};if err=json.Unmarshal(data,&requests);err!=nil{t.Fatal(err)};rows:=[]map[string]any{}
 for _,r:=range requests {
  imported,err:=ImportNode(context.Background(),"mixed",[]byte(r.SVG),generousImportLimits());if err!=nil{t.Fatal(r.Name,err)}
  aspect:=d2scene.AspectRatio{Align:d2scene.AlignXMidYMid,Fit:d2scene.AspectMeet};if r.Aspect=="none"{aspect.Align=d2scene.AlignNone}
  content:=d2scene.NewNode(nil);content.Transform=imported.ViewportTransform;content.Children=[]*d2scene.Node{imported.Root}
  viewport:=d2scene.NewNode(nil);viewport.Children=[]*d2scene.Node{content};w,h:=imported.Width,imported.Height
  viewport.Clip=&d2scene.Clip{Transform:d2scene.Identity(),Path:d2scene.Path{Commands:[]d2scene.PathCommand{d2scene.MoveTo(0,0),d2scene.LineTo(w,0),d2scene.LineTo(w,h),d2scene.LineTo(0,h),d2scene.ClosePath()}}}
  host:=d2scene.NewNode(d2scene.Image{Asset:"vector",Box:d2scene.Box{X:r.Box[0],Y:r.Box[1],Width:r.Box[2],Height:r.Box[3]},Aspect:aspect})
  document:=d2scene.NewDocument(d2scene.Box{X:r.Canvas[0],Y:r.Canvas[1],Width:r.Canvas[2],Height:r.Canvas[3]},host);document.Assets["vector"]=d2scene.VectorAsset{ViewBox:d2scene.Box{Width:w,Height:h},Root:viewport};for id,asset:=range imported.Assets{document.Assets[id]=asset}
  frame,err:=d2raster.Render(context.Background(),document,d2raster.FrameOptions{Scale:r.Scale,Background:color.White,MaxWidth:256,MaxHeight:256,MaxPixels:65536,MaxNodes:10000,MaxDepth:100,MaxPathCommands:100000,MaxAnimationTracks:1,MaxAnimationKeyframes:1,MaxAssets:10,MaxAssetBytes:2<<20,MaxDecodedAssetBytes:2<<20,MaxImportDepth:10,MaxOffscreenBytes:8<<20,MaxEvenOddClipWork:8<<20});if err!=nil{t.Fatal(r.Name,err)}
  pixels:=[]byte{};for y:=0;y<frame.Bounds().Dy();y++{for x:=0;x<frame.Bounds().Dx();x++{c:=frame.NRGBAAt(x,y);pixels=append(pixels,c.R,c.G,c.B,c.A)}};digest:=sha256.Sum256(pixels);if folder:=os.Getenv("CODEX_MIXED_PNG_PIXELS_DIR");folder!=""{if err:=os.MkdirAll(folder,0755);err!=nil{t.Fatal(err)};if err:=os.WriteFile(folder+"/"+r.Name+".rgba",pixels,0644);err!=nil{t.Fatal(err)}}
  display,err:=d2scene.AspectRatioMatrix(d2scene.Box{Width:w,Height:h},d2scene.Box{Width:r.Box[2],Height:r.Box[3]},aspect);if err!=nil{t.Fatal(err)}
  prefix:=d2scene.Scale(r.Scale,r.Scale).Mul(d2scene.Translate(-r.Canvas[0],-r.Canvas[1])).Mul(d2scene.Translate(r.Box[0],r.Box[1])).Mul(display).Mul(imported.ViewportTransform);images:=[]map[string]any{}
  var walk func(*d2scene.Node,d2scene.Matrix);walk=func(node *d2scene.Node,parent d2scene.Matrix){matrix:=parent.Mul(node.Transform);if primitive,ok:=node.Primitive.(d2scene.Image);ok{images=append(images,map[string]any{"matrix":[6]float64{matrix.A,matrix.B,matrix.C,matrix.D,matrix.E,matrix.F},"box":[4]float64{primitive.Box.X,primitive.Box.Y,primitive.Box.Width,primitive.Box.Height}})};for _,child:=range node.Children{walk(child,matrix)}};walk(imported.Root,prefix)
  rows=append(rows,map[string]any{"request":r,"images":images,"dimensions":[2]int{frame.Bounds().Dx(),frame.Bounds().Dy()},"sha256":hex.EncodeToString(digest[:])})
 }
 data,err=json.Marshal(rows);if err!=nil{t.Fatal(err)};if err=os.WriteFile(os.Getenv("CODEX_MIXED_PNG_OUTPUT"),append(data,'\n'),0644);err!=nil{t.Fatal(err)}
}
