import {test} from 'node:test';
import assert from 'node:assert/strict';
import {firstDifference, atPath, summarize, normalizeSvg} from './wasm-differential-scoreboard.mjs';

test('difference paths preserve dotted, empty and numeric keys', () => {
  const a = {'a.b': {'': [1, 2]}}, b = {'a.b': {'': [1, 3]}};
  const path = firstDifference(a, b);
  assert.deepEqual(path, ['a.b', '', '1']);
  assert.equal(atPath(a, path), 2);
  assert.deepEqual(firstDifference({}, {'a.b': null}), ['a.b']);
});
test('all diff values are bounded, including objects and arrays', () => {
  for (const value of ['x'.repeat(10000), {large: 'x'.repeat(10000)}, Array(10000).fill(0), undefined]) {
    assert.ok(summarize(value).length <= 600);
  }
});
test('normalization preserves XML declaration and trailing newline', () => {
  assert.equal(normalizeSvg('<?xml?><svg data-d2-version="v0.9.0"/>\n'), '<?xml?><svg data-d2-version=""/>\n');
});
