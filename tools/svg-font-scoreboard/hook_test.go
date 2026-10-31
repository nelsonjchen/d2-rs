// SPDX-License-Identifier: MPL-2.0
// Observe original calls and font registration; preserve every assertion.
package d2svg_test

import (
 "crypto/sha256"
 "encoding/json"
 "fmt"
 "os"
 "path/filepath"
 "sync"
 "testing"

 "github.com/d2lang/d2/d2renderers/d2fonts"
 "github.com/d2lang/d2/d2renderers/d2svg"
 "github.com/d2lang/d2/d2target"
)
var d2rsMu sync.Mutex
var d2rsCounts = map[string]int{}
var d2rsFamilies = map[string][]any{}
func d2rsRecord(t *testing.T, request any, output any, err error) {
 t.Helper()
 if os.Getenv("D2RS_SVG_ADMISSION_DUMP") == "" { return }
 d2rsMu.Lock(); defer d2rsMu.Unlock()
 name := fmt.Sprintf("%s#%d", t.Name(), d2rsCounts[t.Name()]); d2rsCounts[t.Name()]++
 message := ""; if err != nil { message = err.Error() }
 row := map[string]any{"name":name,"test":t.Name(),"request":request,"got":map[string]any{"output":output,"error":message}}
 data,e:=json.Marshal(row);if e!=nil {t.Fatal(e)}
 key:=fmt.Sprintf("%x",sha256.Sum256([]byte(name)))
 if e:=os.WriteFile(filepath.Join(os.Getenv("D2RS_SVG_ADMISSION_DUMP"),key+".json"),data,0600);e!=nil {t.Fatal(e)}
}
func d2rsAddFontFamily(t *testing.T, name string, regular, italic, bold, semibold []byte) (*d2fonts.FontFamily,error) {
 family,err:=d2fonts.AddFontFamily(name,regular,italic,bold,semibold)
 if err==nil {d2rsMu.Lock(); d2rsFamilies[t.Name()]=append(d2rsFamilies[t.Name()],map[string]any{"name":name,"regular":regular,"italic":italic,"bold":bold,"semibold":semibold}); d2rsMu.Unlock()}
 return family,err
}
func d2rsRender(t *testing.T, d *d2target.Diagram, opts *d2svg.RenderOpts) ([]byte,error) {
 out,err:=d2svg.Render(d,opts)
 d2rsMu.Lock();families:=d2rsFamilies[t.Name()];d2rsMu.Unlock()
 d2rsRecord(t,map[string]any{"api":"Render","diagram":d,"opts":opts,"families":families},out,err)
 return out,err
}
func d2rsThemeCSS(t *testing.T, hash string, theme, dark *int64, overrides,darkOverrides *d2target.ThemeOverrides) (string,error) {
 out,err:=d2svg.ThemeCSS(hash,theme,dark,overrides,darkOverrides)
 d2rsRecord(t,map[string]any{"api":"ThemeCSS","hash":hash,"opts":map[string]any{"ThemeID":theme,"DarkThemeID":dark,"ThemeOverrides":overrides,"DarkThemeOverrides":darkOverrides}},out,err)
 return out,err
}
