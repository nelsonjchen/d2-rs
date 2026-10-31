// SPDX-License-Identifier: MPL-2.0
package imgbundler

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"sync"
	"testing"

	"github.com/d2lang/d2/lib/imageasset"
	"github.com/d2lang/d2/lib/simplelog"
)

type d2rsResponse struct {
	Status  int                 `json:"status"`
	Length  int64               `json:"length"`
	Headers map[string][]string `json:"headers"`
	Body    string              `json:"body"`
}
type d2rsTransport struct {
	original  http.RoundTripper
	mu        sync.Mutex
	responses map[string]d2rsResponse
	calls     int
}

func (r *d2rsTransport) RoundTrip(req *http.Request) (*http.Response, error) {
	response, err := r.original.RoundTrip(req)
	if err != nil {
		return response, err
	}
	body, err := io.ReadAll(response.Body)
	_ = response.Body.Close()
	if err != nil {
		return nil, err
	}
	response.Body = io.NopCloser(bytes.NewReader(body))
	r.mu.Lock()
	defer r.mu.Unlock()
	r.calls++
	r.responses[req.URL.String()] = d2rsResponse{response.StatusCode, response.ContentLength, response.Header.Clone(), base64.StdEncoding.EncodeToString(body)}
	return response, nil
}

type d2rsSession struct {
	options   imageasset.Options
	transport *d2rsTransport
}

var d2rsSessions = map[*imageasset.Resolver]d2rsSession{}
var d2rsCalls = map[string]int{}

