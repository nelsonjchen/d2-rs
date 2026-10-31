// SPDX-License-Identifier: MPL-2.0
package netpolicy

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"os"
	"path/filepath"
	"reflect"
	"sync"
	"testing"
)

type d2rsClientTrace struct {
	mu         sync.Mutex
	operations []interface{}
	lookups    []interface{}
	dials      []string
	answers    map[string][]string
	routes     map[string]interface{}
	server     string
	hits       []string
}

var d2rsClientTraces sync.Map

func d2rsSession(t *testing.T) *d2rsClientTrace {
	value, loaded := d2rsClientTraces.LoadOrStore(t.Name(), &d2rsClientTrace{operations: []interface{}{}, hits: []string{}, lookups: []interface{}{}, dials: []string{}, answers: map[string][]string{}, routes: map[string]interface{}{}})
	trace := value.(*d2rsClientTrace)
	if !loaded {
		t.Cleanup(func() {
			dir := os.Getenv("D2RS_NETPOLICY_DUMP")
			if dir == "" {
				return
			}
			trace.mu.Lock()
			defer trace.mu.Unlock()
			data, err := json.Marshal(map[string]interface{}{"name": t.Name(), "test": t.Name(), "operations": trace.operations, "server": trace.server, "routes": trace.routes, "answers": trace.answers})
			if err != nil {
				t.Fatal(err)
			}
			if err = os.WriteFile(filepath.Join(dir, fmt.Sprintf("%x.json", sha256.Sum256([]byte(t.Name())))), data, 0600); err != nil {
				t.Fatal(err)
			}
		})
	}
	return trace
}
func d2rsRecord(t *testing.T, request, got interface{}) {
	trace := d2rsSession(t)
	trace.mu.Lock()
	defer trace.mu.Unlock()
	trace.operations = append(trace.operations, map[string]interface{}{"request": request, "got": got})
}
func d2rsErrorKind(err error) string {
	if err == nil {
		return ""
	}
	if errors.Is(err, ErrBlockedAddress) {
		return "blocked"
	}
	if errors.Is(err, context.Canceled) {
		return "cancelled"
	}
	return "other"
}
func d2rsNew(t *testing.T, base *http.Client, policy Policy) (*http.Client, error) {
	return d2rsClient(t, base, policy, nil, nil, false)
}
func d2rsInjected(t *testing.T, base *http.Client, policy Policy, resolver ipResolver, dial dialContextFunc) (*http.Client, error) {
	return d2rsClient(t, base, policy, resolver, dial, true)
}
func d2rsClient(t *testing.T, base *http.Client, policy Policy, resolver ipResolver, dial dialContextFunc, injected bool) (*http.Client, error) {
	trace := d2rsSession(t)
	kind := "default"
	hooks := []string{}
	var oldTransport http.RoundTripper
	if base != nil {
		oldTransport = base.Transport
		switch tr := base.Transport.(type) {
		case *http.Transport:
			kind = "standard"
			for name, present := range map[string]bool{"dial": tr.Dial != nil, "dial_context": tr.DialContext != nil, "dial_tls": tr.DialTLS != nil, "dial_tls_context": tr.DialTLSContext != nil, "proxy": tr.Proxy != nil, "proxy_response": tr.OnProxyConnectResponse != nil, "proxy_header": tr.GetProxyConnectHeader != nil, "proxy_headers": tr.ProxyConnectHeader != nil, "tls_next": tr.TLSNextProto != nil} {
				if present {
					hooks = append(hooks, name)
				}
			}
		case nil:
		default:
			kind = "custom"
		}
	}
	var client *http.Client
	var err error
	if injected {
		observed := resolverFunc(func(ctx context.Context, network, host string) ([]netip.Addr, error) {
			result, e := resolver.LookupNetIP(ctx, network, host)
			values := []string{}
			for _, address := range result {
				values = append(values, address.String())
			}
			trace.mu.Lock()
			trace.answers[host] = values
			trace.lookups = append(trace.lookups, map[string]string{"network": network, "host": host})
			trace.mu.Unlock()
			return result, e
		})
		watched := func(ctx context.Context, network, address string) (net.Conn, error) {
			trace.mu.Lock()
			trace.dials = append(trace.dials, address)
			trace.mu.Unlock()
			return dial(ctx, network, address)
		}
		client, err = newHTTPClient(base, policy, observed, watched)
	} else {
		client, err = NewHTTPClient(base, policy)
	}
	preserved := base == nil || reflect.ValueOf(base.Transport) == reflect.ValueOf(oldTransport)
	custom := false
	if client != nil && kind == "custom" {
		custom = reflect.ValueOf(client.Transport) == reflect.ValueOf(oldTransport)
	}
	d2rsRecord(t, map[string]interface{}{"op": "new", "transport": kind, "allow_private": policy.AllowPrivateNetworks, "hooks": hooks, "injected": injected}, map[string]interface{}{"accepted": err == nil, "custom_preserved": custom, "base_preserved": preserved})
	if client != nil {
		if tr, ok := client.Transport.(*http.Transport); ok {
			original := tr.DialContext
			tr.DialContext = func(ctx context.Context, network, address string) (net.Conn, error) {
				conn, e := original(ctx, network, address)
				// A cancelled direct dial is the original caller-hook isolation assertion.
				if ctx.Err() != nil {
					d2rsRecord(t, map[string]interface{}{"op": "dial", "address": address, "cancelled": true}, map[string]interface{}{"error_kind": d2rsErrorKind(e)})
				}
				return conn, e
			}
		}
	}
	return client, err
}

