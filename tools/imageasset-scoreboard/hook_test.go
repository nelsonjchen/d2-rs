package imageasset

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"sync"
	"testing"
)

var d2rsCaptureMu sync.Mutex
var d2rsResolvers = map[*Resolver]int{}
var d2rsSequence int
var d2rsHTTPCalls = map[*Resolver][]map[string]any{}
var d2rsCaches = map[*MemoryCache]int{}

// Observe the injected client's final response, retaining the original client,
// redirect callback and every original assertion. Only finite selected tests
// install this hook. Restore the response body before loadHTTP sees it.
func d2rsHTTPDo(r *Resolver, request *http.Request) (*http.Response, error) {
	response, err := r.client.Do(request)
	call := map[string]any{"url": request.URL.String(), "request_headers": request.Header}
	if err != nil {
		var limitErr *LimitError
		call["error"] = "HTTP request failed"
		call["unavailable"] = response == nil && !errors.As(err, &limitErr)
	} else {
		data, readErr := io.ReadAll(response.Body)
		response.Body.Close()
		if readErr != nil {
			panic(readErr)
		}
		response.Body = io.NopCloser(bytes.NewReader(data))
		call["status"] = response.StatusCode
		call["content_length"] = response.ContentLength
		call["headers"] = response.Header
		call["body"] = data
	}
	d2rsHTTPCalls[r] = append(d2rsHTTPCalls[r], call)
	return response, err
}

// Preserve mustResolve's original failure assertion while observing its call.
func d2rsMustResolve(t *testing.T, r *Resolver, source string) *Resource {
	t.Helper()
	resource, err := d2rsResolve(t, r, context.Background(), source)
	if err != nil {
		t.Fatal(err)
	}
	return resource
}

// Observe original calls without changing their inputs, contexts or assertions.
func d2rsResolve(t *testing.T, r *Resolver, ctx context.Context, source string) (*Resource, error) {
	if os.Getenv("D2RS_IMAGEASSET_CAPTURE") == "" {
		return r.Resolve(ctx, source)
	}
	d2rsCaptureMu.Lock()
	defer d2rsCaptureMu.Unlock()
	session, ok := d2rsResolvers[r]
	if !ok {
		session = len(d2rsResolvers)
		d2rsResolvers[r] = session
	}
	request := map[string]any{"test": t.Name(), "session": session, "source": source, "base_dir": r.baseDir, "limits": r.limits}
	cache, cached := r.cache.(*MemoryCache)
	if cached {
		id, exists := d2rsCaches[cache]
		if !exists {
			id = len(d2rsCaches)
			d2rsCaches[cache] = id
		}
		request["cache"] = map[string]any{"id": id, "max_entries": cache.maxEntries, "max_bytes": cache.maxBytes, "prefix": r.cachePrefix}
	}
	if !hasPrefixFold(source, "data:") {
		p := source
		if !filepath.IsAbs(p) {
			p = filepath.Join(r.baseDir, p)
		}
		p = filepath.Clean(p)
		if data, e := os.ReadFile(p); e == nil {
			request["files"] = map[string][]byte{p: data}
		}
	}
	resource, err := r.Resolve(ctx, source)
	if calls := d2rsHTTPCalls[r]; len(calls) != 0 {
		request["http"] = calls
		delete(d2rsHTTPCalls, r)
	}
	result := map[string]any{"assets": r.assetCountSnapshot(), "encoded": r.cumulativeEncodedBytesSnapshot(), "decoded": r.cumulativeDecodedBytesSnapshot()}
	if cached {
		result["cache"] = map[string]any{"entries": len(cache.entries), "encoded": cache.bytes}
	}
	if err != nil {
		result["error"] = err.Error()
		result["unavailable"] = errors.Is(err, ErrUnavailable)
	} else {
		uri, e := resource.DataURIContext(context.Background())
		if e != nil {
			t.Fatal(e)
		}
		result["resource"] = map[string]any{"kind": resource.Kind(), "mime_type": resource.MIMEType(), "data": resource.cloneBytes(), "pixel_width": resource.PixelWidth(), "pixel_height": resource.PixelHeight(), "decoded_bytes": resource.DecodedBytes(), "fetched_bytes": resource.fetchedByteCount(), "decompressed_bytes": resource.decompressedByteCount(), "data_uri": string(uri)}
	}
	row := map[string]any{"request": request, "expected": result}
	data, e := json.Marshal(row)
	if e != nil {
		t.Fatal(e)
	}
	path := filepath.Join(os.Getenv("D2RS_IMAGEASSET_CAPTURE"), "calls.jsonl")
	f, e := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0600)
	if e != nil {
		t.Fatal(e)
	}
	defer f.Close()
	if _, e = f.Write(append(data, '\n')); e != nil {
		t.Fatal(e)
	}
	d2rsSequence++
	return resource, err
}
