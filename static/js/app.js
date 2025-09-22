// Main Application Controller

class TurboPixApp {
  constructor() {
    this.state = new utils.SimpleState({
      currentView: 'all',
      isLoading: false,
      isMobile: window.innerWidth <= 768,
      sidebarOpen: false,
      sortBy: 'date_desc',
      totalPhotos: 0,
      selectedPhotos: [],
    });

    this.init();
  }

  init() {
    if (window.logger) {
      window.logger.info('TurboPix App initializing', {
        userAgent: navigator.userAgent,
        url: window.location.href,
      });
    }

    this.bindEvents();
    this.setupNavigation();
    this.setupViewControls();
    this.setupResponsiveLayout();
    this.loadInitialData();
    this.startPerformanceMonitoring();
  }

  bindEvents() {
    // Navigation
    utils.$$('.nav-item').forEach((item) => {
      utils.on(item, 'click', () => {
        const view = item.dataset.view;
        if (view) {
          this.switchView(view);
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
    const isMobile = window.innerWidth <= 768;
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
      menuBtn.innerHTML = '☰';
      menuBtn.title = 'Menu';
      header.insertBefore(menuBtn, header.firstChild);

      utils.on(menuBtn, 'click', () => {
        this.toggleMobileSidebar();
      });
    }

    // Add search toggle
    const searchContainer = utils.$('.search-container');
    if (searchContainer && !utils.$('.mobile-search-btn')) {
      const searchBtn = utils.createElement('button', 'mobile-search-btn view-btn');
      searchBtn.innerHTML = '🔍';
      searchBtn.title = 'Search';

      // Insert after search container since header-actions was removed
      searchContainer.parentNode.insertBefore(searchBtn, searchContainer.nextSibling);

      utils.on(searchBtn, 'click', () => {
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

    await this.loadViewData(view);
  }

  async loadViewData(view) {
    if (!window.photoGrid) return;

    const sortBy = this.state.get('sortBy');
    const [sort, order] = sortBy.split('_');

    const filters = { sort, order: order || 'desc' };

    try {
      switch (view) {
        case 'all':
          await window.photoGrid.loadPhotos(null, filters, true);
          this.updateViewTitle('All Photos');
          break;

        case 'favorites':
          const favoritePhotos = await api.getFavoritePhotos();
          this.displayFavoritePhotos(favoritePhotos);
          this.updateViewTitle('Favorite Photos');
          break;

        case 'videos':
          // This would need backend support for file type filtering
          await window.photoGrid.loadPhotos('type:video', filters, true);
          this.updateViewTitle('Videos');
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

  updateViewTitle(title) {
    const titleEl = utils.$('#current-view-title');
    if (titleEl) {
      titleEl.textContent = title;
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
    const isMobile = window.innerWidth <= 768;

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
        utils.showToast('Connection', 'Server connection lost', 'warning', 3000);
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

  // Public API
  getCurrentView() {
    return this.state.get('currentView');
  }

  refreshCurrentView() {
    const currentView = this.state.get('currentView');
    this.loadViewData(currentView);
  }

  getSelectedPhotos() {
    if (window.photoGrid) {
      return window.photoGrid.getSelectedPhotos();
    }
    return [];
  }
}

// Initialize app when DOM is ready
document.addEventListener('DOMContentLoaded', () => {
  utils.performance.mark('app-init-start');

  window.turboPixApp = new TurboPixApp();

  utils.performance.mark('app-init-end');
  utils.performance.measure('app-init', 'app-init-start', 'app-init-end');

  console.log('🚀 TurboPix app initialized');
});
