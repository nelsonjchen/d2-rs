// Copied only into the pinned instrumentation checkout by cli-scoreboard.py.
// validateCmd's original fmt.Printf writes to os.Stdout, outside xmain.State.
package d2cli

import (
	"io"
	"os"

	"github.com/d2lang/util-go/xmain"
)

func d2rsValidateWriter(process, captured io.Writer) io.Writer {
	if os.Getenv("D2RS_CLI_CAPTURE") == "" {
		return process
	}
	return io.MultiWriter(process, captured)
}

func d2rsValidateOutput(ms *xmain.State) io.Writer {
	return d2rsValidateWriter(os.Stdout, ms.Stdout)
}
