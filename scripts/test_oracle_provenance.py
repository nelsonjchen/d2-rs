"""Parity captures must reject stale reference binaries and trace patches."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import references


class OracleProvenanceTests(unittest.TestCase):
    def test_rejects_each_mismatched_provenance_field(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / 'oracle'
            binary.write_bytes(b'oracle executable')
            trace = root / 'tools/patches/d2-v090-stage-trace.patch'
            trace.parent.mkdir(parents=True)
            trace.write_bytes(b'instrumentation')
            expected = {
                'source_commit': references.MANIFEST['d2']['commit'],
                'go_toolchain': references.MANIFEST['d2']['go_toolchain'],
                'binary_sha256': references.digest(binary),
                'trace_patch_sha256': references.digest(trace),
            }
            provenance = binary.with_suffix('.provenance.json')
            with patch.object(references, 'ROOT', root):
                provenance.write_text(json.dumps(expected))
                self.assertEqual(references.verify_oracle(binary, trace=True), expected)
                for key in expected:
                    with self.subTest(key=key):
                        provenance.write_text(json.dumps({**expected, key: 'stale'}))
                        with self.assertRaisesRegex(RuntimeError, key):
                            references.verify_oracle(binary, trace=True)
                provenance.write_text(json.dumps(expected))
                binary.write_bytes(b'replaced executable')
                with self.assertRaisesRegex(RuntimeError, 'binary_sha256'):
                    references.verify_oracle(binary, trace=True)

    def test_uninstrumented_oracle_cannot_claim_trace_patch(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / 'oracle'
            binary.write_bytes(b'oracle')
            binary.with_suffix('.provenance.json').write_text(json.dumps({
                'source_commit': references.MANIFEST['d2']['commit'],
                'go_toolchain': references.MANIFEST['d2']['go_toolchain'],
                'binary_sha256': references.digest(binary),
                'trace_patch_sha256': 'unexpected instrumentation',
            }))
            with self.assertRaisesRegex(RuntimeError, 'trace_patch_sha256'):
                references.verify_oracle(binary)


if __name__ == '__main__':
    unittest.main()
