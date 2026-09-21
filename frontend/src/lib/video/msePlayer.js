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
 * @param {{streamUrl: string, mime: string, duration: number|null, onState: (state: string) => void, onError: (error: Error) => void, onEncoder: (encoder: string|null) => void}} options
 */
export function createStreamPlayer(
  videoEl,
  { streamUrl, mime, duration, onState, onError, onEncoder }
) {
  let mediaSource = null;
  let sourceBuffer = null;
  let controller = null;
  let destroyed = false;
  let restartTimer = null;
  // A start() run is in flight (fetch/MediaSource setup not yet finished). The
  // run has not positioned the element yet, so a `seeking` event now cannot be
  // acted on immediately — it is kept as `pendingSeek` until setup is done.
  let starting = false;
  // The user seek that landed during that window. The run in flight positions
  // the element on ITS OWN offset when it attaches, so dropping the target
  // would snap the scrubber back to the offset the run asked for and lose the
  // position the user picked. The newest target wins; it is consumed once the
  // run that stored it has finished setting up.
  let pendingSeek = null;
  // The position this player assigned itself. The element fires `seeking` for
  // that assignment too; treating it as user intent would restart the stream
  // in a loop and leave the element paused at 0.
  let expectedSeek = null;
  // One failure report per run: a bad delivery raises `error` on both the
  // SourceBuffer and the element, and each report would otherwise advance the
  // viewer's escalation ladder a step.
  let failureReported = false;
  // Tears down the current run's media-error listeners.
  let detachRunErrors = null;

  // The declared duration of the SOURCE, when the server knows it. The media
  // source grows its own duration to the end of the media appended to it, so a
  // stream-copied fragment whose last packet lands past the source's duration
  // (common on a seek restart) inflates it: a 20.02 s video reads 21 s.
  const declaredDuration =
    typeof duration === 'number' && Number.isFinite(duration) && duration > 0 ? duration : null;

  const state = (value) => {
    if (!destroyed) onState?.(value);
  };

  /**
   * Report a failure of the run `signal` belongs to. A superseded run's late
   * error must not reach the viewer: it would escalate a ladder step for a run
   * that is already the escalated one.
   */
  function reportError(signal, error) {
    if (destroyed || signal?.aborted || failureReported) return;
    failureReported = true;
    onError?.(error);
  }

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
   * Drop the buffered media in `[start, end)` and wait for the update to end.
   * Chromium refuses to shrink the duration below the buffered end and names
   * this as the way out, so it is not optional.
   */
  async function removeRange(buffer, start, end) {
    while (buffer.updating) await once(buffer, 'updateend');
    await new Promise((resolve, reject) => {
      const cleanup = () => {
        buffer.removeEventListener('updateend', onEnd);
        buffer.removeEventListener('error', onError);
      };
      const onEnd = () => {
        cleanup();
        resolve();
      };
      const onError = () => {
        cleanup();
        reject(new Error('the buffered tail could not be removed'));
      };
      buffer.addEventListener('updateend', onEnd);
      buffer.addEventListener('error', onError);
      try {
        buffer.remove(start, end);
      } catch (error) {
        cleanup();
        reject(error);
      }
    });
  }

  /**
   * Re-assert the declared duration after an append.
   *
   * Appending media whose end lands past the declared duration makes the media
   * source adopt that later end as its duration, which the element then reports
   * (a 20.02 s source whose seek-restart fragment runs to 21 s reads 21 s, and
   * its seek bar grows past the end of the video). The server's declared
   * duration is the truth for the whole timeline, so put it back — only when it
   * is known, and only once no update is in flight, because the setter refuses
   * to run during one.
   */
  async function clampDuration(buffer) {
    if (declaredDuration === null || destroyed) return;
    while (buffer.updating) await once(buffer, 'updateend');
    if (destroyed || buffer.buffered.length === 0) return;
    const end = buffer.buffered.end(buffer.buffered.length - 1);
    if (end <= declaredDuration) return;
    if (mediaSource?.readyState !== 'open') return;
    try {
      // Chromium will not take the shorter duration while media past it is
      // still buffered ("Setting duration below highest presentation timestamp
      // of any buffered coded frames is disallowed"), so the phantom tail goes
      // first — it is media beyond the end of the source, nothing playable.
      await removeRange(buffer, declaredDuration, end);
      if (destroyed || mediaSource.readyState !== 'open') return;
      mediaSource.duration = declaredDuration;
    } catch {
      /* an update raced this; the next append clamps again */
    }
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
      } catch (error) {
        if (error.name !== 'InvalidStateError') throw error;
        continue;
      }
      await clampDuration(buffer);
      return;
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
        // The chunked body ends cleanly even when the run did not: the server
        // kills the conversion at its deadline and ffmpeg can die mid-stream,
        // and both simply close the response. Telling that apart from a
        // finished run needs the declared duration — a body that ends with the
        // buffer still short of it stopped part-way, and the viewer must see
        // the failure (the ladder and "play original anyway") instead of the
        // silent stop a bogus `ended` produces. The slack absorbs the last
        // fragment's rounding.
        const bufferedEnd =
          buffer.buffered.length > 0 ? buffer.buffered.end(buffer.buffered.length - 1) : 0;
        if (declaredDuration !== null && !signal.aborted && bufferedEnd < declaredDuration - 2) {
          reportError(signal, new Error('the delivered stream ended early'));
          return;
        }
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

  /**
   * The position a user picked while the run in flight was still setting up,
   * when it is not the offset that run attached. Consumed either way: a stored
   * target belongs to the run that saw it, never to a later run.
   */
  function takePendingSeek(attached) {
    if (pendingSeek === null) return null;
    const target = pendingSeek;
    pendingSeek = null;
    return Math.abs(target - attached) > 0.25 ? target : null;
  }

  async function start(seconds) {
    if (destroyed || starting) return;
    starting = true;
    controller?.abort();
    controller = new AbortController();
    const signal = controller.signal;
    // The run about to be attached owns the failures from here on: whatever the
    // superseded run reports late belongs to a mode the viewer already left.
    detachRunErrors?.();
    detachRunErrors = null;
    failureReported = false;
    clearTimeout(restartTimer);
    try {
      mediaSource?.endOfStream?.();
    } catch {
      /* already closed */
    }

    let reader;
    let buffer;
    try {
      // Fetch before the MediaSource exists: the MIME this run's bytes are in
      // is only known once the server answers, and a refused (503) run must not
      // leave a half-built source behind for the viewer's retry to trip over.
      const response = await fetch(urlFor(seconds), { signal });
      if (!response.ok || !response.body) {
        throw new StreamHttpError(response.status, {
          startAt: seconds,
          retryAfterMs: retryAfterMs(response),
        });
      }

      // Each run's SourceBuffer is typed from the MIME the server advertises
      // FOR THAT RUN, never from the decision's: an escalated rung emits
      // different codecs (remux copies the source tokens, audio/transcode
      // re-encode audio to AAC), and Chromium rejects an append whose init
      // segment does not match the buffer's declared type — which would burn
      // the remaining rungs on a delivery that was perfectly playable.
      const advertisedMime = response.headers.get('x-turbopix-mime');
      const runMime = advertisedMime && mseSupported(advertisedMime) ? advertisedMime : mime;

      // Which encoder produced this run's bytes. An absent header means no video
      // encoding happened (a remux or a copy), which the UI renders as "no
      // hint" — never as a CPU conversion.
      const encoderHeader = response.headers.get('x-turbopix-encoder');
      onEncoder?.(encoderHeader && encoderHeader.trim() !== '' ? encoderHeader.trim() : null);

      mediaSource = new MediaSource();
      videoEl.src = URL.createObjectURL(mediaSource);
      await once(mediaSource, 'sourceopen');
      if (destroyed) return;

      if (declaredDuration !== null) {
        mediaSource.duration = declaredDuration;
      }
      buffer = mediaSource.addSourceBuffer(runMime);
      buffer.timestampOffset = seconds;
      // The run's own buffer: `isBuffered` (and therefore the seek restart)
      // asks about the media source that is actually attached to the element.
      sourceBuffer = buffer;

      // The delivered bytes can turn out to be undecodable (a remux the
      // browser cannot actually play). Chromium reports that on the
      // SourceBuffer and on the element — never on `appendBuffer` — so without
      // these listeners the viewer would sit on a frozen frame instead of
      // escalating the mode (FR-009).
      const onMediaError = () => {
        reportError(signal, new Error('the delivered stream failed to decode'));
      };
      buffer.addEventListener('error', onMediaError);
      videoEl.addEventListener('error', onMediaError);
      detachRunErrors = () => {
        buffer.removeEventListener('error', onMediaError);
        videoEl.removeEventListener('error', onMediaError);
      };

      // Show "waiting for a free conversion slot" when the server holds the
      // request open because every worker is busy.
      restartTimer = setTimeout(() => state('waiting'), 1500);
      reader = response.body.getReader();
      // First bytes arrived: we are buffering, not waiting.
      const first = await reader.read();
      clearTimeout(restartTimer);
      if (destroyed) return;
      if (first.done) {
        // A zero-byte source answers 200 with an empty body, and ffmpeg dying
        // before the init segment looks identical: no bytes ever arrived, so
        // there is nothing to play. The viewer must get the failure — and with
        // it the "play original anyway" escape hatch — rather than the
        // non-error buffering notice a silent return leaves up.
        reportError(signal, new Error('the stream delivered no media'));
        return;
      }
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
        if (error.name !== 'AbortError') reportError(signal, error);
      });
    } catch (error) {
      if (error.name !== 'AbortError') reportError(signal, error);
      return;
    } finally {
      // Setup finished (or bailed): `seeking` events are user intent again.
      starting = false;
    }
    // A seek that landed while this run was setting up is newer than the offset
    // the run asked for — and the run has just moved the element back to that
    // older offset. Honour the stored target instead, so the position the user
    // picked wins.
    const pending = takePendingSeek(seconds);
    if (pending !== null) {
      start(pending).catch((error) => reportError(controller?.signal, error));
      return;
    }
    // Reaching this point means the stream is live: every bail-out above
    // returned. The pump then feeds the rest of the chunks in the background,
    // bailing out as soon as this run is superseded.
    try {
      await pump({ reader, buffer, source: mediaSource, signal });
    } catch (error) {
      if (error.name !== 'AbortError') reportError(signal, error);
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
    if (destroyed) return;
    const target = videoEl.currentTime;
    // The player's own `currentTime` assignment is not user intent.
    if (expectedSeek !== null && Math.abs(target - expectedSeek) <= 0.25) return;
    if (starting) {
      // The run in flight will position the element on its own offset, so this
      // target is remembered rather than dropped: it is honoured as soon as
      // that run has attached.
      pendingSeek = target;
      return;
    }
    if (isBuffered(target)) return;
    start(target).catch((error) => reportError(controller?.signal, error));
  };
  videoEl.addEventListener('seeking', onSeeking);
  videoEl.addEventListener('seeked', onSeeked);
  videoEl.addEventListener('playing', onPlaying);

  function destroy() {
    destroyed = true;
    pendingSeek = null;
    clearTimeout(restartTimer);
    controller?.abort();
    detachRunErrors?.();
    detachRunErrors = null;
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
