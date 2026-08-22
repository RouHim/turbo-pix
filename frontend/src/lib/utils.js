// Utility functions
import { addToast } from './state.svelte.js';
import { APP_CONSTANTS } from './constants.js';
import { logger } from './logger.js';
import { t, translateError, getLocale } from './i18n.js';
import { get } from 'svelte/store';

// ── Format helpers ──────────────────────────────────────────────────────────

/**
 * True for backend filter-prefix queries (type:, location:, is_favorite:),
 * which the search pipeline routes through the regular (non-semantic) path.
 * Keep in sync with SearchBar.performSearch.
 * @param {string} q
 * @returns {boolean}
 */
export function isPrefixQuery(q) {
  return q.startsWith('type:') || q.startsWith('location:') || q.startsWith('is_favorite:');
}

/**
 * True for filenames whose extension is a supported video container.
 * @param {string} filename
 * @returns {boolean}
 */
export function isVideoFile(filename) {
  if (!filename) return false;
  const ext = filename.toLowerCase().substring(filename.lastIndexOf('.'));
  return APP_CONSTANTS.VIDEO_EXTENSIONS.includes(ext);
}

/**
 * True for filenames whose extension is a supported RAW format.
 * @param {string} filename
 * @returns {boolean}
 */
export function isRawFile(filename) {
  if (!filename) return false;
  const ext = filename.toLowerCase().substring(filename.lastIndexOf('.'));
  return APP_CONSTANTS.RAW_EXTENSIONS.includes(ext);
}

/**
 * True for photos that are actually collage records (isCollage flag or a collage id).
 * @param {object|null} photo
 * @returns {boolean}
 */
export function isCollagePhoto(photo) {
  return Boolean(photo?.isCollage || photo?.collageId != null);
}

/**
 * True for photo records whose metadata can be edited (JPEG/PNG).
 * @param {object|null} p - Photo record with a mime_type field
 * @returns {boolean}
 */
export function isFormatSupported(p) {
  if (!p?.mime_type) return false;
  const supported = ['image/jpeg', 'image/jpg', 'image/png'];
  return supported.includes(p.mime_type.toLowerCase());
}

/**
 * Format a file size in bytes to a human-readable string.
 * @param {number} bytes
 * @returns {string}
 */
export function formatFileSize(bytes) {
  if (!Number.isFinite(bytes) || bytes < 0) {
    return get(t)('ui.unknown', { default: 'Unknown' });
  }
  if (bytes === 0) return get(t)('ui.file_size_zero', { default: '0 Bytes' });
  const k = 1024;
  const sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB', 'PB'];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(k)), sizes.length - 1);
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
    return get(t)('ui.unknown', { default: 'Unknown' });
  }
}

/**
 * Format a duration in seconds to HH:MM:SS or MM:SS.
 * @param {number} seconds
 * @returns {string}
 */
export function formatDuration(seconds) {
  if (!Number.isFinite(seconds) || seconds < 0) {
    return get(t)('ui.unknown', { default: 'Unknown' });
  }
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

  const monthName = get(t)(`ui.months.${monthKey}`, {
    default: monthKey.charAt(0).toUpperCase() + monthKey.slice(1),
  });
  const weekdayName = get(t)(`ui.weekdays.${weekdayKey}`, {
    default: weekdayKey.charAt(0).toUpperCase() + weekdayKey.slice(1),
  });
  const locale = getLocale() || 'en';

  if (locale === 'de') {
    return `${weekdayName}, ${day}. ${monthName} ${year}`;
  }

  return `${weekdayName}, ${monthName} ${day}, ${year}`;
}

// ── Timing helpers ──────────────────────────────────────────────────────────

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

  const errorMessage =
    error.message || get(t)('errors.unexpectedError', { default: 'An unexpected error occurred' });

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
 * Browser video codec support detection.
 *
 * The capability set is serialized into a comma-joined string
 * (`h264-8,h264-10,hevc,av1,vp9,vp8`) sent to the server as the
 * `X-TurboPix-Codecs` header so the serve-time decision (Direct Play / remux /
 * transcode) is authoritative on the backend rather than guessed client-side.
 *
 * Probing follows Jellyfin's convention: `canPlayType` results of `probably`
 * OR `maybe` count as supported (accepting only `probably` under-reports —
 * Firefox routinely answers `maybe` for codecs it can actually decode).
 */
