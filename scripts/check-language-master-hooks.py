#!/usr/bin/env python3
"""Replay upstream master-only internal tests with every call recorded.

usage: check-language-master-hooks.py <family>

Fixture generator for the `lang-master-*` recipes
(tools/fixtures/langlibelk-language-compiler.toml). Each family is a set of
original upstream tests (assertions active) whose calls are routed through an
env-gated observer hook from tools/language-master-scoreboard: the test files
are rewritten in the recipe workdir with the same call-site substitutions the
language-*-master-scoreboard.py scripts apply to the trace checkout, and both
the rewritten files and the hook are added with `go test -overlay`, so the
upstream tree is untouched. Every observed session the hook dumps becomes one
fixture case. `cargo test -p d2-rs-language --features oracle-fixtures` does
the native comparison (docs/FIXTURES.md).

Determinism: tests that call t.TempDir() record a random directory
(`$TMPDIR/<TestName><random digits>/<NNN>`) in requests and diagnostics. The
capture runs with TMPDIR set to a workdir directory and rewrites that prefix to
the fixed `/tmp/d2rs-tempdir/<TestName>/<NNN>` in each raw dump before it is
decoded, in requests and results alike, so the case is what Go reports for
that directory and two runs give identical bytes.
"""
import hashlib
import json
import re
import subprocess
import sys

from fixture_recipe import go_env, workdir
from language_oracle import fixture, upstream, write
from references import MANIFEST, ROOT

HOOKS = ROOT / "tools/language-master-scoreboard"
TEMPDIR_PLACEHOLDER = "/tmp/d2rs-tempdir"


def class_index(text):
    text = re.sub(r"\bcompileExpansionIR\(t,", "d2rsClassIR(t,", text)
    return text.replace("c.getClassMap(", "d2rsClassLookup(t, c, ")


def graph(text):
    text = re.sub(r"\bcompileExpansionIR\(t,", "d2rsGraphIR(t,", text)
    return text.replace("compileIRContext(", "d2rsGraphCompile(t, ")


def import_depth(text):
    text = text.replace("d2ir.Compile(", "d2rsIRCompile(t, ").replace("d2parser.Parse(", "d2rsIRParse(t, ")
    return text + "\nvar _ = d2parser.Parse // capture import anchor\n"


def ir_internal(text):
    text = text.replace("d2parser.Parse(", "d2rsInternalParse(t, ")
    for kind, symbol in (("variable", "Variable"), ("glob", "Glob"), ("edge", "Edge"), ("work", "EdgeExpansionWork")):
        name = "new" + (symbol + "ExpansionBudget" if kind != "work" else symbol + "Budget") + "("
        text = text.replace(name, f'd2rsInternalBudgetNew(t, "{kind}", ')
    for old, new in [
        ("c.compileArray(dst, array, ast)", "d2rsInternalArray(t, c, dst, array, ast)"),
        ("compileIndexTestSource(t,", "d2rsInternalBase(t,"),
        ("d2parser.ParseMapKey(", "d2rsInternalKey(t, "),
        ("for cancelAt := 1; cancelAt < 128; cancelAt++ {", "d2rsInternalRange(t, 1, 128)\nfor cancelAt := 1; cancelAt < 128; cancelAt++ {"),
        ("root.CreateEdge(NewEdgeIDs(key)[0], refctx, c)", "d2rsInternalCreate(t, root, NewEdgeIDs(key)[0], refctx, c)"),
        ("root.getEdgesForCompile(NewEdgeIDs(key)[0], refctx, c)", "d2rsInternalSelect(t, root, NewEdgeIDs(key)[0], refctx, c)"),
        ("ReserveVariableExpansionAliases(context.Background(), nil)", "d2rsInternalNilReserve(t, context.Background(), nil, false)"),
        ("ReserveVariableExpansionCopy(context.Background(), nil)", "d2rsInternalNilReserve(t, context.Background(), nil, true)"),
    ]:
        text = text.replace(old, new)
    return text + "\nvar _ = d2parser.Parse // capture import anchor\n"


# The rewrites below are those of scripts/language-{master,ir,ast-clone,
# ir-copy,small}-master-scoreboard.py.
def compiler(text):
    assert text.startswith("package d2compiler_test"), "compiler hook needs the external test package"
    text = text.replace("d2compiler.Compile(", "d2rsMasterCompile(t, ")
    if "d2compiler." not in text:
        text += "\nvar _ = d2compiler.Compile // d2rs master capture import anchor\n"
    return text


