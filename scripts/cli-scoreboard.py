#!/usr/bin/env python3
"""Inventory every pinned CLI case/golden and run the original test contracts.

Only the trace checkout (../d2-trace) is instrumented. Go must pass a case before its Rust
contract can pass. Golden matching is a separate tier: retained historical
goldens must also be reproduced by Go, never inferred from a passing test.
Binary artifacts compare exact bytes conservatively (including PDF/PPTX);
PNG uses the existing complete decoded-image gate. SVG uses the e2e gate's
documented XML/version/newline normalization. No golden is silently omitted.
GIF compares every composited RGBA frame, delay, canvas and loop count.
"""
from __future__ import annotations

import argparse
import collections
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import signal
import subprocess
import time
import xml.etree.ElementTree as ET
import zipfile

from references import verify_source
import references

ROOT = Path(__file__).resolve().parents[1]
TRACE = references.TRACE_CHECKOUT
UPSTREAM = ROOT.parent / "d2"
PPTX_RELEASE_CASES = {
    "TestCLI_E2E/" + name for name in [
        "how_to_solve_problems_pptx", "pptx-theme-overrides", "stdout_pptx", "no-nav-pptx",
    ]
}
# ci/release/_build.sh stamps this symbol for the pinned v0.9.0 release.
PPTX_RELEASE = {"version": "v0.9.0", "ldflags": "-X github.com/d2lang/d2/lib/version.Version=v0.9.0"}


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


RASTER = module("cli_raster_gate", ROOT / "scripts/check-render-parity.py")
E2E = module("cli_svg_gate", ROOT / "scripts/e2e-scoreboard.py")


def install():
    verify_source()
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=TRACE, text=True).strip()
    assert revision == references.SOURCE_COMMIT, revision
    source = (UPSTREAM / "e2etests-cli/main_test.go").read_text()
    path = TRACE / "e2etests-cli/main_test.go"
    # Preserve other instrumentation and require exact pinned insertion points.
    text = path.read_text()
    for old, new in [
        ("Run:  d2cli.Run,", "Run:  d2rsCLIRun(d2cli.Run),"),
        ("dir, cleanup := assert.TempDir(t)", "dir, cleanup := d2rsCLITempDir(t)"),
        ("\t\t\tdefer cleanup()", "\t\t\tdefer cleanup()\n\t\t\tdefer d2rsCLICapture(t, dir)"),
        ("assert.WriteFile(tb, filepath.Join(dir, fp), []byte(data), 0644)",
         "assert.WriteFile(tb, filepath.Join(dir, fp), []byte(data), 0644)\n\td2rsCLIInput(dir, fp, data)"),
    ]:
        assert source.count(old) == 1
        if new not in text:
            assert text.count(old) == 1
            text = text.replace(old, new)
    path.write_text(text)
    # The pinned Go library's development version is v0.8.1-HEAD. Keep all
    # original assertions; apply only the established SVG version allowance.
    for file in [path, TRACE / "e2etests-cli/raster_test.go"]:
        text = file.read_text().replace("assert.Testdata(t,", "d2rsCLITestdata(t,")
        text = text.replace("assert.TestdataDir(t,", "d2rsCLITestdataDir(t,")
        if file.name == "raster_test.go":
            text = text.replace("directory := t.TempDir()", "directory := t.TempDir()\n            defer d2rsCLICapture(t, directory)") if "defer d2rsCLICapture" not in text else text
        file.write_text(text)
    shutil.copyfile(ROOT / "tools/cli-scoreboard/hook_test.go", TRACE / "e2etests-cli/d2rs_scoreboard_test.go")
    # validateCmd prints directly to process stdout, bypassing xmain.State.
    # Tee that very write to the capture sink; retain its process output and
    # compare the captured bytes without path/text normalization.
    validate = (UPSTREAM / "d2cli/validate.go").read_text()
    old = 'fmt.Printf("Success! [%s] is valid D2.\\n", inputPath)'
    assert validate.count(old) == 1
    validate = validate.replace(old, 'fmt.Fprintf(d2rsValidateOutput(ms), "Success! [%s] is valid D2.\\n", inputPath)')
    (TRACE / "d2cli/validate.go").write_text(validate)
    for file in ["validate_output.go", "validate_output_test.go"]:
        shutil.copyfile(ROOT / "tools/cli-scoreboard" / file, TRACE / "d2cli" / ("d2rs_" + file))
    pdf = (UPSTREAM / "lib/pdf/pdf.go").read_text()
    old = "newGofPDF.SetMargins(0, 0, 0)"
    assert pdf.count(old) == 1
    (TRACE / "lib/pdf/pdf.go").write_text(pdf.replace(old, old + "\n\td2rsPDFClock(newGofPDF)"))
    for helper, target in [("pdf_clock.go", "d2rs_clock.go"), ("pdf_clock_test.go", "d2rs_clock_test.go")]:
        shutil.copyfile(ROOT / "tools/cli-scoreboard" / helper, TRACE / "lib/pdf" / target)
    names = re.findall(r'\bname:\s*"([^"]+)"', source)
    assert names and len(names) == len(set(names))
    raster = (UPSTREAM / "e2etests-cli/raster_test.go").read_text()
    png_names = re.search(r'fixture := range \[\]string\{([^}]+)\}', raster).group(1)
    extra = ["TestPNGFixtures/" + name for name in re.findall(r'"([^"]+)"', png_names)]
    extra += ["TestGIFFixtures/" + value for value in re.findall(r'\{intervalMS:\s*(\d+)', raster)]
    return ["TestCLI_E2E/" + name for name in names] + extra


