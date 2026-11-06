// SPDX-License-Identifier: MPL-2.0
package imageasset

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"testing"

	"github.com/d2lang/d2/lib/localfile"
)

var d2rsPolicyRows []interface{}
var d2rsPolicyIDs map[*Resolver]int
var d2rsPolicyCaches map[*MemoryCache]int

func d2rsPolicyStart(t *testing.T) {
	d2rsPolicyRows = []interface{}{}
	d2rsPolicyIDs = map[*Resolver]int{}
	d2rsPolicyCaches = map[*MemoryCache]int{}
	t.Cleanup(func() {
		if dir := os.Getenv("D2RS_IMAGEASSET_POLICY"); dir != "" {
			data, e := json.Marshal(map[string]interface{}{"name": t.Name(), "operations": d2rsPolicyRows})
			if e != nil {
				t.Fatal(e)
			}
			if e = os.WriteFile(filepath.Join(dir, t.Name()+".json"), data, 0600); e != nil {
				t.Fatal(e)
			}
		}
	})
}
func d2rsPolicyErr(e error) string {
	if e != nil {
		return e.Error()
	}
	return ""
}
func d2rsPolicyRecord(request, got interface{}) {
	d2rsPolicyRows = append(d2rsPolicyRows, map[string]interface{}{"request": request, "got": got})
}
func d2rsPolicyTemp(t *testing.T) string {
	path := t.TempDir()
	d2rsPolicyRecord(map[string]interface{}{"op": "mkdir", "path": path, "mode": 0700}, map[string]interface{}{"error": ""})
	return path
}
func d2rsPolicyMkdir(path string, mode os.FileMode) error {
	e := os.Mkdir(path, mode)
	d2rsPolicyRecord(map[string]interface{}{"op": "mkdir", "path": path, "mode": mode}, map[string]interface{}{"error": d2rsPolicyErr(e)})
	return e
}
func d2rsPolicyWrite(path string, data []byte, mode os.FileMode) error {
	e := os.WriteFile(path, data, mode)
	d2rsPolicyRecord(map[string]interface{}{"op": "write", "path": path, "body": data, "mode": mode}, map[string]interface{}{"error": d2rsPolicyErr(e)})
	return e
}
func d2rsPolicySymlink(target, path string) error {
	e := os.Symlink(target, path)
	d2rsPolicyRecord(map[string]interface{}{"op": "symlink", "target": target, "path": path}, map[string]interface{}{"error": d2rsPolicyErr(e)})
	return e
}
func d2rsPolicyNew(o Options) (*Resolver, error) {
	policy, root := "denied", ""
	if path, ok := o.LocalFiles.RootPath(); ok {
		policy, root = "rooted", path
	} else if _, e := o.LocalFiles.CacheKey("."); !errors.Is(e, localfile.ErrDenied) {
		policy = "unrestricted"
	}
	request := map[string]interface{}{"op": "new", "base_dir": o.BaseDir, "policy": policy, "root": root, "limits": o.Limits, "namespace": o.CacheNamespace, "cache": 0}
	if cache, ok := o.Cache.(*MemoryCache); ok {
		id, found := d2rsPolicyCaches[cache]
		if !found {
			id = len(d2rsPolicyCaches) + 1
			d2rsPolicyCaches[cache] = id
		}
		request["cache"], request["cache_entries"], request["cache_bytes"] = id, cache.maxEntries, cache.maxBytes
	}
	r, e := New(o)
	id := len(d2rsPolicyIDs) + 1
	d2rsPolicyIDs[r], request["id"] = id, id
	d2rsPolicyRecord(request, map[string]interface{}{"error": d2rsPolicyErr(e)})
	return r, e
}
func d2rsPolicyResolve(r *Resolver, ctx context.Context, source string) (*Resource, error) {
	resource, e := r.Resolve(ctx, source)
	got := map[string]interface{}{"error": d2rsPolicyErr(e), "unavailable": errors.Is(e, ErrUnavailable), "resource": nil}
	if resource != nil {
		got["resource"] = map[string]interface{}{"kind": resource.Kind().String(), "mime": resource.MIMEType(), "width": resource.PixelWidth(), "height": resource.PixelHeight(), "decoded": resource.DecodedBytes(), "data": resource.cloneBytes()}
	}
	if cache, ok := r.cache.(*MemoryCache); ok {
		got["cache_entries"], got["cache_bytes"] = len(cache.entries), cache.bytes
	}
	d2rsPolicyRecord(map[string]interface{}{"op": "resolve", "id": d2rsPolicyIDs[r], "source": source}, got)
	return resource, e
}
