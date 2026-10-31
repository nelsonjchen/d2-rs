// SPDX-License-Identifier: MPL-2.0
// Oracle hook (docs/FIXTURES.md): mixed primary/emoji text frames over backgrounds and opacity.
// Harvested from the v0.9.0-era probe d2raster/mixed_color_paint_probe_test.go; retargeted to the manifest
// pin. Inert unless run by scripts/regen-fixtures.py.
package d2raster
import("testing";"context";"encoding/json";"os";"image/color";"math";"golang.org/x/image/math/fixed";"github.com/d2lang/d2/d2renderers/internal/fontface";"github.com/d2lang/d2/d2renderers/d2fonts";"github.com/d2lang/d2/d2renderers/d2scene")
func TestD2RSMixedColorPaint(t *testing.T) {
 out := os.Getenv("D2RS_RECIPE_OUTPUT"); if out == "" { t.Skip("run via scripts/regen-fixtures.py") }

 resolver,err:=d2fonts.NewBundledFallbackResolver(nil,d2fonts.BundledFallbackLimits{MaxRequestedRunes:16,MaxBundledBytes:8*1024*1024,MaxResolvedBytes:8*1024*1024});if err!=nil {t.Fatal(err)}
 fonts,err:=resolver.ResolveFallbacks(context.Background(),d2fonts.FallbackRequest{Runes:[]rune("😀")});if err!=nil {t.Fatal(err)}
 primary,err:=os.ReadFile("../d2fonts/ttf/SourceSansPro-Regular.ttf");if err!=nil{t.Fatal(err)}
 primaryFace,err:=fontface.ParseFace(primary,0);if err!=nil{t.Fatal(err)};notoFace,err:=fontface.ParseFace(fonts[0].Data,0);if err!=nil{t.Fatal(err)}
 rows:=[]any{};for _,size:=range []float64{10,16,13.6} {for _,text:=range []string{"Agm 😀 Agm","😀 A 😀","é 😀 é","Ж́ 😀 é"} {for _,background:=range []color.NRGBA{{},{R:245,G:246,B:253,A:255},{R:70,G:100,B:130,A:127}} {for _,opacity:=range []float64{1,0.5} {
  node:=d2scene.NewNode(d2scene.TextRun{Text:text,Origin:d2scene.Point{X:70.25,Y:42.75},Anchor:d2scene.AnchorMiddle,Font:d2scene.Font{Family:"Source Sans Pro",Size:size,Asset:"primary"},Fallbacks:[]d2scene.AssetID{"noto"},Fill:d2scene.SolidPaint{Color:color.NRGBA{R:75,G:78,B:96,A:255}}});node.Opacity=opacity
  doc:=d2scene.NewDocument(d2scene.Box{Width:140,Height:80},node);doc.Assets["primary"]=d2scene.FontAsset{MIMEType:"font/ttf",Data:primary};doc.Assets["noto"]=d2scene.FontAsset{MIMEType:"font/ttf",Data:fonts[0].Data};options:=testOptions();options.MaxNodes=20000;options.Background=background
  shaped,shapeErr:=new(fontface.ShapingWorkspace).ShapeTextTransient(context.Background(),text,fixed.Int26_6(math.Round(size*64)),[]fontface.ShapeFace{{ID:"primary",Face:primaryFace},{ID:"noto",Face:notoFace}},fontface.ShapeLimits{Runes:1000,Faces:64,CoverageChecks:10000,Runs:1000,Glyphs:1000});if shapeErr!=nil{t.Fatal(shapeErr)}
  frame,err:=Render(context.Background(),doc,options);row:=map[string]any{"shaped":shaped,"text":text,"size":size,"opacity":opacity,"background":[]uint8{background.R,background.G,background.B,background.A},"error":""};if err!=nil {row["error"]=err.Error()}else{row["width"]=frame.Bounds().Dx();row["height"]=frame.Bounds().Dy();row["pixels"]=frame.Pix};rows=append(rows,row)
 }}}}
 b,err:=json.MarshalIndent(rows,"","  ");if err!=nil{t.Fatal(err)};if err:=os.WriteFile(out,b,0644);err!=nil{t.Fatal(err)}
}
