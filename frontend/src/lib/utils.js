// Utility functions
import { addToast } from './state.svelte.js';
import { APP_CONSTANTS } from './constants.js';
import { logger } from './logger.js';
import { t, translateError, getLocale } from './i18n.js';
import { get } from 'svelte/store';

// ── Format helpers ──────────────────────────────────────────────────────────

/**
 * Format a file size in bytes to a human-readable string.
 * @param {number} bytes
 * @returns {string}
 */
export function formatFileSize(bytes) {
  if (bytes === 0) return '0 Bytes';
  const k = 1024;
  const sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB', 'PB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
}

/**
 * Format a date string into a localized readable format.
 * @param {string} dateString
 * @returns {string}
 */
export function formatDate(dateString) {
  try {
    const date = new Date(dateString);
    const locale = getLocale() || navigator.language || 'en';
    return date.toLocaleDateString(locale, {
      year: 'numeric',
      month: 'long',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    });
  } catch {
    return 'Unknown';
  }
}

/**
 * Format a duration in seconds to HH:MM:SS or MM:SS.
 * @param {number} seconds
 * @returns {string}
 */
export function formatDuration(seconds) {
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const secs = Math.floor(seconds % 60);

  if (hours > 0) {
    return `${hours}:${minutes.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}`;
  }
  return `${minutes}:${secs.toString().padStart(2, '0')}`;
}

/**
 * Format a collage date string (YYYY-MM-DD) into a localized readable format.
 * @param {string} dateString - Date in YYYY-MM-DD format
 * @returns {string} Formatted date string
 */
export function formatCollageDate(dateString) {
  if (!dateString || typeof dateString !== 'string') {
    return dateString;
  }

  const parts = dateString.split('-').map((value) => parseInt(value, 10));
  if (parts.length !== 3 || parts.some((value) => Number.isNaN(value))) {
    return dateString;
  }

  const [year, month, day] = parts;
  const monthKey = APP_CONSTANTS?.MONTH_KEYS?.[month - 1];
  if (!monthKey) {
    return dateString;
  }

  const date = new Date(Date.UTC(year, month - 1, day));
  const weekdayKey = APP_CONSTANTS?.WEEKDAY_KEYS?.[date.getUTCDay()];
  if (!weekdayKey) {
    return dateString;
  }

  const monthName =
    get(t)(`ui.months.${monthKey}`) || monthKey.charAt(0).toUpperCase() + monthKey.slice(1);
  const weekdayName =
    get(t)(`ui.weekdays.${weekdayKey}`) || weekdayKey.charAt(0).toUpperCase() + weekdayKey.slice(1);
  const locale = getLocale() || 'en';

  if (locale === 'de') {
    return `${weekdayName}, ${day}. ${monthName} ${year}`;
  }

  return `${weekdayName}, ${monthName} ${day}, ${year}`;
}

// ── Timing helpers ──────────────────────────────────────────────────────────

/**
 * Debounce a function call.
 * @param {Function} func
 * @param {number} delay - milliseconds
 * @returns {Function}
 */
export function debounce(func, delay) {
  let timeout;
  return function executedFunction(...args) {
    const later = () => {
      clearTimeout(timeout);
      func(...args);
    };
    clearTimeout(timeout);
    timeout = setTimeout(later, delay);
  };
}

/**
 * Throttle a function call.
 * @param {Function} func
 * @param {number} limit - milliseconds
 * @returns {Function}
 */
export function throttle(func, limit) {
  let inThrottle;
  return function (...args) {
    if (!inThrottle) {
      func.apply(this, args);
      inThrottle = true;
      setTimeout(() => (inThrottle = false), limit);
    }
  };
}

// ── Toast notifications ─────────────────────────────────────────────────────

/**
 * Display a toast notification via the reactive toast system.
 * @param {string} title
 * @param {string} message
 * @param {'info'|'success'|'error'} type
 * @param {number} duration - ms, 0 = persistent
 */
export function showToast(title, message = '', type = 'info', duration = 3000) {
  try {
    addToast(title, message, type, duration);
  } catch {
    /* state may not be ready */
  }
}

// ── Error handling ──────────────────────────────────────────────────────────

/**
 * Log an error and display a toast notification.
 * @param {Error} error
 * @param {string} context - description of where the error occurred
 */
export function handleError(error, context = '') {
  if (logger) {
    logger.error(`Error in ${context}`, error, { context });
  } else {
    console.error(`Error in ${context}:`, error);
  }

  const errorMessage = error.message || 'An unexpected error occurred';

  // Use i18n if available
  let translatedMessage = errorMessage;
  let translatedTitle = 'Error';

  try {
    translatedMessage = translateError(errorMessage);
    const titleTranslation = get(t)('notifications.error');
    if (titleTranslation && titleTranslation !== 'notifications.error') {
      translatedTitle = titleTranslation;
    }
  } catch {
    // Fall back to untranslated strings
  }

  showToast(translatedTitle, translatedMessage, 'error');
}

// ── URL helpers ─────────────────────────────────────────────────────────────

/**
 * Get the full-resolution photo URL.
 * @param {string} photoHash
 * @returns {string}
 */
export function getPhotoUrl(photoHash) {
  return `/api/photos/${photoHash}/file`;
}

