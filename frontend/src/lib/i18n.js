import { init, register, locale, _ } from 'svelte-i18n';
import { get } from 'svelte/store';
import en from '../i18n/en.json';
import de from '../i18n/de.json';

register('en', () => Promise.resolve(en));
register('de', () => Promise.resolve(de));

export function initI18n(defaultLocale) {
  const saved = localStorage.getItem('turbo-pix-locale');
  const initial = ['en', 'de'].includes(saved)
    ? saved
    : ['en', 'de'].includes(defaultLocale)
      ? defaultLocale
      : 'en';
  init({ fallbackLocale: 'en', initialLocale: initial });
}

export function setLocale(l) {
  if (!['en', 'de'].includes(l)) l = 'en';
  locale.set(l);
  localStorage.setItem('turbo-pix-locale', l);
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
    const translated = get(t)(key);
    if (translated && translated !== key) return translated;
  }

  // Fuzzy match: check if error message contains any known pattern
  for (const [pattern, k] of Object.entries(errorMap)) {
    if (errorMessage.includes(pattern)) {
      const translated = get(t)(k);
      if (translated && translated !== k) return translated;
    }
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
