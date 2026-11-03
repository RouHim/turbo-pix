/**
 * The streamed-playback bookkeeping: escalation ladder, retry arming, the
 * newest requested position, the offset the current run began at, and the
 * lost-run budget.
 *
 * `PhotoViewer` owns the DOM and the media players; this module owns the
 * decisions those callbacks make, so the rules can be driven in node tests
 * without a media element or a live transcoder. Every method is a no-op or a
 * value: nothing here touches the DOM.
 */

export const STREAM_LADDER = ['remux', 'audio', 'transcode'];

/** Consecutive lost-run replays allowed on one rung before the ladder takes over. */
export const LOST_RUN_RETRY_LIMIT = 2;

/**
 * The rung after `currentMode`, or `null` when the ladder is exhausted. The
 * last rung has no successor, which is what terminates the ladder.
 */
export function nextStreamMode(currentMode) {
  const index = STREAM_LADDER.indexOf(currentMode);
  return index >= 0 && index < STREAM_LADDER.length - 1 ? STREAM_LADDER[index + 1] : null;
}

const DEFAULT_TIMERS = {
  // Wrapped, not referenced: a browser's `setTimeout` is a Window method and
  // throws "Illegal invocation" when called as a property of another object.
  setTimeout: (callback, delay) => globalThis.setTimeout(callback, delay),
  clearTimeout: (handle) => globalThis.clearTimeout(handle),
};

/**
 * @param {object} [options]
 * @param {() => number} [options.getPosition] Current media-element position.
 * @param {{ setTimeout: Function, clearTimeout: Function }} [options.timers]
 *   Injectable timer pair (tests capture the callback instead of waiting).
 */
export function createStreamSession({ getPosition = () => 0, timers = DEFAULT_TIMERS } = {}) {
  // The newest position the user asked for, in seconds: every run the player
  // begins and every position picked on the element moves it.
  let intentOffset = 0;
  // The offset the run most recently begun plays from. A run that never
  // attached leaves the element at 0, so this is the only witness of where it
  // was asked to play.
  let runStartOffset = 0;
  // The one pending saturation retry; arming replaces it.
  let retryTimer = null;
  // Budget spent on the current rung.
  let lostRunRetries = 0;
  let lostRunMode = null;

  const disarmRetry = () => {
    if (retryTimer === null) return;
    timers.clearTimeout(retryTimer);
    retryTimer = null;
  };

  const rememberIntent = (seconds) => {
    if (Number.isFinite(seconds)) intentOffset = seconds;
    disarmRetry();
  };

  const beginRun = (seconds) => {
    if (Number.isFinite(seconds)) runStartOffset = seconds;
    rememberIntent(seconds);
  };

  return {
    get intentOffset() {
      return intentOffset;
    },
    get runStartOffset() {
      return runStartOffset;
    },
    get retryPending() {
      return retryTimer !== null;
    },

    /** Drop a pending saturation retry without waiting for it. */
    disarmRetry,

    /**
     * Record the newest position the user asked for and drop a retry armed for
     * an older one. `onRunStart` and `onUserSeek` both report here.
     */
    rememberIntent,

    /**
     * Record the offset the run the player just began plays from, and its own
     * newest-position bookkeeping.
     */
    beginRun,

    /**
     * Arm the one pending saturation retry. A newer run or a user seek
     * disarms it, so it cannot outlive the position it was armed for.
     */
    scheduleRetry(callback, delayMs) {
      disarmRetry();
      retryTimer = timers.setTimeout(() => {
        retryTimer = null;
        callback();
      }, delayMs);
    },

    /** Clear the lost-run budget; the rung it was spent against stays. */
    forgetLostRuns() {
      lostRunRetries = 0;
    },

    /**
     * Whether a lost run may be replayed on its own rung, claiming the retry
     * when it may. A loss on another rung starts its own count.
     */
    claimLostRunRetry(attemptedMode) {
      if (attemptedMode !== lostRunMode) {
        lostRunMode = attemptedMode;
        lostRunRetries = 0;
      }
      if (lostRunRetries >= LOST_RUN_RETRY_LIMIT) return false;
      lostRunRetries += 1;
      return true;
    },

    /**
     * The offset a run started after a failure plays from: the larger of the
     * element's own position and the offset the failed run started at. A
     * position at or past the declared duration restarts at 0, because a run
     * there can only end immediately.
     */
    resumeOffset(decision) {
      const position = getPosition();
      const resumed = Math.max(
        Number.isFinite(position) ? position : 0,
        Number.isFinite(runStartOffset) ? runStartOffset : 0
      );
      if (resumed <= 0) return 0;
      const duration = decision?.duration;
      if (Number.isFinite(duration) && duration > 0 && resumed >= duration) return 0;
      return resumed;
    },
  };
}
