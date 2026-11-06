// SPDX-License-Identifier: MPL-2.0
// Observe complete CaptureFrames sessions while retaining original assertions.
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

var d2rsFrameMu sync.Mutex

func CaptureFrames(ctx context.Context, d *d2target.Diagram, o *Options, seconds []float64, traffic bool, emit func(int, []byte) error) error {
	path := os.Getenv("D2RS_FRAME_CAPTURE")
	if path == "" {
		return d2rsOriginalCaptureFrames(ctx, d, o, seconds, traffic, emit)
	}
	initial := ""
	if ctx != nil && ctx.Err() != nil {
		initial = ctx.Err().Error()
	}
	var frames [][]byte
	callbackError, cancelAfter := "", -1
	var wrapped func(int, []byte) error
	if emit != nil {
		wrapped = func(i int, p []byte) error {
			frames = append(frames, append([]byte(nil), p...))
			err := emit(i, p)
			if err != nil {
				callbackError = err.Error()
			}
			if ctx != nil && ctx.Err() != nil {
				cancelAfter = i
			}
			return err
		}
	}
	// Admission tests contain NaN/Inf. Preserve their exact IEEE values as strings.
	times := make([]string, len(seconds))
	for i, t := range seconds {
		times[i] = frameTime(t)
	}
	request := map[string]any{"diagram": d, "options": o, "seconds": times, "traffic": traffic, "callback": emit != nil, "context": initial}
	err := d2rsOriginalCaptureFrames(ctx, d, o, seconds, traffic, wrapped)
	request["callbackError"], request["cancelAfter"] = callbackError, cancelAfter
	message := ""
	if err != nil {
		message = err.Error()
	}
	pcs := make([]uintptr, 64)
	n := runtime.Callers(2, pcs)
	stack := runtime.CallersFrames(pcs[:n])
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
		panic("frame capture outside original test")
	}
	d2rsFrameMu.Lock()
	defer d2rsFrameMu.Unlock()
	f, e := os.OpenFile(path, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0600)
	if e != nil {
		panic(e)
	}
	defer f.Close()
	if e = json.NewEncoder(f).Encode(map[string]any{"test": test, "request": request, "want": map[string]any{"frames": frames, "error": message}}); e != nil {
		panic(e)
	}
	return err
}
