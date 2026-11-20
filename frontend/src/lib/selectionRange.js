/**
 * Returns the surface keys between the anchor and the card under the finger,
 * in display order. Both bounds are inclusive and direction does not matter.
 * A key that is not part of the loaded surface claims no range.
 */
export function rangeKeys(orderedKeys, anchorKey, targetKey) {
  if (!Array.isArray(orderedKeys) || orderedKeys.length === 0) return null;

  const from = orderedKeys.indexOf(anchorKey);
  const to = orderedKeys.indexOf(targetKey);
  if (from === -1 || to === -1) return null;

  return from <= to ? orderedKeys.slice(from, to + 1) : orderedKeys.slice(to, from + 1);
}

/**
 * Replaces the live selection with `base` plus the current range. Recomputing
 * from the base on every move is what lets a backwards drag shrink the range
 * instead of leaving a trail of previously hovered cards selected.
 */
export function withRange(base, orderedKeys, anchorKey, targetKey) {
  const keys = rangeKeys(orderedKeys, anchorKey, targetKey);
  if (!keys) return base;

  const next = { ...(base || {}) };
  for (const key of keys) next[key] = true;
  return next;
}
