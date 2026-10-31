package d2cli

import (
	"bytes"
	"io"
	"strings"
	"testing"
)

func TestD2rsValidateOutput(t *testing.T) {
	for _, enabled := range []bool{false, true} {
		t.Run(map[bool]string{false: "ordinary", true: "captured"}[enabled], func(t *testing.T) {
			value := ""
			if enabled {
				value = t.TempDir()
			}
			t.Setenv("D2RS_CLI_CAPTURE", value)
			var process, captured bytes.Buffer
			// Preserve newlines, spaces, non-ASCII paths and arbitrary bytes.
			want := "Success! [/a path/日本語.d2] is valid D2.\n\x00tail  \n"
			n, err := io.Copy(d2rsValidateWriter(&process, &captured), strings.NewReader(want))
			if err != nil || n != int64(len(want)) || process.String() != want {
				t.Fatalf("process stdout changed: %q, %d, %v", process.String(), n, err)
			}
			if enabled && captured.String() != want || !enabled && captured.Len() != 0 {
				t.Fatalf("unexpected capture: %q", captured.String())
			}
		})
	}
}
