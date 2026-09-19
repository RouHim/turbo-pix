import { test, afterEach } from 'node:test';
import assert from 'node:assert/strict';
import { createStreamPlayer } from '../frontend/src/lib/video/msePlayer.js';

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

class FakeSourceBuffer {
  constructor(mime) {
    this.mime = mime;
    this.updating = false;
    this.timestampOffset = 0;
    this.buffered = { length: 0 };
    this.appended = [];
  }

  appendBuffer(chunk) {
    this.appended.push(chunk);
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
    const buffer = new FakeSourceBuffer(mime);
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
    return Promise.resolve({ ok: true, status: 200, body: { getReader: () => reader } });
  };
  return { fetchImpl, calls };
}

function createPlayer(video, options = {}) {
  globalThis.MediaSource = FakeMediaSource;
  URL.createObjectURL = () => 'blob:fake-media-source';
  URL.revokeObjectURL = () => {};
  return createStreamPlayer(video, {
    streamUrl: '/api/photos/abc/video/stream?client=h264-8%2Caac&mode=transcode',
    mime: 'video/mp4; codecs="avc1.42E01E,mp4a.40.2"',
    duration: 2,
    onState: options.onState ?? (() => {}),
    onError: options.onError ?? (() => {}),
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
