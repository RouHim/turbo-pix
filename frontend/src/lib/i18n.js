import { init, register, locale, _ } from 'svelte-i18n';
import { get } from 'svelte/store';
import en from '../i18n/en.json';
import de from '../i18n/de.json';

register('en', () => Promise.resolve(en));
register('de', () => Promise.resolve(de));

const SUPPORTED = ['en', 'de'];

function readSavedLocale() {
  try {
    const saved = localStorage.getItem('turbo-pix-locale');
    return SUPPORTED.includes(saved) ? saved : null;
  } catch {
    return null;
  }
}

function detectBrowserLocale() {
  try {
    const nav = navigator;
    const candidates = [
      nav.language,
      ...(nav.languages || []),
      nav.userLanguage,
      nav.browserLanguage,
      nav.systemLanguage,
    ];
    for (const lang of candidates) {
      if (!lang) continue;
      const code = String(lang).split('-')[0].toLowerCase();
      if (SUPPORTED.includes(code)) return code;
    }
  } catch {
    /* navigator unavailable */
  }
  return null;
}

export function initI18n(defaultLocale) {
  const initial =
    readSavedLocale() ||
    (SUPPORTED.includes(defaultLocale) ? defaultLocale : null) ||
    detectBrowserLocale() ||
    'en';
  init({ fallbackLocale: 'en', initialLocale: initial });
  try {
    if (typeof document !== 'undefined') document.documentElement.lang = initial;
  } catch {
    /* ignore */
  }
}

export function setLocale(l) {
  if (!SUPPORTED.includes(l)) l = 'en';
  locale.set(l);
  try {
    localStorage.setItem('turbo-pix-locale', l);
    if (typeof document !== 'undefined') document.documentElement.lang = l;
  } catch {
    /* storage unavailable */
  }
}

export { _ as t };

/**
 * Map backend error strings to translated messages using the current locale dictionary.
 * @param {string} errorMessage - Raw error message from the API
 * @returns {string} Translated error message or the original if no mapping found
 */
export function translateError(errorMessage) {
  const errorMap = {
    'Photo not found': 'errors.photoNotFound',
    'Database error': 'errors.databaseError',
    'Search failed': 'errors.searchError',
    'Failed to load photo': 'errors.failedToLoadPhoto',
    'Failed to load image': 'errors.failedToLoadImage',
    'Failed to read photo file': 'errors.failedToReadPhotoFile',
    'Invalid thumbnail size': 'errors.invalidThumbnailSize',
    'Server connection lost': 'errors.connectionLost',
    'Photo directory is mounted as read-only': 'errors.readOnlyFilesystem',
    'Insufficient file permissions': 'errors.permissionDenied',
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
