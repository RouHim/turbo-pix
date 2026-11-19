import { test } from 'node:test';
import assert from 'node:assert/strict';

import {
  PANEL_SETTLE_RATIO,
  PANEL_SETTLE_VELOCITY,
  drawerOpenProgress,
  sheetOpenProgress,
  shouldSettleOpen,
} from '../frontend/src/lib/gestures/panelSettle.js';

test('constants match the interaction spec', () => {
  assert.equal(PANEL_SETTLE_RATIO, 0.3);
  assert.equal(PANEL_SETTLE_VELOCITY, 0.3);
});

test('GIVEN a closed drawer WHEN it is dragged 30% of its width THEN it counts as open', () => {
  const width = 280;
  const translateX = -width + width * PANEL_SETTLE_RATIO;
  assert.equal(drawerOpenProgress(translateX, width), 0.3);
  assert.equal(shouldSettleOpen({ startedOpen: false, progress: 0.3, velocity: 0 }), true);
});

test('GIVEN a closed drawer WHEN it is dragged less than 30% THEN it snaps back', () => {
  assert.equal(shouldSettleOpen({ startedOpen: false, progress: 0.29, velocity: 0 }), false);
});

test('GIVEN a short closing drag on an open drawer WHEN it is flicked down THEN it closes', () => {
  assert.equal(shouldSettleOpen({ startedOpen: true, progress: 0.9, velocity: -0.4 }), false);
});

test('GIVEN an open drawer WHEN it is dragged less than 30% down and released THEN it stays open', () => {
  assert.equal(shouldSettleOpen({ startedOpen: true, progress: 0.8, velocity: 0 }), true);
});

test('GIVEN an open drawer WHEN it is dragged more than 30% down and released THEN it closes', () => {
  assert.equal(shouldSettleOpen({ startedOpen: true, progress: 0.6, velocity: 0 }), false);
});

test('GIVEN a nearly closed drawer WHEN it is flicked open THEN velocity wins over distance', () => {
  assert.equal(shouldSettleOpen({ startedOpen: false, progress: 0.1, velocity: 0.35 }), true);
});

test('GIVEN an open sheet WHEN it is dragged 30% down THEN it counts as closing', () => {
  const height = 400;
  const translateY = height * PANEL_SETTLE_RATIO;
  assert.equal(sheetOpenProgress(translateY, height), 0.7);
  assert.equal(shouldSettleOpen({ startedOpen: true, progress: 0.7, velocity: 0 }), false);
});

test('GIVEN a closed sheet WHEN it is dragged 30% up THEN it counts as open', () => {
  const height = 400;
  const translateY = height - height * PANEL_SETTLE_RATIO;
  assert.equal(sheetOpenProgress(translateY, height), 0.3);
  assert.equal(shouldSettleOpen({ startedOpen: false, progress: 0.3, velocity: 0 }), true);
});

test('GIVEN a short upward flick on a closed sheet THEN velocity opens it', () => {
  assert.equal(shouldSettleOpen({ startedOpen: false, progress: 0.1, velocity: 0.35 }), true);
});