def bounded_run(command, env, directory, log, timeout, extra_directories=()):
    """Bound both captures and logs, and stop the whole subprocess tree."""
    started = time.monotonic()
    reason = None
    with log.open("wb") as stream:
        process = subprocess.Popen(command, cwd=TRACE, env=env, stdout=stream,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        try:
            while process.poll() is None:
                sizes = []
                for output_root in [directory, *extra_directories]:
                    for path in output_root.rglob("*"):
                        try:
                            if path.is_file():
                                sizes.append(path.stat().st_size)
                        except FileNotFoundError:
                            # Upstream tests remove temporary .got files.
                            pass
                if any(size > 128 * 1024**2 for size in sizes) or sum(sizes) > 1024**3:
                    reason = "budget-exceeded"
                elif shutil.disk_usage(ROOT).free < 20 * 1024**3:
                    reason = "free-space-exhausted"
                elif time.monotonic() - started > timeout:
                    reason = "timeout"
                if reason:
                    break
                time.sleep(0.5)
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
            process.wait()
    return {"exit": process.returncode, "failure": reason}


def events(path):
    results = {}
    for line in path.read_text(errors="replace").splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        name, action = event.get("Test"), event.get("Action")
        if name and action in {"pass", "fail", "skip"}:
            results[name] = action
    return results


def compare_file(expected, actual, *, golden=False):
    if not expected.is_file() or not actual.is_file():
        return {"matched": False, "error": "missing artifact"}
    if not expected.stat().st_size or not actual.stat().st_size:
        return {"matched": False, "error": "empty artifact"}
    extension = expected.suffix
    if not extension:
        # Captured stdout has no filename extension; its payload still obeys
        # the same format gates as a file output.
        prefix = expected.read_bytes()[:64].lstrip()
        extension = (".svg" if prefix.startswith((b"<svg", b"<?xml")) else
                     ".gif" if prefix.startswith((b"GIF87a", b"GIF89a")) else
                     ".pptx" if prefix.startswith(b"PK\x03\x04") else "")
    if extension == ".png":
        return RASTER.compare_png_outputs(expected, actual)
    if extension == ".gif":
        try:
            return {"matched": gif_frames(expected) == gif_frames(actual)}
        except (OSError, ValueError, EOFError) as error:
            return {"matched": False, "error": str(error)}
    left, right = expected.read_bytes(), actual.read_bytes()
    if not golden and extension == ".svg":
        return {"matched": E2E.normalize(left.decode()) == E2E.normalize(right.decode())}
    if extension == ".pptx" and left != right:
        # Classification only: the strict complete-artifact verdict stays false.
        # lib/version/version.go is v0.8.1-HEAD in the pinned source; release
        # builds stamp v0.9.0. Do not rewrite the native release's AppVersion.
        try:
            versions = []
            for path in [expected, actual]:
                with zipfile.ZipFile(path) as archive:
                    app = ET.fromstring(archive.read("docProps/app.xml"))
                versions.append(app.findtext("{*}AppVersion"))
            if versions == ["0.8.1", "0.9.0"]:
                return {"matched": False, "metadata_differences": [{
                    "field": "docProps/app.xml/AppVersion", "go": versions[0], "rust": versions[1],
                    "reason": "Pinned Go source lib/version/version.go is v0.8.1-HEAD; "
                              "the native CLI reports release v0.9.0. No version normalization applied.",
                }]}
        except (OSError, ValueError, KeyError, zipfile.BadZipFile, ET.ParseError):
            pass
    return {"matched": left == right}


def gif_frames(path):
    from PIL import Image
    if not path.read_bytes().endswith(b";"):
        raise ValueError("missing GIF trailer")
    with Image.open(path) as image:
        if image.format != "GIF" or image.n_frames == 0:
            raise ValueError("missing GIF frames")
        canvas, loop = image.size, image.info.get("loop", -1)
        frames = []
        for index in range(image.n_frames):
            image.seek(index)
            frames.append((image.size, image.info.get("duration", 0),
                           hashlib.sha256(image.convert("RGBA").tobytes()).hexdigest()))
    return canvas, loop, frames


def inventory(directory):
    return {p.relative_to(directory).as_posix(): p for p in sorted(directory.rglob("*")) if p.is_file()}


def board_manifest_inventory(manifest):
    """The owned-file inventory of one d2cli board output manifest, after
    verifying every digest against that side's own published bytes."""
    record = json.loads(manifest.read_text())
    if record.get("format") != "d2-board-output" or record.get("version") != 1:
        raise ValueError("unknown board output manifest format")
    for entry in record["files"]:
        path = manifest.parent / entry["path"]
        if hashlib.sha256(path.read_bytes()).hexdigest() != entry["sha256"]:
            raise ValueError(f"manifest digest does not match {entry['path']}")
    return [entry["path"] for entry in record["files"]], record["directories"]


def compare_board_manifest(expected, actual):
    # The digests cover SVG bytes (compared under the version allowance) and
    # PNG encoder bytes (compared by pixels); each side must describe its own
    # files exactly, and both must own the same paths and directories.
    try:
        return {"matched": board_manifest_inventory(expected) == board_manifest_inventory(actual)}
    except (OSError, ValueError, KeyError, TypeError) as error:
        return {"matched": False, "error": str(error)}


def compare_golden_manifest(golden, actual):
    """A board manifest golden digests SVGs generated with the pinned library
    version. Apply the SVG version allowance before hashing each native file
    (only where the native digest describes that file), then require the
    manifest bytes to equal the golden exactly."""
    version = re.search(r'Version = "([^"]+)"', (UPSTREAM / "lib/version/version.go").read_text()).group(1)
    record = json.loads(actual.read_text())
    for entry in record.get("files", []):
        path = actual.parent / entry["path"]
        data = path.read_bytes()
        if path.suffix == ".svg" and hashlib.sha256(data).hexdigest() == entry["sha256"]:
            data = re.sub(rb'(data-d2-version|d2Version)="[^"]*"', b'\\1="' + version.encode() + b'"', data)
            entry["sha256"] = hashlib.sha256(data).hexdigest()
    rehashed = json.dumps(record, indent=2).encode() + b"\n"
    return {"matched": rehashed == golden.read_bytes()}


def compare_case(go_dir, rust_dir):
    if not go_dir.is_dir() or not rust_dir.is_dir():
        return {"matched": False, "error": "missing case capture"}
    go_files, rust_files = inventory(go_dir), inventory(rust_dir)
    if go_files.keys() != rust_files.keys():
        return {"matched": False, "error": "artifact inventory differs",
                "go_artifacts": sorted(go_files), "rust_artifacts": sorted(rust_files)}
    # Cases such as validation/errors legitimately have no generated output;
    # their original upstream assertions remain the separate contract gate.
    empty_path = go_dir / "empty-inputs.json"
    empty_inputs = json.loads(empty_path.read_text()) if empty_path.exists() else {}
    artifacts = []
    for name in go_files:
        # The source fixtures can deliberately write empty icon inputs. They
        # are compared as input bytes, never accepted as rendered artifacts.
        if name.startswith("files/") and name.removeprefix("files/") in empty_inputs:
            result = {"matched": go_files[name].read_bytes() == rust_files[name].read_bytes() == b"",
                      "input": True}
        elif PurePosixPath(name).name == RASTER.BOARD_OUTPUT_MANIFEST:
            result = compare_board_manifest(go_files[name], rust_files[name])
        else:
            result = compare_file(go_files[name], rust_files[name])
        artifacts.append({"name": name, **result})
    return {"matched": all(row["matched"] for row in artifacts), "artifacts": artifacts}


def score(names, capture, run_results, *, pptx_release=False):
    outcomes = {engine: events(capture / f"{engine}.jsonl") for engine in ["go", "rust"]}
    release_events = events(capture / "go-release.jsonl") if pptx_release else {}
    rows = []
    for name in names:
        row = {"name": name, "go": outcomes["go"].get(name, "missing"),
               "rust": outcomes["rust"].get(name, "missing")}
        if any(run_results[engine]["failure"] for engine in run_results):
            row["outcome"] = "capture-failed"
        elif row["go"] != "pass":
            row["outcome"] = "upstream-skip" if row["go"] == "skip" else "go-unreproduced"
        elif name.endswith(("/stdout_pdf_partial_write_error", "/stdout_pptx_close_error")):
            row["outcome"] = "unsupported-native-writer"
        elif row["rust"] != "pass":
            row["outcome"] = "rust-failed"
        elif pptx_release and name in PPTX_RELEASE_CASES and release_events.get(name) != "pass":
            row["outcome"] = "go-release-unreproduced"
        else:
            oracle = "go"
            source_capture_failed = False
            if pptx_release and name in PPTX_RELEASE_CASES:
                oracle = "go-release"
                row["go_release"] = release_events[name]
                row["source_comparison"] = compare_case(capture / "go/cases" / name, capture / "rust/cases" / name)
                source_capture_failed = ("error" in row["source_comparison"] or
                                         any("error" in item for item in row["source_comparison"].get("artifacts", [])))
            row.update(compare_case(capture / f"{oracle}/cases" / name, capture / "rust/cases" / name))
            if source_capture_failed:
                row["matched"] = False
            row["outcome"] = ("source-capture-failed" if source_capture_failed else
                              "contract-match" if row["matched"] else "artifact-mismatch")
        rows.append(row)
    golden_rows = []
    for golden in sorted((UPSTREAM / "e2etests-cli/testdata").rglob("*.exp.*")):
        relative = golden.relative_to(UPSTREAM / "e2etests-cli/testdata").as_posix()
        base = relative.split(".exp.", 1)[0]
        candidates = [name for name in names if base == name or base.startswith(name + "/")]
        if relative.startswith("raster/"):
            fixture = Path(base).name
            raster_name = ("TestGIFFixtures/" + fixture.removeprefix("animation-")
                           if fixture.startswith("animation-") else "TestPNGFixtures/" + fixture)
            if raster_name in names:
                candidates.append(raster_name)
        name = max(candidates, key=len) if candidates else None
        row = {"name": relative, "case": name, "sha256": hashlib.sha256(golden.read_bytes()).hexdigest()}
        if name is None:
            row["outcome"] = "unbound-golden"
        elif outcomes["go"].get(name) != "pass" or run_results["go"]["failure"]:
            row["outcome"] = "go-unreproduced"
        else:
            files = inventory(capture / "go/cases" / name)
            # Output names differ from golden names (e.g. center/hello-world.svg).
            # Bind by exact bytes/pixels, never by a guessed filename alone.
            matching = [key for key, path in files.items()
                        if (path.suffix == golden.suffix or key.startswith("stdout/"))
                        and compare_file(golden, path, golden=True)["matched"]]
            row["go_bindings"] = matching
            if not matching:
                row["outcome"] = "stale-or-unbound-golden"
            elif outcomes["rust"].get(name) != "pass" or run_results["rust"]["failure"]:
                row["outcome"] = "rust-failed"
            else:
                matches = [compare_golden_manifest(golden, capture / "rust/cases" / name / key)
                           if PurePosixPath(key).name == RASTER.BOARD_OUTPUT_MANIFEST else
                           compare_file(golden, capture / "rust/cases" / name / key) for key in matching]
                row["outcome"] = "golden-match" if all(item["matched"] for item in matches) else "golden-mismatch"
        golden_rows.append(row)
    return {"cases": rows, "goldens": golden_rows,
            "summary": {"cases": dict(collections.Counter(row["outcome"] for row in rows)),
                        "goldens": dict(collections.Counter(row["outcome"] for row in golden_rows))}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust", type=Path, default=ROOT / "target/diagnostic/d2-rs")
    parser.add_argument("--capture", required=True, type=Path, help="fresh disposable directory, <=1 GiB")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--reuse", action="store_true", help="score an existing completed capture")
    parser.add_argument("--pdf-epoch", type=int, help="opt-in shared UTC Unix clock and FPDF catalog sort; full PDF bytes still compare exactly")
    parser.add_argument("--pptx-release", action="store_true", help="also run all four PPTX output cases with the pinned Go v0.9.0 release linker stamp; preserve source captures and golden qualification")
    args = parser.parse_args()
    if args.pdf_epoch is not None and not 0 <= args.pdf_epoch <= 253402300799:
        parser.error("--pdf-epoch must be in 0..=253402300799")
    pdf_clock = ({"epoch_seconds": args.pdf_epoch, "timezone": "UTC",
                  "catalog_order": "FPDF SetCatalogSort(true)"} if args.pdf_epoch is not None else None)
    names = install()
    capture = args.capture.resolve()
    if args.reuse:
        run_results = json.loads((capture / "runs.json").read_text())
        manifest_path = capture / "manifest.json"
        if (pdf_clock is not None or args.pptx_release) and not manifest_path.exists():
            parser.error("--reuse explicit profiles require a captured manifest")
        if manifest_path.exists():
            manifest = json.loads(manifest_path.read_text())
            if manifest.get("pdf_clock") != pdf_clock:
                parser.error("--reuse PDF clock differs from the captured controls")
            if manifest.get("pptx_release") != (PPTX_RELEASE if args.pptx_release else None):
                parser.error("--reuse PPTX release profile differs from the captured controls")
            if hashlib.sha256(args.rust.read_bytes()).hexdigest() != manifest["rust_sha256"]:
                parser.error("--reuse binary differs from the captured binary")
    else:
        capture.mkdir(parents=True, exist_ok=False)
        work = capture / "work"
        work.mkdir()
        manifest = {"source_commit": references.SOURCE_COMMIT,
                    "rust_sha256": hashlib.sha256(args.rust.read_bytes()).hexdigest(),
                    "pdf_clock": pdf_clock,
                    "pptx_release": PPTX_RELEASE if args.pptx_release else None,
                    "input_sha256": {str(path.relative_to(UPSTREAM)): hashlib.sha256(path.read_bytes()).hexdigest()
                                     for path in [UPSTREAM / "e2etests-cli/main_test.go", UPSTREAM / "e2etests-cli/raster_test.go",
                                                  UPSTREAM / "d2cli/validate.go", UPSTREAM / "lib/pdf/pdf.go",
                                                  UPSTREAM / "lib/version/version.go", UPSTREAM / "ci/release/_build.sh"]},
                    "hook_sha256": hashlib.sha256((ROOT / "tools/cli-scoreboard/hook_test.go").read_bytes()).hexdigest(),
                    "hook_files_sha256": {path.name: hashlib.sha256(path.read_bytes()).hexdigest()
                                          for path in sorted((ROOT / "tools/cli-scoreboard").glob("*.go"))},
                    "budget_bytes": 1024**3, "per_file_bytes": 128 * 1024**2}
        (capture / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        env = {**os.environ, "GOMODCACHE": str(ROOT / ".reference/go-mod/pkg/mod"),
               "GOCACHE": str(ROOT / ".reference/go-build"), "GOFLAGS": "-mod=mod", "GOPROXY": "off",
               "TMPDIR": str(work), "GOTMPDIR": str(work),
               "D2RS_CLI_WORK": str(work / "cases")}
        env.pop("D2RS_CLI_BINARY", None)
        env.pop("SOURCE_DATE_EPOCH", None)
        if args.pdf_epoch is not None:
            env["SOURCE_DATE_EPOCH"] = str(args.pdf_epoch)
        # Never allow inherited golden-accept settings to rewrite the oracle.
        env.pop("TESTDATA_ACCEPT", None)
        env.pop("TA", None)
        command = ["go", "test", "-json", "./e2etests-cli", "-run", "^(TestCLI_E2E|TestPNGFixtures|TestGIFFixtures)$", "-count=1", "-parallel=4", "-timeout=12m"]
        run_results = {}
        for engine in ["go", "rust"]:
            env["D2RS_CLI_CAPTURE"] = str(capture / engine)
            if engine == "rust":
                env["D2RS_CLI_BINARY"] = str(args.rust.resolve())
            print(f"Running pinned CLI contracts: {engine}", flush=True)
            run_results[engine] = bounded_run(command, env, capture, capture / f"{engine}.jsonl", 780)
            print(run_results[engine], flush=True)
        if args.pptx_release:
            # Run unmodified upstream PPTX assertions in a separate release-
            # stamped test binary. Never overwrite the original source capture.
            env.pop("D2RS_CLI_BINARY", None)
            env["D2RS_CLI_CAPTURE"] = str(capture / "go-release")
            selected = "|".join(re.escape(name.split("/", 1)[1]) for name in sorted(PPTX_RELEASE_CASES))
            release_command = ["go", "test", "-json", "-ldflags=" + PPTX_RELEASE["ldflags"],
                               "./e2etests-cli", "-run", "^TestCLI_E2E$/(" + selected + ")$",
                               "-count=1", "-parallel=4", "-timeout=12m"]
            print("Running pinned CLI PPTX contracts: Go release v0.9.0", flush=True)
            run_results["go_release"] = bounded_run(release_command, env, capture, capture / "go-release.jsonl", 780)
            print(run_results["go_release"], flush=True)
        (capture / "runs.json").write_text(json.dumps(run_results, indent=2) + "\n")
    manifest_path = capture / "manifest.json"
    report = {"rust": str(args.rust.resolve()), "runs": run_results,
              "capture": str(capture), "manifest": json.loads(manifest_path.read_text()) if manifest_path.exists() else None,
              **score(names, capture, run_results, pptx_release=args.pptx_release)}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=1) + "\n")
    print(json.dumps(report["summary"], indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
