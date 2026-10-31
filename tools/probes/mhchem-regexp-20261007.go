// SPDX-License-Identifier: MPL-2.0
package d2latex
import("encoding/json";"os";"testing";_ "unsafe")
// ABI mirrors of original unexported match result. No source is replaced.
type codexJsGroup struct { text string; Length int }
type codexJsMatch struct { Index int; Length int; text string; groups []codexJsGroup }
//go:linkname originalMhchemCompile github.com/d2lang/mathjax-go/internal/mhchem.compileJSRegexp
func originalMhchemCompile(string)(*byte,error)
//go:linkname originalMhchemFind github.com/d2lang/mathjax-go/internal/mhchem.(*jsRegexp).FindStringMatch
func originalMhchemFind(*byte,string)(*codexJsMatch,error)
func TestCodexMhchemRegexp(t *testing.T){
 data,e:=os.ReadFile(os.Getenv("CODEX_MHC_INPUT"));if e!=nil{t.Fatal(e)}
 var rows []map[string]any;if e=json.Unmarshal(data,&rows);e!=nil{t.Fatal(e)}
 for _,r:=range rows {
  re,e:=originalMhchemCompile(r["source"].(string));if e!=nil{t.Fatal(e)}
  m,e:=originalMhchemFind(re,r["input"].(string));if e!=nil{t.Fatal(e)};r["match"]=nil
  if m!=nil{groups:=[]string{};for _,g:=range m.groups{groups=append(groups,g.text)};r["match"]=map[string]any{"index":m.Index,"length":m.Length,"text":m.text,"groups":groups}}
 }
 out,e:=json.MarshalIndent(rows,"","  ");if e!=nil{t.Fatal(e)}
 if e=os.WriteFile(os.Getenv("CODEX_MHC_OUTPUT"),append(out,'\n'),0644);e!=nil{t.Fatal(e)}
}
