"""Contracts for exact public-output parity."""
import unittest
from exact_json import decode_output, difference


class ExactJsonTests(unittest.TestCase):
    def test_negative_zero_is_preserved(self):
        self.assertIsNotNone(difference(decode_output('-0'), decode_output('0')))
        self.assertIsNone(difference(decode_output('-0'), decode_output('-0.0')))

    def test_encoding_only_numeric_differences_are_allowed(self):
        self.assertIsNone(difference(decode_output('{"x":42}'), decode_output('{"x":42.0}')))

    def test_large_identity_is_not_rounded_away(self):
        self.assertIsNotNone(difference(9007199254740993, 9007199254740992.0))
        self.assertIsNotNone(difference(9007199254740992.0, 9007199254740993))
        self.assertIsNotNone(difference(9007199254740993, 9007199254740992))

    def test_fields_types_and_array_order_are_semantic(self):
        for left, right in [({'x': 1}, {}), (True, 1), ([1, 2], [2, 1])]:
            with self.subTest(left=left, right=right):
                self.assertIsNotNone(difference(left, right))


if __name__ == '__main__':
    unittest.main()
