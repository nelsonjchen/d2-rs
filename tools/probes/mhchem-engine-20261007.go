// SPDX-License-Identifier: MPL-2.0
package d2latex
import("encoding/json";"os";"testing";"reflect";_ "unsafe")
//go:linkname originalMhchemEngineToTeX github.com/d2lang/mathjax-go/internal/mhchem.ToTeX
func originalMhchemEngineToTeX(string,string)(string,error)
func TestCodexMhchemEngine(t *testing.T){
 data,e:=os.ReadFile(os.Getenv("CODEX_MHC_INPUT"));if e!=nil{t.Fatal(e)}
 var rows []map[string]any;if e=json.Unmarshal(data,&rows);e!=nil{t.Fatal(e)}
 for _,r:=range rows {
  tex,e:=originalMhchemEngineToTeX(r["input"].(string),r["mode"].(string));r["tex"]=tex;r["error"]=nil
  if e!=nil{x:=reflect.ValueOf(e).Elem();r["error"]=map[string]any{"id":x.FieldByName("ID").String(),"message":e.Error()}}
  if want,ok:=r["upstream_want"];ok&&(e!=nil||tex!=want.(string)){t.Fatalf("upstream source assertion differs: %v",r)}
 }
 out,e:=json.MarshalIndent(rows,"","  ");if e!=nil{t.Fatal(e)}
 if e=os.WriteFile(os.Getenv("CODEX_MHC_OUTPUT"),append(out,'\n'),0644);e!=nil{t.Fatal(e)}
}
