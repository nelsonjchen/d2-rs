// SPDX-License-Identifier: MPL-2.0
// Oracle hook (docs/FIXTURES.md): probeResource SVG codec outcomes for generated requests.
// Harvested from the v0.9.0-era probe lib/imageasset/codec_contract_probe_test.go; retargeted to the manifest
// pin. Inert unless run by scripts/regen-fixtures.py.
package imageasset
import("context";"encoding/base64";"encoding/json";"os";"testing")
func TestD2RSSVGAssetCodec(t *testing.T) {
 out := os.Getenv("D2RS_RECIPE_OUTPUT"); if out == "" { t.Skip("run via scripts/regen-fixtures.py") }

 type row struct{Name string `json:"name"`;Data string `json:"data_base64"`;Declared bool `json:"declared"`;Limit int64 `json:"limit"`;Error string `json:"error"`;Decoded string `json:"decoded_base64"`}
 data,err:=os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"));if err!=nil{t.Fatal(err)};rows:=[]row{};if err=json.Unmarshal(data,&rows);err!=nil{t.Fatal(err)}
 for index:=range rows {bytes,decodeErr:=base64.StdEncoding.DecodeString(rows[index].Data);if decodeErr!=nil{t.Fatal(decodeErr)};hint:="";if rows[index].Declared{hint="image/svg+xml"};limits:=generousLimits();if rows[index].Limit>0{limits.MaxSVGBytes=rows[index].Limit};resource,probeErr:=probeResource(context.Background(),loadedSource{data:bytes,mimeHint:hint},limits,func(int64,int64)(func(),error){return func(){},nil});if probeErr!=nil{rows[index].Error=probeErr.Error()}else{rows[index].Decoded=base64.StdEncoding.EncodeToString(resource.cloneBytes())}}
 data,err=json.MarshalIndent(rows,"","  ");if err!=nil{t.Fatal(err)};if err=os.WriteFile(out,data,0644);err!=nil{t.Fatal(err)}
}
