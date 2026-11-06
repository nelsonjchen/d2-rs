// Oracle-only reproducibility controls; installed in the trace checkout.
package pdf

import (
	"fmt"
	"os"
	"strconv"
	"strings"
	"time"

	"codeberg.org/go-pdf/fpdf"
)

func d2rsPDFClock(document *fpdf.Fpdf) {
	value, present := os.LookupEnv("SOURCE_DATE_EPOCH")
	if !present {
		return
	}
	seconds, err := strconv.ParseUint(value, 10, 64)
	if err != nil || value == "" || strings.IndexFunc(value, func(r rune) bool { return r < '0' || r > '9' }) >= 0 || seconds > 253402300799 {
		document.SetError(fmt.Errorf("SOURCE_DATE_EPOCH must be decimal Unix seconds in 0..=253402300799"))
		return
	}
	instant := time.Unix(int64(seconds), 0).UTC()
	document.SetCreationDate(instant)
	document.SetModificationDate(instant)
	// FPDF's documented test control; no output bytes are rewritten.
	document.SetCatalogSort(true)
}
