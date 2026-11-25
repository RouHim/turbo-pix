import test from 'node:test';
import assert from 'node:assert/strict';

import { createTimelineLoader } from '../frontend/src/lib/timelineCache.js';

test('concurrent loads share one request and the result stays cached', async () => {
  let calls = 0;
  let resolveRequest;
  const request = () => {
    calls += 1;
    return new Promise((resolve) => {
      resolveRequest = resolve;
    });
  };
  const loadTimeline = createTimelineLoader(request);

  const first = loadTimeline();
  const second = loadTimeline();
  assert.equal(calls, 1, 'a second caller joins the in-flight request');

  const payload = { density: [{ year: 2024, month: 1, count: 2 }] };
  resolveRequest(payload);
  assert.equal(await first, payload);
  assert.equal(await second, payload);

  assert.equal(await loadTimeline(), payload, 'the finished value is cached');
  assert.equal(calls, 1);
});

test('a forced reload does not join the initial read and wins the cache', async () => {
  const resolvers = [];
  let calls = 0;
  const request = () => {
    calls += 1;
    return new Promise((resolve) => resolvers.push(resolve));
  };
  const loadTimeline = createTimelineLoader(request);

  const stale = loadTimeline();
  const fresh = loadTimeline(true);
  assert.equal(calls, 2, 'the forced reload starts its own request');

  const staleData = { density: [{ year: 2020, month: 1, count: 1 }] };
  const freshData = { density: [{ year: 2025, month: 2, count: 1 }] };
  resolvers[1](freshData);
  assert.equal(await fresh, freshData);
  resolvers[0](staleData);
  assert.equal(await stale, staleData);

  assert.equal(await loadTimeline(), freshData, 'the stale answer cannot win the cache');
  assert.equal(calls, 2);
});

test('forced reloads in flight share one request', async () => {
  let calls = 0;
  let resolveRequest;
  const request = () => {
    calls += 1;
    return new Promise((resolve) => {
      resolveRequest = resolve;
    });
  };
  const loadTimeline = createTimelineLoader(request);

  const first = loadTimeline();
  resolveRequest({ density: [] });
  await first;

  const reloadA = loadTimeline(true);
  const reloadB = loadTimeline(true);
  assert.equal(calls, 2, 'two components handling one reload cost one request');

  const payload = { density: [{ year: 2025, month: 3, count: 4 }] };
  resolveRequest(payload);
  assert.equal(await reloadA, payload);
  assert.equal(await reloadB, payload);
});

test('invalidate drops the cache and a late answer cannot refill it', async () => {
  let calls = 0;
  const resolvers = [];
  const request = () => {
    calls += 1;
    return new Promise((resolve) => resolvers.push(resolve));
  };
  const loadTimeline = createTimelineLoader(request);

  const superseded = loadTimeline();
  loadTimeline.invalidate();
  const current = loadTimeline();
  assert.equal(calls, 2, 'the invalidation forces a fresh read');

  const staleData = { density: [{ year: 2020, month: 1, count: 1 }] };
  const currentData = { density: [{ year: 2026, month: 2, count: 3 }] };
  resolvers[0](staleData);
  assert.equal(await superseded, staleData);
  resolvers[1](currentData);
  assert.equal(await current, currentData);

  assert.equal(await loadTimeline(), currentData, 'the stale answer cannot refill the cache');
  assert.equal(calls, 2);
});

test('a failed request is not cached and the next call retries', async () => {
  let calls = 0;
  const request = async () => {
    calls += 1;
    if (calls === 1) throw new Error('offline');
    return { density: [] };
  };
  const loadTimeline = createTimelineLoader(request);

  await assert.rejects(loadTimeline(), /offline/);
  assert.deepEqual(await loadTimeline(), { density: [] });
  assert.equal(calls, 2);
});
