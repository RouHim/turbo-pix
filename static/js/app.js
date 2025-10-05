// Main Application Controller

// Constants
const MOBILE_BREAKPOINT = 768;

class TurboPixApp {
  constructor() {
    this.state = new utils.SimpleState({
      currentView: 'all',
      isLoading: false,
      isMobile: window.innerWidth <= MOBILE_BREAKPOINT,
      sidebarOpen: false,
      sortBy: 'date_desc',
      totalPhotos: 0,
      selectedPhotos: [],
      timelineFilter: null, // { year: 2011, month: 5 } or null
    });
  }

  async init() {
    if (window.logger) {
      window.logger.info('TurboPix App initializing', {
        userAgent: navigator.userAgent,
        url: window.location.href,
      });
    }

    // Initialize i18n system first
    await this.initializeI18n();

    this.bindEvents();
    this.setupNavigation();
    this.setupViewControls();
    this.setupResponsiveLayout();
    this.initTheme();
    this.loadInitialData();
    this.startPerformanceMonitoring();
  }

  async initializeI18n() {
    try {
      // Create i18nManager instance
      if (!window.i18nManager) {
        window.i18nManager = new window.I18nManager();
      }

      // Initialize the i18n system
      await window.i18nManager.initializeI18n();

      if (window.logger) {
        window.logger.info('i18n system initialized');
      }
    } catch (error) {
      if (window.logger) {
        window.logger.error('Failed to initialize i18n', error);
      }
    }
  }

  bindEvents() {
    // Navigation
    utils.$$('.nav-item').forEach((item) => {
      utils.on(item, 'click', () => {
        const view = item.dataset.view;
        if (view) {
          this.switchView(view);
          // Close mobile sidebar after selection
          this.closeMobileSidebar();
        }
      });
    });

    // View mode toggles
    // Removed - view switching functionality removed

    // Sort controls
    const sortSelect = utils.$('#sort-select');
    if (sortSelect) {
      utils.on(sortSelect, 'change', (e) => {
        this.setSortBy(e.target.value);
      });
    }

    // Theme toggle
    const themeToggle = utils.$('#theme-toggle');
    if (themeToggle) {
      utils.on(themeToggle, 'click', this.toggleTheme.bind(this));
    }

    // Mobile menu
    this.setupMobileMenu();

    // Window events
    utils.on(
      window,
      'resize',
      utils.throttle(() => {
        this.handleResize();
      }, 250)
    );

    utils.on(window, 'beforeunload', () => {
      this.saveState();
    });

    // Custom events
    utils.on(window, 'favoriteToggled', (e) => {
      this.handleFavoriteToggled(e.detail);
    });

    // Health check
    this.startHealthChecking();
  }

  setupNavigation() {
    // Set initial active nav item
    const initialView = this.state.get('currentView');
    const navItem = utils.$(`.nav-item[data-view="${initialView}"]`);
    if (navItem) {
      navItem.classList.add('active');
    }

    // Skip loading collections and cameras for sidebar - features removed
  }

  setupViewControls() {
    // Set initial sort
    const sortBy = this.state.get('sortBy');
    const sortSelect = utils.$('#sort-select');
    if (sortSelect) {
      sortSelect.value = sortBy;
    }
  }

  setupResponsiveLayout() {
    const isMobile = window.innerWidth <= MOBILE_BREAKPOINT;
    this.state.set('isMobile', isMobile);

    if (isMobile) {
      this.setupMobileLayout();
    }
  }

