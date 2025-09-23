class I18nManager {
  constructor() {
    this.currentLocale = 'en';
    this.translations = {};
    this.initialized = false;
  }

  detectBrowserLocale() {
    const saved = localStorage.getItem('turbo-pix-locale');
    if (saved && this.isValidLocale(saved)) {
      return saved;
    }

    const browserLocale = navigator.language.split('-')[0];
    return this.isValidLocale(browserLocale) ? browserLocale : 'en';
  }

  isValidLocale(locale) {
    return ['en', 'de'].includes(locale);
  }

  async initializeI18n() {
    if (this.initialized) return;

    const locale = this.detectBrowserLocale();
    await this.setLocale(locale);
    this.initialized = true;
  }

  async setLocale(locale) {
    if (!this.isValidLocale(locale)) {
      locale = 'en';
    }

    this.currentLocale = locale;

    // Load translations if not already loaded
    if (!this.translations[locale]) {
      if (locale === 'en' && window.EN_TRANSLATIONS) {
        this.translations.en = window.EN_TRANSLATIONS;
      } else if (locale === 'de' && window.DE_TRANSLATIONS) {
        this.translations.de = window.DE_TRANSLATIONS;
      }
    }

    localStorage.setItem('turbo-pix-locale', locale);
    this.updateDOM();
  }

  getLocale() {
    return this.currentLocale;
  }

  updateDOM() {
    const currentTranslations = this.translations[this.currentLocale];
    if (!currentTranslations) return;

    // Update elements with data-i18n
    document.querySelectorAll('[data-i18n]').forEach((element) => {
      const key = element.getAttribute('data-i18n');
      const translation = this.getTranslationByKey(key, currentTranslations);
      if (translation) {
        element.textContent = translation;
      }
    });

    // Update placeholder attributes
    document.querySelectorAll('[data-i18n-placeholder]').forEach((element) => {
      const key = element.getAttribute('data-i18n-placeholder');
      const translation = this.getTranslationByKey(key, currentTranslations);
      if (translation) {
        element.placeholder = translation;
      }
    });

    // Update title attributes
    document.querySelectorAll('[data-i18n-title]').forEach((element) => {
      const key = element.getAttribute('data-i18n-title');
      const translation = this.getTranslationByKey(key, currentTranslations);
      if (translation) {
        element.title = translation;
      }
    });

    // Update alt attributes
    document.querySelectorAll('[data-i18n-alt]').forEach((element) => {
      const key = element.getAttribute('data-i18n-alt');
      const translation = this.getTranslationByKey(key, currentTranslations);
      if (translation) {
        element.alt = translation;
      }
    });
  }

  getTranslationByKey(key, translations = null) {
    if (!translations) {
      translations = this.translations[this.currentLocale];
    }

    if (!translations) return null;

    const parts = key.split('.');
    let current = translations;

    for (const part of parts) {
      if (current && typeof current === 'object' && part in current) {
        current = current[part];
      } else {
        return null;
      }
    }

    return typeof current === 'string' ? current : null;
  }

  t(key) {
    return this.getTranslationByKey(key) || key;
  }

  translateError(errorMessage) {
    const currentTranslations = this.translations[this.currentLocale];
    if (!currentTranslations || !currentTranslations.errors) return errorMessage;

    const errorMap = {
      'Photo not found': currentTranslations.errors.photoNotFound,
      'Database error': currentTranslations.errors.databaseError,
      'Search failed': currentTranslations.errors.searchError,
      'Failed to load photo': currentTranslations.errors.failedToLoadPhoto,
      'Failed to load image': currentTranslations.errors.failedToLoadImage,
      'Failed to read photo file': currentTranslations.errors.failedToReadPhotoFile,
      'Invalid thumbnail size': currentTranslations.errors.invalidThumbnailSize,
      'Server connection lost': currentTranslations.errors.connectionLost,
    };

    const exactMatch = errorMap[errorMessage];
    if (exactMatch) return exactMatch;

    for (const [pattern, translation] of Object.entries(errorMap)) {
      if (errorMessage.includes(pattern)) {
        return translation;
      }
    }

    if (errorMessage.toLowerCase().includes('database')) {
      return currentTranslations.errors.databaseError;
    }

    return errorMessage;
  }
}

// Export for module usage (global instance created in app.js)
if (typeof module !== 'undefined' && module.exports) {
  module.exports = { I18nManager };
} else {
  window.I18nManager = I18nManager;
}