def ir_glob(text):
    text = re.sub(r"\bCompile\(ast,", "d2rsIRCompile(t, ast,", text).replace("d2parser.Parse(", "d2rsIRParse(t, ")
    return text + "\nvar _ = d2parser.Parse // capture import anchor\n"


def ast_clone(text):
    text = text.replace("d2parser.Parse(", "d2rsCloneParse(t, ").replace(
        "cloneASTMapContext(ctx, ast)", "d2rsCloneASTMapContext(t, ctx, ast)")
    return text + "\nvar _ = d2parser.Parse // original capture import anchor\n"


def ir_copy(text):
    text = text.replace("d2parser.Parse(", "d2rsCopyParse(t, ")
    text = re.sub(r"(?<![\w.])Compile\(", "d2rsCopyCompile(t, ", text)
    text = text.replace("ReserveVariableExpansionCopy(", "d2rsCopyReserve(t, ")
    text = text.replace("variableExpansionNodeUnits(", "d2rsCopyUnits(t, ")
    return text + "\nvar _ = d2parser.Parse // original capture import anchor\n"


def replacing(*pairs):
    def rewrite(text):
        for old, new in pairs:
            assert old in text, old
            text = text.replace(old, new)
        return text
    return rewrite


def encode_class_results(case, raw):
    """Dictionary-encode the 8k repeated lookup results (values + indexes).

    `source_sha256` is the SHA-256 of the observer's raw dump of the session.
    """
    values, indexes, seen = [], [], {}
    for result in case["got"]["results"]:
        key = json.dumps(result, sort_keys=True)
        if key not in seen:
            seen[key] = len(values)
            values.append(result)
        indexes.append(seen[key])
    got = dict(case["got"], results={"values": values, "indexes": indexes})
    return {"name": case["name"], "source_sha256": hashlib.sha256(raw).hexdigest(), "request": case["request"], "got": got}


def error_only(case, raw):
    """master-variable-expansion keeps only the compiler diagnostic of each call."""
    return {"name": case["name"], "request": case["request"], "error": case["got"]["error"]}


COMPILE_TESTS = ["d2compiler/compile_test.go", "d2compiler/edge_expansion_test.go",
                 "d2compiler/gradient_security_test.go", "d2compiler/isometric_config_test.go",
                 "d2compiler/variable_expansion_test.go"]

