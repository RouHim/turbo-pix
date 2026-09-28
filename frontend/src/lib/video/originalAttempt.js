/**
 * Decides whether the browser can play a video's original file, from the media
 * element alone — no `canPlayType` guess and no server verdict.
 *
 * The element is already pointed at the original when the attempt is armed. A
 * decoded frame is the proof; an element error is the refutation; a reported
 * stall that goes a grace window without a frame is the timeout. The module
 * only observes: it never assigns `src`, never calls `play()`, and never
 * touches styles or the UI.
 *
 * Two limits are inherent to this observation and are repaired by the caller,
 * not hidden here:
 *
 * 1. A delivery that is served without ever reporting a stall keeps the
 *    attempt open indefinitely. That is deliberate (FR-004: the window must not
 *    expire while the delivery is still being served — the element shows its
 *    own loading state), and it is why the watchdog re-arms the deadline on
 *    every tick while no stall is pending.
 * 2. A stalled delivery that resumes without any element event (pre-metadata
 *    resumption: Chromium fires neither `progress` nor metadata until enough
 *    bytes arrive) expires `graceMs` after the reported stall, converting a
 *    file that would have played. The grace window is the mitigation: it is
 *    measured from the last arming, which is at most 250 ms after the last
 *    delivery signal, so a served delivery is never cut short by more than one
 *    watchdog tick.
 */

export const ORIGINAL_ATTEMPT_GRACE_MS = 5000;

/** How often the deadline is re-armed while the delivery is being served. */
const WATCHDOG_MS = 250;

/** A decoded first frame; both mean the video track plays. */
const FRAME_EVENTS = ['loadeddata', 'playing'];

/** The element refusing the media outright. */
const FAILURE_EVENTS = ['error'];

/** The delivery is being served; each one clears a pending stall. */
const DELIVERY_EVENTS = [
  'loadstart',
  'loadedmetadata',
  'durationchange',
  'progress',
  'suspend',
  'canplay',
  'seeked',
  'timeupdate',
];

/** Fetching, but no data is forthcoming: the deadline stops being re-armed. */
const STALL_EVENTS = ['stalled'];

/**
 * Arms an attempt on `videoEl` and resolves exactly once with
 * `{ verdict, reason, frameObserved }`.
 *
 * `graceMs` is the window a reported stall may go without a frame before the
 * original is declared unplayable; `audioPlayable()` is asked only once a frame
 * proves the video track plays, because a browser can decode the video of an
 * MP4 whose audio codec it does not support and play it in silence.
 *
 * @param {HTMLVideoElement} videoEl
 * @param {{ graceMs?: number, audioPlayable?: () => boolean }} [options]
 * @returns {{ promise: Promise<{ verdict: 'playable'|'unplayable'|'cancelled', reason: null|'error'|'timeout'|'audio'|'cancelled', frameObserved: boolean }>, cancel: () => void }}
 */
export function startOriginalAttempt(
  videoEl,
  { graceMs = ORIGINAL_ATTEMPT_GRACE_MS, audioPlayable = () => true } = {}
) {
  let settled = false;
  let stalled = false;
  let deadlineTimer = null;
  let watchdogTimer = null;
  /** @type {Array<[string, () => void]>} */
  const attached = [];
  /** @type {(result: { verdict: string, reason: string|null, frameObserved: boolean }) => void} */
  let resolve;
  const promise = new Promise((settleP) => {
    resolve = settleP;
  });

  /** Stops every timer and listener the attempt installed; safe to repeat. */
  const detach = () => {
    if (deadlineTimer !== null) {
      clearTimeout(deadlineTimer);
      deadlineTimer = null;
    }
    if (watchdogTimer !== null) {
      clearInterval(watchdogTimer);
      watchdogTimer = null;
    }
    for (const [type, handler] of attached) videoEl.removeEventListener(type, handler);
    attached.length = 0;
  };

  const settle = (verdict, reason, frameObserved) => {
    if (settled) return;
    settled = true;
    detach();
    resolve({ verdict, reason, frameObserved });
  };

  /**
   * (Re)starts the window from `now`. Called at arming time, on every watchdog
   * tick while no stall is pending, and on every delivery signal, so the
   * verdict lands at most `graceMs` after the last arming.
   */
  const armDeadline = () => {
    clearTimeout(deadlineTimer);
    deadlineTimer = setTimeout(() => settle('unplayable', 'timeout', false), graceMs);
  };

  const onFrame = () => {
    if (audioPlayable()) settle('playable', null, true);
    else settle('unplayable', 'audio', true);
  };

  const listen = (type, handler) => {
    attached.push([type, handler]);
    videoEl.addEventListener(type, handler);
  };

  const cancel = () => settle('cancelled', 'cancelled', false);

  // An element that already holds a decoded frame needs no observation at all.
  if (videoEl.readyState >= 2 && videoEl.videoWidth > 0) {
    onFrame();
    return { promise, cancel };
  }

  for (const type of FRAME_EVENTS) listen(type, onFrame);
  for (const type of FAILURE_EVENTS) listen(type, () => settle('unplayable', 'error', false));
  for (const type of DELIVERY_EVENTS) {
    listen(type, () => {
      stalled = false;
      armDeadline();
    });
  }
  for (const type of STALL_EVENTS) {
    listen(type, () => {
      stalled = true;
    });
  }

  armDeadline();
  watchdogTimer = setInterval(() => {
    if (!stalled) armDeadline();
  }, WATCHDOG_MS);

  return { promise, cancel };
}

/**
 * Remembers, for this session only, the videos whose original already failed
 * to play. The viewer consults it before re-arming an attempt it knows will
 * fail again; nothing is persisted, so a reload starts clean.
 *
 * @returns {{ has: (hash: string) => boolean, record: (hash: string) => void, clear: (hash: string) => void }}
 */
export function createOriginalFailureRegistry() {
  const failures = new Set();
  return {
    has: (hash) => failures.has(hash),
    record: (hash) => {
      failures.add(hash);
    },
    clear: (hash) => {
      failures.delete(hash);
    },
  };
}
