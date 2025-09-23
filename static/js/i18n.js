import { detectLocale, i18n, isLocale } from '../i18n/i18n-util.js';
import { loadLocaleAsync } from '../i18n/i18n-util.async.js';

class I18nManager {
  constructor() {
    this.currentLocale = 'en';
    this.LL = null;
    this.initialized = false;
  }

  detectBrowserLocale() {
    const saved = localStorage.getItem('turbo-pix-locale');
    if (saved && isLocale(saved)) {
      return saved;
    }

    const detected = detectLocale(() => {
      const nav = navigator;
      return [
        nav.language,
        ...(nav.languages || []),
        nav.userLanguage,
        nav.browserLanguage,
        nav.systemLanguage,
      ].filter(Boolean);
    });

    return detected || 'en';
  }

  async initializeI18n() {
    if (this.initialized) return this.LL;

    const locale = this.detectBrowserLocale();
    await this.setLocale(locale);
    this.initialized = true;
    return this.LL;
  }

  async setLocale(locale) {
    if (!isLocale(locale)) {
      locale = 'en';
    }

    this.currentLocale = locale;
    this.LL = await loadLocaleAsync(locale);

    localStorage.setItem('turbo-pix-locale', locale);

    this.updateDOM();
    return this.LL;
  }

  getLocale() {
    return this.currentLocale;
  }

  updateDOM() {
    if (!this.LL) return;

    document.querySelectorAll('[data-i18n]').forEach((element) => {
      const key = element.getAttribute('data-i18n');
      const translation = this.getTranslationByKey(key);
      if (translation) {
        element.textContent = translation;
      }
    });

    document.querySelectorAll('[data-i18n-placeholder]').forEach((element) => {
      const key = element.getAttribute('data-i18n-placeholder');
      const translation = this.getTranslationByKey(key);
      if (translation) {
        element.placeholder = translation;
      }
    });

    document.querySelectorAll('[data-i18n-title]').forEach((element) => {
      const key = element.getAttribute('data-i18n-title');
      const translation = this.getTranslationByKey(key);
      if (translation) {
        element.title = translation;
      }
    });
  }

  getTranslationByKey(key) {
    if (!this.LL) return null;

    const parts = key.split('.');
    let current = this.LL;

    for (const part of parts) {
      if (current && typeof current === 'object' && part in current) {
        current = current[part];
      } else {
        return null;
      }
    }

    return typeof current === 'string' ? current : null;
  }

  translateError(errorMessage) {
    if (!this.LL) return errorMessage;

    const errorMap = {
      'Photo not found': this.LL.errors.photoNotFound(),
      'Database error': this.LL.errors.databaseError(),
      'Search failed': this.LL.errors.searchError(),
      'Failed to load photo': this.LL.errors.failedToLoadPhoto(),
      'Failed to load image': this.LL.errors.failedToLoadImage(),
      'Failed to read photo file': this.LL.errors.failedToReadPhotoFile(),
      'Invalid thumbnail size': this.LL.errors.invalidThumbnailSize(),
      'Server connection lost': this.LL.errors.connectionLost(),
    };

    const exactMatch = errorMap[errorMessage];
    if (exactMatch) return exactMatch;

    for (const [pattern, translation] of Object.entries(errorMap)) {
      if (errorMessage.includes(pattern)) {
        return translation;
      }
    }

    if (errorMessage.toLowerCase().includes('database')) {
      return this.LL.errors.databaseError();
    }

    return errorMessage;
  }

  t(key) {
    return this.getTranslationByKey(key) || key;
  }
}

const i18nManager = new I18nManager();

export { i18nManager };
export default i18nManager;
