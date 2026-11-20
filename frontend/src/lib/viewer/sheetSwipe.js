/** Horizontal travel before the sheet header claims the gesture. */
export const SHEET_HEADER_SWIPE_THRESHOLD_PX = 24;

/**
 * Classifies a header drag. The gesture is horizontal only when it clearly
 * outperforms the vertical axis; leaving the sheet drag the vertical branch.
 */
export function sheetHeaderSwipeDirection(
  deltaX,
  deltaY,
  threshold = SHEET_HEADER_SWIPE_THRESHOLD_PX
) {
  if (Math.abs(deltaX) <= Math.abs(deltaY)) return null;
  if (Math.abs(deltaX) < threshold) return null;
  return deltaX < 0 ? 'next' : 'previous';
}
