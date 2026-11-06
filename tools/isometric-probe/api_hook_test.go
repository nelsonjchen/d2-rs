// SPDX-License-Identifier: MPL-2.0
// Env-gated observation of the original public API tests, with assertions active.
package d2isometricimg

import (
	"context"
	"encoding/json"
	"github.com/d2lang/d2/d2target"
	"os"
	"runtime"
	"strings"
	"sync"
)

var d2rsAPIMutex sync.Mutex

func d2rsAPIRecord(api string, ctx context.Context, d *d2target.Diagram, o *Options, result any, err error) {
	path := os.Getenv("D2RS_ISOMETRIC_API")
	if path == "" {
		return
	}
	request := map[string]any{"api": api, "diagram": d, "options": o, "context": ""}
	if ctx != nil && ctx.Err() != nil {
		request["context"] = ctx.Err().Error()
	}
	want := map[string]any{"result": result, "error": ""}
	if err != nil {
		want["error"] = err.Error()
	}
	d2rsAPIMutex.Lock()
	defer d2rsAPIMutex.Unlock()
	f, e := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0600)
	if e != nil {
		panic(e)
	}
	defer f.Close()
	pcs := make([]uintptr, 64)
	n := runtime.Callers(2, pcs)
	frames := runtime.CallersFrames(pcs[:n])
	test := ""
	for {
		frame, more := frames.Next()
		if i := strings.Index(frame.Function, ".Test"); i >= 0 {
			test = strings.Split(frame.Function[i+1:], ".")[0]
			break
		}
		if !more {
			break
		}
	}
	if test == "" {
		panic("API capture outside an original test")
	}
	if e = json.NewEncoder(f).Encode(map[string]any{"test": test, "request": request, "want": want}); e != nil {
		panic(e)
	}
}
func RenderPage(ctx context.Context, d *d2target.Diagram, o *Options) (*Page, error) {
	p, e := d2rsOriginalRenderPage(ctx, d, o)
	d2rsAPIRecord("page", ctx, d, o, p, e)
	return p, e
}
func Render(ctx context.Context, d *d2target.Diagram, o *Options) ([]byte, error) {
	p, e := d2rsOriginalRender(ctx, d, o)
	d2rsAPIRecord("render", ctx, d, o, p, e)
	return p, e
}
