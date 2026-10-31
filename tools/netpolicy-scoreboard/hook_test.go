// SPDX-License-Identifier: MPL-2.0
package netpolicy

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"net/http"
	"net/netip"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
)

type d2rsTrace struct {
	mu   sync.Mutex
	rows []interface{}
}

var d2rsTraces sync.Map

func d2rsSession(t *testing.T) *d2rsTrace {
	name := strings.Split(t.Name(), "/")[0]
	value, loaded := d2rsTraces.LoadOrStore(name, &d2rsTrace{rows: []interface{}{}})
	trace := value.(*d2rsTrace)
	if !loaded {
		t.Cleanup(func() {
			dir := os.Getenv("D2RS_NETPOLICY_DUMP")
			if dir == "" {
				return
			}
			trace.mu.Lock()
			defer trace.mu.Unlock()
			b, e := json.Marshal(map[string]interface{}{"name": name, "test": name, "operations": trace.rows})
			if e != nil {
				t.Fatal(e)
			}
			if e = os.WriteFile(filepath.Join(dir, fmt.Sprintf("%x.json", sha256.Sum256([]byte(name)))), b, 0600); e != nil {
				t.Fatal(e)
			}
		})
	}
	return trace
}
func d2rsRecord(t *testing.T, request, got interface{}) {
	s := d2rsSession(t)
	s.mu.Lock()
	defer s.mu.Unlock()
	s.rows = append(s.rows, map[string]interface{}{"test": t.Name(), "request": request, "got": got})
}
func d2rsError(e error) string {
	if e != nil {
		return e.Error()
	}
	return ""
}
func d2rsPublic(t *testing.T, address netip.Addr) bool {
	got := isPublicAddress(address)
	d2rsRecord(t, map[string]interface{}{"op": "public", "address": address.String()}, got)
	return got
}
func d2rsClient(t *testing.T, base *http.Client, policy Policy, resolver ipResolver, dial dialContextFunc) (*http.Client, error) {
	var lookups, dials []interface{}
	var answers []string
	var lookupError, dialError string
	watchedResolver := resolverFunc(func(ctx context.Context, network, host string) ([]netip.Addr, error) {
		result, e := resolver.LookupNetIP(ctx, network, host)
		answers = []string{}
		for _, a := range result {
			answers = append(answers, a.String())
		}
		lookupError = d2rsError(e)
		lookups = append(lookups, map[string]interface{}{"network": network, "host": host})
		return result, e
	})
	watchedDial := func(ctx context.Context, network, address string) (net.Conn, error) {
		result, e := dial(ctx, network, address)
		dialError = d2rsError(e)
		dials = append(dials, map[string]interface{}{"network": network, "address": address})
		return result, e
	}
	client, e := newHTTPClient(base, policy, watchedResolver, watchedDial)
	if e != nil {
		return client, e
	}
	transport := client.Transport.(*http.Transport)
	original := transport.DialContext
	transport.DialContext = func(ctx context.Context, network, address string) (net.Conn, error) {
		lookups = []interface{}{}
		dials = []interface{}{}
		answers = []string{}
		lookupError = ""
		dialError = ""
		conn, e := original(ctx, network, address)
		d2rsRecord(t, map[string]interface{}{"op": "dial", "network": network, "address": address, "answers": answers, "lookup_error": lookupError, "dial_error": dialError}, map[string]interface{}{"error": d2rsError(e), "blocked": errors.Is(e, ErrBlockedAddress), "lookups": lookups, "dials": dials})
		return conn, e
	}
	return client, nil
}
