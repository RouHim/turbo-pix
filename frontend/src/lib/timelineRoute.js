// Route ⇄ selection mapping for the timeline filter. The route carries the
// start bound in `year`/`month` (today's params, so existing links and saved
// searches keep working) and the end bound in `to_year`/`to_month`.
import { clampSelectionToModel, fromMonthIndex, toMonthIndex } from './timeline.js';

export const EMPTY_DATE_FILTER = { year: null, month: null, to_year: null, to_month: null };

const parseYear = (value) => (Number.isInteger(value) && value >= 1 ? value : null);
const parseMonth = (value) => (Number.isInteger(value) && value >= 1 && value <= 12 ? value : null);

/**
 * Validate, order and canonicalise a route date filter.
 *
 * Canonical form matters twice: the URL round-trips (Back/Forward, saved
 * searches) and the clamp effect can compare route against clamped filter
 * without writing on every render.
 */
export const normalizeDateFilter = (raw) => {
  const year = parseYear(raw?.year);
  if (year === null) return EMPTY_DATE_FILTER;
  const month = parseMonth(raw?.month);
  const toYear = parseYear(raw?.to_year);
  if (toYear === null) return { year, month, to_year: null, to_month: null };

  // An absent start month means January and an absent end month December, so
  // order the bounds as they read and only then resolve those roles: swapping
  // the resolved indices would hand a December to the start bound and a
  // January to the end bound.
  const rawStart = { year, month };
  const rawEnd = { year: toYear, month: parseMonth(raw?.to_month) };
  const reversed =
    toMonthIndex(rawEnd.year, rawEnd.month ?? 12) <
    toMonthIndex(rawStart.year, rawStart.month ?? 1);
  const [lower, upper] = reversed ? [rawEnd, rawStart] : [rawStart, rawEnd];

  const start = { year: lower.year, month: lower.month ?? 1 };
  const end = { year: upper.year, month: upper.month ?? 12 };

  // A range of one period is that period; a range of one whole year is that year.
  if (start.year === end.year && start.month === end.month) {
    return { year: start.year, month: start.month, to_year: null, to_month: null };
  }
  if (start.year === end.year && start.month === 1 && end.month === 12) {
    return { year: start.year, month: null, to_year: null, to_month: null };
  }

  return {
    year: start.year,
    month: start.month === 1 ? null : start.month,
    to_year: end.year,
    to_month: end.month === 12 ? null : end.month,
  };
};

/**
 * Active selection, clamped to the months the library actually has; `null`
 * without overlap.
 *
 * A range (an explicit end bound) narrows to the overlap. A bare period keeps
 * its own lower boundary — `?year=2012` must select January–December 2012 even
 * when the library starts that March — so `filterFromSelection` hands the same
 * canonical filter back and the clamp effect does not rewrite the URL.
 */
export const selectionFromFilter = (filter, model) => {
  if (!filter?.year || !model || model.length === 0) return null;
  const toYear = filter.to_year ?? null;
  const startIndex = toMonthIndex(filter.year, filter.month ?? 1);
  const endIndex =
    toYear === null
      ? toMonthIndex(filter.year, filter.month ?? 12)
      : toMonthIndex(toYear, filter.to_month ?? 12);

  if (toYear !== null) return clampSelectionToModel({ startIndex, endIndex }, model);

  // A bare period keeps its own lower boundary, so only its end is clamped.
  return endIndex < model.minIndex || startIndex > model.maxIndex
    ? null
    : { startIndex, endIndex: Math.min(endIndex, model.maxIndex) };
};

export const filterFromSelection = (selection) => {
  if (!selection) return EMPTY_DATE_FILTER;
  const start = fromMonthIndex(selection.startIndex);
  const end = fromMonthIndex(selection.endIndex);
  return normalizeDateFilter({
    year: start.year,
    month: start.month,
    to_year: end.year,
    to_month: end.month,
  });
};

export const filterEquals = (a, b) =>
  a.year === b.year && a.month === b.month && a.to_year === b.to_year && a.to_month === b.to_month;
