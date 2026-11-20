/** Seconds a single video double-tap seeks, independent of the clip length. */
export const VIDEO_SEEK_STEP_SECONDS = 10;

/**
 * The middle third is deliberately inert: native video controls live there,
 * and a double-tap in the middle must not seek.
 */
export const VIDEO_SEEK_CENTER_RATIO = 1 / 3;

/**
 * Maps a horizontal tap position inside the video box to a seek direction.
 * Returns null for the middle third or a surface with no measurable width.
 */
export function seekDirectionForX(x, left, width) {
  if (!Number.isFinite(width) || width <= 0) return null;

  const ratio = (x - left) / width;
  const halfCenter = VIDEO_SEEK_CENTER_RATIO / 2;
  if (ratio < 0.5 - halfCenter) return 'back';
  if (ratio > 0.5 + halfCenter) return 'forward';
  return null;
}

/**
 * Clamps a relative seek to the playable range. An unknown or non-positive
 * duration has no upper bound, but zero is always the floor.
 */
export function seekTarget(currentTime, duration, direction, step = VIDEO_SEEK_STEP_SECONDS) {
  if (direction !== 'back' && direction !== 'forward') return currentTime;

  const current = Number.isFinite(currentTime) ? currentTime : 0;
  const delta = direction === 'back' ? -step : step;
  const target = current + delta;

  if (target < 0) return 0;
  if (Number.isFinite(duration) && duration > 0) return Math.min(target, duration);
  return target;
}