  setupMobileLayout() {
    // Add mobile menu button
    const header = utils.$('.header-content');
    if (header && !utils.$('.menu-btn')) {
      const menuBtn = utils.createElement('button', 'menu-btn');
      menuBtn.innerHTML = window.iconHelper.getSemanticIcon('menu', { size: 20 });
      menuBtn.title = window.i18nManager ? window.i18nManager.t('ui.menu') : 'Menu';
      header.insertBefore(menuBtn, header.firstChild);

      utils.on(menuBtn, 'click', () => {
        this.toggleMobileSidebar();
      });
    }

    // Create mobile search container if it doesn't exist
    if (!utils.$('.mobile-search')) {
      const mobileSearch = utils.createElement('div', 'mobile-search');
      const searchContainer = utils.createElement('div', 'search-container');

      const searchInput = utils.createElement('input', 'search-input');
      searchInput.type = 'text';
      searchInput.id = 'mobile-search-input';
      searchInput.placeholder = window.i18nManager
        ? window.i18nManager.t('ui.search_photos_placeholder')
        : 'Search photos...';

      const searchBtn = utils.createElement('button', 'search-btn');
      searchBtn.textContent = window.i18nManager ? window.i18nManager.t('ui.search') : 'Search';

      searchContainer.appendChild(searchInput);
      searchContainer.appendChild(searchBtn);
      mobileSearch.appendChild(searchContainer);

      const headerEl = utils.$('.header');
      if (headerEl) {
        headerEl.parentNode.insertBefore(mobileSearch, headerEl.nextSibling);
      }

      // Connect mobile search to main search functionality
      if (window.searchManager) {
        utils.on(searchInput, 'input', (e) => {
          const mainSearchInput = utils.$('#search-input');
          if (mainSearchInput) mainSearchInput.value = e.target.value;
        });

        utils.on(searchBtn, 'click', () => {
          window.searchManager.performSearch(searchInput.value);
        });

        utils.on(searchInput, 'keypress', (e) => {
          if (e.key === 'Enter') {
            window.searchManager.performSearch(searchInput.value);
          }
        });
      }
    }

    // Add search toggle button
    const headerActions = utils.$('.header-actions');
    if (headerActions && !utils.$('.mobile-search-btn')) {
      const searchToggle = utils.createElement('button', 'mobile-search-btn view-btn');
      searchToggle.innerHTML = window.iconHelper.getSemanticIcon('search', { size: 20 });
      searchToggle.title = window.i18nManager ? window.i18nManager.t('ui.search') : 'Search';

      headerActions.insertBefore(searchToggle, utils.$('.theme-toggle'));

      utils.on(searchToggle, 'click', () => {
        this.toggleMobileSearch();
      });
    }
  }

  setupMobileMenu() {
    // Create overlay for mobile sidebar
    if (!utils.$('.sidebar-overlay')) {
      const overlay = utils.createElement('div', 'sidebar-overlay');
      document.body.appendChild(overlay);

      utils.on(overlay, 'click', () => {
        this.closeMobileSidebar();
      });
    }
  }

  async loadInitialData() {
    try {
      utils.showLoading();

      // Load photos based on current view
      const currentView = this.state.get('currentView');
      await this.loadViewData(currentView);
    } catch (error) {
      if (window.logger) {
        window.logger.error('Error loading initial data', error, {
          component: 'App',
          method: 'loadInitialData',
        });
      }
      utils.handleError(error, 'App.loadInitialData');
    } finally {
      utils.hideLoading();
    }
  }

  async switchView(view) {
    if (view === this.state.get('currentView')) return;

    this.state.set('currentView', view);
    this.updateActiveNavItem(view);
    this.updateViewTitle(view);

    await this.loadViewData(view);
  }

  async loadViewData(view) {
    if (!window.photoGrid) return;

    const sortBy = this.state.get('sortBy');
    const [sort, order] = sortBy.split('_');

    const filters = { sort, order: order || 'desc' };

    // Add timeline filter if active
    const timelineFilter = this.state.get('timelineFilter');
    if (timelineFilter) {
      if (timelineFilter.year) filters.year = timelineFilter.year;
      if (timelineFilter.month) filters.month = timelineFilter.month;
    }

    try {
      switch (view) {
        case 'all':
          await window.photoGrid.loadPhotos(null, filters, true);
          break;

        case 'favorites':
          const favoritePhotos = await api.getFavoritePhotos();
          this.displayFavoritePhotos(favoritePhotos);
          break;

        case 'videos':
          // This would need backend support for file type filtering
          await window.photoGrid.loadPhotos('type:video', filters, true);
          break;
      }
    } catch (error) {
      if (window.logger) {
        window.logger.error(`Error loading ${view} view`, error, {
          component: 'App',
          method: 'loadViewData',
          view,
        });
      }
      utils.handleError(error, `App.loadViewData(${view})`);
    }
  }

