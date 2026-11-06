// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// Development reference only; never shipped as part of d2-rs.

package engine

// Recipe `tala-tree-publication` (tools/fixtures/tala-oracle.toml), run by
// scripts/tala_oracle_fixtures.py: the complete GraphJSON graph after each
// pre-placement stage of an ordinary seeded layout. The stage plan is the
// default plan's prefix, run through runAllStages so stage-0 resets and
// context checks are the real ones.

import (
	"context"
	"encoding/json"
	"os"
	"strconv"
	"testing"

	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/graphjson"
	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/layoutgraph"
)

func TestD2RSTreePublication(t *testing.T) {
	out := os.Getenv("D2RS_TALA_STAGES_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	data, err := os.ReadFile(os.Getenv("D2RS_TALA_STAGES_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	seed, err := strconv.ParseInt(os.Getenv("D2RS_TALA_STAGES_SEED"), 10, 64)
	if err != nil {
		t.Fatal(err)
	}
	last := os.Getenv("D2RS_TALA_STAGES_LAST")
	ctx := context.Background()
	var input layoutgraph.Graph
	if err := graphjson.Unmarshal(ctx, data, &input); err != nil {
		t.Fatal(err)
	}
	graph, err := layoutgraph.Clone(ctx, &input)
	if err != nil {
		t.Fatal(err)
	}
	type record struct {
		Stage string          `json:"stage"`
		Graph json.RawMessage `json:"graph"`
	}
	var records []record
	var plan []pipelineStage
	for _, stage := range defaultPipelineStages {
		run, name := stage.run, stage.name
		plan = append(plan, pipelineStage{name: name, run: func(p *pipeline, ctx context.Context) error {
			if err := run(p, ctx); err != nil {
				return err
			}
			encoded, err := graphjson.Marshal(ctx, p.graph)
			if err != nil {
				return err
			}
			records = append(records, record{Stage: name, Graph: encoded})
			return nil
		}})
		if name == last {
			break
		}
	}
	if plan[len(plan)-1].name != last {
		t.Fatalf("stage %q is not in the default plan", last)
	}
	p := newPipeline(graph, seed, false)
	p.stages = plan
	if err := p.runAllStages(ctx); err != nil {
		t.Fatal(err)
	}
	encoded, err := json.Marshal(records)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, encoded, 0o644); err != nil {
		t.Fatal(err)
	}
}
