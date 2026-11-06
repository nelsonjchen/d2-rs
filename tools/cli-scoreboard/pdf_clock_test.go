package pdf

import (
	"testing"
	"time"

	"codeberg.org/go-pdf/fpdf"
)

func TestD2rsPDFClock(t *testing.T) {
	for _, tc := range []struct{ value, date string }{
		{"0", "19700101000000"},
		{"951827696", "20000229123456"},
		{"253402300799", "99991231235959"},
		{"", ""}, {"-1", ""}, {"+1", ""}, {" 1", ""}, {"1.0", ""},
		{"253402300800", ""}, {"18446744073709551616", ""},
	} {
		t.Run(tc.value, func(t *testing.T) {
			t.Setenv("SOURCE_DATE_EPOCH", tc.value)
			document := fpdf.New("P", "pt", "A4", "")
			d2rsPDFClock(document)
			if tc.date == "" {
				if !document.Err() {
					t.Fatal("invalid clock accepted")
				}
				return
			}
			if document.Err() {
				t.Fatal(document.Error())
			}
			for _, date := range []time.Time{document.GetCreationDate(), document.GetModificationDate()} {
				if date.Location() != time.UTC || date.Format("20060102150405") != tc.date {
					t.Fatalf("date = %v", date)
				}
			}
			if !document.GetCatalogSort() {
				t.Fatal("catalog order is not deterministic")
			}
		})
	}
}
