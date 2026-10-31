// SPDX-License-Identifier: MPL-2.0
// Original Go D2 v0.9.0 public raster controls; loaded only through an overlay.
package d2raster
import("context";"crypto/sha256";"encoding/json";"fmt";"image";"image/color";"image/draw";"os";"path/filepath";"testing";"github.com/d2lang/d2/d2renderers/d2scene";"github.com/d2lang/d2/d2renderers/d2fonts")
func TestCodexRasterBands(t *testing.T){
 out:=os.Getenv("CODEX_RASTER_BANDS_OUTPUT");if out==""{t.Fatal("missing output")};rows:=[]map[string]any{}
 face,ok:=d2fonts.FontFaces.Lookup(d2fonts.Font{Family:d2fonts.SourceSansPro,Style:d2fonts.FONT_STYLE_REGULAR});if !ok{t.Fatal("font")}
 for _,mode:=range []string{"plain","opacity","nested","multiply","mask","clip","shadow"}{for _,scale:=range []int{1,2}{
  first:=d2scene.NewNode(d2scene.Rect{Box:d2scene.Box{X:10.25,Y:110.75,Width:110.5,Height:100.75},Fill:d2scene.SolidPaint{Color:color.NRGBA{R:237,G:240,B:253,A:255}},Stroke:&d2scene.Stroke{Paint:d2scene.SolidPaint{Color:color.NRGBA{R:13,G:50,B:178,A:255}},Width:2,Join:d2scene.JoinRound,Cap:d2scene.CapButt,MiterLimit:4}})
  second:=d2scene.NewNode(d2scene.Rect{Box:d2scene.Box{X:60.125,Y:180.25,Width:120.25,Height:100.5},Fill:d2scene.SolidPaint{Color:color.NRGBA{R:230,G:90,B:40,A:255}}})
  label:=d2scene.NewNode(d2scene.TextRun{Text:"default",Origin:d2scene.Point{X:150.25,Y:300.6666666666667},Anchor:d2scene.AnchorMiddle,Font:d2scene.Font{Asset:"font",Size:24},Fill:d2scene.SolidPaint{Color:color.NRGBA{R:10,G:15,B:37,A:255}}})
  g:=d2scene.NewNode(nil);g.Children=[]*d2scene.Node{first,second,label}
  switch mode{case "opacity":g.Opacity=.6;case "nested":inner:=d2scene.NewNode(nil);inner.Opacity=.7;inner.Children=g.Children;g.Children=[]*d2scene.Node{inner};g.Opacity=.6;case "multiply":inner:=d2scene.NewNode(nil);inner.Blend=d2scene.BlendMultiply;inner.Children=[]*d2scene.Node{second,label};g.Children=[]*d2scene.Node{first,inner};case "mask":g.Mask=&d2scene.Mask{Type:d2scene.MaskLuminance,Transform:d2scene.Identity(),Root:d2scene.NewNode(d2scene.Rect{Box:d2scene.Box{Width:260,Height:400},Fill:d2scene.SolidPaint{Color:color.NRGBA{R:255,G:255,B:255,A:191}}})};case "clip":g.Clip=&d2scene.Clip{Transform:d2scene.Identity(),Path:d2scene.Path{Commands:[]d2scene.PathCommand{d2scene.MoveTo(20,90),d2scene.LineTo(190,90),d2scene.LineTo(190,310),d2scene.LineTo(20,310),d2scene.ClosePath()}}};case "shadow":g.Filters=[]d2scene.Filter{d2scene.DropShadow{OffsetX:3,OffsetY:5,Color:color.NRGBA{R:61,G:69,B:116,A:102}}};g.Children=[]*d2scene.Node{first,second}}
  doc:=d2scene.NewDocument(d2scene.Box{Width:260,Height:400},g);doc.Assets["font"]=d2scene.FontAsset{MIMEType:"font/ttf",Data:face};opts:=testOptions();opts.Scale=float64(scale);opts.Background=color.White
  for _,band:=range []int{0,64,256}{
   var frame *image.NRGBA;var err error
   if band==0{frame,err=Render(context.Background(),doc,opts)}else{frame=image.NewNRGBA(image.Rect(0,0,260*scale,400*scale));err=RenderBands(context.Background(),doc,opts,band,func(part *image.NRGBA)error{draw.Draw(frame,part.Bounds(),part,part.Bounds().Min,draw.Src);return nil})};if err!=nil{t.Fatal(mode,scale,band,err)}
   name:=fmt.Sprintf("%s-scale%d-band%d",mode,scale,band);data,err:=EncodePNG(context.Background(),frame);if err!=nil{t.Fatal(err)};if err=os.WriteFile(filepath.Join(out,name+".png"),data,0644);err!=nil{t.Fatal(err)}
   rows=append(rows,map[string]any{"mode":mode,"scale":scale,"band_height":band,"width":frame.Bounds().Dx(),"height":frame.Bounds().Dy(),"rgba_sha256":fmt.Sprintf("%x",sha256.Sum256(frame.Pix))})
  }
 }}
 data,err:=json.MarshalIndent(rows,"","  ");if err!=nil{t.Fatal(err)};if err=os.WriteFile(filepath.Join(out,"source-frames.json"),append(data,'\n'),0644);err!=nil{t.Fatal(err)}
}
