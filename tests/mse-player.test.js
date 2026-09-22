import { test, afterEach } from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

// The module under test is overridable so the same cases can be run against a
// different revision, e.g.
//   MSE_PLAYER_MODULE=/tmp/mse-prefix.mjs node --test tests/mse-player.test.js
const modulePath = process.env.MSE_PLAYER_MODULE
  ? path.resolve(process.env.MSE_PLAYER_MODULE)
  : fileURLToPath(new URL('../frontend/src/lib/video/msePlayer.js', import.meta.url));
const { createStreamPlayer, StreamHttpError } = await import(pathToFileURL(modulePath).href);

const realFetch = globalThis.fetch;
const realMediaSource = globalThis.MediaSource;
const realCreateObjectURL = URL.createObjectURL;
const realRevokeObjectURL = URL.revokeObjectURL;

afterEach(() => {
  createdSources.length = 0;
  globalThis.fetch = realFetch;
  globalThis.MediaSource = realMediaSource;
  URL.createObjectURL = realCreateObjectURL;
  URL.revokeObjectURL = realRevokeObjectURL;
});

/** Flush the microtask/promise turns one stream run needs to reach playback. */
async function settle(turns = 5) {
  for (let i = 0; i < turns; i += 1) {
    await new Promise((resolve) => setImmediate(resolve));
  }
}

/**
 * Settle until `value` stops changing, bounded. A pump that is waiting for the
 * viewer has no further progress to make, while one that is not keeps going —
 * so "the delivery has come to rest" is the observation that tells a bounded
 * pump from an unbounded one, and it cannot be expressed as a fixed number of
 * turns.
 */
async function settleUntilStable(value, quietTurns = 5, turns = 500) {
  let last = value();
  let quiet = 0;
  for (let i = 0; i < turns && quiet < quietTurns; i += 1) {
    await settle(1);
    const now = value();
    quiet = now === last ? quiet + 1 : 0;
    last = now;
  }
}

/**
 * SourceBuffer stand-in with Chromium's `updating` contract:
 *  - `timestampOffset` and `appendBuffer` throw `InvalidStateError` while an
 *    update is in flight (Chromium: "The timestamp offset may not be set while
 *    the SourceBuffer is updating.");
 *  - an append completes asynchronously, fires `updateend`, and the element's
 *    own seek/play can start a follow-up internal update in that same task —
 *    the window in which the app's own `updating` check is already stale.
 */
class FakeSourceBuffer {
  constructor(mime, mediaSource = null) {
    this.mime = mime;
    this.mediaSource = mediaSource;
    this.updating = false;
    this.appended = [];
    this.removals = [];
    this.offsetAssignments = [];
    this.listeners = new Map();
    this._offset = 0;
    // Left edge of the buffered range on the `timestampOffset` timeline: null
    // while nothing has been evicted, because the range starts at the run's
    // offset. A leading eviction moves it up, exactly as Chromium reports the
    // trim — keep it apart from `_offset`, which stays the run's
    // `timestampOffset` (the player reads it back to decide whether the run's
    // own start lies before the end of the source).
    this._rangeStart = null;
    // Right edge of the buffered range on the `timestampOffset` timeline, or
    // null while nothing is buffered: the run's first chunk is its
    // initialization segment, which carries no coded frames, so appending it
    // alone leaves the buffer empty (exactly as Chromium reports it).
    this._end = null;
    this._followUpStarted = false;
  }

  /**
   * The buffered media, on the timeline playback actually lives on: ffmpeg
   * rebases every run to timestamp 0 and `timestampOffset` places its fragments
   * on the real timeline. A range that always started at 0 would answer
   * `isBuffered()` about a different timeline than the one the element seeks
   * on — and would claim media where a run that has only delivered its init
   * segment has none.
   */
  get buffered() {
    const start = this._rangeStart ?? this._offset;
    // A removal that takes the whole range away (the duration clamp dropping a
    // phantom tail, an eviction that leaves nothing) leaves nothing buffered,
    // not an inverted range.
    if (this._end === null || this._end <= start) return { length: 0 };
    return { length: 1, start: () => start, end: () => this._end };
  }

  get timestampOffset() {
    return this._offset;
  }

  set timestampOffset(value) {
    if (this.updating) {
      throw new DOMException(
        'The timestamp offset may not be set while the SourceBuffer is updating.',
        'InvalidStateError'
      );
    }
    this._offset = value;
    this.offsetAssignments.push(value);
  }

  appendBuffer(chunk) {
    if (this.updating) {
      throw new DOMException(
        'The SourceBuffer is updating and cannot be appended to.',
        'InvalidStateError'
      );
    }
    this.appended.push(chunk);
    // Everything after the initialization segment is a one-second fragment,
    // placed at the run's offset.
    if (this.appended.length > 1) {
      this._end = (this._end ?? this._offset) + 1;
      // Chromium grows the media source's duration to the end of the media it
      // is handed: appending a fragment that runs past the declared duration is
      // how a 20.02 s source starts reporting 21 s.
      if (typeof this.mediaSource?.duration === 'number') {
        this.mediaSource.duration = Math.max(this.mediaSource.duration, this._end);
      }
    }
    this.updating = true;
    setImmediate(() => this._completeUpdate());
  }

  /**
   * Chromium's removal contract: dropping a range takes the buffer offline and
   * refuses to start while an update is already running (which is also why the
   * duration cannot shrink below the buffered end until the tail is gone).
   *
   * The media the range covered is gone afterwards, on both sides of the
   * removal: an eviction trims played-out media off the front (the range then
   * starts where the removal ended) and the duration clamp drops a phantom tail
   * (the range then ends where the removal began). Both are what the tests
   * observe, so the stand-in models them rather than only recording the call.
   */
  remove(start, end) {
    if (this.updating) {
      throw new DOMException(
        'The SourceBuffer is updating and cannot be removed from.',
        'InvalidStateError'
      );
    }
    this.removals.push([start, end]);
    const rangeStart = this._rangeStart ?? this._offset;
    if (this._end !== null && end > rangeStart && start < this._end) {
      if (start <= rangeStart) {
        this._rangeStart = end;
        if (this._end <= end) this._end = null;
      } else {
        this._end = start;
      }
    }
    this.updating = true;
    setImmediate(() => this._completeUpdate());
  }

  addEventListener(type, handler) {
    if (!this.listeners.has(type)) this.listeners.set(type, new Set());
    this.listeners.get(type).add(handler);
  }

  removeEventListener(type, handler) {
    this.listeners.get(type)?.delete(handler);
  }

  fire(type) {
    for (const handler of [...(this.listeners.get(type) ?? [])]) handler();
  }

  /** Append finished; fire `updateend` and start one follow-up update. */
  _completeUpdate() {
    this.updating = false;
    this.fire('updateend');
    if (this._followUpStarted) return;
    this._followUpStarted = true;
    // Same task: the element's seek/play re-initialises the demuxer, so the
    // buffer is busy again before the awaiting caller resumes.
    this.updating = true;
    setImmediate(() => {
      this.updating = false;
      this.fire('updateend');
    });
  }
}

/** Every MediaSource the player created, oldest first. */
const createdSources = [];

class FakeMediaSource {
  static isTypeSupported() {
    return true;
  }

  constructor() {
    createdSources.push(this);
    this.readyState = 'closed';
    this.duration = NaN;
    this.sourceBuffers = [];
    this.listeners = new Map();
    // A real MediaSource reaches `open` on a queued task.
    setImmediate(() => {
      this.readyState = 'open';
      this.fire('sourceopen');
    });
  }

  addSourceBuffer(mime) {
    const buffer = new FakeSourceBuffer(mime, this);
    this.sourceBuffers.push(buffer);
    return buffer;
  }

  endOfStream() {
    this.readyState = 'ended';
  }

  addEventListener(type, handler) {
    this.listeners.set(type, handler);
  }

  fire(type) {
    this.listeners.get(type)?.();
  }
}

/**
 * A media element stand-in: assigning `currentTime` queues a `seeking` event,
 * exactly like a real element (which is what the player must not mistake for
 * user intent).
 */