  displayFavoritePhotos(favoritePhotos) {
    if (window.photoGrid) {
      window.photoGrid.clearGrid();
      if (favoritePhotos.photos && favoritePhotos.photos.length > 0) {
        window.photoGrid.renderPhotos(favoritePhotos.photos);
      } else {
        window.photoGrid.showEmptyState();
      }
    }
  }

  updateActiveNavItem(view) {
    utils.$$('.nav-item').forEach((item) => {
      item.classList.remove('active');
    });

    if (view) {
      const activeItem = utils.$(`.nav-item[data-view="${view}"]`);
      if (activeItem) {
        activeItem.classList.add('active');
      }
    }
  }

  updateViewTitle(view) {
    const titleEl = utils.$('#current-view-title');
    if (titleEl && window.i18nManager) {
      // Map view names to i18n keys
      const titleKeys = {
        all: 'all_photos',
        favorites: 'favorites',
        videos: 'videos',
      };

      const i18nKey = titleKeys[view] || 'all_photos';
      const localizedTitle = window.i18nManager.t(`ui.${i18nKey}`);
      titleEl.textContent = localizedTitle;
    }
  }

  async setSortBy(sortBy) {
    this.state.set('sortBy', sortBy);

    // Update UI select element to reflect new sort
    const sortSelect = document.getElementById('sort-select');
    if (sortSelect) {
      sortSelect.value = sortBy;
    }

    // Reload current view with new sorting
    const currentView = this.state.get('currentView');
    await this.loadViewData(currentView);
  }

  handleResize() {
    const wasMobile = this.state.get('isMobile');
    const isMobile = window.innerWidth <= MOBILE_BREAKPOINT;

    this.state.set('isMobile', isMobile);

    if (isMobile && !wasMobile) {
      this.setupMobileLayout();
    } else if (!isMobile && wasMobile) {
      this.closeMobileSidebar();
    }

    // Update grid layout
    if (window.photoGrid) {
      window.photoGrid.updateGridLayout();
    }
  }

  toggleMobileSidebar() {
    const isOpen = this.state.get('sidebarOpen');
    if (isOpen) {
      this.closeMobileSidebar();
    } else {
      this.openMobileSidebar();
    }
  }

  openMobileSidebar() {
    const sidebar = utils.$('.sidebar');
    const overlay = utils.$('.sidebar-overlay');

    if (sidebar) sidebar.classList.add('open');
    if (overlay) overlay.classList.add('show');

    this.state.set('sidebarOpen', true);
  }

  closeMobileSidebar() {
    const sidebar = utils.$('.sidebar');
    const overlay = utils.$('.sidebar-overlay');

    if (sidebar) sidebar.classList.remove('open');
    if (overlay) overlay.classList.remove('show');

    this.state.set('sidebarOpen', false);
  }

  toggleMobileSearch() {
    const mobileSearch = utils.$('.mobile-search');
    if (mobileSearch) {
      mobileSearch.classList.toggle('show');
      if (mobileSearch.classList.contains('show')) {
        const searchInput = mobileSearch.querySelector('.search-input');
        if (searchInput) searchInput.focus();
      }
    }
  }

  handleFavoriteToggled(data) {
    const { photoId, isFavorite } = data;

    // Update UI if we're in favorites view
    const currentView = this.state.get('currentView');
    if (currentView === 'favorites' && !isFavorite) {
      // Remove from favorites view
      const photoCard = utils.$(`[data-photo-id="${photoId}"]`);
      if (photoCard) {
        photoCard.remove();
      }
    }
  }

