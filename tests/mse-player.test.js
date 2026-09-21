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
    this.buffered = { length: 0 };
    this._end = 0;
    this.appended = [];
    this.removals = [];
    this.offsetAssignments = [];
    this.listeners = new Map();
    this._offset = 0;
    this._followUpStarted = false;
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
    this._end = this.appended.length;
    this.buffered = { length: 1, start: () => 0, end: () => this._end };
    // Chromium grows the media source's duration to the end of the media it is
    // handed: appending a fragment that runs past the declared duration is how
    // a 20.02 s source starts reporting 21 s.
    if (typeof this.mediaSource?.duration === 'number') {
      this.mediaSource.duration = Math.max(
        this.mediaSource.duration,
        this.buffered.end(this.buffered.length - 1)
      );
    }
    this.updating = true;
    setImmediate(() => this._completeUpdate());
  }

  /**
   * Chromium's removal contract: dropping a range takes the buffer offline and
   * refuses to start while an update is already running (which is also why the
   * duration cannot shrink below the buffered end until the tail is gone).
   */
  remove(start, end) {
    if (this.updating) {
      throw new DOMException(
        'The SourceBuffer is updating and cannot be removed from.',
        'InvalidStateError'
      );
    }
    this.removals.push([start, end]);
    this._end = Math.min(this._end, start);
    this.buffered = { length: 1, start: () => 0, end: () => this._end };
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

/** A fetch whose body yields three chunks and then ends. */
function fakeFetch() {
  const calls = [];
  const fetchImpl = (url) => {
    calls.push(url);
    let reads = 0;
    const reader = {
      read() {
        reads += 1;
        if (reads <= 3) {
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
  });
}

test('the player does not treat its own seek as user intent', async () => {
  const video = fakeVideo();
  const { fetchImpl, calls } = fakeFetch();
  globalThis.fetch = fetchImpl;
  const player = createPlayer(video);

  await player.start(0);
  await settle();

  // A `seeking` event for the position the player itself controls (here the
  // element reporting 0 while nothing is buffered yet) must NOT rebuild the
  // MediaSource: that loop leaves the element paused at 0 with the first frame
  // on screen, which is exactly the playback hang this guards against.
  video.dispatch('seeking');
  await settle();

  assert.equal(calls.length, 1, 'exactly one stream run');
  assert.equal(video.playCalls, 1);
  assert.equal(video.listenerCount('playing'), 1);

  player.destroy();
  assert.equal(video.listenerCount('seeking'), 0, 'destroy removes its listeners');
  assert.equal(video.listenerCount('playing'), 0);
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

  // The restart positions the element itself; that seek must not loop either.
  await settle();
  video.dispatch('seeking');
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
  assert.equal(errors[0].startAt, 0, 'the run reports the offset it asked for');
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

test('a refused run carries its start offset so the retry resumes the seek', async () => {
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
  assert.equal(errors[0].status, 503);
  // Without the offset the viewer's retry would restart the video at 0:00 and
  // silently throw the user's seek away.
  assert.equal(errors[0].startAt, 15, 'the retry can resume at the seek target');
  assert.equal(errors[0].retryAfterMs, 2000);

  player.destroy();
});
test('appended media cannot inflate the declared duration', async () => {
  const video = fakeVideo();
  globalThis.fetch = fakeFetch().fetchImpl;
  const player = createPlayer(video);

  await player.start(0);
  await settle();

  // The declared duration is what the server measured on the source, so the
  // timeline starts out exact — the element's seek bar must not be 0:00 → ∞.
  assert.equal(createdSources.at(-1).duration, 2, 'the initial duration is the declared one');

  await settle(10);

  // The run's fragments end past the declared duration (the fetch yields three
  // 1 s chunks for a 2 s source, which is what a stream-copied seek restart
  // looks like): the media source adopts 3 s as its duration and would report
  // it from the element, so the player drops the phantom tail and re-asserts
  // the declared value.
  const buffer = createdSources.at(-1).sourceBuffers[0];
  assert.equal(buffer.appended.length, 3, 'the scenario is exercised');
  assert.deepEqual(buffer.removals, [[2, 3]], 'media past the declared end is dropped');
  assert.equal(buffer.buffered.end(0), 2, 'the buffer no longer runs past the source');
  assert.equal(createdSources.at(-1).duration, 2, 'buffered excess must not grow the timeline');

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
  const { fetchImpl, calls } = fakeFetch();
  globalThis.fetch = fetchImpl;
  const errors = [];
  const player = createPlayer(video, { onError: (error) => errors.push(error) });

  await player.start(0);
  await settle(10);
  const firstRun = createdSources.at(-1).sourceBuffers[0];

  // A user seek restarts the stream: the old run is superseded, its media is
  // gone, and an error it reports late belongs to a mode the viewer left.
  video._currentTime = 15;
  video.dispatch('seeking');
  await settle();

  firstRun.fire('error');
  await settle();

  assert.equal(errors.length, 0, 'a superseded run must not fail the current one');
  assert.equal(calls.length, 2, 'the seek restart is still the current run');

  // The run now attached still reports its own failure.
  createdSources.at(-1).sourceBuffers[0].fire('error');
  await settle();
  assert.equal(errors.length, 1, 'the current run reports once');

  player.destroy();
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