function fakeVideo() {
  const listeners = new Map();
  return {
    src: '',
    _currentTime: 0,
    playCalls: 0,
    seeks: [],
    get currentTime() {
      return this._currentTime;
    },
    set currentTime(value) {
      this._currentTime = value;
      this.seeks.push(value);
      setImmediate(() => this.dispatch('seeking'));
    },
    play() {
      this.playCalls += 1;
      return Promise.resolve();
    },
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

/** A fetch whose body yields `chunks` chunks and then ends. */
function fakeFetch(chunks = 3) {
  const calls = [];
  const fetchImpl = (url) => {
    calls.push(url);
    let reads = 0;
    const reader = {
      read() {
        reads += 1;
        if (reads <= chunks) {
          return Promise.resolve({ done: false, value: new Uint8Array([0, 0, 0, 24]) });
        }
        return Promise.resolve({ done: true });
      },
    };
    return Promise.resolve({
      ok: true,
      status: 200,
      headers: new Headers(),
      body: { getReader: () => reader },
    });
  };
  return { fetchImpl, calls };
}

/**
 * `fakeFetch` whose body delivers the run's initialization segment and then
 * holds every later fragment until `release()`: the window in which the run is
 * attached, typed, positioned on its offset — and has no media buffered yet.
 * The server emits at most one fragment per second, so this is the normal
 * start-up window of a streamed run.
 */
function initSegmentFetch() {
  const calls = [];
  let release;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  const fetchImpl = (url) => {
    calls.push(url);
    let reads = 0;
    const reader = {
      read() {
        reads += 1;
        if (reads === 1) {
          return Promise.resolve({ done: false, value: new Uint8Array([0, 0, 0, 24]) });
        }
        return gate.then(() =>
          reads <= 3 ? { done: false, value: new Uint8Array([0, 0, 0, 24]) } : { done: true }
        );
      },
    };
    return Promise.resolve({
      ok: true,
      status: 200,
      headers: new Headers(),
      body: { getReader: () => reader },
    });
  };
  return { fetchImpl, calls, release: () => release() };
}

/**
 * `fakeFetch` whose response is held back until `release()` is called: the
 * window in which a run is still setting up — no MediaSource attached, no
 * position on the element — and the user is free to move the scrubber.
 */
function gatedFetch() {
  const { fetchImpl, calls } = fakeFetch();
  let release;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  return {
    calls,
    release: () => release(),
    fetchImpl: (url) => gate.then(() => fetchImpl(url)),
  };
}

/**
 * `fakeFetch` whose body delivers the run's initialization segment and then
 * leaves every later `read()` pending until `fail(index, error)` rejects it
 * (`index` counts the runs in request order). That pending read is where a run
 * raises an error from its OWN code path — the pump's `await reader.read()` —
 * which a superseded run still owns after the replacement has detached its
 * SourceBuffer listeners, so a test can reach the run body's `catch` instead of
 * firing an event nothing listens for.
 */
function stalledBodyFetch() {
  const calls = [];
  const failures = [];
  const fetchImpl = (url) => {
    calls.push(url);
    let reads = 0;
    let fail;
    const stalled = new Promise((_, reject) => {
      fail = reject;
    });
    failures.push(fail);
    const reader = {
      read() {
        reads += 1;
        return reads === 1
          ? Promise.resolve({ done: false, value: new Uint8Array([0, 0, 0, 24]) })
          : stalled;
      },
    };
    return Promise.resolve({
      ok: true,
      status: 200,
      headers: new Headers(),
      body: { getReader: () => reader },
    });
  };
  return { fetchImpl, calls, fail: (index, error) => failures[index](error) };
}

/**
 * A long run: `chunks` chunks — an initialization segment plus one 1 s fragment
 * each — with `advance(chunk)` called before chunk `chunk` is handed over. That
 * callback is the element's clock, which a viewer watching the stream arrive
 * keeps close to the buffered end; the short `fakeFetch` cases above pin what a
 * run does with a handful of chunks, this one what the player does with a run
 * long enough to have to bound.
 *
 * `reads()` is how many times the body was read: the count a parked pump stops
 * growing (it is the pump's only consumer), so it is what tells "the pump is
 * waiting for the viewer" apart from "the pump is busy".
 */
function longRunFetch(advance, chunks) {
  const calls = [];
  let readCount = 0;
  const fetchImpl = (url) => {
    calls.push(url);
    let reads = 0;
    const reader = {
      read() {
        reads += 1;
        readCount += 1;
        if (reads <= chunks) advance(reads);
        return Promise.resolve(
          reads <= chunks ? { done: false, value: new Uint8Array([0, 0, 0, 24]) } : { done: true }
        );
      },
    };
    return Promise.resolve({
      ok: true,
      status: 200,
      headers: new Headers(),
      body: { getReader: () => reader },
    });
  };
  return { fetchImpl, calls, reads: () => readCount };
}

function createPlayer(video, options = {}) {
  globalThis.MediaSource = FakeMediaSource;
  URL.createObjectURL = () => 'blob:fake-media-source';
  URL.revokeObjectURL = () => {};
  return createStreamPlayer(video, {
    streamUrl: '/api/photos/abc/video/stream?client=h264-8%2Caac&mode=transcode',
    mime: options.mime ?? 'video/mp4; codecs="avc1.42E01E,mp4a.40.2"',
    duration: options.duration ?? 2,
    onState: options.onState ?? (() => {}),
    onError: options.onError ?? (() => {}),
    onEncoder: options.onEncoder ?? (() => {}),
    onRunStart: options.onRunStart ?? (() => {}),
    onUserSeek: options.onUserSeek ?? (() => {}),
  });
}

test('the player does not treat its own seek as user intent', async () => {
  const video = fakeVideo();
  const { fetchImpl, calls, release } = initSegmentFetch();
  globalThis.fetch = fetchImpl;
  const player = createPlayer(video, { duration: 0 });

  // The run attaches at 15 s and positions the element there itself, which
  // makes the element fire `seeking` for 15 s. Nothing that far down the
  // timeline is buffered yet — the run has delivered only its initialization
  // segment, which carries no samples — so the guarded position is genuinely
  // unbuffered and the guard is the only thing standing between that event and
  // a restart run that would re-request the same offset in a loop.
  const run = player.start(15);
  await settle(10);

  assert.deepEqual(video.seeks, [15], 'the run parked the element on its own offset');
  assert.equal(
    createdSources.at(-1).sourceBuffers[0].buffered.length,
    0,
    'the guarded position is not buffered'
  );
  assert.equal(calls.length, 1, 'exactly one stream run');
  assert.equal(video.playCalls, 1);
  assert.equal(video.listenerCount('playing'), 1);

  release();
  await settle(10);
  assert.equal(calls.length, 1, 'the run is still the only one after its media arrives');

  player.destroy();
  assert.equal(video.listenerCount('seeking'), 0, 'destroy removes its listeners');
  assert.equal(video.listenerCount('playing'), 0);
  await run;
});

test('appending another chunk does not re-show the preparing notice', async () => {
  const video = fakeVideo();
  const states = [];
  const { fetchImpl, release } = initSegmentFetch();
  globalThis.fetch = fetchImpl;
  const player = createPlayer(video, { onState: (value) => states.push(value) });

  const run = player.start(0);
  await settle(10);
  assert.deepEqual(states, ['buffering'], "one notice when the run's first bytes arrive");

  video.dispatch('playing');
  assert.deepEqual(states, ['buffering', 'playing'], 'playback hides it');

  // The run outlives start-up by design (the server emits at most one fragment
  // per second), so another chunk always lands after the element started
  // playing. Reporting `buffering` for it would put "video is being prepared"
  // back on top of the running video — and nothing would take it down again,
  // because only a later `playing` hides the notice.
  release();
  await settle(10);

  assert.deepEqual(states, ['buffering', 'playing', 'ended'], 'later chunks are not a new notice');

  player.destroy();
  await run;
});

test('a stall during playback still reports buffering', async () => {
  const video = fakeVideo();
  const states = [];
  const { fetchImpl, release } = initSegmentFetch();
  globalThis.fetch = fetchImpl;
  const player = createPlayer(video, { onState: (value) => states.push(value) });

  const run = player.start(0);
  await settle(10);

  // Before the element plays, "no data yet" is the normal state of a run that
  // is still filling its buffer: it is already covered by the start-up notice.
  video.dispatch('waiting');
  assert.deepEqual(states, ['buffering'], 'setting up is not a stall');

  video.dispatch('playing');
  // Playback stalled: the clock was running and the media ran out. The viewer
  // hides its notice on `playing`, so this is the only thing that can surface
  // the wait — the per-chunk report never did (it fired while data flowed).
  video.dispatch('waiting');
  assert.deepEqual(states, ['buffering', 'playing', 'buffering'], 'the stall surfaces');

  video.dispatch('playing');
  assert.equal(states.at(-1), 'playing', 'playback resuming hides it again');

  release();
  player.destroy();
  assert.equal(video.listenerCount('waiting'), 0, 'destroy removes its listeners');
  await run;
});

test('a new run releases the blob URL it replaces', async () => {
  const video = fakeVideo();
  globalThis.fetch = fakeFetch().fetchImpl;
  const player = createPlayer(video);
  let created = 0;
  const revoked = [];
  URL.createObjectURL = () => `blob:run-${(created += 1)}`;
  URL.revokeObjectURL = (url) => revoked.push(url);

  await player.start(0);
  await settle(10);
  assert.equal(video.src, 'blob:run-1', 'the run attached its own media source');
  assert.deepEqual(revoked, [], 'the live run keeps its URL');

  // A seek outside the buffered range starts a new run in the same player: the
  // MediaSource it replaces — and its SourceBuffer, and its entry in the
  // document's blob-URL store — has to be released with its URL, or scrubbing
  // through a video leaks one per seek.
  video._currentTime = 15;
  video.dispatch('seeking');
  await settle(10);

  assert.equal(created, 2, 'the seek started its own run');
  assert.deepEqual(revoked, ['blob:run-1'], "the superseded run's URL is released");
  assert.equal(video.src, 'blob:run-2', 'the element holds the live run');

  player.destroy();
  assert.deepEqual(revoked, ['blob:run-1', 'blob:run-2'], 'destroy releases the live URL');
});

test('a user seek outside the buffered range restarts the stream at the target', async () => {
  const video = fakeVideo();
  const { fetchImpl, calls } = fakeFetch();
  globalThis.fetch = fetchImpl;
  const player = createPlayer(video);

  await player.start(0);
  await settle();

  // User seeks to 15 s: the media element moves itself and fires `seeking`.
  video._currentTime = 15;
  video.dispatch('seeking');
  await settle();

  assert.equal(calls.length, 2, 'the seek starts a second stream run');
  assert.match(calls[1], /start=15\.000/);

  // The restart run maps its zero-based segments back onto the real timeline:
  // its own SourceBuffer carries the seek target as the timestamp offset.
  const restartBuffer = createdSources.at(-1).sourceBuffers[0];
  assert.equal(restartBuffer.timestampOffset, 15);
  assert.equal(createdSources[0].sourceBuffers[0].timestampOffset, 0);

  // The restart plays from the offset the user picked: the element already
  // stands there (the user's own seek moved it), so the run parks nothing anew
  // — and cued no `seeking` either, which must not loop into yet another run.
  await settle();
  assert.equal(video.currentTime, 15, 'the element plays from the offset the user picked');
  await settle();
  assert.equal(calls.length, 2, 'the restart seek must not start another run');

  player.destroy();
});

test('chunks survive the internal update the element starts right after an append', async () => {
  const video = fakeVideo();
  const { fetchImpl, calls } = fakeFetch();
  globalThis.fetch = fetchImpl;
  const errors = [];
  const player = createPlayer(video, { onError: (error) => errors.push(error) });

  await player.start(0);
  await settle(10);

  // Chromium's `updating` guard is not a race the app can check away: the
  // element's own seek/play starts an internal update between the check and
  // the call. Before the fix the pump set `timestampOffset` per chunk, threw
  // InvalidStateError, reported a stream error and tore the run down.
  assert.deepEqual(errors, [], 'the timestampOffset/append race must not fail the stream');
  assert.equal(calls.length, 1, 'no stream restart');
  const buffer = createdSources.at(-1).sourceBuffers[0];
  assert.deepEqual(buffer.offsetAssignments, [0], 'the offset is set once, at buffer creation');
  assert.equal(buffer.appended.length, 3, 'every chunk of the run reached the buffer');

  player.destroy();
});

test('playback is reported only when the element actually plays', async () => {
  const video = fakeVideo();
  const states = [];
  globalThis.fetch = fakeFetch().fetchImpl;
  const player = createPlayer(video, { onState: (value) => states.push(value) });

  await player.start(0);
  await settle();

  assert.ok(!states.includes('playing'), 'not playing before the element says so');
  assert.ok(states.includes('buffering'), 'buffering once the first chunk is appended');

  video.dispatch('playing');
  assert.equal(states.at(-1), 'playing');

  player.destroy();
});

test('a seek inside the buffered range does not restart the stream', async () => {
  const video = fakeVideo();
  const { fetchImpl, calls } = fakeFetch();
  globalThis.fetch = fetchImpl;
  const player = createPlayer(video);

  await player.start(0);
  await settle(10);

  // The element can serve this target from the buffer it already holds:
  // restarting the run would re-request and re-convert media nobody needs.
  // The run's fragments are buffered on the offset timeline (0 s for a run
  // attached at 0), so 1 s is genuinely inside the range.
  const buffer = createdSources.at(-1).sourceBuffers[0];
  assert.deepEqual([buffer.buffered.start(0), buffer.buffered.end(0)], [0, 2], 'the run holds 2 s');
  video._currentTime = 1;
  video.dispatch('seeking');
  await settle();

  assert.equal(calls.length, 1, 'a buffered target must not start a stream run');

  player.destroy();
});

test('a refused run reports the HTTP status so saturation is distinguishable', async () => {
  const video = fakeVideo();
  const errors = [];
  globalThis.fetch = () =>
    Promise.resolve({
      ok: false,
      status: 503,
      headers: new Headers({ 'retry-after': '2' }),
      body: null,
    });
  const player = createPlayer(video, { onError: (error) => errors.push(error) });

  await player.start(0);
  await settle();

  // The viewer keys its retry on `503`; a bare Error would be indistinguishable
  // from a real playback failure and would end the user's playback.
  assert.equal(errors.length, 1, 'the refusal reaches the viewer');
  assert.ok(errors[0] instanceof StreamHttpError, 'the failure is typed');
  assert.equal(errors[0].status, 503, 'the status survives to the viewer');
  assert.equal(errors[0].retryAfterMs, 2000, "the server's pacing hint is translated");

  player.destroy();
});

test('a refusal without Retry-After leaves the pacing to the viewer', async () => {
  const video = fakeVideo();
  const errors = [];
  globalThis.fetch = () => Promise.resolve({ ok: false, status: 503, body: null });
  const player = createPlayer(video, { onError: (error) => errors.push(error) });

  await player.start(0);
  await settle();

  assert.equal(errors.length, 1);
  assert.equal(errors[0].retryAfterMs, null, 'no hint must not become a bogus delay');

  player.destroy();
});

test('a refused seek-restarted run reports its status and pacing hint', async () => {
  const video = fakeVideo();
  const { fetchImpl } = fakeFetch();
  const requested = [];
  globalThis.fetch = (url) => {
    requested.push(url);
    return url.includes('start=15.000')
      ? Promise.resolve({
          ok: false,
          status: 503,
          headers: new Headers({ 'retry-after': '2' }),
          body: null,
        })
      : fetchImpl(url);
  };
  const errors = [];
  const player = createPlayer(video, { onError: (error) => errors.push(error) });

  await player.start(0);
  await settle(10);

  // A user seek starts its own run at the target; the pool refuses that run.
  video._currentTime = 15;
  video.dispatch('seeking');
  await settle();

  assert.equal(requested.length, 2, 'the seek started a second run');
  assert.match(requested[1], /start=15\.000/);
  assert.equal(errors.length, 1, 'the refusal reached the viewer');
  // The refusal of a seek-restarted run is no different from any other: the
  // viewer keys its retry on the status and the pacing hint, and resumes the
  // newest position the user asked for — its own `streamIntentOffset`, never an
  // offset carried by the refused run.
  assert.equal(errors[0].status, 503);
  assert.equal(errors[0].retryAfterMs, 2000);

  player.destroy();
});
test('appended media cannot inflate the declared duration', async () => {
  const video = fakeVideo();
  // An initialization segment plus three 1 s fragments: 3 s of media for a 2 s
  // source, which is what a stream-copied seek restart looks like.
  globalThis.fetch = fakeFetch(4).fetchImpl;
  const player = createPlayer(video);

  await player.start(0);
  await settle();

  // The declared duration is what the server measured on the source, so the
  // timeline starts out exact — the element's seek bar must not be 0:00 → ∞.
  assert.equal(createdSources.at(-1).duration, 2, 'the initial duration is the declared one');

  await settle(10);

  // The run's fragments end past the declared duration: the media source
  // adopts 3 s as its duration and would report it from the element, so the
  // player drops the phantom tail and re-asserts the declared value.
  const buffer = createdSources.at(-1).sourceBuffers[0];
  assert.equal(buffer.appended.length, 4, 'the scenario is exercised');
  assert.deepEqual(buffer.removals, [[2, 3]], 'media past the declared end is dropped');
  assert.equal(buffer.buffered.end(0), 2, 'the buffer no longer runs past the source');
  assert.equal(createdSources.at(-1).duration, 2, 'buffered excess must not grow the timeline');

  player.destroy();
});

test('played-out media is evicted so a long run cannot grow without limit', async () => {
  const video = fakeVideo();
  const { fetchImpl } = longRunFetch((chunk) => {
    // A viewer playing the run as it arrives: the clock the player evicts
    // against advances one second per delivered fragment, as it does for a
    // stream being watched in real time.
    video._currentTime = chunk - 1;
  }, 120);
  globalThis.fetch = fetchImpl;
  const player = createPlayer(video, { duration: 120 });

  await player.start(0);
  await settle(10);

  const buffer = createdSources.at(-1).sourceBuffers[0];
  const start = buffer.buffered.start(0);
  const end = buffer.buffered.end(0);
  assert.equal(end, 119, 'the run delivered all of its media');
  assert.ok(start > 0, 'the played-out beginning must not be held for the whole run');
  assert.ok(
    end - start <= 60,
    `the buffer must stay bounded behind the playhead, held ${end - start} s of ${end} s`
  );

  player.destroy();
});

test('media ahead of the playhead is never evicted', async () => {
  const video = fakeVideo();
  // A run shorter than the look-ahead window, so it is delivered in full while
  // the element sits at 0 s: a viewer that never plays cannot drain a window, so
  // a longer run would (correctly) stop being read.
  const { fetchImpl, calls } = longRunFetch(() => {}, 20);
  globalThis.fetch = fetchImpl;
  const player = createPlayer(video, { duration: 20 });

  await player.start(0);
  await settle(10);

  const buffer = createdSources.at(-1).sourceBuffers[0];
  assert.deepEqual(buffer.removals, [], 'nothing has played out, so nothing may be dropped');
  assert.deepEqual(
    [buffer.buffered.start(0), buffer.buffered.end(0)],
    [0, 19],
    'the whole run stays buffered while the element sits at 0 s'
  );

  // AND a seek forward into that media is served from the buffer: dropping
  // ahead of the playhead would re-convert what has already streamed.
  video._currentTime = 10;
  video.dispatch('seeking');
  await settle(5);
  assert.equal(calls.length, 1, 'a buffered target must start no stream run');

  player.destroy();
});

test('a buffer that cannot evict keeps its media instead of failing the run', async () => {
  const video = fakeVideo();
  const errors = [];
  globalThis.fetch = longRunFetch((chunk) => {
    video._currentTime = chunk - 1;
  }, 60).fetchImpl;
  const player = createPlayer(video, { duration: 60, onError: (error) => errors.push(error) });

  // MSE allows a SourceBuffer whose type cannot be evicted: `remove` is not
  // there at all. The run keeps streaming — growth is the lesser fault, and a
  // run failed over an unsupported eviction burns the viewer's escalation
  // ladder for a video that is playing fine.
  const run = player.start(0);
  await settle(5);
  const buffer = createdSources.at(-1).sourceBuffers[0];
  buffer.remove = undefined;
  await run;
  await settle(10);

  assert.deepEqual(errors, [], 'an unavailable eviction is not a run failure');
  assert.deepEqual(
    [buffer.buffered.start(0), buffer.buffered.end(0)],
    [0, 59],
    'the media it cannot evict stays'
  );

  player.destroy();
});

test('delivery that outruns the clock is held to the look-ahead window', async () => {
  const video = fakeVideo();
  // 4x delivery: the fake clock advances a quarter second per delivered
  // fragment. That is the normal shape of this endpoint — the server pipes the
  // run out unpaced, so a remux arrives in seconds — and it is the case the
  // played-out eviction alone cannot bound: the write head runs away from the
  // viewer, and everything it has written is ahead of the playhead.
  const { fetchImpl, reads } = longRunFetch((chunk) => {
    video._currentTime = (chunk - 1) / 4;
  }, 120);
  globalThis.fetch = fetchImpl;
  const player = createPlayer(video, { duration: 120 });

  // Not awaited: the run is still parked on its window, and `start()` only
  // settles once its pump returns (see the destroy below).
  const run = player.start(0);
  await settle(5);
  const buffer = createdSources.at(-1).sourceBuffers[0];
  // Let the delivery come to rest: the pump either waits for the viewer here or
  // consumes the whole body, and the count of reads is what says which.
  await settleUntilStable(reads);

  const end = buffer.buffered.end(0);
  assert.ok(
    end - video._currentTime <= 31,
    `no more than the look-ahead window may be held ahead of the playhead (held ${end - video._currentTime} s)`
  );
  // AND the pump stopped reading rather than consuming everything it was
  // handed: the window is what it stopped at, not the end of the body.
  assert.ok(reads() < 121, `the run must not consume its whole body (${reads()} reads)`);

  player.destroy();
  await run;
});

test('a paused viewer is neither polled nor buffered past the window', async () => {
  const video = fakeVideo();
  // The element never moves: playback has not started (or the viewer paused),
  // so nothing drains the window the pump fills.
  const { fetchImpl, reads } = longRunFetch(() => {}, 120);
  globalThis.fetch = fetchImpl;
  const player = createPlayer(video, { duration: 120 });

  const run = player.start(0);
  await settle(5);
  const buffer = createdSources.at(-1).sourceBuffers[0];
  // The pump parks one read past its window: the chunk it holds back is the read
  // that keeps a body ending under the wait observable. Waiting for the reads to
  // come to rest is what lets the measurement below be about the parked state.
  await settleUntilStable(reads);

  const appendedWhileParked = buffer.appended.length;
  const readsWhileParked = reads();
  assert.ok(
    appendedWhileParked <= 33,
    `a viewer that plays nothing must not accumulate media (appended ${appendedWhileParked})`
  );

  // Waiting on the element is not polling it: with the viewer still, the pump
  // must consume nothing at all, however many turns pass.
  await settle(50);
  assert.equal(buffer.appended.length, appendedWhileParked, 'a parked pump appends nothing');
  assert.equal(reads(), readsWhileParked, 'a parked pump reads nothing');

  player.destroy();
  await run;
});

test('a window the viewer drains resumes the pump and the run finishes', async () => {
  const video = fakeVideo();
  const states = [];
  const errors = [];
  const { fetchImpl, reads } = longRunFetch(() => {}, 120);
  globalThis.fetch = fetchImpl;
  const player = createPlayer(video, {
    duration: 120,
    onState: (state) => states.push(state),
    onError: (error) => errors.push(error),
  });

  const run = player.start(0);
  await settle(5);
  const buffer = createdSources.at(-1).sourceBuffers[0];
  // The delivery comes to rest at the window, with the viewer still: that is
  // the state the drain below is about.
  await settleUntilStable(reads);
  const parkedEnd = buffer.buffered.end(0);
  assert.ok(parkedEnd < 119, `the pump must wait for the viewer (buffered to ${parkedEnd} s)`);

  // The viewer plays what the run delivered: each move of its playhead lets the
  // pump top the window back up, until the body ends and the run is over.
  for (let i = 0; i < 60 && reads() <= 120; i += 1) {
    video._currentTime = Math.min(video._currentTime + 5, 119);
    video.dispatch('timeupdate');
    await settle(3);
  }
  await settle(5);

  assert.equal(reads(), 121, 'the pump read the run to its end');
  assert.equal(buffer.buffered.end(0), 119, 'the whole run is buffered');
  assert.ok(states.includes('ended'), 'the completed run reports its end');
  assert.deepEqual(errors, [], 'nothing about this run is a failure');

  player.destroy();
  await run;
});

test('a body that ends while the pump waits still reports the truncation', async () => {
  const video = fakeVideo();
  const errors = [];
  const states = [];
  // The body ends while the parked pump is holding its window full: that is the
  // server's stall watchdog killing a run whose client stopped reading, which
  // closes the response before the declared duration. The pump is waiting on the
  // element rather than reading, so the end of the body has to reach it from
  // inside that wait — a wait that only watched the playhead would park here
  // forever and never report anything. 32 fragments is what fills the window
  // (the element never plays, so nothing drains it) and the read that finds the
  // body ended is the one issued at that point.
  const { fetchImpl, reads } = longRunFetch(() => {}, 32);
  globalThis.fetch = fetchImpl;
  const player = createPlayer(video, {
    duration: 120,
    onState: (state) => states.push(state),
    onError: (error) => errors.push(error),
  });

  const run = player.start(0);
  await settleUntilStable(reads);

  assert.equal(errors.length, 1, 'the killed run must be reported, never swallowed');
  assert.match(errors[0].message, /ended early/);
  assert.ok(!states.includes('ended'), 'a body short of the declared duration is not a clean end');

  player.destroy();
  await run;
});

test('a run that has ended still gets its played-out media trimmed', async () => {
  const video = fakeVideo();
  // A run the viewer never played during: it arrives in full, ends, and leaves
  // its media in the buffer with the append loop gone.
  const { fetchImpl } = longRunFetch(() => {}, 20);
  globalThis.fetch = fetchImpl;
  const player = createPlayer(video, { duration: 20 });

  await player.start(0);
  await settle(10);
  const buffer = createdSources.at(-1).sourceBuffers[0];
  assert.deepEqual(buffer.removals, [], 'the element has played nothing out yet');

  // The element's clock standing past the media of a finished run (a seek out
  // of the buffered range is what leaves it there). Nothing but the playhead
  // can drop the media the viewer has passed now, and it has to: the renderer
  // would otherwise hold it for the rest of the playback.
  video._currentTime = 45;
  video.dispatch('timeupdate');
  await settle(5);

  assert.ok(buffer.removals.length > 0, 'the played-out media must go without the pump');
  assert.equal(buffer.buffered.start(0), 15, 'the head beyond the window is what goes');
  assert.equal(buffer.buffered.end(0), 19, 'nothing ahead of the playhead is touched');

  player.destroy();
});

test('a permanently disabled conversion pool is not a slot wait', async () => {
  const video = fakeVideo();
  const errors = [];
  // The server's own answer for `TURBO_PIX_MAX_TRANSCODES=0`: the same 503 and
  // the same `Retry-After` a saturated pool sends, with the refusal body that
  // names it as disabled (src/handlers_video.rs, `StreamStartError::Disabled`).
  globalThis.fetch = () =>
    Promise.resolve({
      ok: false,
      status: 503,
      headers: new Headers({ 'retry-after': '5' }),
      json: () => Promise.resolve({ error: 'conversion disabled' }),
    });
  const player = createPlayer(video, { onError: (error) => errors.push(error) });

  await player.start(0);
  await settle();

  assert.equal(errors.length, 1, 'the refusal reaches the viewer');
  assert.equal(errors[0].status, 503, 'it is still a 503 the viewer can read');
  assert.equal(
    errors[0].permanent,
    true,
    'no slot will ever free, so waiting for one must not be the answer'
  );
  assert.equal(errors[0].retryAfterMs, 5000, "the server's hint still survives");

  player.destroy();
});

test('a saturated pool is still a wait the viewer paces', async () => {
  const video = fakeVideo();
  const errors = [];
  // The other 503 the endpoint sends: workers are busy, a slot comes free.
  globalThis.fetch = () =>
    Promise.resolve({
      ok: false,
      status: 503,
      headers: new Headers({ 'retry-after': '2' }),
      json: () => Promise.resolve({ error: 'no conversion slot available' }),
    });
  const player = createPlayer(video, { onError: (error) => errors.push(error) });

  await player.start(0);
  await settle();

  assert.equal(errors.length, 1);
  assert.equal(errors[0].status, 503);
  assert.equal(errors[0].permanent, false, 'a busy pool must keep its waiting notice');
  assert.equal(errors[0].retryAfterMs, 2000);

  player.destroy();
});

test('undecodable delivered bytes reach the viewer exactly once', async () => {
  const video = fakeVideo();
  globalThis.fetch = fakeFetch().fetchImpl;
  const errors = [];
  const player = createPlayer(video, { onError: (error) => errors.push(error) });

  await player.start(0);
  await settle();

  // Chromium reports a failed parse on the SourceBuffer and then on the
  // element. The viewer escalates one ladder step per report, so both must
  // collapse into a single failure.
  const buffer = createdSources.at(-1).sourceBuffers[0];
  buffer.fire('error');
  video.dispatch('error');
  await settle();

  assert.equal(errors.length, 1, 'one failure per run');
  assert.ok(errors[0] instanceof Error, 'the viewer gets a plain playback failure');
  assert.equal(errors[0].status, undefined, 'a decode failure is not a saturation refusal');

  player.destroy();
  assert.equal(video.listenerCount('error'), 0, 'destroy detaches the media-error listener');
});

test("a superseded run's late error does not escalate the run that replaced it", async () => {
  const video = fakeVideo();
  const { fetchImpl, calls, fail } = stalledBodyFetch();
  globalThis.fetch = fetchImpl;
  const errors = [];
  // The body of every run here stops after its initialization segment, so each
  // run sits inside `pump` with a pending `read()`. That is the code path from
  // which a superseded run raises an error of its own — the run body's `catch`
  // — and it survives the supersession, unlike the SourceBuffer listeners
  // `start()` detaches from the run it replaces: firing `error` on the old
  // buffer reaches nothing and would let the attribution guard rot unnoticed.
  // A declared duration is honest here: the replacement run starts at 15 s on a
  // 2 s source, which is the run that has nothing left to deliver, not the
  // truncated one.
  const player = createPlayer(video, { duration: 2, onError: (error) => errors.push(error) });

  const first = player.start(0);
  await settle(10);
  assert.equal(calls.length, 1, 'the first run is in flight');

  // A user seek restarts the stream: the old run is superseded, its media is
  // gone, and an error it reports late belongs to a mode the viewer left.
  video._currentTime = 15;
  video.dispatch('seeking');
  await settle(10);
  assert.equal(calls.length, 2, 'the seek restart is still the current run');

  // The superseded run's pending read now fails, and its own pump reports that
  // failure with the signal it was superseded on.
  fail(0, new Error('the superseded delivery failed'));
  await settle(10);

  assert.equal(errors.length, 0, 'a superseded run must not fail the current one');

  // The run now attached still reports its own failure.
  createdSources.at(-1).sourceBuffers[0].fire('error');
  await settle();
  assert.equal(errors.length, 1, 'the current run reports once');

  player.destroy();
  await first;
});
test("a run's buffer is typed from the MIME the server advertises for that run", async () => {
  const video = fakeVideo();
  const errors = [];
  // The decision advertised an AC-3 remux (what the client asked for), but the
  // rung that actually ran is an audio conversion that emits AAC: Chromium
  // rejects an append whose init segment does not match the buffer's declared
  // type, so the run's own MIME has to win or the ladder burns its remaining
  // rungs on bytes that were perfectly playable.
  const decisionMime = 'video/mp4; codecs="avc1.42E01E,ac-3"';
  const runMime = 'video/mp4; codecs="avc1.42E01E,mp4a.40.2"';
  globalThis.fetch = () => {
    let reads = 0;
    return Promise.resolve({
      ok: true,
      status: 200,
      headers: new Headers({ 'x-turbopix-mime': runMime }),
      body: {
        getReader: () => ({
          read() {
            reads += 1;
            return reads === 1
              ? Promise.resolve({ done: false, value: new Uint8Array([0, 0, 0, 24]) })
              : Promise.resolve({ done: true });
          },
        }),
      },
    });
  };
  const player = createPlayer(video, {
    // No declared duration: this case is about the MIME the run's bytes are
    // typed with, not about the timeline — the body here closes after the
    // initialization segment, which a declared duration would report as the
    // truncation it is.
    duration: 0,
    mime: decisionMime,
    onError: (error) => errors.push(error),
  });

  await player.start(0);
  await settle();

  assert.equal(createdSources.at(-1).sourceBuffers[0].mime, runMime);
  assert.deepEqual(errors, [], 'a delivered run is not a failure');

  player.destroy();
});

test('a run without an advertised MIME keeps the decision MIME', async () => {
  const video = fakeVideo();
  const { fetchImpl } = fakeFetch();
  globalThis.fetch = fetchImpl;
  const player = createPlayer(video);

  await player.start(0);
  await settle();

  assert.equal(
    createdSources.at(-1).sourceBuffers[0].mime,
    'video/mp4; codecs="avc1.42E01E,mp4a.40.2"',
    'the decision MIME is the fallback when the server advertises none'
  );

  player.destroy();
});

test('reports the run encoder to the caller, and null when the header is absent', async () => {
  // GIVEN a run that names a hardware encoder
  const video = fakeVideo();
  const seen = [];
  globalThis.MediaSource = FakeMediaSource;
  globalThis.fetch = () =>
    Promise.resolve({
      ok: true,
      status: 200,
      headers: new Headers({ 'x-turbopix-encoder': 'h264_vaapi' }),
      body: { getReader: () => ({ read: () => Promise.resolve({ done: true }) }) },
    });
  const hardwarePlayer = createPlayer(video, { onEncoder: (encoder) => seen.push(encoder) });
  await hardwarePlayer.start(0);

  // AND a second run that sends no such header (a copy delivery)
  const copyVideo = fakeVideo();
  globalThis.fetch = () =>
    Promise.resolve({
      ok: true,
      status: 200,
      headers: new Headers(),
      body: { getReader: () => ({ read: () => Promise.resolve({ done: true }) }) },
    });
  const copyPlayer = createPlayer(copyVideo, { onEncoder: (encoder) => seen.push(encoder) });
  await copyPlayer.start(0);

  // THEN the caller sees the value, then null — never undefined and never ''
  assert.deepEqual(seen, ['h264_vaapi', null]);

  hardwarePlayer.destroy();
  copyPlayer.destroy();
});

test('a stream that ends before the declared duration reports a failure, not `ended`', async () => {
  const video = fakeVideo();
  const { fetchImpl } = fakeFetch();
  globalThis.fetch = fetchImpl;
  const states = [];
  const errors = [];
  // The body here yields three 1 s chunks and then ends, but the source is
  // 20 s: the run stopped part-way. The server kills a conversion at its
  // deadline and ffmpeg can die mid-stream, and both merely close the chunked
  // response — the declared duration is the only signal that the last frames
  // never arrived.
  const player = createPlayer(video, {
    duration: 20,
    onState: (value) => states.push(value),
    onError: (error) => errors.push(error),
  });

  await player.start(0);
  await settle(10);

  assert.equal(errors.length, 1, 'the truncated run reaches the viewer');
  assert.match(errors[0].message, /ended early/);
  // Reporting a clean end is what hid the notice and left the user stranded
  // with no ladder step and no "play original anyway".
  assert.ok(!states.includes('ended'), 'a truncated stream must not report a clean end');

  player.destroy();
});

test('a run that buffers nothing at all is a failure even for a 2 s source', async () => {
  const video = fakeVideo();
  const states = [];
  const errors = [];
  // `fakeFetch(1)` delivers the run's initialization segment and then closes the
  // body: the run gets past start-up and ends with no coded frame buffered —
  // what ffmpeg dying mid-stream or the server killing the conversion at its
  // deadline looks like. The source is 2 s, the duration of the shipped
  // `test_video_hevc.mp4`: with a flat 2 s slack the threshold was
  // `2 - 2 <= 0`, which the buffered end of 0 can never fall below, so the
  // truncation was reported as a clean `ended` — the viewer hid its notice,
  // never advanced the ladder and never offered "play original anyway".
  globalThis.fetch = fakeFetch(1).fetchImpl;
  const player = createPlayer(video, {
    duration: 2,
    onState: (value) => states.push(value),
    onError: (error) => errors.push(error),
  });

  await player.start(0);
  await settle(10);

  assert.equal(errors.length, 1, 'the truncated run reaches the viewer');
  assert.match(errors[0].message, /ended early/);
  assert.ok(!states.includes('ended'), 'a buffer with no coded frames is not a clean end');

  player.destroy();
});

test('a run that starts at the end of the clip ends cleanly, not as a truncation', async () => {
  const video = fakeVideo();
  const { fetchImpl } = fakeFetch();
  globalThis.fetch = fetchImpl;
  const states = [];
  const errors = [];
  // The element clamps a seek to the end of the clip onto the media duration
  // (a scrubber drag to the far right, `currentTime = duration`), and the
  // server still answers that offset with a stream head. The run's fragment
  // lands past the declared end, so the clamp strips it and the buffer is empty
  // by construction: the run has nothing left to deliver. Judged against the
  // declared duration — a threshold that stays above 0 for every duration — it
  // reported a truncation, so the viewer escalated its ladder up to three times
  // for a position that cannot deliver anything and ended on a misleading
  // "Video conversion failed".
  const player = createPlayer(video, {
    duration: 2,
    onState: (value) => states.push(value),
    onError: (error) => errors.push(error),
  });

  await player.start(2);
  await settle(10);

  assert.equal(errors.length, 0, 'a run with nothing left to deliver is not a truncation');
  assert.ok(states.includes('ended'), 'the run reports the clean end the viewer expects');

  player.destroy();
});

test('a stream that delivers no media reports a failure', async () => {
  const video = fakeVideo();
  const errors = [];
  // A zero-byte source answers 200 with an empty body, and ffmpeg dying before
  // the init segment looks identical. Before the fix the player returned
  // silently and the viewer sat on its non-error buffering notice.
  globalThis.fetch = () =>
    Promise.resolve({
      ok: true,
      status: 200,
      headers: new Headers({ 'x-transcode-warning': 'empty' }),
      body: { getReader: () => ({ read: () => Promise.resolve({ done: true }) }) },
    });
  const player = createPlayer(video, { onError: (error) => errors.push(error) });

  await player.start(0);
  await settle();

  assert.equal(errors.length, 1, 'the empty run reaches the viewer');
  assert.match(errors[0].message, /no media/);

  player.destroy();
});

test('a seek during a starting run is not dropped: the newest target wins', async () => {
  const video = fakeVideo();
  const { fetchImpl, calls, release } = gatedFetch();
  globalThis.fetch = fetchImpl;
  const player = createPlayer(video);

  // The run for 10 s is still waiting on the server, so this player has not
  // positioned the element yet: `starting` spans the whole round trip.
  const run = player.start(10);
  await settle(2);

  // The user drag-scrubs to 20 s and then to 25 s: the element moves itself and
  // fires `seeking` for both assignments, while the 10 s run is still in setup.
  video._currentTime = 20;
  video.dispatch('seeking');
  video._currentTime = 25;
  video.dispatch('seeking');
  await settle();

  release();
  await run;
  await settle(10);

  // Both seeks used to be dropped (`starting` returns early) and the run then
  // assigned the element ITS own 10 s, so the scrubber snapped back to a
  // position the user had already left.
  assert.equal(calls.length, 2, 'the seek that landed during setup starts its own run');
  assert.match(calls[1], /start=25\.000/, 'the newest target wins');
  const seekBuffer = createdSources.at(-1).sourceBuffers[0];
  assert.equal(
    seekBuffer.timestampOffset,
    25,
    "the new run's segments map onto the user's position"
  );
  assert.equal(video.currentTime, 25, 'the element ends on the position the user picked');

  player.destroy();
});

test('a user seek inside the own-seek window still restarts while the run has no media', async () => {
  const video = fakeVideo();
  const { fetchImpl, calls, release } = initSegmentFetch();
  globalThis.fetch = fetchImpl;
  // No declared duration: this case is about the window before the run's first
  // media fragment, not about the timeline.
  const player = createPlayer(video, { duration: 0 });

  const run = player.start(15);
  await settle(10);

  // The run attached at 15 s and positioned the element there. Its init segment
  // carries no samples, so nothing near 15 s is buffered and the element's own
  // seek cannot complete — `seeked` cannot fire until the first fragment.
  assert.equal(calls.length, 1, 'one run so far');
  assert.deepEqual(video.seeks, [15], 'the element was parked on the run offset');
  assert.equal(
    createdSources.at(-1).sourceBuffers[0].buffered.length,
    0,
    'the run has no media buffered yet'
  );

  // The user drags the scrubber 0.1 s further while that run is still waiting
  // for its first fragment. The target sits inside the 0.25 s window the player
  // uses to recognise its own assignment, but this event is the user's and the
  // target is not buffered: swallowing it parks the element on a position the
  // run — which only ever appends forward from its own offset — never fills.
  video._currentTime = 15.1;
  video.dispatch('seeking');
  await settle();

  assert.equal(calls.length, 2, 'the user seek started its own run');
  assert.match(calls[1], /start=15\.100/);
  assert.equal(
    createdSources.at(-1).sourceBuffers[0].timestampOffset,
    15.1,
    "the new run's segments map onto the user's position"
  );

  player.destroy();
  release();
  await run;
});

test('a run that moved nothing arms no guard, and a seek back to its offset is honoured', async () => {
  const video = fakeVideo();
  const calls = [];
  let releaseRuns;
  const runGate = new Promise((resolve) => {
    releaseRuns = resolve;
  });
  // The first two runs answer at once but deliver only their initialization
  // segment — the element carries no media, so a seek near an offset is
  // genuinely unbuffered and only the own-seek guard can swallow it. The run
  // after them is held at the request: the setup window of a run the server has
  // not answered yet, in which the user is free to move the scrubber.
  globalThis.fetch = (url) => {
    calls.push(url);
    return (calls.length <= 2 ? Promise.resolve() : runGate).then(() => {
      let reads = 0;
      const reader = {
        read() {
          reads += 1;
          if (reads === 1) {
            return Promise.resolve({ done: false, value: new Uint8Array([0, 0, 0, 24]) });
          }
          return runGate.then(() =>
            reads <= 3 ? { done: false, value: new Uint8Array([0, 0, 0, 24]) } : { done: true }
          );
        },
      };
      return { ok: true, status: 200, headers: new Headers(), body: { getReader: () => reader } };
    });
  };
  const player = createPlayer(video, { duration: 0 });

  const first = player.start(0);
  await settle(10);

  // The element already stood at 0, so the run moved nothing: it cued no
  // `seeking`, and no own-seek guard may be left armed for it — nothing would
  // ever consume one.
  assert.equal(calls.length, 1, 'one run so far');
  assert.deepEqual(video.seeks, [], 'a run attached at 0 does not move the element');
  assert.equal(
    createdSources.at(-1).sourceBuffers[0].buffered.length,
    0,
    'the run has delivered only its initialization segment'
  );

  // The first `seeking` to arrive after that run is the user's, and its target
  // sits 0.1 s from the offset an unconditionally armed guard would hold. That
  // run is no longer setting up, nothing near 0.1 s is buffered, and a run only
  // ever appends forward from its own offset — so a guard left armed by a run
  // that moved nothing would swallow this newest target and park the element on
  // a position no run ever fills.
  video._currentTime = 0.1;
  video.dispatch('seeking');
  await settle(2);
  assert.equal(calls.length, 2, 'the user seek started its own run');
  assert.match(calls[1], /start=0\.100/, 'the guard of the 0 s run must not swallow it');

  // The user seeks forward; the run for that offset is still fetching.
  video._currentTime = 30;
  video.dispatch('seeking');
  await settle(2);
  assert.equal(calls.length, 3, 'the seek started its own run');
  assert.match(calls[2], /start=30\.000/);

  // ... and, while that run is still setting up, drags back to the beginning:
  // the offset of the run that moved nothing. The run in flight will park the
  // element on 30 s, so the newest target is remembered and honoured as soon as
  // it has attached.
  video._currentTime = 0;
  video.dispatch('seeking');
  await settle(2);
  assert.equal(calls.length, 3, 'a seek during setup is stored, not started');

  releaseRuns();
  await settle(40);

  assert.equal(calls.length, 4, 'the seek back to the start is not dropped');
  assert.match(calls[3], /start=0\.000/, 'the newest target wins');
  assert.equal(video.currentTime, 0, 'the element ends where the user left it');
  assert.equal(
    createdSources.at(-1).sourceBuffers[0].timestampOffset,
    0,
    "the newest run's segments map onto the user's position"
  );

  player.destroy();
  await first;
});

test('every run the player begins is reported before its request is issued', async () => {
  const video = fakeVideo();
  const { fetchImpl, calls } = fakeFetch();
  globalThis.fetch = fetchImpl;
  const started = [];
  const player = createPlayer(video, {
    // A viewer that armed something for the previous run has to learn a newer
    // run exists before that run can be torn down: the request being issued is
    // the first point at which it is a run in flight.
    onRunStart: (seconds) => started.push([seconds, calls.length]),
  });

  await player.start(0);
  await settle(10);
  assert.deepEqual(started, [[0, 0]], 'the requested run reports itself at its top');

  // A seek outside the buffered range restarts the stream inside the same
  // player: a run nobody asked `start()` for directly, and the one a retry
  // armed for an earlier refusal must never tear down.
  video._currentTime = 15;
  video.dispatch('seeking');
  await settle(10);

  assert.equal(calls.length, 2, 'the seek started its own run');
  assert.deepEqual(
    started,
    [
      [0, 0],
      [15, 1],
    ],
    'the seek restart reports itself too'
  );

  player.destroy();
});

test('a throwing onRunStart rejects its run and leaves the player usable', async () => {
  const video = fakeVideo();
  const { fetchImpl, calls } = fakeFetch();
  globalThis.fetch = fetchImpl;
  let starts = 0;
  const player = createPlayer(video, {
    // The report is caller-supplied code running in the run's prologue: a bug
    // in it must not latch the player's run state. Before the fix this threw
    // out of `start()` with the run latch still set, so every later entry point
    // — `start()` and the seek restart — returned silently and the element
    // stayed frozen on whatever frame it held, with `destroy()` the only way
    // out.
    onRunStart: () => {
      starts += 1;
      if (starts === 1) throw new Error('the viewer cannot handle this run');
    },
  });

  await assert.rejects(
    player.start(0),
    /cannot handle this run/,
    "the throw reaches the caller as that run's rejection"
  );
  assert.deepEqual(calls, [], 'the failed run issued no request');

  // The latch was released with the throw: the next run is a run.
  await player.start(0);
  await settle(10);
  assert.equal(starts, 2, 'the replacement run reported itself too');
  assert.equal(calls.length, 1, 'the replacement run was actually issued');
  assert.equal(video.playCalls, 1, 'the element was positioned and played');

  // ... and so is a seek restart, the other entry point the latch used to
  // wedge.
  video._currentTime = 30;
  video.dispatch('seeking');
  await settle(10);
  assert.equal(calls.length, 2, 'the seek started its own run');
  assert.match(calls[1], /start=30\.000/);

  player.destroy();
});

test('a guard the arriving seek does not match is not left armed', async () => {
  const video = fakeVideo();
  const { fetchImpl, calls, release } = initSegmentFetch();
  globalThis.fetch = fetchImpl;
  const player = createPlayer(video, { duration: 0 });

  const run = player.start(10);
  // Stop as soon as the run has moved the element: at that instant its own
  // `seeking` is cued but not yet dispatched (the element queues it), so this
  // is the window between an assignment and the event that consumes its guard.
  for (let i = 0; i < 10 && video.seeks.length === 0; i += 1) {
    await settle(1);
  }
  assert.deepEqual(video.seeks, [10], 'the run moved the element to its offset');
  assert.equal(calls.length, 1, 'one run so far');

  // The user drags to 40 before that queued event runs, so the event the player
  // then sees carries the user's target and does not match the armed 10 s. It
  // has to retire the arm: the assignment's own event will never come back to
  // consume it, and it would swallow the next user seek to land near 10 s.
  video._currentTime = 40;
  video.dispatch('seeking');
  await settle(10);
  assert.equal(calls.length, 2, 'the user seek started its own run');
  assert.match(calls[1], /start=40\.000/);

  // 0.1 s from the offset of the run that armed that guard: inside the 0.25 s
  // window, but the guard belongs to a seek the user has already left, so this
  // newest target must start its own run instead of being swallowed.
  video._currentTime = 10.1;
  video.dispatch('seeking');
  await settle(2);

  assert.equal(calls.length, 3, 'the left-behind guard must not swallow the seek');
  assert.match(calls[2], /start=10\.100/, 'the newest target wins');

  player.destroy();
  release();
  await run;
});

test('the player reports the position the user picked, and never its own', async () => {
  const video = fakeVideo();
  const { fetchImpl, calls, release } = initSegmentFetch();
  globalThis.fetch = fetchImpl;
  const userSeeks = [];
  const player = createPlayer(video, {
    duration: 0,
    onUserSeek: (target) => userSeeks.push(target),
  });

  // The run attaches at 10 s and positions the element there itself, so the
  // element fires `seeking` for the player's own assignment. A caller reading
  // that as intent (the viewer disarms its saturation retry on this hook) would
  // act on every run the player ever begins.
  const run = player.start(10);
  await settle(10);
  assert.deepEqual(video.seeks, [10], 'the run parked the element on its own offset');
  assert.deepEqual(userSeeks, [], 'the player positioning the element is not user intent');

  // The user drags to 25 s: nothing near that target is buffered (the run has
  // delivered only its initialization segment), so this move also starts its
  // own run — but the caller is told about the seek itself, not merely about
  // the run it happens to start.
  video._currentTime = 25;
  video.dispatch('seeking');
  await settle(10);

  assert.deepEqual(userSeeks, [25], 'the user seek is reported with its target');
  assert.equal(calls.length, 2, 'the unbuffered target started its own run');

  player.destroy();
  release();
  await run;
});

test('a seek the element can serve from its buffer is reported too', async () => {
  const video = fakeVideo();
  const { fetchImpl, calls } = fakeFetch();
  globalThis.fetch = fetchImpl;
  const userSeeks = [];
  const player = createPlayer(video, {
    onUserSeek: (target) => userSeeks.push(target),
  });

  await player.start(0);
  await settle(10);
  const buffer = createdSources.at(-1).sourceBuffers[0];
  assert.deepEqual([buffer.buffered.start(0), buffer.buffered.end(0)], [0, 2]);

  // The one user move that starts no run: the target is already buffered, so a
  // caller watching only the runs it begins would never learn that the offset
  // it armed something for is one the user has just left.
  video._currentTime = 1;
  video.dispatch('seeking');
  await settle();

  assert.equal(calls.length, 1, 'a buffered target must not start a stream run');
  assert.deepEqual(userSeeks, [1], 'the buffered seek is still user intent');

  player.destroy();
});

test('a seek that lands while a run sets up is reported as user intent', async () => {
  const video = fakeVideo();
  const { fetchImpl, calls, release } = gatedFetch();
  globalThis.fetch = fetchImpl;
  const userSeeks = [];
  const player = createPlayer(video, {
    onUserSeek: (target) => userSeeks.push(target),
  });

  // The run for 10 s is still waiting on the server, so this player has not
  // positioned the element yet: the seek below is stored for that run rather
  // than acted on at once. It is still the user leaving the offset a caller's
  // arming waited for, which is why the hook covers this branch as well.
  const run = player.start(10);
  await settle(2);
  video._currentTime = 25;
  video.dispatch('seeking');
  await settle();

  assert.deepEqual(userSeeks, [25], 'a seek during setup is user intent');
  assert.deepEqual(video.seeks, [], 'the run in flight has not parked the element yet');

  release();
  await run;
  await settle(10);

  assert.equal(calls.length, 2, 'the stored target starts its run once setup finishes');
  assert.match(calls[1], /start=25\.000/);

  player.destroy();
});

test('a refused pending-seek run is started at the newest target, not the parked element', async () => {
  const video = fakeVideo();
  const calls = [];
  let releaseFirst;
  const firstRun = new Promise((resolve) => {
    releaseFirst = resolve;
  });
  const userSeeks = [];
  globalThis.fetch = (url) => {
    calls.push(url);
    // The run the user's newest target starts is the one the saturated pool
    // refuses.
    if (url.includes('start=25.000')) {
      return Promise.resolve({
        ok: false,
        status: 503,
        headers: new Headers({ 'retry-after': '1' }),
        body: null,
      });
    }
    // The earlier run is held at the request: the window in which the user is
    // free to move the scrubber past what that run will deliver.
    return firstRun.then(() => {
      let reads = 0;
      const reader = {
        read() {
          reads += 1;
          return reads === 1
            ? Promise.resolve({ done: false, value: new Uint8Array([0, 0, 0, 24]) })
            : new Promise(() => {});
        },
      };
      return { ok: true, status: 200, headers: new Headers(), body: { getReader: () => reader } };
    });
  };
  const errors = [];
  const player = createPlayer(video, {
    duration: 0,
    onError: (error) => errors.push(error),
    onUserSeek: (target) => userSeeks.push(target),
  });

  // The drag-scrub to 25 s lands while the run for 10 s is still setting up, so
  // it is stored for that run — which then attaches and parks the element on
  // ITS offset, the older one.
  const run = player.start(10);
  await settle(2);
  video._currentTime = 25;
  video.dispatch('seeking');
  await settle(2);
  releaseFirst();
  await settle(20);

  // The element stands on the earlier run's parked offset, which is precisely
  // what a retry reading the element would resume at — discarding the seek it
  // was armed to resume. The run the stored target started asks for the offset
  // the user actually picked, and the user seek that left the parked offset was
  // reported, so a viewer that disarms on that report resumes this offset and
  // nothing else.
  assert.equal(video.currentTime, 10, 'the earlier run parked the element on its own offset');
  assert.deepEqual(userSeeks, [25], 'the user seek was reported, never the parked offset');
  assert.equal(calls.length, 2, 'the stored target started its own run');
  assert.match(calls[1], /start=25\.000/);
  assert.equal(errors.length, 1, 'the refusal reached the viewer');
  assert.equal(errors[0].status, 503);

  player.destroy();
  await run;
});

test('a target stored by a failed run cannot steer a later run', async () => {
  const video = fakeVideo();
  const calls = [];
  let refuseFirst;
  const firstResponse = new Promise((resolve) => {
    refuseFirst = resolve;
  });
  globalThis.fetch = (url) => {
    calls.push(url);
    // The first run is the one the saturated pool refuses — but only after the
    // seek below has landed inside its setup window, so the target is stored
    // against a run that then dies before it can hand it off.
    if (url.includes('start=0.000')) {
      return firstResponse.then(() => ({
        ok: false,
        status: 503,
        headers: new Headers({ 'retry-after': '1' }),
        body: null,
      }));
    }
    // Every later run delivers its initialization segment and then holds: it
    // has attached and parked the element on its offset, so its hand-off is the
    // moment a leftover target would surface.
    let reads = 0;
    const reader = {
      read() {
        reads += 1;
        return reads === 1
          ? Promise.resolve({ done: false, value: new Uint8Array([0, 0, 0, 24]) })
          : new Promise(() => {});
      },
    };
    return Promise.resolve({
      ok: true,
      status: 200,
      headers: new Headers(),
      body: { getReader: () => reader },
    });
  };
  const player = createPlayer(video, { duration: 0 });

  // The drag to 25 s lands while the run for 0 s is still waiting on the
  // server, so the target is stored for that run. The refusal then ends the run
  // before it ever attaches, let alone hands a target off — the target is left
  // behind with nothing that owns it.
  const run = player.start(0);
  await settle(2);
  video._currentTime = 25;
  video.dispatch('seeking');
  await settle(2);
  refuseFirst();
  await run;
  await settle(5);
  assert.equal(calls.length, 1, 'the refused run issued its request and nothing else');
  assert.deepEqual(video.seeks, [], 'the refused run never attached, so it parked nothing');

  // The user picks a new position. Its run is the current intent, and the
  // target the refused run stored must not supersede it: the element already
  // stands on the new target, and a run started at the stale one would drag it
  // back to an offset the user has left.
  video._currentTime = 40;
  video.dispatch('seeking');
  await settle(20);

  assert.equal(calls.length, 2, 'the stale target did not start a run of its own');
  assert.match(calls[1], /start=40\.000/, 'the newest target is the one played');
  assert.deepEqual(video.seeks, [], 'no run parked the element on the abandoned offset');

  player.destroy();
  await run;
});

test('a waiting of a later run that has not played is not a stall', async () => {
  const video = fakeVideo();
  const states = [];
  const { fetchImpl, calls, release } = initSegmentFetch();
  globalThis.fetch = fetchImpl;
  const player = createPlayer(video, { onState: (value) => states.push(value) });

  const run = player.start(0);
  await settle(10);
  // The first run plays: the element decoded media and its clock is running.
  video.dispatch('playing');
  assert.deepEqual(states, ['buffering', 'playing'], 'the first run noticed once, then played');

  // The user seeks outside the buffered range, which attaches a fresh run. That
  // run starts from scratch — an initialization segment and nothing else — so
  // it has not played anything yet: its `waiting` is the normal no-data state
  // of a run filling its buffer, exactly like the first run's was. Only the
  // reset of `startedPlaying` at the run's top keeps the flag the previous run
  // set from surviving into this one, where `onWaiting` would report a stall
  // the element never had — and the viewer reads `buffering` as liveness
  // (it clears `streamWaiting` and shows the preparing notice over media that
  // has not started).
  video._currentTime = 15;
  video.dispatch('seeking');
  await settle(10);
  assert.equal(calls.length, 2, 'the seek started its own run');
  assert.deepEqual(
    states,
    ['buffering', 'playing', 'buffering'],
    'the restart run announces its own start-up once'
  );

  video.dispatch('waiting');
  assert.deepEqual(
    states,
    ['buffering', 'playing', 'buffering'],
    'a waiting before this run played is not a stall of the run it replaced'
  );

  release();
  player.destroy();
  await run;
});

test('the slot-wait notice is armed before the stream request is issued', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const video = fakeVideo();
  const states = [];
  const { fetchImpl, calls } = fakeFetch();
  let release;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  const requested = [];
  // A response held back until `release()`, with the request itself recorded as
  // it is issued: `calls` (pushed by `fakeFetch` when it builds the response)
  // stays empty for as long as the server has not answered.
  globalThis.fetch = (url) => {
    requested.push(url);
    return gate.then(() => fetchImpl(url));
  };
  const player = createPlayer(video, { onState: (value) => states.push(value) });

  const run = player.start(0);
  await settle(2);
  assert.equal(requested.length, 1, 'the run issued its request');
  assert.equal(calls.length, 0, 'the server has not answered yet');
  assert.deepEqual(states, [], 'nothing is announced before the notice is due');

  // The hold the notice names happens BEFORE a response exists — a saturated
  // pool keeps the request pending for its whole queue wait, with neither
  // headers nor bytes — so the notice has to be reachable right here. Armed
  // after the response resolved, this timer could only ever fire for a run that
  // already holds a conversion slot and is slow to emit its first fragment.
  t.mock.timers.tick(1499);
  assert.deepEqual(states, [], 'the notice waits out its delay');
  t.mock.timers.tick(1);
  assert.deepEqual(states, ['waiting'], 'the notice tracks the hold, not a slow first fragment');

  release();
  await settle(10);
  assert.deepEqual(
    states,
    ['waiting', 'buffering', 'ended'],
    'the first bytes replace the notice, and the run then plays out'
  );

  player.destroy();
  await run;
});

test('a refused run leaves no slot-wait notice behind', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const video = fakeVideo();
  const states = [];
  const errors = [];
  globalThis.fetch = () =>
    Promise.resolve({ ok: false, status: 503, headers: new Headers(), body: null });
  const player = createPlayer(video, {
    onState: (value) => states.push(value),
    onError: (error) => errors.push(error),
  });

  await player.start(0);
  await settle();
  assert.equal(errors.length, 1, 'the refusal reached the viewer');
  assert.deepEqual(states, [], 'a refused run announces nothing');

  // The notice armed for that run went with it. Firing its timer now would put
  // "waiting for a free conversion slot" on top of the failure the viewer is
  // already handling, and re-arm the escape hatch the retry just took down.
  t.mock.timers.tick(5000);
  assert.deepEqual(states, [], "the dead run's notice never fires");

  player.destroy();
});

