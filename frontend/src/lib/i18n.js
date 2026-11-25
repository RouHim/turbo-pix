import { init, register, locale, _ } from 'svelte-i18n';
import { get } from 'svelte/store';
import en from '../i18n/en.json';
import de from '../i18n/de.json';

register('en', () => Promise.resolve(en));
register('de', () => Promise.resolve(de));

const SUPPORTED = ['en', 'de'];

export function initI18n(defaultLocale) {
  const initial = SUPPORTED.includes(defaultLocale) ? defaultLocale : 'en';
  init({ fallbackLocale: 'en', initialLocale: initial });
  try {
    if (typeof document !== 'undefined') document.documentElement.lang = initial;
  } catch {
    /* ignore */
  }
}

export { _ as t };

/**
 * Map backend error strings to translated messages using the current locale dictionary.
 * @param {string} errorMessage - Raw error message from the API
 * @returns {string} Translated error message or the original if no mapping found
 */
export function translateError(errorMessage) {
  // Raw strings the backend actually emits (see warp_helpers ErrorResponse and
  // image_editor.rs); entries for strings no producer emits are dead weight.
  const errorMap = {
    'Photo not found': 'errors.photoNotFound',
    'Database error': 'errors.databaseError',
    'Failed to load image': 'errors.failedToLoadImage',
    'Photo directory is mounted as read-only': 'errors.readOnlyFilesystem',
    'Permission denied': 'errors.permissionDenied',
  };

  const key = errorMap[errorMessage];
  if (key) {
    const translated = get(_)(key);
    if (translated && translated !== key) return translated;
  }

  // Fuzzy match: check if error message contains any known pattern
  for (const [pattern, k] of Object.entries(errorMap)) {
    if (errorMessage.includes(pattern)) {
      const translated = get(_)(k);
      if (translated && translated !== k) return translated;
    }
  }

  // Case-insensitive database fallback (backend error casing varies)
  if (errorMessage.toLowerCase().includes('database')) {
    const translated = get(_)('errors.databaseError');
    if (translated && translated !== 'errors.databaseError') return translated;
  }

  return errorMessage;
}

/**
 * Get the current locale string (e.g. 'en', 'de').
 * @returns {string}
 */
export function getLocale() {
  return get(locale) || 'en';
}
