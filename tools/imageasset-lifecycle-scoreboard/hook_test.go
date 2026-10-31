// SPDX-License-Identifier: MPL-2.0
package imageasset

import (
	"context"
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"testing"
)

var d2rsLifecycleRows []interface{}
var d2rsLifecycleIDs = map[*Resolver]int{}

func d2rsLifecycleStart(t *testing.T) {
	d2rsLifecycleRows = []interface{}{}
	d2rsLifecycleIDs = map[*Resolver]int{}
	t.Cleanup(func() {
		if dir := os.Getenv("D2RS_IMAGEASSET_LIFECYCLE"); dir != "" {
			data, e := json.Marshal(map[string]interface{}{"name": t.Name(), "operations": d2rsLifecycleRows})
			if e != nil {
				t.Fatal(e)
			}
			if e = os.WriteFile(filepath.Join(dir, t.Name()+".json"), data, 0600); e != nil {
				t.Fatal(e)
			}
		}
	})
}
func d2rsLifecycleRecord(request, got interface{}) {
	d2rsLifecycleRows = append(d2rsLifecycleRows, map[string]interface{}{"request": request, "got": got})
}
func d2rsLifecycleError(err error) string {
	if err != nil {
		return err.Error()
	}
	return ""
}
func d2rsLifecycleURI(t *testing.T, r *Resource, ctx context.Context) ([]byte, error) {
	var raw []byte
	mime := ""
	if r != nil {
		raw = r.cloneBytes()
		mime = r.MIMEType()
	}
	canceled := ctx != nil && ctx.Err() != nil
	out, e := r.DataURIContext(ctx)
	d2rsLifecycleRecord(map[string]interface{}{"op": "data-uri", "nil": r == nil, "data": raw, "mime": mime, "canceled": canceled}, map[string]interface{}{"data": append([]byte(nil), out...), "error": d2rsLifecycleError(e)})
	return out, e
}
func d2rsLifecycleNew(t *testing.T, o Options) (*Resolver, error) {
	transport := "default"
	hasClient := o.HTTPClient != nil
	if hasClient && o.HTTPClient.Transport != nil {
		switch o.HTTPClient.Transport.(type) {
		case *http.Transport:
			transport = "standard"
		case *closeTrackingRoundTripper:
			transport = "tracking"
		default:
			t.Fatal("uncaptured transport")
		}
	}
	r, e := New(o)
	id := len(d2rsLifecycleIDs) + 1
	d2rsLifecycleIDs[r] = id
	got := map[string]interface{}{"error": d2rsLifecycleError(e)}
	if e == nil {
		got["owned"] = r.ownsHTTPTransport
		got["isolated"] = !hasClient || o.HTTPClient.Transport == nil || r.client.Transport != o.HTTPClient.Transport
	}
	d2rsLifecycleRecord(map[string]interface{}{"op": "new", "id": id, "client": hasClient, "transport": transport, "allow_private": o.NetworkPolicy.AllowPrivateNetworks, "limits": o.Limits}, got)
	return r, e
}
func d2rsLifecycleClose(t *testing.T, r *Resolver) {
	id := 0
	if r != nil {
		id = d2rsLifecycleIDs[r]
	}
	var tracker *closeTrackingRoundTripper
	if r != nil {
		tracker, _ = r.client.Transport.(*closeTrackingRoundTripper)
	}
	r.CloseIdleConnections()
	got := map[string]interface{}{}
	if tracker != nil {
		got["closed"] = tracker.closed.Load()
	}
	d2rsLifecycleRecord(map[string]interface{}{"op": "close", "id": id}, got)
}
