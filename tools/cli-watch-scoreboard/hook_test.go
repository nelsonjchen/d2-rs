// SPDX-License-Identifier: MPL-2.0
// Observe the original assertions; requests carry no expected output.
package d2cli

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

var d2rsWatchCalls []map[string]any
var d2rsPreviousToken string

func d2rsWatchRecord(t *testing.T, input, result map[string]any) {
	t.Helper()
	d2rsWatchCalls = append(d2rsWatchCalls, map[string]any{"test": t.Name(), "call": len(d2rsWatchCalls), "input": input, "result": result})
	data, err := json.Marshal(d2rsWatchCalls)
	if err != nil { t.Fatal(err) }
	if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_WATCH_CAPTURE"), "calls.json"), data, 0600); err != nil { t.Fatal(err) }
}

func d2rsWatchToken(t *testing.T) (string, error) {
	token, err := newWatchAccessToken()
	errorText := ""
	if err != nil { errorText = err.Error() }
	d2rsWatchRecord(t, map[string]any{"operation":"token"}, map[string]any{"error":errorText, "length":len(token), "url_safe":!strings.ContainsAny(token,"+/="), "distinct":token != d2rsPreviousToken})
	d2rsPreviousToken = token
	return token, err
}

func d2rsWatchAuthorize(t *testing.T, w *watcher, recorder *httptest.ResponseRecorder, req *http.Request) bool {
	allowed := w.authorizeRequest(recorder, req)
	d2rsWatchRecord(t, map[string]any{"operation":"authorize", "token":w.accessToken, "cookie_name":w.accessCookieName, "target":req.URL.RequestURI(), "cookie":req.Header.Get("Cookie")}, map[string]any{"allowed":allowed, "code":recorder.Code, "headers":recorder.Header(), "body":recorder.Body.String()})
	return allowed
}

func TestD2rsWatchParsingWitnesses(t *testing.T) {
	for _, tc := range []struct{name, target, cookie string}{
		{"duplicate_wrong_first", "/", "d2-watch-12345=wrong; d2-watch-12345=correct-token"},
		{"duplicate_right_first", "/", "d2-watch-12345=correct-token; d2-watch-12345=wrong"},
		{"quoted_cookie", "/", `d2-watch-12345="correct-token"`},
		{"invalid_cookie_then_valid", "/", `d2-watch-12345="unterminated; d2-watch-12345=correct-token`},
		{"empty_token_with_cookie", "/?token=", "d2-watch-12345=correct-token"},
		{"malformed_token_with_cookie", "/?token=%zz", "d2-watch-12345=correct-token"},
		{"malformed_neighbor", "/?keep=%zz&token=correct-token&good=1", ""},
		{"semicolon_token_with_cookie", "/?token=wrong;ignored", "d2-watch-12345=correct-token"},
		{"semicolon_neighbor", "/?keep=a;b&token=correct-token&good=1", ""},
		{"encoded_path", "/a%2Fb%20c?token=correct-token", ""},
		{"query_bytes", "/?keep=%FF&token=correct-token", ""},
		{"duplicate_query", "/?token=correct-token&token=wrong&keep=2&keep=1", ""},
	} {
		t.Run(tc.name, func(t *testing.T) {
			req := httptest.NewRequest(http.MethodGet, "http://localhost"+tc.target, nil)
			if tc.cookie != "" { req.Header.Set("Cookie",tc.cookie) }
			w := &watcher{accessToken:"correct-token",accessCookieName:"d2-watch-12345"}
			d2rsWatchAuthorize(t,w,httptest.NewRecorder(),req)
		})
	}
	if len(d2rsWatchCalls) == 0 { t.Fatal(fmt.Errorf("empty capture")) }
}
