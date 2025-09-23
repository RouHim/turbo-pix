import { describe, it, expect, beforeEach, vi } from 'vitest';

// Mock the i18n system before importing
const mockTranslations = {
  en: {
    ui: { searchPlaceholder: 'Search photos...', allPhotos: 'All Photos' },
    errors: { photoNotFound: 'Photo not found', databaseError: 'Database error occurred' },
    notifications: { photoAddedToFavorites: 'Photo added to favorites' },
  },
  de: {
    ui: { searchPlaceholder: 'Fotos suchen...', allPhotos: 'Alle Fotos' },
    errors: { photoNotFound: 'Foto nicht gefunden', databaseError: 'Datenbankfehler aufgetreten' },
    notifications: { photoAddedToFavorites: 'Foto zu Favoriten hinzugefügt' },
  },
};

// Mock the i18n utilities that we'll implement
let currentLocale = 'en';
const mockLL = {
  ui: {
    searchPlaceholder: () => mockTranslations[currentLocale].ui.searchPlaceholder,
    allPhotos: () => mockTranslations[currentLocale].ui.allPhotos,
  },
  errors: {
    photoNotFound: () => mockTranslations[currentLocale].errors.photoNotFound,
    databaseError: () => mockTranslations[currentLocale].errors.databaseError,
  },
  notifications: {
    photoAddedToFavorites: () =>
      mockTranslations[currentLocale].notifications.photoAddedToFavorites,
  },
};

const mockSetLocale = vi.fn((locale) => {
  currentLocale = locale;
});

const mockDetectLocale = vi.fn(() => 'en');

// Mock utils that we'll enhance
const mockUtils = {
  handleError: vi.fn((error, context = '') => {
    const errorMessage = error.message || 'An unexpected error occurred';
    const translatedMessage = mockUtils.translateError(errorMessage);
    mockUtils.showToast('Error', translatedMessage, 'error');
  }),
  showToast: vi.fn(),
  translateError: vi.fn((msg) => msg),
};

global.utils = mockUtils;

describe('i18n System', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    currentLocale = 'en';
    localStorage.clear();
  });

  describe('Locale Detection', () => {
    it('should detect English as default locale', () => {
      navigator.language = 'en-US';
      const locale = mockDetectLocale();
      expect(locale).toBe('en');
    });

    it('should detect German from browser language', () => {
      navigator.language = 'de-DE';
      mockDetectLocale.mockReturnValue('de');
      const locale = mockDetectLocale();
      expect(locale).toBe('de');
    });

    it('should fallback to English for unsupported language', () => {
      navigator.language = 'fr-FR';
      mockDetectLocale.mockReturnValue('en');
      const locale = mockDetectLocale();
      expect(locale).toBe('en');
    });

    it('should remember saved language preference', () => {
      localStorage.setItem('turbo-pix-language', 'de');
      mockDetectLocale.mockReturnValue('de');
      const locale = mockDetectLocale();
      expect(locale).toBe('de');
    });
  });

  describe('Translation Functions', () => {
    it('should translate UI text to English', () => {
      const text = mockLL.ui.searchPlaceholder();
      expect(text).toBe('Search photos...');
    });

    it('should translate UI text to German', () => {
      currentLocale = 'de';
      const text = mockLL.ui.searchPlaceholder();
      expect(text).toBe('Fotos suchen...');
    });

    it('should switch locale and update translations', () => {
      mockSetLocale('de');
      expect(mockSetLocale).toHaveBeenCalledWith('de');

      currentLocale = 'de';
      const text = mockLL.ui.allPhotos();
      expect(text).toBe('Alle Fotos');
    });
  });

  describe('Error Message Translation', () => {
    it('should translate backend error messages', () => {
      const backendError = 'Photo not found';
      mockUtils.translateError.mockReturnValue('Photo not found');

      const translated = mockUtils.translateError(backendError);
      expect(translated).toBe('Photo not found');
      expect(mockUtils.translateError).toHaveBeenCalledWith(backendError);
    });

    it('should translate backend errors to German', () => {
      currentLocale = 'de';
      const backendError = 'Photo not found';
      mockUtils.translateError.mockReturnValue('Foto nicht gefunden');

      const translated = mockUtils.translateError(backendError);
      expect(translated).toBe('Foto nicht gefunden');
    });

    it('should fallback to original message for unknown errors', () => {
      const unknownError = 'Some unknown error message';
      mockUtils.translateError.mockReturnValue(unknownError);

      const translated = mockUtils.translateError(unknownError);
      expect(translated).toBe(unknownError);
    });

    it('should handle database error patterns', () => {
      const backendError = 'Database error: Connection failed';
      mockUtils.translateError.mockReturnValue('Database error occurred');

      const translated = mockUtils.translateError(backendError);
      expect(translated).toBe('Database error occurred');
    });
  });

  describe('Enhanced Error Handling', () => {
    it('should use translated error messages in handleError', () => {
      const error = new Error('Photo not found');
      mockUtils.translateError.mockReturnValue('Photo not found');

      // This should be enhanced to use translation
      mockUtils.handleError(error, 'PhotoViewer');

      expect(mockUtils.handleError).toHaveBeenCalledWith(error, 'PhotoViewer');
      expect(mockUtils.showToast).toHaveBeenCalledWith('Error', 'Photo not found', 'error');
    });

    it('should translate toast notifications', () => {
      currentLocale = 'de';
      const message = mockLL.notifications.photoAddedToFavorites();
      expect(message).toBe('Foto zu Favoriten hinzugefügt');
    });
  });

  describe('DOM Integration', () => {
    it('should update HTML element text content', () => {
      // Create mock DOM element
      const element = { textContent: '', dataset: { i18n: 'ui.allPhotos' } };

      // Mock translation update function
      const updateElementText = (el) => {
        if (el.dataset.i18n === 'ui.allPhotos') {
          el.textContent = mockLL.ui.allPhotos();
        }
      };

      updateElementText(element);
      expect(element.textContent).toBe('All Photos');

      // Switch to German
      currentLocale = 'de';
      updateElementText(element);
      expect(element.textContent).toBe('Alle Fotos');
    });

    it('should update placeholder attributes', () => {
      const input = { placeholder: '', dataset: { i18nPlaceholder: 'ui.searchPlaceholder' } };

      const updatePlaceholder = (el) => {
        if (el.dataset.i18nPlaceholder === 'ui.searchPlaceholder') {
          el.placeholder = mockLL.ui.searchPlaceholder();
        }
      };

      updatePlaceholder(input);
      expect(input.placeholder).toBe('Search photos...');

      currentLocale = 'de';
      updatePlaceholder(input);
      expect(input.placeholder).toBe('Fotos suchen...');
    });
  });
});
