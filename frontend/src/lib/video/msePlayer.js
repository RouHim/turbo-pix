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

/**
 * The stream endpoint refused a run with an HTTP status. `503` means "no
 * conversion slot right now" — a saturated worker pool the viewer answers by
 * waiting and retrying — while anything else is a real playback failure.
 *
 * The refusal carries everything the retry needs: the offset this run asked for
 * (resuming the same position, not 0:00) and the server's `Retry-After` pacing
 * hint when it sent one.
 */
export class StreamHttpError extends Error {
  /**
   * @param {number} status
   * @param {{startAt?: number, retryAfterMs?: number|null}} [details]
   */
  constructor(status, { startAt = 0, retryAfterMs = null } = {}) {
    super(`stream HTTP ${status}`);
    this.name = 'StreamHttpError';
    this.status = status;
    this.startAt = startAt;
    this.retryAfterMs = retryAfterMs;
  }
}

/**
 * `Retry-After` as milliseconds, or `null` when the server sent no usable hint.
 * The endpoint answers whole seconds; the spec's other form (an HTTP-date) is
 * left to the caller's default rather than mis-read as a delay.
 *
 * @param {Response} response
 * @returns {number|null}
 */
function retryAfterMs(response) {
  const raw = response.headers?.get?.('retry-after');
  if (raw === null || raw === undefined) return null;
  const seconds = Number(String(raw).trim());
  if (!Number.isFinite(seconds) || seconds < 0) return null;
  return seconds * 1000;
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

  /**
   * Append one chunk, waiting out any in-flight update: appending while the
   * buffer updates throws InvalidStateError, and the element's own seek can
   * start an internal update between the check and the call.
   */
  async function appendWhenReady(buffer, value) {
    for (let attempt = 0; attempt < 2; attempt += 1) {
      while (buffer.updating) await once(buffer, 'updateend');
      try {
        buffer.appendBuffer(value);
        return;
      } catch (error) {
        if (error.name !== 'InvalidStateError') throw error;
      }
    }
    throw new Error('SourceBuffer stayed busy');
  }

  /**
   * Feed one stream run's chunks into that run's own SourceBuffer, until the
   * stream ends or the run is superseded (`signal` aborted by a newer run or
   * by `destroy()`). Without the signal check a superseded pump would keep
   * appending the previous photo's/offset's media into the new buffer.
   */
  async function pump({ reader, buffer, source, signal }) {
    for (;;) {
      const { done, value } = await reader.read();
      if (destroyed || signal.aborted) return;
      if (done) {
        try {
          if (source.readyState === 'open') source.endOfStream();
        } catch {
          /* the source may already be closed */
        }
        state('ended');
        return;
      }
      await appendWhenReady(buffer, value);
      if (destroyed || signal.aborted) return;
      if (buffer.buffered.length > 0) state('buffering');
    }
  }

  async function start(seconds) {
    if (destroyed || starting) return;
    starting = true;
    controller?.abort();
    clearTimeout(restartTimer);
    try {
      mediaSource?.endOfStream?.();
    } catch {
      /* already closed */
    }

    let reader;
    let buffer;
    let signal;
    try {
      mediaSource = new MediaSource();
      videoEl.src = URL.createObjectURL(mediaSource);
      await once(mediaSource, 'sourceopen');
      if (destroyed) return;

      if (typeof duration === 'number' && Number.isFinite(duration) && duration > 0) {
        mediaSource.duration = duration;
      }
      buffer = mediaSource.addSourceBuffer(mime);
      buffer.timestampOffset = seconds;
      // The run's own buffer: `isBuffered` (and therefore the seek restart)
      // asks about the media source that is actually attached to the element.
      sourceBuffer = buffer;

      controller = new AbortController();
      signal = controller.signal;
      const response = await fetch(urlFor(seconds), { signal });
      if (!response.ok || !response.body) {
        throw new StreamHttpError(response.status, {
          startAt: seconds,
          retryAfterMs: retryAfterMs(response),
        });
      }
      // Show "waiting for a free conversion slot" when the server holds the
      // request open because every worker is busy.
      restartTimer = setTimeout(() => state('waiting'), 1500);
      reader = response.body.getReader();
      // First bytes arrived: we are buffering, not waiting.
      const first = await reader.read();
      clearTimeout(restartTimer);
      if (destroyed || first.done) return;
      state('buffering');
      await appendWhenReady(buffer, first.value);
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
    // returned. The pump then feeds the rest of the chunks in the background,
    // bailing out as soon as this run is superseded.
    try {
      await pump({ reader, buffer, source: mediaSource, signal });
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
    clearTimeout(restartTimer);
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