test('a refused autoplay does not fail the run', async () => {
  const video = fakeVideo();
  const errors = [];
  const { fetchImpl, calls, release } = initSegmentFetch();
  globalThis.fetch = fetchImpl;
  // The viewer opened this run without a user activation (a `?photo=…` deep
  // link, a reload), so the browser's autoplay policy refuses audible
  // playback. The conversion behind the stream is perfectly healthy, and the
  // element keeps its native `controls` for a one-click start — reporting this
  // as a failure would tear the run down, climb the viewer's ladder and end on
  // "video conversion failed" for a stream that was about to play.
  let playCalls = 0;
  video.play = () => {
    playCalls += 1;
    return Promise.reject(
      new DOMException(
        'play() failed because the user did not interact with the document',
        'NotAllowedError'
      )
    );
  };
  const player = createPlayer(video, { duration: 0, onError: (error) => errors.push(error) });

  let settled = false;
  const run = player.start(0).then(
    () => {
      settled = true;
    },
    () => {
      settled = true;
    }
  );
  await settle(10);

  assert.equal(playCalls, 1, 'the run reached playback');
  assert.equal(errors.length, 0, 'a refused autoplay is not a stream failure');
  assert.equal(settled, false, 'the run stays live');
  assert.equal(calls.length, 1, 'no replacement run is started');
  assert.equal(video.listenerCount('seeking'), 1, 'the player is not torn down');

  player.destroy();
  release();
  await run;
});

test('any other play() rejection still fails the run', async () => {
  const video = fakeVideo();
  const errors = [];
  globalThis.fetch = fakeFetch().fetchImpl;
  // Only `AbortError` (a superseded source) and `NotAllowedError` (the
  // autoplay policy) are the element declining to start; every other rejection
  // is the run's own failure and must reach the viewer.
  const rejection = new DOMException('the media could not be played', 'NotSupportedError');
  video.play = () => Promise.reject(rejection);
  const player = createPlayer(video, { onError: (error) => errors.push(error) });

  await player.start(0);
  await settle();

  assert.equal(errors.length, 1, 'the failure reaches the viewer');
  assert.equal(errors[0], rejection, 'the viewer gets the rejection itself');

  player.destroy();
});
