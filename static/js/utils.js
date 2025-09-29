// Utility functions

// DOM helpers
const $ = (selector) => document.querySelector(selector);
const $$ = (selector) => document.querySelectorAll(selector);
const createElement = (tag, className = '', content = '') => {
  const element = document.createElement(tag);
  if (className) element.className = className;
  if (content) element.textContent = content;
  return element;
};

// Event helpers
const on = (element, event, handler) => element.addEventListener(event, handler);
const off = (element, event, handler) => element.removeEventListener(event, handler);
const emit = (element, event, data = null) => {
  const customEvent = new CustomEvent(event, { detail: data });
  element.dispatchEvent(customEvent);
};

// Format helpers
const formatFileSize = (bytes) => {
  if (bytes === 0) return '0 Bytes';
  const k = 1024;
  const sizes = ['Bytes', 'KB', 'MB', 'GB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
};

const formatDate = (dateString) => {
  try {
    const date = new Date(dateString);
    return date.toLocaleDateString('en-US', {
      year: 'numeric',
      month: 'long',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    });
  } catch {
    return 'Unknown';
  }
};

const formatDuration = (seconds) => {
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const secs = Math.floor(seconds % 60);

  if (hours > 0) {
    return `${hours}:${minutes.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}`;
  }
  return `${minutes}:${secs.toString().padStart(2, '0')}`;
};

// Debounce function
const debounce = (func, wait) => {
  let timeout;
  return function executedFunction(...args) {
    const later = () => {
      clearTimeout(timeout);
      func(...args);
    };
    clearTimeout(timeout);
    timeout = setTimeout(later, wait);
  };
};

// Throttle function
const throttle = (func, limit) => {
  let inThrottle;
  return function (...args) {
    if (!inThrottle) {
      func.apply(this, args);
      inThrottle = true;
      setTimeout(() => (inThrottle = false), limit);
    }
  };
};

// Loading indicator
const showLoading = () => {
  const indicator = $('#loading-indicator');
  if (indicator) indicator.classList.add('show');
};

const hideLoading = () => {
  const indicator = $('#loading-indicator');
  if (indicator) indicator.classList.remove('show');
};

// Toast notifications
const showToast = (title, message, type = 'info', duration = 4000) => {
  const container = $('#toast-container');
  if (!container) return;

  const toast = createElement('div', `toast ${type}`);
  toast.innerHTML = `
        <div class="toast-title">${title}</div>
        <div class="toast-message">${message}</div>
    `;

  container.appendChild(toast);

  // Trigger animation
  setTimeout(() => toast.classList.add('show'), 10);

  // Auto remove
  setTimeout(() => {
    toast.classList.remove('show');
    setTimeout(() => container.removeChild(toast), 300);
  }, duration);
};

// Image lazy loading
const createLazyImage = (src, alt = '', className = '') => {
  const img = createElement('img', `lazy-image ${className}`);
  img.alt = alt;

  const observer = new IntersectionObserver((entries) => {
    entries.forEach((entry) => {
      if (entry.isIntersecting) {
        const image = entry.target;
        image.src = src;
        image.onload = () => image.classList.add('loaded');
        image.onerror = () => {
          image.src =
            'data:image/svg+xml;base64,PHN2ZyB3aWR0aD0iMjAwIiBoZWlnaHQ9IjIwMCIgdmlld0JveD0iMCAwIDIwMCAyMDAiIGZpbGw9Im5vbmUiIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyI+CjxyZWN0IHdpZHRoPSIyMDAiIGhlaWdodD0iMjAwIiBmaWxsPSIjRjVGNUY1Ii8+CjxwYXRoIGQ9Ik04MCA4MEM4MCA3MS4xNjM0IDg3LjE2MzQgNjQgOTYgNjRIMTA0QzExMi44MzcgNjQgMTIwIDcxLjE2MzQgMTIwIDgwVjEyMEMxMjAgMTI4LjgzNyAxMTIuODM3IDEzNiAxMDQgMTM2SDk2Qzg3LjE2MzQgMTM2IDgwIDEyOC44MzcgODAgMTIwVjgwWiIgZmlsbD0iI0RERERERCIvPgo8L3N2Zz4K';
          image.classList.add('loaded');
        };
        observer.unobserve(image);
      }
    });
  });

  observer.observe(img);
  return img;
};

// Error handling
const handleError = (error, context = '') => {
  if (window.logger) {
    window.logger.error(`Error in ${context}`, error, { context });
  } else {
    console.error(`Error in ${context}:`, error);
  }

  const errorMessage = error.message || 'An unexpected error occurred';

  // Use i18n manager if available
  let translatedMessage = errorMessage;
  let translatedTitle = 'Error';

  if (window.i18nManager && window.i18nManager.LL) {
    translatedMessage = window.i18nManager.translateError(errorMessage);
    translatedTitle = window.i18nManager.LL.notifications.error();
  }

  showToast(translatedTitle, translatedMessage, 'error');
};

// URL helpers
const getPhotoUrl = (photoHash) => `/api/photos/${photoHash}/file`;
const getThumbnailUrl = (photo, size = 'medium') =>
  `/api/thumbnails/hash/${photo.hash_sha256}/${size}`;
const getVideoUrl = (photoHash) => `/api/photos/${photoHash}/video`;

// Local storage helpers
const storage = {
  get: (key, defaultValue = null) => {
    try {
      const item = localStorage.getItem(key);
      return item ? JSON.parse(item) : defaultValue;
    } catch {
      return defaultValue;
    }
  },

  set: (key, value) => {
    try {
      localStorage.setItem(key, JSON.stringify(value));
      return true;
    } catch (e) {
      console.warn('Failed to save to localStorage:', e);
      return false;
    }
  },

  remove: (key) => {
    try {
      localStorage.removeItem(key);
      return true;
    } catch {
      return false;
    }
  },
};

// State management
class SimpleState {
  constructor(initialState = {}) {
    this.state = { ...initialState };
    this.listeners = {};
  }

  get(key) {
    return this.state[key];
  }

  set(key, value) {
    const oldValue = this.state[key];
    this.state[key] = value;
    this.emit(key, value, oldValue);
  }

  update(updates) {
    Object.keys(updates).forEach((key) => {
      this.set(key, updates[key]);
    });
  }

  subscribe(key, callback) {
    if (!this.listeners[key]) {
      this.listeners[key] = [];
    }
    this.listeners[key].push(callback);

    // Return unsubscribe function
    return () => {
      this.listeners[key] = this.listeners[key].filter((cb) => cb !== callback);
    };
  }

  emit(key, newValue, oldValue) {
    if (this.listeners[key]) {
      this.listeners[key].forEach((callback) => {
        callback(newValue, oldValue);
      });
    }
  }
}

// Touch/gesture helpers for mobile
const touchHandler = {
  startX: 0,
  startY: 0,

  handleTouchStart(e) {
    this.startX = e.touches[0].clientX;
    this.startY = e.touches[0].clientY;
  },

  handleTouchEnd(e, onSwipeLeft, onSwipeRight, onSwipeUp, onSwipeDown) {
    if (!this.startX || !this.startY) return;

    const endX = e.changedTouches[0].clientX;
    const endY = e.changedTouches[0].clientY;

    const diffX = this.startX - endX;
    const diffY = this.startY - endY;

    // Determine if it's more horizontal or vertical
    if (Math.abs(diffX) > Math.abs(diffY)) {
      // Horizontal swipe
      if (Math.abs(diffX) > 50) {
        // Minimum swipe distance
        if (diffX > 0 && onSwipeLeft) onSwipeLeft();
        else if (diffX < 0 && onSwipeRight) onSwipeRight();
      }
    } else {
      // Vertical swipe
      if (Math.abs(diffY) > 50) {
        // Minimum swipe distance
        if (diffY > 0 && onSwipeUp) onSwipeUp();
        else if (diffY < 0 && onSwipeDown) onSwipeDown();
      }
    }

    this.startX = 0;
    this.startY = 0;
  },
};

// Performance helpers
const performanceUtils = {
  mark: (name) => {
    if (window.performance && window.performance.mark) {
      window.performance.mark(name);
    }
  },

  measure: (name, startMark, endMark) => {
    if (window.performance && window.performance.measure) {
      window.performance.measure(name, startMark, endMark);
    }
  },

  getEntries: () => {
    if (window.performance && window.performance.getEntries) {
      return window.performance.getEntries();
    }
    return [];
  },
};

// Export to global scope
window.utils = {
  $,
  $$,
  createElement,
  on,
  off,
  emit,
  formatFileSize,
  formatDate,
  formatDuration,
  debounce,
  throttle,
  showLoading,
  hideLoading,
  showToast,
  createLazyImage,
  handleError,
  getPhotoUrl,
  getThumbnailUrl,
  getVideoUrl,
  storage,
  SimpleState,
  touchHandler,
  performance: performanceUtils,
};