# family -> runs, output, compact, case order, case transform, layout.
# A run is (package, hook, {test file: rewrite}, test names or None for every
# Test in the files); runs are captured separately and concatenated. Case
# order is by session name within each run ("name"), by session name over
# all runs ("name-all"), or "dump" (the observer's sha256(name) file order).
# Layout "cases" writes {"source_commit", "cases"}; "case" writes the single
# observed session as {"source_commit", "case"}.
FAMILIES = {
    "class-index": ([("d2compiler", "class_index_hook_test.go",
                      {"d2compiler/variable_expansion_internal_test.go": class_index},
                      ["TestClassLookupBuildsOneReusableIndex", "TestClassLookupIndexConstructionIsCancelable"])],
                    "master-class-index.json", True, "dump", encode_class_results, "cases"),
    "graph-materialization": ([("d2compiler", "graph_hook_test.go",
                                {"d2compiler/variable_expansion_internal_test.go": graph},
                                ["TestGraphMaterializationBoundsUnusedAliasesAcrossBoards",
                                 "TestCompileCancellationDuringGraphMaterialization"])],
                              "master-graph-materialization.json", False, "name", None, "cases"),
    "import-depth": ([("d2ir", "ir_import_hook_test.go", {"d2ir/import_depth_test.go": import_depth}, None),
                      ("d2ir", "ir_import_hook_test.go", {"d2ir/import_test.go": import_depth},
                       ["TestNilFSDeniesImports", "TestCanceledContextPropagatesFromImportedParse"])],
                     "master-import-depth.json", False, "dump", None, "cases"),
    "ir-internal": ([("d2ir", "ir_internal_hook_test.go",
                      {"d2ir/expansion_test.go": ir_internal, "d2ir/glob_expansion_test.go": ir_internal,
                       "d2ir/edge_expansion_test.go": ir_internal},
                      ["TestVariableExpansionBudget", "TestCompileArrayStopsOnCancellation", "TestGlobExpansionBudgetDefault",
                       "TestEdgeExpansionBudget", "TestEdgeExpansionWorkBudget", "TestEdgeGlobExpansionObservesCancellation",
                       "TestEdgeSelectorGlobExpansionObservesCancellation"])],
                    "master-ir-internal.json", True, "name", None, "cases"),
    "compiler-bounds": ([("d2compiler", "compiler_hook_test.go", {path: compiler for path in COMPILE_TESTS},
                          ["TestNilFSDeniesImports", "TestCompilePreservesSafeOrdinaryLinks",
                           "TestCompileRejectsDangerousOrdinaryLinks", "TestGridCapacityLimit", "TestSingleGridDimensionLimit",
                           "TestIsometricConfig", "TestIsometricConfigFromImport", "TestCompileRejectsGradientStopMarkup",
                           "TestCompileEdgeExpansionLimit", "TestCompileEdgeExpansionWorkLimit",
                           "TestCompileEdgeSelectorExpansionLimit", "TestCompileEdgeExpansionDoesNotChargeExplicitEdges"])],
                        "master-compiler-bounds.json", False, "dump", None, "cases"),
    "variable-expansion": ([("d2compiler", "compiler_hook_test.go", {path: compiler for path in COMPILE_TESTS},
                             ["TestClassApplicationCartesianExpansionIsBounded", "TestCompositeVariableAliasingRemainsValid",
                              "TestCompositeVariableCycles", "TestCompositeVariableForwardSpreadFailsClosed",
                              "TestCompositeVariableSpreadsRemainValid",
                              "TestMarkdownNestedVariableNamesAreBoundedBeforeFlattening",
                              "TestMarkdownSubstitutionSupportsQuotedVariableNamesWithClosingBraces",
                              "TestMarkdownVariableMatcherConstructionIsCumulativeAcrossBlocks",
                              "TestUnusedCompositeAliasBoardCopiesAreBounded", "TestVariableExpansionBoundaryAndValidation",
                              "TestVariableExpansionDefaultLimit", "TestVariableExpansionLimits"])],
                           "master-variable-expansion.json", False, "name", error_only, "cases"),
    "glob-expansion": ([("d2ir", "ir_glob_hook_test.go", {"d2ir/glob_expansion_test.go": ir_glob},
                         ["TestGlobExpansionBudgetDoesNotCountExplicitFields", "TestDefaultRecursiveGlobFeedbackLimitMessage",
                          "TestLiteralGlobBodyWorkConsumesBudget", "TestTripleGlobSkipsUnresolvedSubstitutionPlaceholder",
                          "TestRecursiveGlobFeedbackIsBounded", "TestGlobGeneratedSubstitutionPlaceholdersConsumeBudget",
                          "TestGlobExpansionRejectsNegativeLimit", "TestGlobArrayNodeWorkConsumesBudget",
                          "TestTwoRuleRecursiveGlobFeedbackIsBoundedByDefault"])],
                       "master-glob-expansion.json", False, "dump", None, "cases"),
    "import-expansion": ([("d2ir", "ir_glob_hook_test.go", {"d2ir/expansion_test.go": ir_glob},
                           ["TestRepeatedImportPeeksUseCompilationBudget", "TestImportedArraySpreadChargesInsertedMembers",
                            "TestImportedVariableExpansionUsesCompilationBudget"])],
                         "master-import-expansion.json", False, "dump", None, "cases"),
    "ir-cancellation": ([("d2ir", "ir_glob_hook_test.go", {"d2ir/expansion_test.go": ir_glob},
                          ["TestCompileCancellationDuringIRConstruction"])],
                        "master-ir-cancellation.json", False, "dump", None, "case"),
    "ast-clone": ([("d2ir", "ast_clone_hook_test.go", {"d2ir/expansion_test.go": ast_clone},
                    ["TestCachedImportASTCloneStopsOnCancellation",
                     "TestCachedImportASTArrayCloneCancellationNeverReturnsPartialValues"])],
                  "master-ast-clone.json", False, "dump", None, "cases"),
    "ir-copy": ([("d2ir", "ir_copy_hook_test.go", {"d2ir/expansion_test.go": ir_copy},
                  ["TestVariableExpansionCopyUnitsIncludeSlices", "TestVariableExpansionCopyReservationsAreCumulative",
                   "TestImportedArraySpreadChargesInsertedMembers"])],
                "master-ir-copy.json", False, "name", None, "cases"),
    "small": ([("d2graph", "small_grid_hook_test.go",
                {"d2graph/grid_diagram_test.go": replacing(("GridCapacity(tc.rows, tc.columns)", "d2rsSmallGrid(t, tc.rows, tc.columns)"))},
                ["TestGridCapacityLimits"]),
               ("d2ir", "small_ir_hook_test.go",
                {"d2ir/index_test.go": replacing(("m.EnsureField(key, nil, true, nil)", "d2rsSmallEnsure(t, m, key, nil, true, nil)")),
                 "d2ir/substitution_cycle_test.go": replacing(("compositeContainsNode(root,", "d2rsSmallContains(t, root,"))},
                ["TestEnsureFieldCreateWithoutCompiler", "TestCompositeContainsNodeHandlesExistingCycle"]),
               ("lib/textmeasure", "small_links_hook_test.go",
                {"lib/textmeasure/markdown_svg_test.go": replacing(("textmeasure.IsDangerousLink(link)", "d2rsSmallDanger(t, link)"),
                                                                    ("textmeasure.SafeMarkdownLink(link)", "d2rsSmallSafe(t, link)"))},
                ["TestDangerousLinkPolicy"])],
              "master-small.json", False, "name-all", None, "cases"),
}


