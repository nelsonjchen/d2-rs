// SPDX-License-Identifier: MPL-2.0
// Source-derived witnesses for d2cli/export.go getOutputFormat/getExportExtension.
// These supplement, and do not modify, the two original export Test functions.
package d2cli

import (
	"encoding/xml"
	"os"
	"path/filepath"
	"testing"
)

func TestD2rsOutputSelectionWitnesses(t *testing.T) {
	for _, tc := range []struct {
		name string
		args []string
	}{
		{"upper_png", []string{"out.PNG"}},
		{"upper_pdf", []string{"out.PDF"}},
		{"upper_gif", []string{"out.GIF"}},
		{"upper_pptx", []string{"out.PPTX"}},
		{"text_alias", []string{"out.text"}},
		{"ascii_alias", []string{"out.ascii"}},
		{"empty_stdout", []string{"--stdout-format=", "-"}},
		{"empty_stdout_text", []string{"--stdout-format=", "out.txt"}},
		{"empty_stdout_unknown", []string{"--stdout-format=", "out.unknown"}},
		{"explicit_mixed_case", []string{"--stdout-format=SvG", "out.PNG"}},
		{"replacement", []string{"out.svg"}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			dir := d2rsModeDir(t)
			if tc.name == "replacement" {
				if err := os.WriteFile(filepath.Join(dir, "out.svg"), []byte("previous output"), 0640); err != nil {
					t.Fatal(err)
				}
			}
			data, err := imageCLI(t, dir, "a -> b", tc.args...)
			if err != nil {
				t.Fatal(err)
			}
			output := tc.args[len(tc.args)-1]
			if output != "-" {
				data, err = os.ReadFile(filepath.Join(dir, output))
				if err != nil {
					t.Fatal(err)
				}
			}
			if output == "out.txt" {
				if len(data) == 0 {
					t.Fatal("empty text output")
				}
				return
			}
			var document struct{ XMLName xml.Name }
			if err := xml.Unmarshal(data, &document); err != nil {
				t.Fatal(err)
			}
			if document.XMLName.Local != "svg" || document.XMLName.Space != "http://www.w3.org/2000/svg" {
				t.Fatalf("expected SVG, got %v", document.XMLName)
			}
		})
	}
}
