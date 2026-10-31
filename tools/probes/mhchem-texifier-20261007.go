// SPDX-License-Identifier: MPL-2.0
package d2latex
import("encoding/json";"os";"testing";"reflect";_ "unsafe")
//go:linkname originalMhchemTexify github.com/d2lang/mathjax-go/internal/mhchem.texify
func originalMhchemTexify([]any,bool)(string,error)
func TestCodexMhchemTexifier(t *testing.T){
 data,e:=os.ReadFile(os.Getenv("CODEX_MHC_INPUT"));if e!=nil{t.Fatal(e)}
 var rows []map[string]any;if e=json.Unmarshal(data,&rows);e!=nil{t.Fatal(e)}
 for _,r:=range rows{values:=[]any{};for _,v:=range r["values"].([]any){values=append(values,v)}
 tex,e:=originalMhchemTexify(values,r["outer"].(bool));r["tex"]=tex;r["error"]=nil
 if e!=nil {x:=reflect.ValueOf(e).Elem();if !x.FieldByName("ID").IsValid(){t.Fatal(e)};r["error"]=map[string]any{"id":x.FieldByName("ID").String(),"message":e.Error()}}
 }
 out,e:=json.MarshalIndent(rows,"","  ");if e!=nil{t.Fatal(e)}
 if e=os.WriteFile(os.Getenv("CODEX_MHC_OUTPUT"),append(out,'\n'),0644);e!=nil{t.Fatal(e)}
}
