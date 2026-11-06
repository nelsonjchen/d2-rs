#!/usr/bin/env python3
"""Build the pinned trace CLI with explicit log/PDF clock injection; restore sources."""
import argparse
import os
import subprocess
from pathlib import Path

import references

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    references.verify_source()
    trace = references.TRACE_CHECKOUT
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=trace, text=True).strip()
    assert revision == references.SOURCE_COMMIT, revision
    output = args.output.resolve()
    assert not output.exists(), "use a fresh output path"
    pretty = trace / "lib/log/prettyhandler.go"
    pdf = trace / "lib/pdf/pdf.go"
    helper = trace / "lib/log/d2rs_clock.go"
    pdf_helper = trace / "lib/pdf/d2rs_clock.go"
    paths = [pretty, pdf, helper, pdf_helper]
    originals = {path: path.read_bytes() if path.exists() else None for path in paths}
    try:
        text = pretty.read_text()
        old = "timestamp := getFadedTimestamp(r.Time)"
        assert text.count(old) == 1
        pretty.write_text(text.replace(old, "timestamp := getFadedTimestamp(d2rsLogClock(r.Time))"))
        helper.write_bytes((ROOT / "tools/cli-scoreboard/log_clock.go").read_bytes())
        text = pdf.read_text()
        if "d2rsPDFClock(newGofPDF)" not in text:
            old = "newGofPDF.SetMargins(0, 0, 0)"
            assert text.count(old) == 1
            pdf.write_text(text.replace(old, old + "\n\td2rsPDFClock(newGofPDF)"))
        pdf_helper.write_bytes((ROOT / "tools/cli-scoreboard/pdf_clock.go").read_bytes())
        env = dict(os.environ, GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
                   GOCACHE=str(ROOT / ".reference/go-build"), GOFLAGS="-mod=mod", GOPROXY="off", GOMAXPROCS="2")
        subprocess.run(["go", "build", "-ldflags", "-X github.com/d2lang/d2/lib/version.Version=v0.9.0",
                        "-o", str(output), "."], cwd=trace, env=env, check=True, timeout=300)
    finally:
        for path, original in originals.items():
            if original is None:
                path.unlink(missing_ok=True)
            else:
                path.write_bytes(original)
        assert all((path.read_bytes() if path.exists() else None) == original
                   for path, original in originals.items())


if __name__ == "__main__":
    main()
