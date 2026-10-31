"""Complete graphjson gate must preserve metadata, order and failures."""
import importlib.util
import unittest
from pathlib import Path
spec = importlib.util.spec_from_file_location("complete", Path(__file__).with_name("check-graphjson-complete.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class CompleteTests(unittest.TestCase):
    def check(self, a, b, **kwargs):
        return module.compare(0, a, b"", 0, b, b"", **kwargs)["passed"]

    def test_complete_metadata_and_array_order(self):
        a = b'{"nodes":[{"id":1},{"id":2}],"edges":[],"CellSize":30}'
        self.assertTrue(self.check(a, a))
        self.assertFalse(self.check(a, b'{"nodes":[{"id":2},{"id":1}],"edges":[],"CellSize":30}'))
        self.assertFalse(self.check(a, b'{"nodes":[{"id":1},{"id":2}],"edges":[]}'))
        self.assertFalse(self.check(a, b'{"nodes":[{"id":1},{"id":2}],"edges":[],"CellSize":31}'))

    def test_duplicate_identity_and_signed_zero(self):
        self.assertFalse(self.check(b'{"nodes":[{"id":1},{"id":1}],"edges":[]}', b'{"nodes":[{"id":1}],"edges":[]}'))
        self.assertFalse(self.check(b'{"nodes":[{"x":-0}],"edges":[]}', b'{"nodes":[{"x":0}],"edges":[]}'))

    def test_invalid_or_missing_inventory(self):
        for a in (b"", b"{}", b"null", b'{"nodes":[],"edges":[],"x":NaN}', b'{"nodes":[],"edges":[],"x":Infinity}'):
            self.assertFalse(self.check(a, a))
        self.assertTrue(self.check(b'{"nodes":[],"edges":[]}', b'{"nodes":[],"edges":[]}'))

    def test_timeouts_crashes_and_exact_errors(self):
        a = b'{"nodes":[],"edges":[]}'
        self.assertFalse(self.check(a, a, source_timeout=True))
        self.assertFalse(self.check(a, a, native_timeout=True))
        self.assertTrue(module.compare(1,b"",b"overflow\n",1,b"",b"overflow\n")["passed"])
        self.assertFalse(module.compare(1,b"",b"overflow\n",0,a,b"")["passed"])
        self.assertFalse(module.compare(-9,b"",b"",-9,b"",b"")["passed"])
        self.assertFalse(module.compare(1,b"",b"overflow\n",1,b"",b"other\n")["passed"])

if __name__ == "__main__":
    unittest.main()
