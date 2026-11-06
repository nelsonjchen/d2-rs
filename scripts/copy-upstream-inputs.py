#!/usr/bin/env python3
"""Copy upstream files from the patched pinned tree into staged outputs.

Recipe helper (docs/FIXTURES.md): argument i names the upstream path, relative
to $D2RS_UPSTREAM, of the i-th entry of $D2RS_RECIPE_OUTPUTS. Used for
verbatim upstream inputs that tests read, so they are not committed.
"""
import os
import shutil
import sys
from pathlib import Path

upstream = Path(os.environ["D2RS_UPSTREAM"])
outputs = os.environ["D2RS_RECIPE_OUTPUTS"].split(os.pathsep)
if len(outputs) != len(sys.argv) - 1:
    sys.exit(f"{len(outputs)} outputs but {len(sys.argv) - 1} upstream paths")
for source, output in zip(sys.argv[1:], outputs):
    Path(output).parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(upstream / source, output)
