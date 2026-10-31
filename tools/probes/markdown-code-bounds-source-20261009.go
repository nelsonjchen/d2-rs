// SPDX-License-Identifier: MPL-2.0
// Virtual trace-worktree probe of original production Ruler code bounds.
package textmeasure
import("encoding/json";"os";"testing";"github.com/d2lang/d2/d2renderers/d2fonts")
func TestCodexCodeBounds(t *testing.T){
 ruler,err:=NewRuler();if err!=nil{t.Fatal(err)};ruler.boundsWithDot=true;rows:=[]map[string]any{}
 texts:=[]string{"","a","a\n","\na","\n\na","a\n\nb","a\n\n"," ","\n "," \n","a\n ","\n \na","\t","\n\t","\t\na","a\r\nb","a\rb","é\n中","😀\nβ","\na\n \n","\x00\na","a\t\nb"}
 for _,family:=range []d2fonts.FontFamily{d2fonts.SourceSansPro,d2fonts.SourceCodePro,d2fonts.HandDrawn}{for _,style:=range []d2fonts.FontStyle{d2fonts.FONT_STYLE_REGULAR,d2fonts.FONT_STYLE_SEMIBOLD,d2fonts.FONT_STYLE_BOLD,d2fonts.FONT_STYLE_ITALIC}{for _,size:=range []int{9,12,16,22,32}{for _,factor:=range []float64{1,1.45,1.5}{for _,text:=range texts{
 ruler.LineHeightFactor=factor;font:=d2fonts.Font{Family:family,Style:style,Size:size};w,h:=ruler.MeasurePrecise(font,text);rows=append(rows,map[string]any{"family":family,"style":style,"size":size,"factor":factor,"text":text,"width":w,"height":h})
 }}}}}
 data,err:=json.Marshal(rows);if err!=nil{t.Fatal(err)};if err=os.WriteFile(os.Getenv("CODEX_CODE_BOUNDS_OUTPUT"),append(data,'\n'),0644);err!=nil{t.Fatal(err)}
}
