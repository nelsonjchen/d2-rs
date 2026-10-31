# Expanded TALA fuzz inputs

These 24 deterministic records extend the three pinned `FuzzAutolayout` corpus records. They use the same seven-parameter encoding, with varied positive, negative, and boundary int64 seeds, graph sizes, iteration counts, compaction levels, connection subsets, and edge probabilities. The records are locally generated test cases, not upstream-provided fuzz discoveries. Their exact tuples and SHA-256 digests are in `manifest.json`.

Run `uv run --no-project python scripts/check-go-fuzz-corpus.py --additional-corpus tests/upstream/tala-fuzz-expanded/corpus --report oracle-cache/reports/go-fuzz-expanded-parity.json` to compare each source-generated graph through pinned Go 1.27.0 and Rust graphjson layout, including normalized stage traces.
