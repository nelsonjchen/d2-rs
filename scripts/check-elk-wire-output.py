#!/usr/bin/env python3
"""elk-go output for one committed ELK wire graph (tools/fixtures/langlibelk-elk-layered.toml).

usage: check-elk-wire-output.py INPUT

Builds tools/elk-oracle against the pinned elk-go module (arm64,
scripts/elk_oracle.py), feeds it the committed wire graph INPUT and writes its
raw stdout (elk.LayoutJSON output plus a newline) to the recipe's staged
output. `cargo test -p d2-rs-elk --features oracle-fixtures` compares.
"""
import os
from pathlib import Path
import sys

from elk_oracle import run, wire_oracle

q = run(wire_oracle(), Path(sys.argv[1]).read_bytes(), timeout=30)
if q.returncode:
    sys.exit(f"elk-go exited {q.returncode}: {q.stderr.decode()[:2000]}")
Path(os.environ["D2RS_RECIPE_OUTPUT"]).write_bytes(q.stdout)
