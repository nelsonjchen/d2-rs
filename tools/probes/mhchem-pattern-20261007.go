// SPDX-License-Identifier: MPL-2.0
package d2latex
import("encoding/json";"os";"testing";"reflect";_ "unsafe")
type codexMatchToken struct { S string; A []string; IsArray bool }
type codexPatternHit struct { Token codexMatchToken; Remainder string }
//go:linkname originalMhchemData github.com/d2lang/mathjax-go/internal/mhchem.LoadData
func originalMhchemData()(*byte,error)
//go:linkname originalMhchemPattern github.com/d2lang/mathjax-go/internal/mhchem.matchPattern
func originalMhchemPattern(*byte,string,string)(*codexPatternHit,error)
func TestCodexMhchemPattern(t *testing.T){
 data,e:=os.ReadFile(os.Getenv("CODEX_MHC_INPUT"));if e!=nil{t.Fatal(e)}
 var rows []map[string]any;if e=json.Unmarshal(data,&rows);e!=nil{t.Fatal(e)}
 tables,e:=originalMhchemData();if e!=nil{t.Fatal(e)}
 for _,r:=range rows {
  m,e:=originalMhchemPattern(tables,r["name"].(string),r["input"].(string));r["hit"]=nil;r["error"]=nil
  if m!=nil {var token any=m.Token.S;if m.Token.IsArray{token=m.Token.A};r["hit"]=map[string]any{"token":token,"remainder":m.Remainder}}
  if e!=nil {x:=reflect.ValueOf(e).Elem();r["error"]=map[string]any{"id":x.FieldByName("ID").String(),"message":e.Error()}}
 }
 out,e:=json.MarshalIndent(rows,"","  ");if e!=nil{t.Fatal(e)}
 if e=os.WriteFile(os.Getenv("CODEX_MHC_OUTPUT"),append(out,'\n'),0644);e!=nil{t.Fatal(e)}
}
