/**
 * Locale-aware date/time formatting. Pure functions: the caller owns the
 * active locale, so the module stays importable outside the Svelte runtime.
 */

const CALENDAR_DATE = /^\d{4}-\d{2}-\d{2}$/;
const FORMATTERS = new Map();
// Authored date voice: an abbreviated month keeps the contact-sheet register
// in both locales (DE `4. Nov. 2026`, EN `Nov 4, 2026`) instead of CLDR's
// numeric German medium date (`04.11.2026`).
const DATE_FIELDS = Object.freeze({ year: 'numeric', month: 'short', day: 'numeric' });

/**
 * Creating an `Intl.DateTimeFormat` is expensive; the grid asks for the same
 * date shape once per card, so keep one formatter per locale and shape.
 *
 * @param {string} locale
 * @param {string} shape
 * @param {Intl.DateTimeFormatOptions} options
 * @returns {Intl.DateTimeFormat}
 */
function getFormatter(locale, shape, options) {
  const key = `${locale}|${shape}`;
  let formatter = FORMATTERS.get(key);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat(locale, options);
    FORMATTERS.set(key, formatter);
  }
  return formatter;
}

/**
 * @param {Date|string|number|null|undefined} value
 * @returns {Date|null}
 */
function toDate(value) {
  if (value === null || value === undefined || value === '') return null;
  const date = value instanceof Date ? value : new Date(value);
  return Number.isNaN(date.getTime()) ? null : date;
}

/**
 * Calendar dates carry no instant or time zone, so they are parsed as UTC.
 * @param {string} value
 * @returns {Date|null}
 */
function toCalendarDate(value) {
  if (typeof value !== 'string' || !CALENDAR_DATE.test(value)) return null;
  const date = new Date(`${value}T00:00:00.000Z`);
  if (Number.isNaN(date.getTime())) return null;
  return date.toISOString().slice(0, 10) === value ? date : null;
}

/**
 * Format an instant (RFC3339 string, Date or epoch) in the browser's time
 * zone. Calendar dates (`YYYY-MM-DD`) belong in `formatWeekdayDate`, which
 * pins UTC instead of shifting the day.
 *
 * @param {Date|string|number|null|undefined} value
 * @param {string} locale
 * @returns {string}
 */
export function formatDate(value, locale = 'en') {
  const date = toDate(value);
  if (!date) return '';
  return getFormatter(locale, 'date', DATE_FIELDS).format(date);
}

/**
 * Format an instant as a long localized date (no time), in the browser's time
 * zone. Used by the current-date hint.
 *
 * @param {Date|string|number|null|undefined} value
 * @param {string} locale
 * @returns {string}
 */
export function formatLongDate(value, locale = 'en') {
  const date = toDate(value);
  if (!date) return '';
  return getFormatter(locale, 'date-long', { dateStyle: 'long' }).format(date);
}

/**
 * @param {Date|string|number|null|undefined} value
 * @param {string} locale
 * @returns {string}
 */
export function formatDateTime(value, locale = 'en') {
  const date = toDate(value);
  if (!date) return '';
  return getFormatter(locale, 'date-time', {
    ...DATE_FIELDS,
    hour: locale === 'de' ? '2-digit' : 'numeric',
    minute: '2-digit',
  }).format(date);
}

/**
 * Format a calendar date (`YYYY-MM-DD`) with its weekday. Calendar dates carry
 * no instant or time zone, so they are formatted in UTC; using the browser's
 * zone would shift the day west of UTC.
 *
 * @param {string} value
 * @param {string} locale
 * @returns {string}
 */
export function formatWeekdayDate(value, locale = 'en') {
  const date = toCalendarDate(value);
  if (!date) return '';
  return getFormatter(locale, 'weekday-date', {
    weekday: 'long',
    year: 'numeric',
    month: 'long',
    day: 'numeric',
    timeZone: 'UTC',
  }).format(date);
}

/**
 * @param {number} month 1-12
 * @param {string} locale
 * @param {boolean} short
 * @returns {string}
 */
export function formatMonthName(month, locale = 'en', short = false) {
  if (!Number.isInteger(month) || month < 1 || month > 12) return '';
  const date = new Date(Date.UTC(2000, month - 1, 1));
  return getFormatter(locale, short ? 'month-short' : 'month-long', {
    month: short ? 'short' : 'long',
    timeZone: 'UTC',
  }).format(date);
}
