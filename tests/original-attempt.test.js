import { test } from 'node:test';
import assert from 'node:assert/strict';

import {
  ORIGINAL_ATTEMPT_GRACE_MS,
  startOriginalAttempt,
  createOriginalFailureRegistry,
} from '../frontend/src/lib/video/originalAttempt.js';

/**
 * Every event the module is allowed to listen to, so a verdict can be checked
 * for having detached all of them.
 */
const OBSERVED_EVENTS = [
  'loadeddata',
  'playing',
  'error',
  'loadstart',
  'loadedmetadata',
  'durationchange',
  'progress',
  'suspend',
  'canplay',
  'seeked',
  'timeupdate',
  'stalled',
];

/**
 * A media element stand-in with Chromium's listener contract: the module only
 * attaches and detaches, and the test drives events through `dispatch`.
 */
function fakeVideo() {
  const listeners = new Map();
  return {
    src: '',
    readyState: 0,
    videoWidth: 0,
    error: null,
    addEventListener(type, handler) {
      if (!listeners.has(type)) listeners.set(type, new Set());
      listeners.get(type).add(handler);
    },
    removeEventListener(type, handler) {
      listeners.get(type)?.delete(handler);
    },
    dispatch(type) {
      for (const handler of [...(listeners.get(type) ?? [])]) handler();
    },
    listenerCount(type) {
      return listeners.get(type)?.size ?? 0;
    },
  };
}

/**
 * Observes settlement without consuming the verdict: `value` stays `null`
 * until the attempt resolves, which is what "still pending" asserts against.
 */
function trackResult(attempt) {
  const tracked = { value: null };
  attempt.promise.then((value) => {
    tracked.value = value;
  });
  return tracked;
}

/** Run the promise reactions the attempt may have queued. */
const flush = () => new Promise((resolve) => setImmediate(resolve));

test('a loaded frame settles the attempt as playable', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout', 'setInterval'] });
  const video = fakeVideo();
  const attempt = startOriginalAttempt(video, { graceMs: ORIGINAL_ATTEMPT_GRACE_MS });
  const result = trackResult(attempt);

  video.dispatch('loadeddata');
  await flush();

  assert.deepEqual(result.value, { verdict: 'playable', reason: null, frameObserved: true });
  for (const type of OBSERVED_EVENTS) {
    assert.equal(video.listenerCount(type), 0, `a ${type} listener survived the verdict`);
  }
});

test('playing settles the attempt as playable', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout', 'setInterval'] });
  const video = fakeVideo();
  const attempt = startOriginalAttempt(video, { graceMs: ORIGINAL_ATTEMPT_GRACE_MS });
  const result = trackResult(attempt);

  video.dispatch('playing');
  await flush();

  assert.deepEqual(result.value, { verdict: 'playable', reason: null, frameObserved: true });
  for (const type of OBSERVED_EVENTS) {
    assert.equal(video.listenerCount(type), 0, `a ${type} listener survived the verdict`);
  }
});

test('an element error settles the attempt as unplayable', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout', 'setInterval'] });
  const video = fakeVideo();
  video.error = { code: 4 };
  const attempt = startOriginalAttempt(video, { graceMs: ORIGINAL_ATTEMPT_GRACE_MS });
  const result = trackResult(attempt);

  video.dispatch('error');
  await flush();

  assert.deepEqual(result.value, { verdict: 'unplayable', reason: 'error', frameObserved: false });
  for (const type of OBSERVED_EVENTS) {
    assert.equal(video.listenerCount(type), 0, `a ${type} listener survived the verdict`);
  }
});

test('an element that already holds a frame is playable at arming', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout', 'setInterval'] });
  const video = fakeVideo();
  video.readyState = 4;
  video.videoWidth = 320;
  const attempt = startOriginalAttempt(video, { graceMs: ORIGINAL_ATTEMPT_GRACE_MS });
  const result = trackResult(attempt);

  await flush();

  assert.deepEqual(result.value, { verdict: 'playable', reason: null, frameObserved: true });
  for (const type of OBSERVED_EVENTS) {
    assert.equal(video.listenerCount(type), 0, `a ${type} listener was left on a ready element`);
  }
});

test('a stall that produces no frame times out', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout', 'setInterval'] });
  const graceMs = ORIGINAL_ATTEMPT_GRACE_MS;
  const video = fakeVideo();
  const attempt = startOriginalAttempt(video, { graceMs });
  const result = trackResult(attempt);

  video.dispatch('stalled');
  t.mock.timers.tick(graceMs - 1);
  await flush();
  assert.equal(result.value, null, 'the attempt expired before its grace window');

  t.mock.timers.tick(1);
  await flush();
  assert.deepEqual(result.value, {
    verdict: 'unplayable',
    reason: 'timeout',
    frameObserved: false,
  });
});

