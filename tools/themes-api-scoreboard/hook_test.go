// SPDX-License-Identifier: MPL-2.0
// Observers preserve every original d2themes assertion and rethrow panics.
package d2themes

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"reflect"
	"sync"
	"testing"

	"github.com/d2lang/d2/lib/svg"
)

var d2rsThemeMu sync.Mutex
var d2rsThemeCounts = map[string]int{}
var d2rsThemeTags sync.Map
var d2rsThemeInner sync.Map

func d2rsThemeRecord(t *testing.T, request, got any) {
	d2rsThemeMu.Lock()
	defer d2rsThemeMu.Unlock()
	n := d2rsThemeCounts[t.Name()]
	d2rsThemeCounts[t.Name()] = n + 1
	name := fmt.Sprintf("%s#%d", t.Name(), n)
	data, err := json.Marshal(map[string]any{"name": name, "test": t.Name(), "request": request, "got": got})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_THEMES_DUMP"), fmt.Sprintf("%x.json", sha256.Sum256([]byte(name)))), data, 0600); err != nil {
		t.Fatal(err)
	}
}
func d2rsThemeNew(t *testing.T, tag string, theme *Theme) *ThemableElement {
	element := NewThemableElement(tag, theme)
	d2rsThemeTags.Store(element, tag)
	return element
}
func d2rsThemeText(t *testing.T, element *ThemableElement, value string) {
	d2rsThemeInner.Store(element, map[string]string{"kind": "text", "value": value})
	element.SetText(value)
}
func d2rsThemeSVG(t *testing.T, element *ThemableElement, value svg.Fragment) {
	d2rsThemeInner.Store(element, map[string]string{"kind": "trusted", "value": value.String()})
	element.SetInnerSVG(value)
}
func d2rsThemeMask(t *testing.T, element *ThemableElement, value string) {
	request := map[string]any{"op": "mask_url", "value": value}
	defer func() {
		if p := recover(); p != nil {
			d2rsThemeRecord(t, request, map[string]any{"panic": true})
			panic(p)
		}
	}()
	element.SetMaskUrl(value)
	d2rsThemeRecord(t, request, map[string]any{"value": ""})
}
func d2rsThemeRender(t *testing.T, element *ThemableElement) (out string) {
	tag, _ := d2rsThemeTags.Load(element)
	fields := map[string]string{}
	numbers := map[string]string{}
	value := reflect.ValueOf(element).Elem()
	for i := 0; i < value.NumField(); i++ {
		field := value.Type().Field(i)
		if field.PkgPath != "" {
			continue
		}
		switch value.Field(i).Kind() {
		case reflect.String:
			fields[field.Name] = value.Field(i).String()
		case reflect.Float64:
			numbers[field.Name] = fmt.Sprintf("%016x", math.Float64bits(value.Field(i).Float()))
		}
	}
	inner, _ := d2rsThemeInner.Load(element)
	request := map[string]any{"op": "render", "tag": tag, "strings": fields, "numbers": numbers, "inner": inner}
	defer func() {
		if p := recover(); p != nil {
			d2rsThemeRecord(t, request, map[string]any{"panic": true})
			panic(p)
		}
	}()
	out = element.Render()
	d2rsThemeRecord(t, request, map[string]any{"value": out})
	return
}