  startHealthChecking() {
    // Check health every 30 seconds
    setInterval(async () => {
      try {
        await api.healthCheck();
        if (window.logger) {
          window.logger.debug('Health check passed', {
            component: 'App',
            timestamp: new Date().toISOString(),
          });
        }
      } catch (error) {
        if (window.logger) {
          window.logger.warn('Health check failed', {
            component: 'App',
            error: error.message,
          });
        } else {
          console.warn('Health check failed:', error);
        }
        utils.showToast(
          window.i18nManager ? window.i18nManager.t('ui.connection') : 'Connection',
          window.i18nManager
            ? window.i18nManager.t('errors.server_connection_lost')
            : 'Server connection lost',
          'warning',
          3000
        );
      }
    }, 30000);
  }

  startPerformanceMonitoring() {
    // Log performance metrics periodically
    setInterval(() => {
      const entries = utils.performance.getEntries();
      const apiCalls = entries.filter((entry) => entry.name.startsWith('api-'));

      if (apiCalls.length > 0) {
        const avgDuration =
          apiCalls.reduce((sum, entry) => sum + entry.duration, 0) / apiCalls.length;
        if (window.logger) {
          window.logger.info('Performance metrics', {
            component: 'App',
            metric: 'average_api_response_time',
            value: avgDuration,
            apiCallsCount: apiCalls.length,
          });
        } else {
          console.log(`Average API response time: ${avgDuration.toFixed(2)}ms`);
        }
      }
    }, 60000); // Every minute
  }

  saveState() {
    // Save important state to localStorage
    const stateToSave = {
      currentView: this.state.get('currentView'),
      sortBy: this.state.get('sortBy'),
    };

    utils.storage.set('appState', stateToSave);
  }

  loadSavedState() {
    const savedState = utils.storage.get('appState');
    if (savedState) {
      this.state.update(savedState);
    }
  }

  initTheme() {
    const savedTheme = utils.storage.get('theme');
    const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
    const theme = savedTheme || (prefersDark ? 'dark' : 'light');
    this.setTheme(theme);
  }

  setTheme(theme) {
    document.documentElement.classList.toggle('dark-theme', theme === 'dark');
    document.documentElement.classList.toggle('light-theme', theme === 'light');

    utils.storage.set('theme', theme);
    this.updateThemeToggle(theme);
  }

  toggleTheme() {
    const currentTheme = document.documentElement.classList.contains('dark-theme')
      ? 'dark'
      : 'light';
    const newTheme = currentTheme === 'dark' ? 'light' : 'dark';
    this.setTheme(newTheme);

    // Visual feedback
    const button = document.getElementById('theme-toggle');
    if (button) {
      button.style.transform = 'scale(1.1)';
      setTimeout(() => (button.style.transform = ''), 200);
    }
  }

  updateThemeToggle(theme) {
    const themeToggle = utils.$('#theme-toggle');
    if (themeToggle) {
      // When in dark mode, button should have 'dark' class to show sun icon
      // When in light mode, button should NOT have 'dark' class to show moon icon
      if (theme === 'dark') {
        themeToggle.classList.add('dark');
      } else {
        themeToggle.classList.remove('dark');
      }
    }
  }

  // Public API
  getCurrentView() {
    return this.state.get('currentView');
  }

  refreshCurrentView() {
    const currentView = this.state.get('currentView');
    this.loadViewData(currentView);
  }

  applyTimelineFilter(filter) {
    this.state.set('timelineFilter', filter);
    this.refreshCurrentView();
  }

  getSelectedPhotos() {
    if (window.photoGrid) {
      return window.photoGrid.getSelectedPhotos();
    }
    return [];
  }
}

// Initialize app when DOM is ready
document.addEventListener('DOMContentLoaded', async () => {
  utils.performance.mark('app-init-start');

  window.turboPixApp = new TurboPixApp();
  await window.turboPixApp.init();

  utils.performance.mark('app-init-end');
  utils.performance.measure('app-init', 'app-init-start', 'app-init-end');

  if (window.logger) {
    window.logger.info('TurboPix app initialized');
  }
});
