/**
 * Shared snap decision for the finger-tracked mobile panels (app drawer and
 * viewer details sheet).
 *
 * Velocity is expressed in "towards open" px/ms: positive opens, negative
 * closes. A real flick beats the distance rule, which is what makes a short
 * fast gesture feel native instead of requiring 30 % of the panel's width.
 */

/** Fraction of the panel's travel that counts as a deliberate drag. */
export const PANEL_SETTLE_RATIO = 0.3;

/** Flick velocity that settles the panel in its velocity direction. */
export const PANEL_SETTLE_VELOCITY = 0.3;

/** Window after a completed drag in which the synthesized click is ignored. */
export const GESTURE_CLICK_SUPPRESS_MS = 200;

export function clamp01(value) {
  return Math.min(1, Math.max(0, value));
}

/** 0 = drawer fully closed (translateX === -width), 1 = fully open. */
export function drawerOpenProgress(translateX, width) {
  if (!width) return 0;
  return clamp01((translateX + width) / width);
}

/** 0 = sheet fully closed (translateY === height), 1 = fully open. */
export function sheetOpenProgress(translateY, height) {
  if (!height) return 0;
  return clamp01((height - translateY) / height);
}

/**
 * Resolve the release state.
 *
 * `startedOpen` matters: dragging an open panel 30 % towards closed should
 * close it, while dragging a closed panel 30 % towards open should open it.
 * A sub-threshold drag therefore returns to where it started.
 */
export function shouldSettleOpen({ startedOpen, progress, velocity }) {
  if (velocity >= PANEL_SETTLE_VELOCITY) return true;
  if (velocity <= -PANEL_SETTLE_VELOCITY) return false;
  return startedOpen ? progress > 1 - PANEL_SETTLE_RATIO : progress >= PANEL_SETTLE_RATIO;
}
