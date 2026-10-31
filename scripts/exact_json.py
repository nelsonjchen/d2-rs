"""Exact semantic JSON comparison for D2 parity gates."""
import json
import struct


def decode_output(data):
    # Go emits IEEE-754 negative zero as -0. Python's default integer parser
    # erases its sign; preserve it without converting large identity integers.
    return json.loads(data, parse_int=lambda s: -0.0 if s == "-0" else int(s))



def difference(left, right, path="$"):
    if type(left) is int and type(right) is int:
        return None if left == right else {"path": path, "go": left, "rust": right}
    if isinstance(left, (int, float)) and not isinstance(left, bool) and isinstance(right, (int, float)) and not isinstance(right, bool):
        # Python's mixed numeric equality compares the integer exactly.
        # Converting both operands first could erase a large EntityID bit.
        if left != right or struct.pack(">d", float(left)) != struct.pack(">d", float(right)):
            return {"path": path, "go": left, "rust": right}
        return None
    if type(left) is not type(right):
        return {"path": path, "go_type": type(left).__name__, "rust_type": type(right).__name__}
    if isinstance(left, dict):
        if left.keys() != right.keys():
            return {"path": path, "missing_rust_keys": sorted(left.keys() - right.keys()), "extra_rust_keys": sorted(right.keys() - left.keys())}
        for key in sorted(left):
            mismatch = difference(left[key], right[key], path + "." + key)
            if mismatch:
                return mismatch
    elif isinstance(left, list):
        if len(left) != len(right):
            return {"path": path, "go_length": len(left), "rust_length": len(right)}
        for index, (a, b) in enumerate(zip(left, right)):
            mismatch = difference(a, b, f"{path}[{index}]")
            if mismatch:
                return mismatch
    elif left != right:
        return {"path": path, "go": left, "rust": right}
    return None
