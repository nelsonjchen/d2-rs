// SPDX-License-Identifier: MPL-2.0
package d2compiler_test
import("encoding/json";"encoding/hex";"encoding/xml";"os";"testing";"github.com/d2lang/d2/lib/textmeasure")
func TestCodexMarkdownBytes(t *testing.T){
 data,e:=os.ReadFile(os.Getenv("CODEX_MD_INPUT"));if e!=nil{t.Fatal(e)};var rows []map[string]any;if e=json.Unmarshal(data,&rows);e!=nil{t.Fatal(e)}
 for _,row:=range rows {
  var rendered string
  if value,ok:=row["source_hex"];ok {raw,e:=hex.DecodeString(value.(string));if e!=nil{t.Fatal(e)};var err error;rendered,err=textmeasure.RenderMarkdown(string(raw));if err!=nil{row["render_error"]=err.Error();continue};row["html_hex"]=hex.EncodeToString([]byte(rendered));rendered="<div>"+rendered+"</div>"
  }else{raw,e:=hex.DecodeString(row["xml_hex"].(string));if e!=nil{t.Fatal(e)};rendered=string(raw)}
  var parsed interface{};if err:=xml.Unmarshal([]byte(rendered),&parsed);err!=nil{if syntax,ok:=err.(*xml.SyntaxError);ok{row["xml_error"]=syntax.Msg}else{row["xml_error"]=err.Error()}}else{row["xml_error"]=""}
 };out,e:=json.MarshalIndent(rows,"","  ");if e!=nil{t.Fatal(e)};if e=os.WriteFile(os.Getenv("CODEX_MD_OUTPUT"),append(out,'\n'),0644);e!=nil{t.Fatal(e)}
}
