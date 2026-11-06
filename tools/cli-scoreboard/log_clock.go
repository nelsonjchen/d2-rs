// Oracle-only reproducibility control; installed only in the trace checkout.
package log

import (
	"os"
	"strconv"
	"time"
)

func d2rsLogClock(ordinary time.Time) time.Time {
	value, present := os.LookupEnv("SOURCE_DATE_EPOCH")
	if !present {
		return ordinary
	}
	seconds, err := strconv.ParseInt(value, 10, 64)
	if err != nil || seconds < 0 || seconds > 253402300799 {
		return ordinary
	}
	return time.Unix(seconds, 0).UTC()
}
