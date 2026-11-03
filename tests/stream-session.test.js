import { test } from 'node:test';
import assert from 'node:assert/strict';

import {
  LOST_RUN_RETRY_LIMIT,
  STREAM_LADDER,
  createStreamSession,
  nextStreamMode,
} from '../frontend/src/lib/video/streamSession.js';

function fakeTimers() {
  const pending = new Map();
  const cleared = new Set();
  let nextId = 1;
  return {
    cleared,
    pending,
    setTimeout(callback, delay) {
      const id = nextId++;
      pending.set(id, { callback, delay });
      return id;
    },
    clearTimeout(id) {
      cleared.add(id);
      pending.delete(id);
    },
    fire(id) {
      const entry = pending.get(id);
      assert.ok(entry, `timer ${id} is not pending`);
      pending.delete(id);
      entry.callback();
    },
    fireAll() {
      for (const id of [...pending.keys()]) this.fire(id);
    },
  };
}

test('the ladder ends at transcode', () => {
  assert.deepEqual(STREAM_LADDER, ['remux', 'audio', 'transcode']);
  assert.equal(nextStreamMode('remux'), 'audio');
  assert.equal(nextStreamMode('audio'), 'transcode');
  assert.equal(nextStreamMode('transcode'), null);
  assert.equal(nextStreamMode('unknown'), null);
});

test('a newer run start replaces the intent and disarms a pending retry', () => {
  const timers = fakeTimers();
  const session = createStreamSession({ timers });

  session.beginRun(12);
  assert.equal(session.intentOffset, 12);
  assert.equal(session.runStartOffset, 12);

  let retried = 0;
  session.scheduleRetry(() => retried++, 1000);
  assert.equal(session.retryPending, true);

  session.beginRun(4);
  assert.equal(session.intentOffset, 4);
  assert.equal(session.runStartOffset, 4);
  assert.equal(session.retryPending, false);
  timers.fireAll();
  assert.equal(retried, 0, 'the superseded retry must not fire');
});

test('a user seek records its position and disarms the retry', () => {
  const timers = fakeTimers();
  const session = createStreamSession({ timers });
  session.beginRun(30);

  let retried = 0;
  session.scheduleRetry(() => retried++, 500);
  session.rememberIntent(31);

  assert.equal(session.intentOffset, 31);
  assert.equal(session.runStartOffset, 30, 'a seek is not a run start');
  assert.equal(session.retryPending, false);
  timers.fireAll();
  assert.equal(retried, 0);
});

test('scheduling a second retry replaces the first', () => {
  const timers = fakeTimers();
  const session = createStreamSession({ timers });
  const fired = [];

  session.scheduleRetry(() => fired.push('first'), 100);
  const firstId = [...timers.pending.keys()][0];
  session.scheduleRetry(() => fired.push('second'), 100);

  assert.equal(timers.cleared.has(firstId), true);
  assert.equal(timers.pending.size, 1);
  timers.fireAll();

  assert.deepEqual(fired, ['second']);
  assert.equal(session.retryPending, false);
});

test('the lost-run budget is per rung and bounded', () => {
  const session = createStreamSession();
  assert.equal(session.claimLostRunRetry('audio'), true);
  assert.equal(session.claimLostRunRetry('audio'), true);
  assert.equal(session.claimLostRunRetry('audio'), false);

  // A different rung starts its own budget.
  assert.equal(session.claimLostRunRetry('transcode'), true);
  assert.equal(session.claimLostRunRetry('transcode'), true);
  assert.equal(session.claimLostRunRetry('transcode'), false);
  assert.equal(LOST_RUN_RETRY_LIMIT, 2);

  session.forgetLostRuns();
  assert.equal(session.claimLostRunRetry('transcode'), true);
});

test('resume takes the larger of the element position and the run start', () => {
  let position = 0;
  const session = createStreamSession({ getPosition: () => position });

  session.beginRun(40);
  position = 0;
  assert.equal(
    session.resumeOffset({ duration: 100 }),
    40,
    'a run that never attached keeps its own start'
  );

  position = 55;
  assert.equal(
    session.resumeOffset({ duration: 100 }),
    55,
    'a parked run keeps the position the viewer reached'
  );

  position = 100;
  assert.equal(session.resumeOffset({ duration: 100 }), 0, 'the end restarts at 0');

  session.beginRun(0);
  position = 0;
  assert.equal(session.resumeOffset({ duration: 0 }), 0);
});
