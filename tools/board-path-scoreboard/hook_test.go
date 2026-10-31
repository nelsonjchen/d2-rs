// SPDX-License-Identifier: MPL-2.0
// Capture original arguments/results without changing their assertions.
package d2cli

import (
    "encoding/json"
    "os"
    "path/filepath"
    "testing"

    "github.com/d2lang/d2/d2target"
)

var d2rsPathCalls []any

func d2rsPathRecord(t *testing.T, input, result any) {
    d2rsPathCalls = append(d2rsPathCalls, map[string]any{"test": t.Name(), "input": input, "result": result})
    data, err := json.Marshal(d2rsPathCalls)
    if err != nil { t.Fatal(err) }
    if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_PATH_CAPTURE"), "calls.json"), data, 0600); err != nil { t.Fatal(err) }
}

func d2rsPathComponent(t *testing.T, name string) string {
    value := boardOutputComponent(name)
    // []byte uses base64 in encoding/json; use integers to preserve bad UTF-8.
    bytes := make([]int, len(name))
    for i := 0; i < len(name); i++ { bytes[i] = int(name[i]) }
    d2rsPathRecord(t, map[string]any{"op": "component", "bytes": bytes}, map[string]any{"value": value})
    return value
}

func d2rsPathDiagram(d *d2target.Diagram) any {
    out := map[string]any{"name": d.Name, "folder": d.IsFolderOnly}
    for kind, children := range map[string][]*d2target.Diagram{"layers": d.Layers, "scenarios": d.Scenarios, "steps": d.Steps} {
        values := make([]any, 0, len(children))
        for _, child := range children { values = append(values, d2rsPathDiagram(child)) }
        out[kind] = values
    }
    return out
}

func d2rsPathValidate(t *testing.T, output string, diagram *d2target.Diagram) error {
    input := map[string]any{"op": "validate", "output": output, "diagram": d2rsPathDiagram(diagram)}
    err := validateBoardOutputPaths(output, diagram)
    message := ""
    if err != nil { message = err.Error() }
    d2rsPathRecord(t, input, map[string]any{"error": message})
    return err
}

func d2rsPathAppendName(t *testing.T, output, name string) (string, error) {
    value, err := appendBoardOutputName(output, name)
    message := ""
    if err != nil { message = err.Error() }
    d2rsPathRecord(t, map[string]any{"op": "append_name", "output": output, "name": name}, map[string]any{"value": value, "error": message})
    return value, err
}

func d2rsPathAppendComponent(t *testing.T, output, component string) (string, error) {
    value, err := appendBoardOutputComponent(output, component)
    message := ""
    if err != nil { message = err.Error() }
    d2rsPathRecord(t, map[string]any{"op": "append_component", "output": output, "component": component}, map[string]any{"value": value, "error": message})
    return value, err
}

func d2rsPathWithin(t *testing.T, root, path string) error {
    err := ensurePathWithin(root, path)
    message := ""
    if err != nil { message = err.Error() }
    d2rsPathRecord(t, map[string]any{"op": "within", "root": root, "path": path}, map[string]any{"error": message})
    return err
}
