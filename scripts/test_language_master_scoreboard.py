#!/usr/bin/env python3
"""Failure gates for the Lane L master inline-test inventory/delta."""
import importlib.util
from pathlib import Path
import unittest
import tempfile
from unittest.mock import patch

def module(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    value = importlib.util.module_from_spec(spec); spec.loader.exec_module(value); return value

delta = module("language-master-scoreboard-delta").delta
scoreboard = module("language-master-scoreboard")

class Gates(unittest.TestCase):
    def report(self, outcome):
        return {"source_commit":"pin", "rows":[{"name":"case#0", "outcome":outcome}]}

    def test_lost_match_fails_for_every_nonpassing_outcome(self):
        for outcome in ("mismatch", "unsupported", "timeout", "rust-failed", "go-fail", "go-missing-event"):
            self.assertEqual(delta(self.report("match"), self.report(outcome))["regressions"], ["case#0"])

    def test_missing_case_fails(self):
        with self.assertRaises(AssertionError): delta(self.report("match"), {"source_commit":"pin", "rows":[]})

    def test_pin_change_fails(self):
        changed = self.report("match"); changed["source_commit"] = "other"
        with self.assertRaises(AssertionError): delta(self.report("match"), changed)

    def test_only_matches_gain(self):
        self.assertEqual(delta(self.report("unsupported"), self.report("mismatch"))["gains"], [])
        self.assertEqual(delta(self.report("unsupported"), self.report("match"))["gains"], ["case#0"])

    def test_inventory_includes_internal_ir_and_library_tests(self):
        tests = scoreboard.inventory()
        self.assertTrue(any(t["path"] == "d2ir/expansion_test.go" for t in tests))
        self.assertTrue(any(t["path"] == "lib/textmeasure/substitutions_test.go" for t in tests))
        self.assertEqual(len({(t["path"],t["test"]) for t in tests}), len(tests))

    def test_install_uninstall_preserves_existing_hooks_and_imports(self):
        with tempfile.TemporaryDirectory() as directory:
            trace = Path(directory)
            target = trace / "d2compiler/gradient_security_test.go"
            target.parent.mkdir()
            original = ('package d2compiler_test\nimport (\n"github.com/d2lang/d2/d2compiler"\n)\n'
                        'func probe(t *testing.T) {\n d2compiler.Compile("input.d2", reader, nil)\n'
                        ' d2rsLanguageDump(t, existing, got, err)\n}\n')
            target.write_text(original)
            tests = [{"path":"d2compiler/gradient_security_test.go", "test":"TestExample"}]
            with patch.object(scoreboard.references, "TRACE_CHECKOUT", trace), \
                 patch.object(scoreboard.references, "verify_source"), \
                 patch.object(scoreboard.subprocess, "check_output", return_value=scoreboard.references.SOURCE_COMMIT):
                scoreboard.install(tests)
                scoreboard.install(tests)
                self.assertIn("d2rsLanguageDump", target.read_text())
                self.assertIn("d2rsMasterCompile(t, ", target.read_text())
                scoreboard.uninstall(tests)
            self.assertEqual(target.read_text(), original)
            self.assertFalse((target.parent / "d2rs_master_language_test.go").exists())

if __name__ == "__main__": unittest.main()