type d2rsRecorder struct {
	http.ResponseWriter
	status int
	body   []byte
	header http.Header
}

func (r *d2rsRecorder) WriteHeader(code int) {
	r.status = code
	r.header = r.Header().Clone()
	r.ResponseWriter.WriteHeader(code)
}
func (r *d2rsRecorder) Write(b []byte) (int, error) {
	if r.status == 0 {
		r.WriteHeader(200)
	}
	r.body = append(r.body, b...)
	return r.ResponseWriter.Write(b)
}
func d2rsServer(t *testing.T, handler http.Handler) *httptest.Server {
	trace := d2rsSession(t)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		recorder := &d2rsRecorder{ResponseWriter: w}
		handler.ServeHTTP(recorder, r)
		trace.mu.Lock()
		defer trace.mu.Unlock()
		trace.hits = append(trace.hits, r.URL.Path)
		trace.routes[r.URL.Path] = map[string]interface{}{"status": recorder.status, "body": string(recorder.body), "location": recorder.header.Get("Location")}
	}))
	trace.server = server.Listener.Addr().String()
	return server
}
func d2rsGet(t *testing.T, client *http.Client, url string) (*http.Response, error) {
	response, err := client.Get(url)
	trace := d2rsSession(t)
	trace.mu.Lock()
	got := map[string]interface{}{"error_kind": d2rsErrorKind(err), "lookups": trace.lookups, "dials": trace.dials, "hits": trace.hits}
	trace.mu.Unlock()
	if response != nil && err == nil {
		body, e := io.ReadAll(response.Body)
		if e != nil {
			t.Fatal(e)
		}
		response.Body.Close()
		response.Body = io.NopCloser(newReader(body))
		got["body"] = string(body)
	}
	d2rsRecord(t, map[string]interface{}{"op": "get", "url": url}, got)
	return response, err
}

type d2rsReader struct{ data []byte }

func newReader(b []byte) *d2rsReader { return &d2rsReader{b} }
func (r *d2rsReader) Read(b []byte) (int, error) {
	if len(r.data) == 0 {
		return 0, io.EOF
	}
	n := copy(b, r.data)
	r.data = r.data[n:]
	return n, nil
}
