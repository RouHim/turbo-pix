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
 * `permanent` marks the one `503` no retry can clear: the server's conversion
 * pool is disabled (`TURBO_PIX_MAX_TRANSCODES=0`), which it names in the
 * refusal body. Waiting for a slot that will never free is a lie the viewer
 * must not tell, so it keys on this instead of on the status alone.
 *
 * The refusal carries the server's `Retry-After` pacing hint when it sent one,
 * and deliberately nothing about the offset this run asked for: the viewer
 * keeps its own newest-intent offset and resumes the position the user picked,
 * which a run the server held for its whole queue wait can be far behind.
 */
export class StreamHttpError extends Error {
  /**
   * @param {number} status
   * @param {{retryAfterMs?: number|null, permanent?: boolean}} [details]
   */
  constructor(status, { retryAfterMs = null, permanent = false } = {}) {
    super(`stream HTTP ${status}`);
    this.name = 'StreamHttpError';
    this.status = status;
    this.retryAfterMs = retryAfterMs;
    this.permanent = permanent;
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

/**
 * Whether a refusal is one no retry can clear.
 *
 * `503` covers two facts on the stream endpoint: the worker pool is *saturated*
 * (every worker busy — wait for a free slot, the `Retry-After` paces when) and
 * the pool is *disabled* (`TURBO_PIX_MAX_TRANSCODES=0`, no worker will ever
 * exist). Both answer the same status and both send a `Retry-After`, and the
 * body is the only carrier that tells them apart — the disabled arm names
 * itself there (`{"error": "conversion disabled"}`). Anything unreadable or
 * unrecognised counts as saturation, which is the answer the viewer already
 * had.
 *
 * @param {Response} response
 * @returns {Promise<boolean>}
 */
async function refusesPermanently(response) {
  try {
    const body = await response.json();
    return body?.error === 'conversion disabled';
  } catch {
    return false;
  }
}

function once(target, event) {
  return new Promise((resolve) => target.addEventListener(event, resolve, { once: true }));
}

/**
 * `onRunStart` reports every run the player begins, with the offset it plays
 * from, at the very top of the run — before its request is issued and before
 * anything it awaits. It covers the run a caller asked for *and* the run the
 * player restarts for a seek inside itself, which a caller that armed
 * something for the previous run (the viewer's saturation retry) cannot
 * otherwise observe until that run delivers its first bytes.
 *
 * `onUserSeek` reports the position the user picked on the element itself, for
 * every `seeking` event that is not the assignment this player makes: the ones
 * that reach the pending-seek store or the buffered-range check. A target the
 * element can already serve starts no run, so it is invisible to the run
 * reports — and it is exactly the move that tells a caller the offset its arming
 * waited for is one the user has left. It never fires for the player's own
 * `currentTime` assignment, which a caller reading as intent would act on for
 * every run.
 *
 * @param {HTMLVideoElement} videoEl
 * @param {{streamUrl: string, mime: string, duration: number|null, onState: (state: string) => void, onError: (error: Error) => void, onEncoder: (encoder: string|null) => void, onRunStart: (seconds: number) => void, onUserSeek: (seconds: number) => void}} options
 */
export function createStreamPlayer(
  videoEl,
  { streamUrl, mime, duration, onState, onError, onEncoder, onRunStart, onUserSeek }
) {
  let mediaSource = null;
  let sourceBuffer = null;
  let controller = null;
  let destroyed = false;
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
  // in a loop and leave the element paused at 0. It is armed together with the
  // assignment (a run that moves nothing arms nothing) and retired by the first
  // `seeking` that arrives, whether or not it matches (see `onSeeking`).
  let expectedSeek = null;
  // One failure report per run: a bad delivery raises `error` on both the
  // SourceBuffer and the element, and each report would otherwise advance the
  // viewer's escalation ladder a step.
  let failureReported = false;
  // Whether the element has actually started playing the current run. The
  // element's `waiting` is what reports a genuine re-entry into buffering — and
  // it also fires while a run sets up, where "no data yet" is simply the normal
  // state, so only a `waiting` after playback is a stall.
  let startedPlaying = false;
  // Tears down the current run's media-error listeners.
  let detachRunErrors = null;
  // The signal of the run that is pumping right now, or null when no pump loop
  // is alive. A pump parked on the look-ahead window still counts: its own loop
  // trims the played-out media as it resumes, so a playhead move does not have
  // to (and must not, on that same buffer, at the same time).
  let pumpSignal = null;
  // The run whose media is attached to the element. A playhead move after that
  // run's pump has ended is what trims its media the rest of the way out (see
  // `onPlayheadMoved`); the signal tells the trim whether the run is still the
  // live one.
  let liveBuffer = null;
  let liveSignal = null;
  // The pumps parked on the look-ahead window. They are woken by the element's
  // own playhead moving and by this player going away — never by a timer: a
  // parked pump must cost nothing while the viewer sits still.
  const lookAheadWaiters = new Set();

  // The declared duration of the SOURCE, when the server knows it. The media
  // source grows its own duration to the end of the media appended to it, so a
  // stream-copied fragment whose last packet lands past the source's duration
  // (common on a seek restart) inflates it: a 20.02 s video reads 21 s.
  const declaredDuration =
    typeof duration === 'number' && Number.isFinite(duration) && duration > 0 ? duration : null;

  // How much already-played media to keep behind the playhead. Chromium frees
  // nothing by itself, and the server pipes the whole run out at ffmpeg speed:
  // without eviction a 20-minute 1080p source leaves ~450 MB of coded frames
  // held by the renderer, and on a client that caps MSE memory the appends
  // start failing — reported as a failed playback, complete with the viewer's
  // ladder and "play original anyway", for a video that was streaming fine.
  // Media this far back is a whole run to replace (the player restarts the
  // stream for any seek the buffer no longer covers).
  const BUFFER_WINDOW_SECONDS = 30;
  // Evict once this much played-out media has accumulated, rather than issuing
  // a SourceBuffer update per appended chunk.
  const EVICT_STEP_SECONDS = 10;
  // How far ahead of the playhead the pump may read the body — the mirror of
  // BUFFER_WINDOW_SECONDS on the read side. The server sends a run unpaced (no
  // `-re`) and Chromium frees nothing by itself, so a pump that reads as fast
  // as it can retains `duration x (1 - 1/speed)`: the whole file for a remux
  // delivered in seconds, which is the growth the append failures above come
  // from. Reading stops while this much media sits buffered ahead of the viewer
  // and resumes when playback has drained it, which bounds the retained bytes
  // whatever the delivery speed.
  //
  // Not reading has a server-side consequence, and it is deliberate: the run's
  // pipe backs up, and `video_stream.rs`'s watchdog kills any run that hands
  // out no chunk for `TURBO_PIX_TRANSCODE_TIMEOUT_SECS` (default 300 s) — the
  // doc comment there names "a client that stopped reading so the pipe backs
  // up" as exactly this case. A viewer paused for longer than that therefore
  // loses its run: the body ends before the declared duration, which this
  // module reports as the ordinary truncation failure — tagged `lostRun`,
  // because a run that stopped is not a run that failed to decode — and the
  // viewer recovers from there. A parked pump must
  // consequently never swallow a body that ended — see `waitForLookAhead` —
  // or a killed run would wedge the player instead of recovering.
  const LOOK_AHEAD_SECONDS = 30;

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
   * How far the write head runs ahead of the viewer, in seconds: the media the
   * run has buffered past the element's current position. Zero while nothing is
   * buffered, negative once playback has run past what is buffered.
   */
  function bufferedAhead(buffer) {
    if (buffer.buffered.length === 0) return 0;
    return buffer.buffered.end(buffer.buffered.length - 1) - videoEl.currentTime;
  }

  /**
   * The element's playhead moving once, or an immediate resolve when this
   * player is going away — `destroy()`, or a run that supersedes the parked one
   * — so that no pump is ever left parked on an element nothing will move
   * again.
   */
  function playheadMoved() {
    if (destroyed) return Promise.resolve();
    return new Promise((resolve) => {
      lookAheadWaiters.add(resolve);
    });
  }

  /**
   * Wake every parked pump. A wake is not a claim that the window has drained:
   * each pump re-checks its own window, `destroyed` and its run's signal.
   */
  function wakeLookAheadWaiters() {
    for (const resolve of lookAheadWaiters) resolve();
    lookAheadWaiters.clear();
  }

  /**
   * Park until the media buffered ahead of the playhead has drained into
   * `LOOK_AHEAD_SECONDS` — the read-side half of the buffer bound.
   *
   * `read` is the body read already in flight. A chunk landing is deliberately
   * NOT a way out of the wait while the window is full: appending it anyway is
   * the unbounded growth this bound exists to stop, so it is held instead — one
   * chunk, never more, because no further read is issued from here. A body that
   * ENDS is a way out, and has to be: the run is over (the server killed it, or
   * it delivered everything), the caller has to act on that — the truncation
   * report for the killed run included — and a pump parked on an element that
   * may never move again would never get there.
   */
  async function waitForLookAhead(read, buffer, signal) {
    let ended = false;
    read.then(
      ({ done }) => {
        ended = done;
        // A body that ends wakes the wait it parked: the flag alone is only
        // read when the loop comes round again, and nothing else would bring it
        // round. A chunk landing deliberately does not wake it — the window is
        // full, so there is nothing to do with it yet.
        if (done) wakeLookAheadWaiters();
      },
      () => {
        // The read itself failed: the caller's `await read` reports it.
        ended = true;
        wakeLookAheadWaiters();
      }
    );
    while (!destroyed && !signal.aborted && !ended && bufferedAhead(buffer) > LOOK_AHEAD_SECONDS) {
      await playheadMoved();
    }
  }

  /**
   * Drop the media `BUFFER_WINDOW_SECONDS` behind the playhead, so a long run
   * cannot hold everything it ever appended.
   *
   * Runs where the appends do, i.e. serialized with them: a `remove` issued
   * while an update is in flight throws, and the element's own seek can start
   * an internal update between the check and the call, so `InvalidStateError`
   * is retried exactly as `appendWhenReady` retries it. Never removes ahead of
   * `currentTime` — a seek forward into buffered media starts no new run, and
   * dropping that media would make an ordinary scrub re-convert what the
   * viewer has already streamed — and never while this run is superseded, whose
   * buffer the replacement has taken over. A buffer that cannot evict (no
   * `remove` method at all, as the MSE spec allows for some types) keeps its
   * media rather than failing the run: growth is the lesser fault.
   *
   * Also driven from the element's playhead once a run's pump has ended (see
   * `onPlayheadMoved`): a run that delivered in full leaves the media behind the
   * viewer in the buffer, and nothing else would ever drop it.
   */
  async function evictPlayedOut(buffer, signal) {
    if (typeof buffer.remove !== 'function') return;
    // A buffer whose media source is gone — the element moved to a newer run's
    // source, the viewer closed — answers `buffered` with a throw ("This
    // SourceBuffer has been removed from the parent media source") rather than
    // an empty range. It holds nothing to evict, and a trim is never worth
    // failing a run over, so it is treated exactly like a buffer that cannot
    // evict at all.
    let start;
    let end;
    try {
      if (buffer.buffered.length === 0) return;
      start = buffer.buffered.start(0);
      end = buffer.buffered.end(buffer.buffered.length - 1);
    } catch {
      return;
    }
    const cutoff = Math.min(videoEl.currentTime - BUFFER_WINDOW_SECONDS, end);
    // Nothing played out yet (or not enough of it to be worth an update of its
    // own): the next chunks widen the span.
    if (!Number.isFinite(cutoff) || cutoff - start < EVICT_STEP_SECONDS) return;
    for (let attempt = 0; attempt < 2; attempt += 1) {
      while (buffer.updating) await once(buffer, 'updateend');
      if (destroyed || signal.aborted) return;
      try {
        buffer.remove(start, cutoff);
      } catch (error) {
        if (error.name === 'InvalidStateError') continue;
        return;
      }
      return;
    }
  }

  /**
   * Feed one stream run's chunks into that run's own SourceBuffer, until the
   * stream ends or the run is superseded (`signal` aborted by a newer run or
   * by `destroy()`). Without the signal check a superseded pump would keep
   * appending the previous photo's/offset's media into the new buffer.
   *
   * The pump reads at most `LOOK_AHEAD_SECONDS` ahead of the playhead (see the
   * constant): the read is issued before the wait and held unappended until the
   * window drains, so the body's end stays observable from inside the wait.
   */
  async function pump({ reader, buffer, source, signal }) {
    for (;;) {
      const read = reader.read();
      await waitForLookAhead(read, buffer, signal);
      if (destroyed || signal.aborted) return;
      const { done, value } = await read;
      if (destroyed || signal.aborted) return;
      if (done) {
        // The chunked body ends cleanly even when the run did not: the server
        // kills the conversion at its deadline and ffmpeg can die mid-stream,
        // and both simply close the response. Telling that apart from a
        // finished run needs the declared duration — a body that ends with the
        // buffer still short of it stopped part-way, and the viewer must see
        // the failure (the ladder and "play original anyway") instead of the
        // silent stop a bogus `ended` produces. The slack absorbs the last
        // fragment's rounding, but never more than half the timeline: a flat
        // 2 s is wider than the whole of any source at or under 2 s, which made
        // the comparison unsatisfiable there — a run that buffered nothing at
        // all (ffmpeg dying before its first fragment, the server killing the
        // conversion at its deadline) fell through to a clean `ended`. Bounded
        // this way the threshold stays above 0 for every duration, so an empty
        // buffer — its end is 0, or absent entirely — is always a failure,
        // whatever the duration.
        //
        // A run that started at or past the declared duration is not that case:
        // the element clamps a seek to the end of the clip onto the media
        // duration, the server still answers that offset with a stream head,
        // and the clamp strips the fragment that landed past the end — so the
        // buffer is empty by construction and the run has nothing left to
        // deliver. Judged as a truncation it escalated the viewer's ladder for a
        // position that cannot play anything and ended on a misleading "Video
        // conversion failed", so only a run whose own start offset is strictly
        // before the end is judged here.
        const bufferedEnd =
          buffer.buffered.length > 0 ? buffer.buffered.end(buffer.buffered.length - 1) : 0;
        if (
          declaredDuration !== null &&
          !signal.aborted &&
          buffer.timestampOffset < declaredDuration
        ) {
          const slack = Math.min(2, declaredDuration / 2);
          if (bufferedEnd < declaredDuration - slack) {
            // A body that ends short of the declared duration is no verdict on
            // the bytes it carried: the server's stall watchdog kills a run
            // whose client stopped reading (a parked pump, the pipe backed
            // up), a crash and a broken pipe all close the chunked response
            // exactly like this. Undecodable bytes report themselves through
            // the SourceBuffer's and the element's own `error` events instead
            // (see `onMediaError`), which is a different signal. The tag is
            // what lets the viewer tell the two apart: a LOST run is replayed
            // on its own mode at the position the viewer reached, never
            // escalated into a heavier conversion that restarts the video at
            // 0:00.
            const lostRun = new Error('the delivered stream ended early');
            lostRun.lostRun = true;
            reportError(signal, lostRun);
            return;
          }
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
      await evictPlayedOut(buffer, signal);
      if (destroyed || signal.aborted) return;
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
    // Whatever target is stored at this point was seen by a run that has
    // already ended: a run's setup is serialized by the `starting` latch, and
    // only the run that stored a target reads it back — `takePendingSeek`, at
    // the hand-off after it has attached. A run that fails before that
    // hand-off (a refused request, a zero-byte body, a rejected `fetch`) leaves
    // its target behind, and the viewer keeps this same player alive through a
    // 503 retry — so the stale target would outlive its run and, at the next
    // run's hand-off, differ from the offset that run attached by more than the
    // tolerance: `start(stale)` would supersede the run the user just asked for
    // and park the element on an offset they have already left. Dropping it
    // here cannot touch a legitimate hand-off, which reads the target it stored
    // before the latch was ever released; `starting` keeps any other run from
    // beginning in the meantime.
    pendingSeek = null;
    // Every run this player begins supersedes the last one, and the caller has
    // to hear about the replacement before this run can be awaited on: a
    // viewer that armed a retry for an earlier, refused run must not have that
    // retry fire into this run and drag playback back to the older offset.
    //
    // The report is the only foreign code in this prologue, so it is the only
    // one that can throw at it — and it has to leave the run latch released
    // when it does, exactly like the run's own failure path below: a rejected
    // `start()` that kept `starting` would make every later `start()` and every
    // seek restart a silent no-op, freezing the element on its last frame with
    // `destroy()` as the only way out. The throw reaches the caller as this
    // run's rejection (nothing was started, so nothing is reported as a run
    // failure); the callback stays ahead of the request, as the ordering
    // contract requires.
    try {
      onRunStart?.(seconds);
    } catch (error) {
      starting = false;
      throw error;
    }
    controller?.abort();
    controller = new AbortController();
    const signal = controller.signal;
    // The run this one supersedes may be parked on the look-ahead window, and
    // nothing in the element would wake it before its own body ends: wake it
    // here so the run being replaced releases its body and exits at once. Its
    // buffer stops being the live one at the same moment: this run detaches the
    // old source below, and until this run has attached its own, a playhead move
    // would trim a buffer that is no longer the element's.
    wakeLookAheadWaiters();
    liveBuffer = null;
    liveSignal = null;
    // The run about to be attached owns the failures from here on: whatever the
    // superseded run reports late belongs to a mode the viewer already left.
    detachRunErrors?.();
    detachRunErrors = null;
    failureReported = false;
    // A run that has only just been attached has not produced a playable frame:
    // a `waiting` from the element now is the normal no-data state, not a stall.
    startedPlaying = false;
    try {
      mediaSource?.endOfStream?.();
    } catch {
      /* already closed */
    }

    let reader;
    let buffer;
    // Show "waiting for a free conversion slot" if the server holds this run
    // open. The hold happens BEFORE a response exists: while every worker is
    // busy the endpoint awaits a free permit for its whole queue wait
    // (`TURBO_PIX_STREAM_QUEUE_WAIT_SECS`) and only then answers — no headers,
    // no bytes — so the timer is armed ahead of the request. Armed after the
    // response resolved it could only ever fire for a run that already holds a
    // slot and is slow to emit its first fragment, leaving the window the
    // notice names uncovered (and a seek-restarted run with no notice at all).
    //
    // The timer belongs to this run: it is cleared on the first bytes and on
    // every exit path below, so a superseded run cannot disarm its successor's.
    const slotTimer = setTimeout(() => state('waiting'), 1500);
    try {
      // Fetch before the MediaSource exists: the MIME this run's bytes are in
      // is only known once the server answers, and a refused (503) run must not
      // leave a half-built source behind for the viewer's retry to trip over.
      const response = await fetch(urlFor(seconds), { signal });
      if (!response.ok) {
        // The refusal body is what tells a saturated pool from a disabled one
        // (see `refusesPermanently`), and it is the only carrier that does.
        throw new StreamHttpError(response.status, {
          retryAfterMs: retryAfterMs(response),
          permanent: await refusesPermanently(response),
        });
      }
      if (!response.body) {
        throw new StreamHttpError(response.status, { retryAfterMs: retryAfterMs(response) });
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

      // Every seek outside the buffered range starts a new run, and each one
      // attaches a fresh MediaSource behind a fresh blob URL. The URL being
      // replaced is released as it is replaced: nothing else can free it (the
      // MediaSource, its SourceBuffer and its entry in the document's blob-URL
      // store would otherwise stay alive for the document's lifetime), and
      // `destroy()` only ever gets to revoke the element's *current* URL.
      if (destroyed) return;
      const previousSrc = videoEl.src;
      mediaSource = new MediaSource();
      videoEl.src = URL.createObjectURL(mediaSource);
      if (previousSrc.startsWith('blob:')) URL.revokeObjectURL(previousSrc);
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
      // And what a playhead move trims once this run's pump has ended.
      liveBuffer = buffer;
      liveSignal = signal;

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

      reader = response.body.getReader();
      // First bytes arrived: we are buffering, not waiting.
      const first = await reader.read();
      clearTimeout(slotTimer);
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
      // The run's first bytes are in: the viewer's notice changes from
      // "waiting for a slot" to "preparing playback". This is the run's only
      // `buffering` — a chunk landing is not news (the notice is hidden by
      // `playing`, and re-reporting it after every append would put "video is
      // being prepared" back on top of a video that is already playing, with no
      // auto-hide to take it away). A stall mid-run still surfaces: the element
      // reports it with `waiting`, which `onWaiting` maps back onto this state.
      state('buffering');
      await appendWhenReady(buffer, first.value);
      // That append is a task boundary — `clampDuration` waits on `updateend`
      // for a source whose declared duration is known — so a `destroy()` can
      // land inside it (viewer close, a swipe to another photo, "play original
      // anyway", the ladder's own teardown). Everything below drives the
      // element the viewer just tore down, and the MediaSource stays attached
      // with the fragment this append just buffered, so the dead run would
      // start (or resume) that media behind a closed viewer or over the photo
      // the user moved to. Bail out, like every other await in the run.
      if (destroyed) return;
      // Position the element on the real timeline (each run starts at 0) and
      // remember it, so the `seeking` event this assignment causes is not
      // mistaken for a user seek. The guard belongs to the assignment: only a
      // position the element does not already hold moves it, and only a move
      // fires the `seeking` event that consumes the guard. A run attached where
      // the element already stands (the standard run at 0) moves nothing, so no
      // guard is armed for it — an armed one could never be consumed, and would
      // swallow the first user seek to land inside its window, which is exactly
      // the seek this guard exists to protect.
      if (videoEl.currentTime !== seconds) {
        expectedSeek = seconds;
        videoEl.currentTime = seconds;
      }
      videoEl.play().catch((error) => {
        // Two rejections are the element declining to start, not a failed run.
        // `AbortError`: this play() was superseded by another source.
        // `NotAllowedError`: the browser's autoplay policy refused audible
        // playback because the run was not started by a user gesture (a
        // `?photo=…` deep link or a reload calls for the video without one).
        // The conversion behind the stream is healthy and the element keeps
        // its native `controls`, so one click starts the media — reporting the
        // refusal as a run failure would tear the run down, climb the viewer's
        // ladder and end on "video conversion failed" (with the "play original
        // anyway" hatch serving bytes this browser cannot play at all) for a
        // conversion that worked. Every other rejection is a real failure.
        if (error.name === 'AbortError' || error.name === 'NotAllowedError') return;
        reportError(signal, error);
      });
    } catch (error) {
      if (error.name !== 'AbortError') reportError(signal, error);
      return;
    } finally {
      // Whatever ended this run's setup — its first bytes, a refusal, a network
      // failure, a supersession or `destroy()` — its slot-wait notice must not
      // outlive it: a stale `waiting` would land on top of the failure the
      // viewer is already handling and re-arm the escape hatch it just took
      // down.
      clearTimeout(slotTimer);
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
      pumpSignal = signal;
      await pump({ reader, buffer, source: mediaSource, signal });
    } catch (error) {
      if (error.name !== 'AbortError') reportError(signal, error);
    } finally {
      // Only this run's own pump clears the flag: a superseded pump returning
      // after its replacement has started must not unlock the playhead-driven
      // trim for the replacement's buffer.
      if (pumpSignal === signal) pumpSignal = null;
    }
  }

  // The element reports real playback (data decoded and the clock running);
  // the viewer hides its conversion notice on this, so it must not be
  // optimistic.
  const onPlaying = () => {
    startedPlaying = true;
    state('playing');
  };
  // Playback stalled: the clock was running and the element ran out of media.
  // That is a genuine entry into buffering — the viewer's notice is hidden
  // while the clock runs, so nothing else can surface the wait — and it is the
  // only re-report of the state; the per-chunk report that used to stand here
  // fired exactly when a chunk had just landed, i.e. when data was flowing.
  const onWaiting = () => {
    if (startedPlaying) state('buffering');
  };
  // The element moved its playhead: playback progressed, or a seek landed. Two
  // things follow, and both belong here rather than in the pump loop —
  // `timeupdate` is the only event that keeps firing once a run is over, and a
  // seek can move the playhead while no pump is running at all.
  //
  //   - a pump parked on the look-ahead window re-checks its window: the media
  //     it was waiting to drain is now playable;
  //   - a run whose pump has ended gets its played-out media dropped. A run
  //     that delivered everything before the viewer watched it (the whole of a
  //     short source) would otherwise hold that media in the renderer for the
  //     rest of playback, with nothing left to evict it.
  //
  // The trim stands down while a pump is alive: that pump evicts after every
  // append on the same buffer, and this player never issues two buffer updates
  // from two places at once for one run. `seeked` carries the playhead moves
  // that are not playback, and it is also what retires the own-seek guard.
  const onPlayheadMoved = () => {
    expectedSeek = null;
    wakeLookAheadWaiters();
    if (pumpSignal === null && liveBuffer !== null) evictPlayedOut(liveBuffer, liveSignal);
  };

  // Seeking outside the buffered range restarts the stream at the target.
  const onSeeking = () => {
    if (destroyed) return;
    const target = videoEl.currentTime;
    // The player's own `currentTime` assignment is not user intent: one
    // assignment fires at most one `seeking`, whose target is the offset the
    // guard holds, so a matching event is the player's own seek and is dropped.
    // Any OTHER event is the user's — and it retires the arm it did not
    // consume, so a guard is never left armed across an event: a stale arm
    // (the user coalesced their move with the assignment's own event, or moved
    // to a different offset) would otherwise swallow the next user seek to land
    // inside its 0.25 s window.
    //
    // The match is tried before the `starting` branch on purpose. The run that
    // hands off to a stored `pendingSeek` has already cued its own `seeking`
    // when the replacement run starts, so that event legitimately arrives while
    // `starting` is true; read as a user target it would send the element back
    // to the offset the run asked for, and the two offsets would keep bouncing.
    // Within the window, a target this close is the run's own assignment.
    if (expectedSeek !== null && Math.abs(target - expectedSeek) <= 0.25) {
      expectedSeek = null;
      return;
    }
    expectedSeek = null;
    // Everything past the guard is the user's own move — whether it starts a
    // run (an unbuffered target), needs none (a buffered one) or is stored for
    // the run in flight (a seek during setup). The caller hears about all
    // three: a move to a position is the user leaving whatever offset an
    // arming of theirs was waiting for.
    onUserSeek?.(target);
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
  videoEl.addEventListener('seeked', onPlayheadMoved);
  videoEl.addEventListener('timeupdate', onPlayheadMoved);
  videoEl.addEventListener('playing', onPlaying);
  videoEl.addEventListener('waiting', onWaiting);

  function destroy() {
    destroyed = true;
    pendingSeek = null;
    controller?.abort();
    // Release every parked pump: it re-checks `destroyed` and returns, instead
    // of holding its body open behind a viewer that is gone.
    wakeLookAheadWaiters();
    liveBuffer = null;
    liveSignal = null;
    detachRunErrors?.();
    detachRunErrors = null;
    videoEl.removeEventListener('seeking', onSeeking);
    videoEl.removeEventListener('seeked', onPlayheadMoved);
    videoEl.removeEventListener('timeupdate', onPlayheadMoved);
    videoEl.removeEventListener('playing', onPlaying);
    videoEl.removeEventListener('waiting', onWaiting);
    try {
      mediaSource?.endOfStream?.();
    } catch {
      /* already closed */
    }
    if (videoEl.src.startsWith('blob:')) URL.revokeObjectURL(videoEl.src);
  }

  return { start, destroy };
}