test('progress after a stall keeps the attempt alive', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout', 'setInterval'] });
  const graceMs = ORIGINAL_ATTEMPT_GRACE_MS;
  const video = fakeVideo();
  const attempt = startOriginalAttempt(video, { graceMs });
  const result = trackResult(attempt);

  video.dispatch('stalled');
  t.mock.timers.tick(4000);
  video.dispatch('progress');
  t.mock.timers.tick(4000);
  await flush();
  assert.equal(result.value, null, 'a served delivery expired the attempt');

  // Past the deadline the `progress` armed: only the cleared stall lets the
  // watchdog keep re-arming the window, so a resumed delivery cannot expire.
  t.mock.timers.tick(graceMs);
  await flush();
  assert.equal(result.value, null, 'a resumed delivery expired the attempt');

  video.dispatch('loadeddata');
  await flush();
  assert.deepEqual(result.value, { verdict: 'playable', reason: null, frameObserved: true });
});

test('keeps the attempt open while the delivery is served', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout', 'setInterval'] });
  const graceMs = ORIGINAL_ATTEMPT_GRACE_MS;
  const video = fakeVideo();
  const attempt = startOriginalAttempt(video, { graceMs });
  const result = trackResult(attempt);

  t.mock.timers.tick(graceMs * 10);
  await flush();

  assert.equal(result.value, null, 'a delivery that never stalls must not expire');
  video.dispatch('loadeddata');
  await flush();
  assert.deepEqual(result.value, { verdict: 'playable', reason: null, frameObserved: true });
});

test('a frame with undecodable audio is not playable', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout', 'setInterval'] });
  const video = fakeVideo();
  const attempt = startOriginalAttempt(video, {
    graceMs: ORIGINAL_ATTEMPT_GRACE_MS,
    audioPlayable: () => false,
  });
  const result = trackResult(attempt);

  video.dispatch('loadeddata');
  await flush();

  assert.deepEqual(result.value, { verdict: 'unplayable', reason: 'audio', frameObserved: true });
});

test('seeking and pausing while the attempt is pending settle nothing', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout', 'setInterval'] });
  const graceMs = ORIGINAL_ATTEMPT_GRACE_MS;
  const video = fakeVideo();
  const attempt = startOriginalAttempt(video, { graceMs });
  const result = trackResult(attempt);

  video.dispatch('pause');
  video.dispatch('seeking');
  video.dispatch('seeked');
  video.dispatch('waiting');
  t.mock.timers.tick(graceMs * 2);
  await flush();
  assert.equal(result.value, null, 'a pause or seek decided the attempt');

  video.dispatch('loadeddata');
  await flush();
  assert.deepEqual(result.value, { verdict: 'playable', reason: null, frameObserved: true });
});

test('the first verdict wins', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout', 'setInterval'] });
  const video = fakeVideo();
  const attempt = startOriginalAttempt(video, { graceMs: ORIGINAL_ATTEMPT_GRACE_MS });
  const result = trackResult(attempt);

  video.dispatch('error');
  video.dispatch('loadeddata');
  await flush();
  assert.deepEqual(result.value, { verdict: 'unplayable', reason: 'error', frameObserved: false });

  const laterVideo = fakeVideo();
  const laterAttempt = startOriginalAttempt(laterVideo, { graceMs: ORIGINAL_ATTEMPT_GRACE_MS });
  const laterResult = trackResult(laterAttempt);
  laterVideo.dispatch('loadeddata');
  laterVideo.dispatch('error');
  await flush();
  assert.deepEqual(laterResult.value, { verdict: 'playable', reason: null, frameObserved: true });
});

test('cancel() settles the attempt as cancelled and detaches', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout', 'setInterval'] });
  const graceMs = ORIGINAL_ATTEMPT_GRACE_MS;
  const video = fakeVideo();
  const attempt = startOriginalAttempt(video, { graceMs });
  const result = trackResult(attempt);

  attempt.cancel();
  attempt.cancel();
  await flush();
  assert.deepEqual(result.value, {
    verdict: 'cancelled',
    reason: 'cancelled',
    frameObserved: false,
  });

  video.dispatch('error');
  t.mock.timers.tick(graceMs * 2);
  await flush();
  assert.deepEqual(result.value, {
    verdict: 'cancelled',
    reason: 'cancelled',
    frameObserved: false,
  });
  for (const type of OBSERVED_EVENTS) {
    assert.equal(video.listenerCount(type), 0, `a ${type} listener survived cancel()`);
  }
});

test('the failure registry is per hash and starts empty', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout', 'setInterval'] });
  const registry = createOriginalFailureRegistry();

  assert.equal(registry.has('a'), false);
  assert.equal(registry.has('b'), false);
  registry.record('a');
  assert.equal(registry.has('a'), true);
  assert.equal(registry.has('b'), false);
  registry.clear('a');
  assert.equal(registry.has('a'), false);

  registry.record('a');
  const fresh = createOriginalFailureRegistry();
  assert.equal(fresh.has('a'), false, 'a second registry shares state with the first');
  assert.equal(registry.has('a'), true);
});