export const videoCodecSupport = {
  _cache: {},
  _clientString: null,

  /**
   * Jellyfin-style `canPlayType` probe: any result other than `no` is taken
   * as "supported" (`probably` and `maybe` both count).
   * @param {string} mimeType - e.g. 'video/mp4; codecs="avc1.42E01E, mp4a.40.2"'
   * @param {HTMLVideoElement} [video] - optional element to use
   * @returns {boolean}
   */
  canPlayType(mimeType, video = undefined) {
    if (typeof document === 'undefined' || !document.createElement) return false;
    const el = video || document.createElement('video');
    if (typeof el.canPlayType !== 'function') return false;
    return !!el.canPlayType(mimeType).replace(/no/, '');
  },

  /**
   * H.264 8-bit (Baseline/Main/High profiles — virtually every browser).
   * Matches Jellyfin's `canPlayH264` probe.
   * @returns {boolean}
   */
  canPlayH264() {
    return this.canPlayType('video/mp4; codecs="avc1.42E01E, mp4a.40.2"');
  },

  /**
   * H.264 High-10 profile (10-bit). Rarely supported in browsers; only sent
   * to the server when genuinely supported so 10-bit H.264 can Direct Play.
   * Profile idc 0x6E = High 10, level 40 (0x28).
   * @returns {boolean}
   */
  canPlayH264High10() {
    return this.canPlayType('video/mp4; codecs="avc1.6E0028, mp4a.40.2"');
  },

  /**
   * Sync HEVC capability hint. Firefox has no HEVC decoder, so it always
   * reports false regardless of `canPlayType` (matches `supportsHEVC`).
   * @returns {boolean}
   */
  canPlayHEVC() {
    const isFirefox =
      typeof navigator !== 'undefined' &&
      typeof navigator.userAgent === 'string' &&
      navigator.userAgent.toLowerCase().includes('firefox');
    if (isFirefox) return false;
    const hevcCodecs = ['hvc1.1.6.L93.B0', 'hvc1.1.6.L120.B0', 'hev1.1.6.L93.B0'];
    return hevcCodecs.some((codec) => this.canPlayType(`video/mp4; codecs="${codec}"`));
  },

  /**
   * AV1 in MP4 (`av01.0.15M.08` = Main 8-bit). AV1 'maybe' is accepted.
   * @returns {boolean}
   */
  canPlayAV1() {
    return this.canPlayType('video/mp4; codecs="av01.0.15M.08"');
  },

  /**
   * VP9 in WebM.
   * @returns {boolean}
   */
  canPlayVP9() {
    return this.canPlayType('video/webm; codecs="vp9"');
  },

  /**
   * VP8 in WebM.
   * @returns {boolean}
   */
  canPlayVP8() {
    return this.canPlayType('video/webm; codecs="vp8"');
  },

  /**
   * The client's supported codec set as a comma-joined capability string for
   * the `X-TurboPix-Codecs` header (server's ClientCodecs::parse format:
   * `h264-8,h264-10,hevc,av1,vp9,vp8`; only supported tokens are emitted).
   * Memoized; call `clearCache()` to recompute.
   * @returns {string}
   */
  getClientCodecsString() {
    if (this._clientString !== null) return this._clientString;
    const parts = [];
    if (this.canPlayH264()) parts.push('h264-8');
    if (this.canPlayH264High10()) parts.push('h264-10');
    if (this.canPlayHEVC()) parts.push('hevc');
    if (this.canPlayAV1()) parts.push('av1');
    if (this.canPlayVP9()) parts.push('vp9');
    if (this.canPlayVP8()) parts.push('vp8');
    this._clientString = parts.join(',');
    return this._clientString;
  },

  /**
   * The value to send as the `X-TurboPix-Codecs` request header.
   * @returns {string}
   */
  get clientCodecsHeader() {
    return this.getClientCodecsString();
  },

  /**
   * Check if browser supports a specific video codec via Media Capabilities.
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
      const supported = this.canPlayType(`video/mp4; codecs="${codec}"`);
      this._cache[cacheKey] = supported;

      if (logger) {
        logger.info('Codec support fallback check', {
          component: 'VideoCodecSupport',
          codec,
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
    // A synchronous answer is available without a DOM/mediaCapabilities
    // round-trip; defer to it so the async API and the header stay consistent.
    return this.canPlayHEVC() || (await this.canPlayCodec('hvc1.1.6.L93.B0', width, height));
  },

  /**
   * Clear the codec support cache.
   */
  clearCache() {
    this._cache = {};
    this._clientString = null;
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
