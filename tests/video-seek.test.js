import { test } from 'node:test';
import assert from 'node:assert/strict';

import {
  VIDEO_SEEK_CENTER_RATIO,
  VIDEO_SEEK_STEP_SECONDS,
  seekDirectionForX,
  seekTarget,
} from '../frontend/src/lib/viewer/videoSeek.js';

test('constants match the agreed video gesture', () => {
  assert.equal(VIDEO_SEEK_STEP_SECONDS, 10);
  assert.equal(VIDEO_SEEK_CENTER_RATIO, 1 / 3);
});

test('GIVEN a video at 10 s WHEN seeking forward THEN the target is 20 s', () => {
  assert.equal(seekTarget(10, 100, 'forward'), 20);
});

test('GIVEN a video at 4 s WHEN seeking back THEN the target clamps at zero', () => {
  assert.equal(seekTarget(4, 100, 'back'), 0);
});

test('GIVEN a video near its end WHEN seeking forward THEN the target clamps at duration', () => {
  assert.equal(seekTarget(95, 100, 'forward'), 100);
});

test('GIVEN an unknown duration WHEN seeking forward THEN only the lower bound applies', () => {
  assert.equal(seekTarget(5, Infinity, 'forward'), 15);
  assert.equal(seekTarget(5, Number.NaN, 'forward'), 15);
});

test('GIVEN a seek without a direction WHEN computing the target THEN the position is unchanged', () => {
  assert.equal(seekTarget(42, 100, null), 42);
});

test('GIVEN the left third of the video WHEN seeking THEN direction is back', () => {
  assert.equal(seekDirectionForX(20, 0, 300), 'back');
});

test('GIVEN the right third of the video WHEN seeking THEN direction is forward', () => {
  assert.equal(seekDirectionForX(280, 0, 300), 'forward');
});

test('GIVEN the middle third of the video WHEN seeking THEN the gesture is a no-op', () => {
  assert.equal(seekDirectionForX(150, 0, 300), null);
});

test('GIVEN a zero-width surface WHEN seeking THEN no direction is claimed', () => {
  assert.equal(seekDirectionForX(10, 0, 0), null);
});
