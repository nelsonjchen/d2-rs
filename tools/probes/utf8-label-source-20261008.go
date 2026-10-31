// SPDX-License-Identifier: MPL-2.0
package d2compiler_test
import("encoding/json";"encoding/hex";"os";"strings";"testing";"unicode/utf8";"github.com/d2lang/d2/d2compiler")
func TestCodexUTF8Labels(t *testing.T){
 data,e:=os.ReadFile(os.Getenv("CODEX_UTF8_INPUT"));if e!=nil{t.Fatal(e)};var rows []map[string]any;if e=json.Unmarshal(data,&rows);e!=nil{t.Fatal(e)}
 for _,row:=range rows {
  g,_,err:=d2compiler.Compile("source.d2",strings.NewReader(row["input"].(string)),nil)
  if err!=nil {row["compile_error"]=err.Error();continue};objects:=[]map[string]any{}
  for _,o:=range g.Objects {v:=o.Label.Value;before:=hex.EncodeToString([]byte(v));o.ApplyTextTransform();v=o.Label.Value;objects=append(objects,map[string]any{"id":o.AbsID(),"before_hex":before,"after_hex":hex.EncodeToString([]byte(v)),"valid_utf8":utf8.ValidString(v)})};row["objects"]=objects
 };out,e:=json.MarshalIndent(rows,"","  ");if e!=nil{t.Fatal(e)};if e=os.WriteFile(os.Getenv("CODEX_UTF8_OUTPUT"),append(out,'\n'),0644);e!=nil{t.Fatal(e)}
}
