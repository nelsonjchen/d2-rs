// SPDX-License-Identifier: MPL-2.0
// Oracle hook (docs/FIXTURES.md): PNG image aspect placement and sampled pixels.
// Harvested from the v0.9.0-era probe tools/probes/png-image-aspect-20261009.go; retargeted to the manifest
// pin. Inert unless run by scripts/regen-fixtures.py.
package d2raster

import (
 "bytes"
 "context"
 "encoding/base64"
 "encoding/json"
 "fmt"
 "image"
 "image/color"
 "image/png"
 "os"
 "strings"
 "testing"
 "github.com/d2lang/d2/d2renderers/d2scene"
)

func TestD2RSPNGImageAspect(t *testing.T) {
 out := os.Getenv("D2RS_RECIPE_OUTPUT"); if out == "" { t.Skip("run via scripts/regen-fixtures.py") }

 aligns:=[]string{"none","xMinYMin","xMidYMin","xMaxYMin","xMinYMid","xMidYMid","xMaxYMid","xMinYMax","xMidYMax","xMaxYMax"}
 aspects:=[]string{"", "xMidYMid", "none meet", "none slice"}
 for _,a:=range aligns { aspects=append(aspects,a+" meet"); if a!="none" {aspects=append(aspects,a+" slice")} }
 boxes:=[][4]float64{{2.125,3.25,13.75,9.125},{-1.75,2.625,8.5,15.25},{5,7,6,4}}
 matrices:=[][6]float64{{1,0,0,1,0,0},{.85,.125,-.2,1.05,4.125,-1.25},{-1,.15,.2,.75,22,1.125},{0,0,0,1,9,0}}
 rows:=[]map[string]any{}
 for sourceIndex,dims:=range [][2]int{{3,2},{2,3}} {
  source:=image.NewNRGBA(image.Rect(0,0,dims[0],dims[1]));for y:=0;y<dims[1];y++ {for x:=0;x<dims[0];x++ {source.SetNRGBA(x,y,color.NRGBA{uint8(25+x*85),uint8(40+y*70),uint8(170-x*35+y*10),uint8(80+(x+y)*35)})}}
  var encoded bytes.Buffer;if err:=png.Encode(&encoded,source);err!=nil{t.Fatal(err)}
  asset,_,err:=prepareRasterAsset(context.Background(),"probe",d2scene.RasterAsset{MIMEType:"image/png",Data:encoded.Bytes(),PixelWidth:dims[0],PixelHeight:dims[1]},1<<20);if err!=nil{t.Fatal(err)}
  for ai,raw:=range aspects { fields:=strings.Fields(raw);align:=d2scene.AlignXMidYMid;fit:=d2scene.AspectMeet;if len(fields)>0 {for index,a:=range aligns {if a==fields[0] {align=d2scene.AspectAlign(index);break}}};if len(fields)>1&&fields[1]=="slice" {fit=d2scene.AspectSlice}
   for bi,b:=range boxes {box:=d2scene.Box{X:b[0],Y:b[1],Width:b[2],Height:b[3]};placement,err:=imagePlacement(box,dims[0],dims[1],d2scene.AspectRatio{Align:align,Fit:fit});if err!=nil{t.Fatal(err)}
    for mi,m:=range matrices {transform:=d2scene.Matrix{A:m[0],B:m[1],C:m[2],D:m[3],E:m[4],F:m[5]};inverse,invertible,err:=makeInverseAffine(transform);if err!=nil{t.Fatal(err)}
     destination:=image.NewRGBA(image.Rect(0,0,24,24));for i:=0;i<len(destination.Pix);i+=4 {copy(destination.Pix[i:i+4],[]byte{17,29,43,255})}
     bounds,err:=transformedImagePixelBounds(box,transform,destination.Bounds());if err!=nil{t.Fatal(err)};if !invertible {bounds=image.Rectangle{}}
     prepared:=&preparedImage{asset:asset,box:box,placement:placement,inverse:inverse,bounds:bounds};if err:=drawSampledPreparedImage(context.Background(),destination,prepared,bounds);err!=nil{t.Fatal(err)}
     rows=append(rows,map[string]any{"name":fmt.Sprintf("s%d-a%d-b%d-m%d",sourceIndex,ai,bi,mi),"aspect":raw,"box":b,"matrix":m,"placement":[4]float64{placement.X,placement.Y,placement.Width,placement.Height},"png":base64.StdEncoding.EncodeToString(encoded.Bytes()),"pixels":base64.StdEncoding.EncodeToString(destination.Pix)})
    }
   }
  }
 }
 data,err:=json.Marshal(rows);if err!=nil{t.Fatal(err)};if err=os.WriteFile(out,append(data,'\n'),0644);err!=nil{t.Fatal(err)}
}
