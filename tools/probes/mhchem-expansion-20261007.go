// SPDX-License-Identifier: MPL-2.0
package d2latex
import("encoding/json";"os";"testing";_ "unsafe")
//go:linkname originalMhchemExpand github.com/d2lang/mathjax-go/internal/mhchem.ToTeX
func originalMhchemExpand(string,string)(string,error)
func TestCodexMhchemExpansion(t *testing.T){data,e:=os.ReadFile(os.Getenv("CODEX_MHC_INPUT"));if e!=nil{t.Fatal(e)};var rows []map[string]any;if e=json.Unmarshal(data,&rows);e!=nil{t.Fatal(e)}
 for _,r:=range rows {input:=r["input"].(string);mode:=r["mode"].(string);expansion,e:=originalMhchemExpand(input,mode);r["expansion"]=expansion;r["expansion_error"]=nil;if e!=nil{r["expansion_error"]=e.Error();continue};expression:="\\"+mode+"{"+input+"}";r["expression"]=expression;svg,e:=Render(expression);if e!=nil{t.Fatal(e)};w,h,e:=Measure(expression);if e!=nil{t.Fatal(e)};r["svg"]=svg;r["width"]=w;r["height"]=h }
 out,e:=json.MarshalIndent(rows,"","  ");if e!=nil{t.Fatal(e)};if e=os.WriteFile(os.Getenv("CODEX_MHC_OUTPUT"),append(out,'\n'),0644);e!=nil{t.Fatal(e)}}
