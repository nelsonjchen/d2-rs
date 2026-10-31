// SPDX-License-Identifier: MPL-2.0
package imageasset

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"sync/atomic"
	"testing"
)

var d2rsNetworkRows []interface{}
var d2rsNetworkFixture map[string]interface{}
var d2rsNetworkIDs map[*Resolver]int
var d2rsNetworkCaches map[*MemoryCache]int
var d2rsNetworkHits atomic.Int32

func d2rsNetworkStart(t *testing.T) {
	d2rsNetworkRows = []interface{}{}
	d2rsNetworkIDs = map[*Resolver]int{}
	d2rsNetworkCaches = map[*MemoryCache]int{}
	d2rsNetworkHits.Store(0)
	t.Cleanup(func() {
		data, e := json.Marshal(map[string]interface{}{"name": t.Name(), "fixture": d2rsNetworkFixture, "operations": d2rsNetworkRows})
		if e != nil {
			t.Fatal(e)
		}
		if e = os.WriteFile(filepath.Join(os.Getenv("D2RS_IMAGEASSET_NETWORK"), t.Name()+".json"), data, 0600); e != nil {
			t.Fatal(e)
		}
	})
}
func d2rsNetworkServer(image []byte, handler http.Handler) *httptest.Server {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { d2rsNetworkHits.Add(1); handler.ServeHTTP(w, r) }))
	d2rsNetworkFixture = map[string]interface{}{"url": server.URL, "body": image}
	return server
}
func d2rsNetworkError(e error) string {
	if e != nil {
		return e.Error()
	}
	return ""
}
func d2rsNetworkRecord(request, got interface{}) {
	d2rsNetworkRows = append(d2rsNetworkRows, map[string]interface{}{"request": request, "got": got})
}
func d2rsNetworkNew(o Options) (*Resolver, error) {
	request := map[string]interface{}{"op": "new", "limits": o.Limits, "private": o.NetworkPolicy.AllowPrivateNetworks, "namespace": o.CacheNamespace, "cache": 0}
	if cache, ok := o.Cache.(*MemoryCache); ok {
		id, found := d2rsNetworkCaches[cache]
		if !found {
			id = len(d2rsNetworkCaches) + 1
			d2rsNetworkCaches[cache] = id
		}
		request["cache"], request["cache_entries"], request["cache_bytes"] = id, cache.maxEntries, cache.maxBytes
	}
	r, e := New(o)
	id := len(d2rsNetworkIDs) + 1
	d2rsNetworkIDs[r], request["id"] = id, id
	d2rsNetworkRecord(request, map[string]interface{}{"error": d2rsNetworkError(e)})
	return r, e
}
func d2rsNetworkResolve(r *Resolver, ctx context.Context, source string) (*Resource, error) {
	resource, e := r.Resolve(ctx, source)
	got := map[string]interface{}{"error": d2rsNetworkError(e), "unavailable": errors.Is(e, ErrUnavailable), "resource": nil, "hits": d2rsNetworkHits.Load()}
	if resource != nil {
		got["resource"] = map[string]interface{}{"kind": resource.Kind().String(), "mime": resource.MIMEType(), "width": resource.PixelWidth(), "height": resource.PixelHeight(), "decoded": resource.DecodedBytes(), "data": resource.cloneBytes()}
	}
	if cache, ok := r.cache.(*MemoryCache); ok {
		got["cache_entries"], got["cache_bytes"] = len(cache.entries), cache.bytes
	}
	d2rsNetworkRecord(map[string]interface{}{"op": "resolve", "id": d2rsNetworkIDs[r], "source": source}, got)
	return resource, e
}
