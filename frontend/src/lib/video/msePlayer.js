/**
 * Media Source Extensions playback for server-side streaming conversions.
 *
 * The server pipes fragmented MP4 from ffmpeg; the browser appends the chunks
 * to a SourceBuffer. ffmpeg rebases every stream run to timestamp 0, so a
 * seek is a *new* stream run whose segments are placed on the real timeline
 * with `SourceBuffer.timestampOffset`.
 */

/** @param {string} mime @returns {boolean} */
export function mseSupported(mime) {
  return typeof MediaSource !== 'undefined' && MediaSource.isTypeSupported(mime);
}

function once(target, event) {
  return new Promise((resolve) => target.addEventListener(event, resolve, { once: true }));
}

/**
 * @param {HTMLVideoElement} videoEl
 * @param {{streamUrl: string, mime: string, duration: number|null, onState: (state: string) => void, onError: (error: Error) => void}} options
 */
export function createStreamPlayer(videoEl, { streamUrl, mime, duration, onState, onError }) {
  let mediaSource = null;
  let sourceBuffer = null;
  let controller = null;
  let destroyed = false;
  let restartTimer = null;
  // A start() run is in flight (fetch/MediaSource setup not yet finished):
  // a `seeking` event during that window is noise, never a user seek.
  let starting = false;
  // The position this player assigned itself. The element fires `seeking` for
  // that assignment too; treating it as user intent would restart the stream
  // in a loop and leave the element paused at 0.
  let expectedSeek = null;

  const state = (value) => {
    if (!destroyed) onState?.(value);
  };

  function urlFor(seconds) {
    const separator = streamUrl.includes('?') ? '&' : '?';
    return `${streamUrl}${separator}start=${seconds.toFixed(3)}`;
  }

  function isBuffered(seconds) {
    if (!sourceBuffer || sourceBuffer.buffered.length === 0) return false;
    for (let i = 0; i < sourceBuffer.buffered.length; i += 1) {
      if (seconds >= sourceBuffer.buffered.start(i) && seconds <= sourceBuffer.buffered.end(i)) {
        return true;
      }
    }
    return false;
  }

  async function pump(reader, start) {
    for (;;) {
      const { done, value } = await reader.read();
      if (destroyed) return;
      if (done) {
        try {
          if (mediaSource?.readyState === 'open') mediaSource.endOfStream();
        } catch {
          /* the source may already be closed */
        }
        state('ended');
        return;
      }
      if (sourceBuffer.updating) await once(sourceBuffer, 'updateend');
      if (destroyed) return;
      sourceBuffer.timestampOffset = start;
      sourceBuffer.appendBuffer(value);
      if (sourceBuffer.buffered.length > 0) state('buffering');
    }
  }

  async function start(seconds) {
    if (destroyed || starting) return;
    starting = true;
    controller?.abort();
    if (restartTimer !== null) clearTimeout(restartTimer);
    try {
      mediaSource?.endOfStream?.();
    } catch {
      /* already closed */
    }

    let reader;
    try {
      mediaSource = new MediaSource();
      videoEl.src = URL.createObjectURL(mediaSource);
      await once(mediaSource, 'sourceopen');
      if (destroyed) return;

      if (typeof duration === 'number' && Number.isFinite(duration) && duration > 0) {
        mediaSource.duration = duration;
      }
      sourceBuffer = mediaSource.addSourceBuffer(mime);
      sourceBuffer.timestampOffset = seconds;

      controller = new AbortController();
      const response = await fetch(urlFor(seconds), { signal: controller.signal });
      if (!response.ok || !response.body) {
        throw new Error(`stream HTTP ${response.status}`);
      }
      // Show "waiting for a free conversion slot" when the server holds the
      // request open because every worker is busy.
      restartTimer = setTimeout(() => state('waiting'), 1500);
      reader = response.body.getReader();
      // First bytes arrived: we are buffering, not waiting.
      const first = await reader.read();
      if (restartTimer !== null) clearTimeout(restartTimer);
      if (destroyed || first.done) return;
      state('buffering');
      sourceBuffer.appendBuffer(first.value);
      // Position the element on the real timeline (each run starts at 0) and
      // remember it, so the `seeking` event this assignment causes is not
      // mistaken for a user seek.
      expectedSeek = seconds;
      if (seconds > 0) videoEl.currentTime = seconds;
      videoEl.play().catch((error) => {
        // AbortError: this play() was superseded by another source. Anything
        // else (e.g. a refused autoplay) is a real failure, not a silent one.
        if (!destroyed && error.name !== 'AbortError') onError?.(error);
      });
    } catch (error) {
      if (!destroyed && error.name !== 'AbortError') onError?.(error);
      return;
    } finally {
      // Setup finished (or bailed): `seeking` events are user intent again.
      starting = false;
    }
    // Reaching this point means the stream is live: every bail-out above
    // returned. The pump then feeds the rest of the chunks in the background.
    try {
      await pump(reader, seconds);
    } catch (error) {
      if (!destroyed && error.name !== 'AbortError') onError?.(error);
    }
  }

  // The element reports real playback (data decoded and the clock running);
  // the viewer hides its conversion notice on this, so it must not be
  // optimistic.
  const onPlaying = () => state('playing');
  const onSeeked = () => {
    expectedSeek = null;
  };

  // Seeking outside the buffered range restarts the stream at the target.
  const onSeeking = () => {
    if (destroyed || starting) return;
    const target = videoEl.currentTime;
    if (expectedSeek !== null && Math.abs(target - expectedSeek) <= 0.25) return;
    if (isBuffered(target)) return;
    start(target).catch((error) => !destroyed && onError?.(error));
  };
  videoEl.addEventListener('seeking', onSeeking);
  videoEl.addEventListener('seeked', onSeeked);
  videoEl.addEventListener('playing', onPlaying);

  function destroy() {
    destroyed = true;
    if (restartTimer !== null) clearTimeout(restartTimer);
    controller?.abort();
    videoEl.removeEventListener('seeking', onSeeking);
    videoEl.removeEventListener('seeked', onSeeked);
    videoEl.removeEventListener('playing', onPlaying);
    try {
      mediaSource?.endOfStream?.();
    } catch {
      /* already closed */
    }
    if (videoEl.src.startsWith('blob:')) URL.revokeObjectURL(videoEl.src);
  }

  return { start, destroy };
}
