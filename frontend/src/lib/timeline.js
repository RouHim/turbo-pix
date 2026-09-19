// Pure date/data model for the desktop timeline selector.
// No Svelte imports: testable with node --test.

export const MONTHS_PER_YEAR = 12;
export const MONTHS_PER_DECADE = 120;

const EMPTY_MODEL = {
  minIndex: 0,
  maxIndex: -1,
  length: 0,
  counts: new Int32Array(0),
  prefix: new Float64Array(1),
  total: 0,
  years: [],
};

/** @param {number} year @param {number} month 1-12 */
export const toMonthIndex = (year, month) => year * MONTHS_PER_YEAR + (month - 1);

/** @param {number} index */
export const fromMonthIndex = (index) => ({
  year: Math.floor(index / MONTHS_PER_YEAR),
  month: (index % MONTHS_PER_YEAR) + 1,
});

/**
 * Dense month model over `[minIndex, maxIndex]`: the payload only contains
 * months with photos, so gaps are zero-filled here once instead of at every
 * paint. `prefix` enables O(1) range counts during pan/zoom.
 *
 * @param {Array<{ year: number, month: number, count: number }>} density
 */
export const buildTimelineModel = (density) => {
  const rows = (density ?? []).filter((row) => row && row.count > 0);
  if (rows.length === 0) return EMPTY_MODEL;

  let minIndex = Infinity;
  let maxIndex = -Infinity;
  for (const row of rows) {
    const index = toMonthIndex(row.year, row.month);
    if (index < minIndex) minIndex = index;
    if (index > maxIndex) maxIndex = index;
  }

  const length = maxIndex - minIndex + 1;
  const counts = new Int32Array(length);
  for (const row of rows) {
    counts[toMonthIndex(row.year, row.month) - minIndex] += row.count;
  }

  const prefix = new Float64Array(length + 1);
  for (let i = 0; i < length; i += 1) prefix[i + 1] = prefix[i] + counts[i];

  // Populated years only, descending: the rail and the mobile dropdown must
  // not grow a column for a year the library has no photos in.
  const years = [...new Set(rows.map((row) => row.year))].sort((a, b) => b - a);

  return { minIndex, maxIndex, length, counts, prefix, total: prefix[length], years };
};

/** Photo count of `[startIndex, endIndex]`, clamped to the model span. */
export const countInRange = (model, startIndex, endIndex) => {
  if (model.length === 0) return 0;
  const start = Math.max(startIndex, model.minIndex);
  const end = Math.min(endIndex, model.maxIndex);
  if (end < start) return 0;
  return model.prefix[end - model.minIndex + 1] - model.prefix[start - model.minIndex];
};

/** @returns {{ startIndex: number, endIndex: number } | null} */
export const normalizeSelection = (a, b) =>
  a === null || a === undefined || b === null || b === undefined
    ? null
    : { startIndex: Math.min(a, b), endIndex: Math.max(a, b) };

export const selectionEquals = (a, b) =>
  (a === null && b === null) ||
  (a !== null && b !== null && a.startIndex === b.startIndex && a.endIndex === b.endIndex);

/** Narrow a selection to the months the library actually spans; null without overlap. */
export const clampSelectionToModel = (selection, model) => {
  if (!selection || model.length === 0) return null;
  const startIndex = Math.max(selection.startIndex, model.minIndex);
  const endIndex = Math.min(selection.endIndex, model.maxIndex);
  return endIndex < startIndex ? null : { startIndex, endIndex };
};

export const clampIndexToModel = (model, index) =>
  Math.min(Math.max(index, model.minIndex), model.maxIndex);

/** `March 1998` for a month index, using the caller's localised month names. */
export const formatPeriodName = (index, monthName) => {
  const { year, month } = fromMonthIndex(index);
  return `${monthName(month)} ${year}`;
};

/**
 * `All Dates`, `2012`, `March 2012`, `2012 – 2015` or
 * `March 2012 – August 2015` — a full-year range collapses to bare years,
 * because a range covering January through December *is* the year filter.
 */
export const formatSelectionLabel = (selection, format) => {
  if (!selection) return format.allDates;
  const start = fromMonthIndex(selection.startIndex);
  const end = fromMonthIndex(selection.endIndex);
  const wholeYears = start.month === 1 && end.month === 12;

  if (wholeYears && start.year === end.year) return String(start.year);
  if (wholeYears) {
    return format.rangeTemplate(String(start.year), String(end.year));
  }
  if (selection.startIndex === selection.endIndex) {
    return formatPeriodName(selection.startIndex, format.monthName);
  }
  return format.rangeTemplate(
    formatPeriodName(selection.startIndex, format.monthName),
    formatPeriodName(selection.endIndex, format.monthName)
  );
};
