// Icon Helper Utility for Feather Icons

class IconHelper {
  constructor() {
    // Wait for feather to be loaded
    if (typeof window.feather === 'undefined') {
      console.warn('Feather icons library not loaded');
    }
  }

  /**
   * Get an icon SVG element
   * @param {string} name - Feather icon name
   * @param {Object} options - Icon options (size, class, stroke-width)
   * @returns {string} SVG markup
   */
  getIcon(name, options = {}) {
    if (typeof window.feather === 'undefined') {
      console.warn('Feather icons library not loaded');
      return '';
    }

    const size = options.size || 24;
    const className = options.class || '';
    const strokeWidth = options.strokeWidth || 2;

    // Get SVG from feather
    const svg = window.feather.icons[name];
    if (!svg) {
      console.warn(`Icon "${name}" not found in feather icons`);
      return '';
    }

    return svg.toSvg({
      width: size,
      height: size,
      class: className,
      'stroke-width': strokeWidth,
    });
  }

  /**
   * Create an icon element (for appending to DOM)
   * @param {string} name - Feather icon name
   * @param {Object} options - Icon options
   * @returns {Element} Icon element
   */
  createIcon(name, options = {}) {
    const wrapper = document.createElement('span');
    wrapper.className = 'icon-wrapper';
    wrapper.innerHTML = this.getIcon(name, options);
    return wrapper;
  }

  /**
   * Replace an element's content with an icon
   * @param {Element} element - Target element
   * @param {string} iconName - Feather icon name
   * @param {Object} options - Icon options
   */
  replaceWithIcon(element, iconName, options = {}) {
    if (!element) return;
    element.innerHTML = this.getIcon(iconName, options);
  }

  /**
   * Initialize all icons with data-icon attribute
   */
  initializeIcons() {
    const iconElements = document.querySelectorAll('[data-icon]');
    iconElements.forEach((el) => {
      const iconName = el.dataset.icon;
      const size = el.dataset.iconSize || 24;
      const className = el.dataset.iconClass || '';
      const strokeWidth = el.dataset.iconStrokeWidth || 2;

      this.replaceWithIcon(el, iconName, {
        size: parseInt(size),
        class: className,
        strokeWidth: parseInt(strokeWidth),
      });
    });
  }

  /**
   * Semantic icon mapping for common UI actions
   */
  static get semanticIcons() {
    return {
      // Actions
      favorite: 'heart',
      favoriteOutline: 'heart',
      download: 'download',
      close: 'x',
      delete: 'trash-2',

      // Navigation
      previous: 'chevron-left',
      next: 'chevron-right',
      back: 'arrow-left',
      forward: 'arrow-right',

      // Zoom/View
      zoomIn: 'plus',
      zoomOut: 'minus',
      fitToScreen: 'minimize',
      fullscreen: 'maximize',
      info: 'info',

      // Interface
      search: 'search',
      menu: 'menu',
      settings: 'settings',
      filter: 'filter',

      // Theme
      darkMode: 'moon',
      lightMode: 'sun',

      // Media
      photo: 'camera',
      video: 'video',
      play: 'play',
      pause: 'pause',

      // Status
      error: 'x-circle',
      warning: 'alert-triangle',
      success: 'check-circle',
      loading: 'loader',
    };
  }

  /**
   * Get a semantic icon by purpose
   * @param {string} purpose - Semantic purpose (e.g., 'favorite', 'download')
   * @param {Object} options - Icon options
   * @returns {string} SVG markup
   */
  getSemanticIcon(purpose, options = {}) {
    const iconName = IconHelper.semanticIcons[purpose];
    if (!iconName) {
      console.warn(`No semantic icon defined for purpose: ${purpose}`);
      return '';
    }
    return this.getIcon(iconName, options);
  }
}

// Initialize global icon helper
window.iconHelper = new IconHelper();

// Auto-initialize icons when DOM is ready
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', () => {
    if (window.iconHelper && typeof window.feather !== 'undefined') {
      window.iconHelper.initializeIcons();
    }
  });
} else {
  if (window.iconHelper && typeof window.feather !== 'undefined') {
    window.iconHelper.initializeIcons();
  }
}
