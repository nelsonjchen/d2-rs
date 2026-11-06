// SPDX-License-Identifier: MPL-2.0
// Test-only observers for pinned lib/svg/text_test.go. Original assertions
// execute on the original results; only API calls in selected tests are wrapped.
package svg

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"testing"
)

var d2rsSVGAPIMu sync.Mutex
var d2rsSVGAPICounts = map[string]int{}

func d2rsSVGAPIRecord(t *testing.T, request, got any, rawErrors ...error) {
	t.Helper()
	root := os.Getenv("D2RS_SVG_API_DUMP")
	if root == "" {
		return
	}
	d2rsSVGAPIMu.Lock()
	defer d2rsSVGAPIMu.Unlock()
	n := d2rsSVGAPICounts[t.Name()]
	d2rsSVGAPICounts[t.Name()] = n + 1
	row := map[string]any{"test": t.Name(), "name": fmt.Sprintf("%s#%d", t.Name(), n), "request": request, "got": got}
	if len(rawErrors) > 0 && rawErrors[0] != nil {
		row["raw_error"] = rawErrors[0].Error()
	}
	b, err := json.Marshal(row)
	if err != nil {
		t.Fatal(err)
	}
	// Subtest slashes are encoded into filenames; the original name stays in JSON.
	key := fmt.Sprintf("%x", sha256.Sum256([]byte(row["name"].(string))))
	if err := os.WriteFile(filepath.Join(root, key+".json"), b, 0600); err != nil {
		t.Fatal(err)
	}
}
func d2rsObserve[T any](t *testing.T, request any, call func() T, encode func(T) any) (got T) {
	defer func() {
		if p := recover(); p != nil {
			d2rsSVGAPIRecord(t, request, map[string]any{"panic": true})
			panic(p)
		}
	}()
	got = call()
	d2rsSVGAPIRecord(t, request, map[string]any{"value": encode(got)})
	return
}
func d2rsString(s string) any  { return s }
func d2rsPair(a Attribute) any { return []any{a.name, []byte(a.value)} }
func d2rsPairs(a []Attribute) any {
	if a == nil {
		return nil
	}
	out := make([][]any, 0, len(a))
	for _, x := range a {
		out = append(out, []any{x.name, []byte(x.value)})
	}
	return out
}
func d2rsEscapeAttribute(t *testing.T, s string) string {
	return d2rsObserve(t, map[string]any{"op": "escape_attribute", "bytes": []byte(s)}, func() string { return EscapeAttribute(s) }, d2rsString)
}
func d2rsAttr(t *testing.T, name, value string) Attribute {
	return d2rsObserve(t, map[string]any{"op": "attr", "name": name, "bytes": []byte(value)}, func() Attribute { return Attr(name, value) }, d2rsPair)
}
func d2rsRenderAttributes(t *testing.T, attrs ...Attribute) string {
	return d2rsObserve(t, map[string]any{"op": "render_attributes", "attributes": d2rsPairs(attrs)}, func() string { return RenderAttributes(attrs...) }, d2rsString)
}
func d2rsHrefAttributes(t *testing.T, s string) []Attribute {
	return d2rsObserve(t, map[string]any{"op": "href_attributes", "bytes": []byte(s)}, func() []Attribute { return HrefAttributes(s) }, d2rsPairs)
}
func d2rsScopedID(t *testing.T, prefix, component, suffix string) ID {
	return d2rsObserve(t, map[string]any{"op": "scoped_id", "prefix": prefix, "component": component, "suffix": suffix}, func() ID { return ScopedID(prefix, component, suffix) }, func(id ID) any { return id.String() })
}
func d2rsLocalIRI(t *testing.T, id ID) string {
	return d2rsObserve(t, map[string]any{"op": "local_iri", "id": id.String()}, func() string { return LocalIRI(id) }, d2rsString)
}
func d2rsParseLocalIRI(t *testing.T, s string) (ID, error) {
	id, err := ParseLocalIRI(s)
	result := map[string]any{"error": err != nil, "id": ""}
	if err == nil {
		result["id"] = id.String()
	}
	d2rsSVGAPIRecord(t, map[string]any{"op": "parse_local_iri", "value": s}, map[string]any{"value": result}, err)
	return id, err
}
func d2rsParseAttributes(t *testing.T, s string) ([]Attribute, error) {
	attrs, err := ParseAttributes(s)
	d2rsSVGAPIRecord(t, map[string]any{"op": "parse_attributes", "value": s}, map[string]any{"value": map[string]any{"error": err != nil, "attributes": d2rsPairs(attrs)}}, err)
	return attrs, err
}
func d2rsText(t *testing.T, s string) Fragment {
	return d2rsObserve(t, map[string]any{"op": "text", "bytes": []byte(s)}, func() Fragment { return Text(s) }, func(f Fragment) any { return f.String() })
}
func d2rsTrustedFragment(t *testing.T, s string) Fragment {
	return d2rsObserve(t, map[string]any{"op": "trusted_fragment", "bytes": []byte(s)}, func() Fragment { return TrustedFragment(s) }, func(f Fragment) any { return f.String() })
}
