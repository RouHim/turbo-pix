import { test } from 'node:test';
import assert from 'node:assert/strict';

import { rangeKeys, withRange } from '../frontend/src/lib/selectionRange.js';

const KEYS = ['a', 'b', 'c', 'd', 'e'];

test('GIVEN a forward drag WHEN computing the range THEN every card between is included', () => {
  assert.deepEqual(rangeKeys(KEYS, 'b', 'd'), ['b', 'c', 'd']);
});

test('GIVEN a backward drag WHEN computing the range THEN the same cards are included', () => {
  assert.deepEqual(rangeKeys(KEYS, 'd', 'b'), ['b', 'c', 'd']);
});

test('GIVEN the anchor and target are the same card THEN only that card is included', () => {
  assert.deepEqual(rangeKeys(KEYS, 'c', 'c'), ['c']);
});

test('GIVEN a card outside the surface WHEN computing the range THEN no range is claimed', () => {
  assert.equal(rangeKeys(KEYS, 'b', 'missing'), null);
  assert.equal(rangeKeys(KEYS, 'missing', 'b'), null);
});

test('GIVEN an empty surface WHEN computing the range THEN no range is claimed', () => {
  assert.equal(rangeKeys([], 'a', 'b'), null);
});

test('GIVEN a base selection WHEN applying a range THEN the base is preserved and not mutated', () => {
  const base = { a: true };
  const next = withRange(base, KEYS, 'b', 'd');
  assert.deepEqual(next, { a: true, b: true, c: true, d: true });
  assert.deepEqual(base, { a: true });
});

test('GIVEN a range that moves back WHEN applying it THEN the earlier tail is removed', () => {
  const base = {};
  const forward = withRange(base, KEYS, 'b', 'd');
  const back = withRange(base, KEYS, 'b', 'c');
  assert.deepEqual(forward, { b: true, c: true, d: true });
  assert.deepEqual(back, { b: true, c: true });
});

test('GIVEN an unknown target WHEN applying a range THEN the base is returned unchanged', () => {
  const base = { a: true };
  assert.deepEqual(withRange(base, KEYS, 'b', 'missing'), base);
});
