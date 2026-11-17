// Pure date-scrollbar model helpers: density slicing, month lookup, anchors and
// window bookkeeping. No Svelte, api or i18n imports — node --test only.

import { MONTHS_PER_YEAR, clampIndexToModel, toMonthIndex } from './timeline.js';

/** Month index (`year * 12 + month - 1`) of a photo reply, or null. */
export const monthOfPhoto = (photo) => {
  const raw = typeof photo?.taken_at === 'string' ? photo.taken_at : null;
  if (!raw) return null;
  const match = /^(\d{4})-(\d{2})/.exec(raw);
  if (!match) return null;
  const month = Number(match[2]);
  return month >= 1 && month <= MONTHS_PER_YEAR ? toMonthIndex(Number(match[1]), month) : null;
};

export const monthCount = (model, index) => {
  if (index === null || index === undefined || model.length === 0) return 0;
  return model.counts[index - model.minIndex] ?? 0;
};

/** Populated months, newest first when `descending`. */
export const monthsWithPhotos = (model, descending) => {
  const months = [];
  for (let index = model.minIndex; index <= model.maxIndex; index += 1) {
    if (model.counts[index - model.minIndex] > 0) months.push(index);
  }
  return descending ? months.reverse() : months;
};

/**
 * Nearest populated month. The dense model contains zero-count months, but a
 * scrubber target must always be a month the library can actually show.
 * Ties prefer the newer side, matching the descending track.
 */
export const snapMonth = (model, index) => {
  if (model.length === 0) return null;
  const clamped = clampIndexToModel(model, index);
  if (model.counts[clamped - model.minIndex] > 0) return clamped;
  for (let step = 1; step <= model.length; step += 1) {
    const newer = clamped + step;
    if (newer <= model.maxIndex && model.counts[newer - model.minIndex] > 0) return newer;
    const older = clamped - step;
    if (older >= model.minIndex && model.counts[older - model.minIndex] > 0) return older;
  }
  return null;
};

/**
 * Nearest populated month at or after `index` in `direction` (+1 = newer,
 * -1 = older). Keyboard steps use this so a step into a gap lands on the next
 * populated month *in the direction of travel*, instead of snapping back to
 * the month the user came from.
 */
export const snapMonthInDirection = (model, index, direction) => {
  if (model.length === 0 || (direction !== 1 && direction !== -1)) return null;
  const clamped = clampIndexToModel(model, index);
  for (
    let current = clamped;
    current >= model.minIndex && current <= model.maxIndex;
    current += direction
  ) {
    if (model.counts[current - model.minIndex] > 0) return current;
  }
  return null;
};

/**
 * The timeline filter is a pure `taken_at` predicate, so the library density
 * restricted to the filtered months IS the filtered listing's density.
 *
 * @param {Array<{ year: number, month: number, count: number }>} density
 * @param {{ year: number|null, month: number|null, to_year: number|null, to_month: number|null }} filter
 */
export const sliceDensity = (density, filter) => {
  if (!filter?.year) return density;
  const start = toMonthIndex(filter.year, filter.month ?? 1);
  const end =
    filter.to_year !== null
      ? toMonthIndex(filter.to_year, filter.to_month ?? 12)
      : filter.month !== null
        ? start
        : toMonthIndex(filter.year, 12);
  return density.filter((row) => {
    const index = toMonthIndex(row.year, row.month);
    return index >= start && index <= end;
  });
};

/**
 * Absolute listing offset of the first photo of `index`. Undated photos sort
 * last in descending order (no adjustment) and first in ascending order.
 *
 * @param {{ total: number, minIndex: number, prefix: Float64Array,
 *   counts: Int32Array }} model
 * @param {number} index month index
 * @param {{ descending?: boolean, totalPhotos?: number|null }} options
 */
export const anchorOffset = (model, index, { descending = true, totalPhotos = null } = {}) => {
  const dated = model.total;
  const before = model.prefix[index - model.minIndex];
  const through = model.prefix[index - model.minIndex + 1];
  const undated = Math.max(0, (totalPhotos ?? dated) - dated);
  return descending ? dated - through : undated + before;
};

/**
 * The page that holds the items immediately before `startOffset`, and how many
 * of its leading items belong to the window. `null` when the window starts at
 * the top of the listing.
 */
export const earlierPage = (startOffset, limit) => {
  if (startOffset <= 0 || limit <= 0) return null;
  const page = Math.floor((startOffset - 1) / limit) + 1;
  const pageStart = (page - 1) * limit;
  return { page, take: startOffset - pageStart };
};

/**
 * Whole-page trim for the bottom edge: how many trailing items to drop and
 * which page the grid's own loader must re-fetch. `null` when there is nothing
 * safe to drop (a single page).
 */
export const bottomTrim = (startOffset, length, limit) => {
  if (length === 0 || limit <= 0) return null;
  const endOffset = startOffset + length;
  const lastPage = Math.floor((endOffset - 1) / limit) + 1;
  const lastPageStart = (lastPage - 1) * limit;
  const drop = endOffset - lastPageStart;
  return drop > 0 && drop < length ? { drop, page: lastPage } : null;
};