/**
 * Get a thumbnail URL for a photo.
 * @param {{hash_sha256: string}} photo
 * @param {'small'|'medium'|'large'} size
 * @returns {string}
 */
export function getThumbnailUrl(photo, size = 'medium') {
  return `/api/photos/${photo.hash_sha256}/thumbnail?size=${size}`;
}

/**
 * Get a video URL for a photo, optionally with transcoding.
 * @param {string} photoHash
 * @param {{transcode?: boolean}} options
 * @returns {string}
 */
export function getVideoUrl(photoHash, options = {}) {
  const params = new URLSearchParams();
  if (options.transcode) {
    params.set('transcode', 'true');
  }
  const queryString = params.toString();
  return `/api/photos/${photoHash}/video${queryString ? `?${queryString}` : ''}`;
}

// ── Local storage helpers ───────────────────────────────────────────────────

/**
 * JSON-aware localStorage wrapper.
 * Preserves the exact JSON encoding used by the original code
 * (theme, viewSettings, searchHistory are stored JSON-encoded).
 */
export const storage = {
  get(key, defaultValue = null) {
    try {
      const item = localStorage.getItem(key);
      return item ? JSON.parse(item) : defaultValue;
    } catch {
      return defaultValue;
    }
  },

  set(key, value) {
    try {
      localStorage.setItem(key, JSON.stringify(value));
      return true;
    } catch (e) {
      console.warn('Failed to save to localStorage:', e);
      return false;
    }
  },

  remove(key) {
    try {
      localStorage.removeItem(key);
      return true;
    } catch {
      return false;
    }
  },
};

// ── Video codec detection ───────────────────────────────────────────────────

/**
 * Browser video codec support detection using Media Capabilities API.
 */
export const videoCodecSupport = {
  _cache: {},

  /**
   * Check if browser supports a specific video codec.
   * @param {string} codec - Codec string (e.g., 'hvc1.1.6.L93.B0' for HEVC)
   * @param {number} width - Video width
   * @param {number} height - Video height
   * @returns {Promise<boolean>}
   */
  async canPlayCodec(codec, width = 1920, height = 1080) {
    const cacheKey = `${codec}-${width}x${height}`;

    if (this._cache[cacheKey] !== undefined) {
      return this._cache[cacheKey];
    }

    // Fallback: basic video element support check
    if (!navigator.mediaCapabilities || !navigator.mediaCapabilities.decodingInfo) {
      const video = document.createElement('video');
      const canPlay = video.canPlayType(`video/mp4; codecs="${codec}"`);
      // IMPORTANT: Only trust 'probably', not 'maybe'
      const supported = canPlay === 'probably';
      this._cache[cacheKey] = supported;

      if (logger) {
        logger.info('Codec support fallback check', {
          component: 'VideoCodecSupport',
          codec,
          canPlay,
          supported,
        });
      }

      return supported;
    }

    try {
      const config = {
        type: 'file',
        video: {
          contentType: `video/mp4; codecs="${codec}"`,
          width,
          height,
          bitrate: 10000000,
          framerate: 30,
        },
      };

      const result = await navigator.mediaCapabilities.decodingInfo(config);
      const supported = result.supported && result.smooth;
      this._cache[cacheKey] = supported;

      if (logger) {
        logger.info('Codec support check', {
          component: 'VideoCodecSupport',
          codec,
          width,
          height,
          supported,
          smooth: result.smooth,
          powerEfficient: result.powerEfficient,
        });
      }

      return supported;
    } catch (error) {
      if (logger) {
        logger.warn('Failed to check codec support', error, {
          component: 'VideoCodecSupport',
          codec,
        });
      }
      this._cache[cacheKey] = false;
      return false;
    }
  },

  /**
   * Check if browser supports HEVC (H.265) codec.
   * Firefox always returns false — HEVC support is unreliable there.
   * @param {number} width - Video width
   * @param {number} height - Video height
   * @returns {Promise<boolean>}
   */
  async supportsHEVC(width = 1920, height = 1080) {
    const isFirefox = navigator.userAgent.toLowerCase().includes('firefox');
    if (isFirefox) {
      if (logger) {
        logger.info('Firefox detected - forcing HEVC transcoding', {
          component: 'VideoCodecSupport',
          userAgent: navigator.userAgent,
        });
      }
      return false;
    }

    const hevcCodecs = ['hvc1.1.6.L93.B0', 'hvc1.1.6.L120.B0', 'hev1.1.6.L93.B0'];

    for (const codec of hevcCodecs) {
      if (await this.canPlayCodec(codec, width, height)) {
        return true;
      }
    }

    return false;
  },

  /**
   * Clear the codec support cache.
   */
  clearCache() {
    this._cache = {};
  },
};

// ── Performance helpers ─────────────────────────────────────────────────────

/**
 * Performance measurement utilities wrapping window.performance.
 */
export const performance = {
  mark(name) {
    if (window.performance && window.performance.mark) {
      window.performance.mark(name);
    }
  },

  measure(name, startMark, endMark) {
    if (window.performance && window.performance.measure) {
      window.performance.measure(name, startMark, endMark);
    }
  },

  getEntries() {
    if (window.performance && window.performance.getEntries) {
      return window.performance.getEntries();
    }
    return [];
  },
};