def canonical_tempdirs(raw: bytes, tmp: str) -> bytes:
    pattern = re.escape(tmp.encode()) + rb"/([^/\"\\]*?)\d+/(\d{3})"
    return re.sub(pattern, lambda m: TEMPDIR_PLACEHOLDER.encode() + b"/" + m[1] + b"/" + m[2], raw)


def capture(index, package, hook, rewrites, tests):
    source = upstream()
    run = workdir() / f"run{index}"
    dump = run / "go"
    dump.mkdir(parents=True)
    tmp = run / "tmp"
    tmp.mkdir()
    overlay = {}
    for relative, rewrite in rewrites.items():
        original = (source / relative).read_text()
        text = rewrite(original)
        assert text != original, f"{relative}: no call site rewritten (upstream test changed?)"
        target = run / relative.replace("/", "__")
        target.write_text(text)
        overlay[str(source / relative)] = str(target)
        if tests is None:
            tests = re.findall(r"func (Test\w+)\(t \*testing.T\)", original)
    overlay[str(source / package / f"d2rs_{hook}")] = str(HOOKS / hook)
    (run / "overlay.json").write_text(json.dumps({"Replace": overlay}))
    env = dict(go_env(), D2RS_MASTER_LANGUAGE_DUMP=str(dump), TMPDIR=str(tmp))
    env.pop("TESTDATA_ACCEPT", None)
    env.pop("TA", None)
    subprocess.run(["go", "test", "-mod=readonly", "-count=1", "-overlay", str(run / "overlay.json"),
                    "-run", "^(" + "|".join(tests) + ")$", f"./{package}"], cwd=source, env=env, check=True)
    raws = [canonical_tempdirs(path.read_bytes(), str(tmp)) for path in sorted(dump.glob("*.json"))]
    assert not any(str(tmp).encode() in raw for raw in raws), "unrecognized temporary path in a dump"
    cases = [json.loads(raw) for raw in raws]
    assert cases, "the hook observed no original call"
    observed = {case["name"].split("#")[0].split("/")[0] for case in cases}
    assert observed == set(tests), f"tests without observed sessions: {sorted(set(tests) - observed)}"
    return list(zip(cases, raws))


def main():
    runs, name, compact, order, transform, layout = FAMILIES[sys.argv[1]]
    rows = []
    for index, (package, hook, rewrites, tests) in enumerate(runs):
        captured = capture(index, package, hook, rewrites, tests)
        if order == "name":
            captured.sort(key=lambda row: row[0]["name"])
        rows += captured
    if order == "name-all":
        rows.sort(key=lambda row: row[0]["name"])
    cases = [transform(case, raw) if transform else case for case, raw in rows]
    names = [case["name"] for case in cases]
    assert len(names) == len(set(names)), "duplicate observed session"
    if layout == "case":
        assert len(cases) == 1, f"{name}: expected one observed session, got {len(cases)}"
        value = {"source_commit": MANIFEST["d2"]["commit"], "case": cases[0]}
    else:
        value = {"source_commit": MANIFEST["d2"]["commit"], "cases": cases}
    write(fixture(name), value, compact=compact)
    print(f"{name}: {len(cases)} observed sessions")


if __name__ == "__main__":
    main()
