import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  MAX_MERGE_PERSONS,
  canMergeSelection,
  mergeNamePrefill,
} from '../frontend/src/lib/peopleMerge.js';

const person = (id, name) => ({ id, name, hidden: false, face_count: 3, cover_face_id: null });

test('merge keeps the keeper name when the keeper is named', () => {
  const selected = [person(1, 'Anna'), person(2, 'Oma')];
  assert.equal(mergeNamePrefill(selected, 1), 'Anna');
  assert.equal(mergeNamePrefill(selected, 2), 'Oma');
});

test('merge borrows the only named source when the keeper is unnamed', () => {
  const selected = [person(1, null), person(2, 'Oma')];
  assert.equal(mergeNamePrefill(selected, 1), 'Oma');
});

test('merge leaves the name empty when several sources are named differently', () => {
  const selected = [person(1, null), person(2, 'Oma'), person(3, 'Papa')];
  assert.equal(mergeNamePrefill(selected, 1), '');
});

test('merge leaves the name empty when nobody is named', () => {
  const selected = [person(1, null), person(2, null), person(3, '   ')];
  assert.equal(mergeNamePrefill(selected, 1), '');
});

test('merge trims the borrowed name', () => {
  const selected = [person(1, ''), person(2, '  Oma  ')];
  assert.equal(mergeNamePrefill(selected, 1), 'Oma');
});

test('merge requires two persons and allows at most the configured limit', () => {
  assert.equal(MAX_MERGE_PERSONS, 21);
  assert.equal(canMergeSelection(1), false);
  assert.equal(canMergeSelection(2), true);
  assert.equal(canMergeSelection(21), true);
  assert.equal(canMergeSelection(22), false);
  assert.equal(canMergeSelection(0), false);
});
