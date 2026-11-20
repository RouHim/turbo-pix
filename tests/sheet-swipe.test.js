import { test } from 'node:test';
import assert from 'node:assert/strict';

import {
  SHEET_HEADER_SWIPE_THRESHOLD_PX,
  sheetHeaderSwipeDirection,
} from '../frontend/src/lib/viewer/sheetSwipe.js';

test('the sheet header needs 24 px of horizontal travel', () => {
  assert.equal(SHEET_HEADER_SWIPE_THRESHOLD_PX, 24);
});

test('GIVEN a leftward header swipe over the threshold THEN the next photo is requested', () => {
  assert.equal(sheetHeaderSwipeDirection(-30, 2, 24), 'next');
});

test('GIVEN a rightward header swipe over the threshold THEN the previous photo is requested', () => {
  assert.equal(sheetHeaderSwipeDirection(30, -2, 24), 'previous');
});

test('GIVEN a vertical drag THEN the horizontal gesture is not claimed', () => {
  assert.equal(sheetHeaderSwipeDirection(20, 40, 24), null);
});

test('GIVEN a movement below the threshold THEN the gesture waits for more travel', () => {
  assert.equal(sheetHeaderSwipeDirection(-20, 0, 24), null);
});

test('GIVEN exactly the threshold THEN the gesture is claimed', () => {
  assert.equal(sheetHeaderSwipeDirection(-24, 0, 24), 'next');
});