func d2rsNew(t *testing.T, options imageasset.Options) (*imageasset.Resolver, error) {
	session := d2rsSession{options: options}
	if options.HTTPClient != nil {
		client := *options.HTTPClient
		transport := client.Transport
		if transport == nil {
			transport = http.DefaultTransport
		}
		session.transport = &d2rsTransport{original: transport, responses: map[string]d2rsResponse{}}
		client.Transport = session.transport
		options.HTTPClient = &client
	}
	resolver, err := imageasset.New(options)
	if err == nil {
		d2rsSessions[resolver] = session
	}
	return resolver, err
}
func d2rsRecord(t *testing.T, request, got interface{}) {
	if dir := os.Getenv("D2RS_IMGBUNDLER_RESOLVER_DUMP"); dir != "" {
		call := d2rsCalls[t.Name()]
		d2rsCalls[t.Name()]++
		name := fmt.Sprintf("%s#%d", t.Name(), call)
		data, err := json.Marshal(map[string]interface{}{"name": name, "test": t.Name(), "request": request, "got": got})
		if err != nil {
			t.Fatal(err)
		}
		id := fmt.Sprintf("%x", sha256.Sum256([]byte(name)))
		if err = os.WriteFile(filepath.Join(dir, id+".json"), data, 0600); err != nil {
			t.Fatal(err)
		}
	}
}
func d2rsError(err error) string {
	if err != nil {
		return err.Error()
	}
	return ""
}
func d2rsRequest(t *testing.T, resolver *imageasset.Resolver, source []byte) map[string]interface{} {
	session, ok := d2rsSessions[resolver]
	if !ok {
		t.Fatal("unobserved resolver")
	}
	return d2rsRequestSession(t, session, source)
}
func d2rsRequestSession(t *testing.T, session d2rsSession, source []byte) map[string]interface{} {
	files := map[string]string{}
	_, hrefs, err := findImageElements(context.Background(), source, false)
	if err != nil {
		t.Fatal(err)
	}
	for _, href := range hrefs {
		path := href
		if !filepath.IsAbs(path) {
			path = filepath.Join(session.options.BaseDir, path)
		}
		if body, err := os.ReadFile(path); err == nil {
			files[path] = base64.StdEncoding.EncodeToString(body)
		}
	}
	request := map[string]interface{}{"source": string(source), "base_dir": session.options.BaseDir, "limits": session.options.Limits, "files": files, "responses": map[string]d2rsResponse{}, "default_response": nil}
	if tr := session.transport; tr != nil {
		tr.mu.Lock()
		defer tr.mu.Unlock()
		request["responses"] = tr.responses
		// The uniform mock in the scheduling witness returns identical responses
		// for every URL; retain that transport rule without issuing extra I/O.
		if t.Name() == "TestResolutionFailureStopsSchedulingNewFetches" {
			var first *d2rsResponse
			for _, r := range tr.responses {
				if first == nil {
					v := r
					first = &v
				} else {
					a, _ := json.Marshal(first)
					b, _ := json.Marshal(r)
					if !bytes.Equal(a, b) {
						t.Fatal("nonuniform scheduling mock")
					}
				}
			}
			request["default_response"] = first
		}
	}
	return request
}
func d2rsBundle(t *testing.T, ctx context.Context, l simplelog.Logger, in []byte, options BundleOptions) ([]byte, error) {
	out, err := BundleWithResolver(ctx, l, in, options)
	request := d2rsRequest(t, options.Resolver, in)
	request["op"] = "resolver-bundle"
	request["local"] = options.Local
	request["remote"] = options.Remote
	request["workers"] = maxImageWorkers
	request["budget"] = int64(maxBundledImageBytes)
	d2rsRecord(t, request, map[string]interface{}{"output": string(out), "error": d2rsError(err), "requests": d2rsSessions[options.Resolver].transport.calls})
	return out, err
}
func d2rsWorkers(t *testing.T, ctx context.Context, l simplelog.Logger, in []byte, matches []imageMatch, hrefs []string, resolver *imageasset.Resolver, budget *bundleBudget) ([]byte, error) {
	return d2rsWorkersLimit(t, ctx, l, in, matches, hrefs, resolver, budget, maxImageWorkers)
}
func d2rsWorkersLimit(t *testing.T, ctx context.Context, l simplelog.Logger, in []byte, matches []imageMatch, hrefs []string, resolver *imageasset.Resolver, budget *bundleBudget, workers int) ([]byte, error) {
	out, err := runResolverWorkersWithLimit(ctx, l, in, matches, hrefs, resolver, budget, workers)
	request := d2rsRequest(t, resolver, in)
	request["op"] = "resolver-workers"
	request["local"] = true
	request["remote"] = true
	request["workers"] = workers
	request["budget"] = budget.maxBundledBytes
	got := map[string]interface{}{"output": string(out), "error": d2rsError(err)}
	if workers == maxImageWorkers {
		got["requests_within_worker_limit"] = d2rsSessions[resolver].transport.calls <= maxImageWorkers
	} else {
		got["requests"] = d2rsSessions[resolver].transport.calls
	}
	d2rsRecord(t, request, got)
	return out, err
}
func d2rsReplacement(t *testing.T, ctx context.Context, resolver *imageasset.Resolver, href string, budget *bundleBudget) ([]byte, error) {
	out, err := resolveReplacement(ctx, resolver, href, budget)
	request := d2rsRequest(t, resolver, nil)
	request["op"] = "resolver-replacement"
	request["href"] = href
	request["budget"] = budget.maxBundledBytes
	d2rsRecord(t, request, map[string]interface{}{"replacement": string(out), "error": d2rsError(err), "used": budget.bundledBytes})
	return out, err
}
func d2rsLegacy(t *testing.T, ctx context.Context, l simplelog.Logger, in []byte, cache bool) ([]byte, error) {
	// Observe the original helper and its newLegacyResolver factory directly.
	original := httpClient
	client := *httpClient
	transport := client.Transport
	if transport == nil {
		transport = http.DefaultTransport
	}
	tr := &d2rsTransport{original: transport, responses: map[string]d2rsResponse{}}
	client.Transport = tr
	httpClient = &client
	defer func() { httpClient = original }()
	out, err := bundleRemoteForTest(ctx, l, in, cache)
	request := d2rsRequestSession(t, d2rsSession{transport: tr, options: imageasset.Options{Limits: imageasset.Limits{
		MaxFetchedBytes: maxImageSize, MaxEncodedBytes: maxImageSize, MaxDecompressedBytes: maxImageSize, MaxSVGBytes: maxImageSize,
		MaxDecodedWidth: maxImageDecodedWidth, MaxDecodedHeight: maxImageDecodedHeight, MaxDecodedPixels: maxImageDecodedPixels,
		MaxAssets: maxImageReferences, MaxCumulativeEncodedBytes: maxBundledImageBytes, MaxCumulativeDecodedBytes: maxBundledImageBytes,
	}}}, in)
	request["op"] = "resolver-bundle"
	request["local"] = false
	request["remote"] = true
	request["workers"] = maxImageWorkers
	request["budget"] = int64(maxBundledImageBytes)
	request["legacy_cache"] = cache
	d2rsRecord(t, request, map[string]interface{}{"output": string(out), "error": d2rsError(err), "requests": tr.calls})
	return out, err
}
