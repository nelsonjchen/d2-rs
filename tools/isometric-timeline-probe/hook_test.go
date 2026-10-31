// SPDX-License-Identifier: MPL-2.0
package d2isometricimg

import (
	"context"
	"encoding/json"
	"github.com/d2lang/d2/d2target"
	"os"
	"runtime"
	"strings"
	"sync"
	"time"
)

var d2rsTimelineMu sync.Mutex

func RenderTimeline(ctx context.Context, d []*d2target.Diagram, o *Options, interval time.Duration) ([]byte, error) {
	initial := ""
	if ctx != nil && ctx.Err() != nil {
		initial = ctx.Err().Error()
	}
	data, err := d2rsOriginalRenderTimeline(ctx, d, o, interval)
	path := os.Getenv("D2RS_TIMELINE_CAPTURE")
	if path == "" {
		return data, err
	}
	pcs := make([]uintptr, 64)
	stack := runtime.CallersFrames(pcs[:runtime.Callers(2, pcs)])
	test := ""
	for {
		f, more := stack.Next()
		if i := strings.Index(f.Function, ".Test"); i >= 0 {
			test = strings.Split(f.Function[i+1:], ".")[0]
			break
		}
		if !more {
			break
		}
	}
	if test == "" {
		panic("GIF observation outside test")
	}
	message := ""
	if err != nil {
		message = err.Error()
	}
	d2rsTimelineMu.Lock()
	defer d2rsTimelineMu.Unlock()
	f, e := os.OpenFile(path, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0600)
	if e != nil {
		panic(e)
	}
	defer f.Close()
	if e = json.NewEncoder(f).Encode(map[string]any{"test": test, "request": map[string]any{"boards": d, "interval_ns": int64(interval), "options": o, "context": initial}, "want": map[string]any{"data": data, "error": message}}); e != nil {
		panic(e)
	}
	return data, err
}
