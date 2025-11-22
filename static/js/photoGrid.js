// Photo Grid Component

/* global InfiniteScroll, PhotoCard, AbortController */
class PhotoGrid {
  constructor(container, options = {}) {
    this.container = container;
    this.options = {
      batchSize: window.APP_CONSTANTS.DEFAULT_BATCH_SIZE,
      threshold: 200,
      retryAttempts: 3,
      ...options,
    };

    this.photos = [];
    this.currentPage = 1;
    this.loading = false;
    this.hasMore = true;
    this.currentQuery = null;
    this.currentFilters = {};
    this.loadingStartTime = null;
    this.abortController = null;
    this.semanticSearchMode = false;
    this.semanticSearchQuery = null;

    this.init();
  }

  init() {
    this.bindEvents();
    this.infiniteScroll = new InfiniteScroll(this, {
      threshold: 800,
      throttleDelay: 250,
    });
  }

  bindEvents() {
    utils.on(
      window,
      'resize',
      utils.throttle(() => {
        this.updateGridLayout();
      }, 250)
    );
  }

  /**
   * Loads photos with pagination and filtering
   * @param {string|null} query - Search query
   * @param {Object} filters - Filter parameters (sort, order, year, month, etc.)
   * @param {boolean} reset - Whether to reset pagination and clear existing photos
   * @returns {Promise<void>}
   */
  async loadPhotos(query = null, filters = {}, reset = true) {
    // Cancel any in-flight request to prevent race conditions
    if (this.abortController) {
      this.abortController.abort();
      this.loading = false; // Reset loading state after cancelling
    }

    // Create new abort controller for this request
    this.abortController = new AbortController();
    const signal = this.abortController.signal;

    this.loading = true;
    this.loadingStartTime = Date.now();
    this.updateLoadingState(true);
    this.infiniteScroll.updateLoadingIndicator();

    try {
      if (reset) {
        this.photos = [];
        this.currentPage = 1;
        this.hasMore = true;
        this.currentQuery = query;
        this.currentFilters = filters;
        this.clearGrid();
      }

      let photos = [];

      // Handle semantic search mode differently
      if (this.semanticSearchMode && this.semanticSearchQuery) {
        const offset = (this.currentPage - 1) * this.options.batchSize;

        utils.performance.mark('semantic-search-start');
        const result = await api.semanticSearch(
          this.semanticSearchQuery,
          this.options.batchSize,
          offset
        );
        utils.performance.mark('semantic-search-end');
        utils.performance.measure(
          'semantic-search',
          'semantic-search-start',
          'semantic-search-end'
        );

        if (result.results && result.results.length > 0) {
          // Convert semantic search results to photo hashes
          const photoHashes = result.results.map((r) => r.hash);

          // Load full photo data for these hashes
          const photosData = await Promise.all(
            photoHashes.map(async (hash) => {
              try {
                return await api.getPhoto(hash);
              } catch (e) {
                console.warn(`Failed to load photo ${hash}:`, e);
                return null;
              }
            })
          );

          photos = photosData.filter((p) => p !== null);

          if (window.logger) {
            window.logger.info('Semantic search results loaded', {
              component: 'PhotoGrid',
              photosCount: photos.length,
              offset,
              query: this.semanticSearchQuery,
            });
          }
        }
      } else {
        // Regular photo loading
        const params = {
          page: this.currentPage,
          limit: this.options.batchSize,
          query: this.currentQuery,
          ...this.currentFilters,
        };

        utils.performance.mark('photos-load-start');
        const response = await api.getPhotos(params, { signal });
        utils.performance.mark('photos-load-end');
        utils.performance.measure('photos-load', 'photos-load-start', 'photos-load-end');

        photos = response.photos || [];
      }

      if (photos.length > 0) {
        this.photos.push(...photos);
        this.renderPhotos(photos);
        this.currentPage++;
        this.hasMore = photos.length === this.options.batchSize;

        if (window.logger) {
          window.logger.info('Photos loaded successfully', {
            component: 'PhotoGrid',
            photosCount: photos.length,
            totalPhotos: this.photos.length,
            page: this.currentPage - 1,
            hasMore: this.hasMore,
          });
        }
      } else {
        this.hasMore = false;
        if (this.photos.length === 0) {
          this.showEmptyState();
        }

        if (window.logger) {
          window.logger.info('No more photos to load', {
            component: 'PhotoGrid',
            totalPhotos: this.photos.length,
          });
        }
      }
    } catch (error) {
      // Ignore abort errors - these are intentional cancellations
      if (error.name === 'AbortError') {
        if (window.logger) {
          window.logger.debug('Photo load request was cancelled', {
            component: 'PhotoGrid',
            query: this.currentQuery,
          });
        }
        return;
      }

      if (window.logger) {
        window.logger.error('Error loading photos', error, {
          component: 'PhotoGrid',
          method: 'loadPhotos',
          query: this.currentQuery,
          page: this.currentPage,
        });
      } else {
        console.error('Error loading photos:', error);
      }
      this.showErrorState(error.message);
      utils.handleError(error, 'PhotoGrid.loadPhotos');
    } finally {
      // Ensure loading indicator shows for at least 300ms so it's visible
      const loadingDuration = Date.now() - this.loadingStartTime;
      const minDisplayTime = 300;
      const remainingTime = Math.max(0, minDisplayTime - loadingDuration);

      setTimeout(() => {
        this.loading = false;
        this.updateLoadingState(false);
        this.infiniteScroll?.updateLoadingIndicator();
        this.infiniteScroll?.recheckAfterLoad();
      }, remainingTime);
    }
  }

  async loadMore() {
    if (!this.hasMore || this.loading) return;
    await this.loadPhotos(this.currentQuery, this.currentFilters, false);
  }

  /**
   * Loads semantic search results with pagination support
   * @param {string} query - Semantic search query
   * @returns {Promise<void>}
   */
  async loadSemanticSearch(query) {
    // Enable semantic search mode
    this.semanticSearchMode = true;
    this.semanticSearchQuery = query;

    // Use regular loadPhotos with reset=true to start fresh
    await this.loadPhotos(null, {}, true);
  }

  renderPhotos(photos) {
    const fragment = document.createDocumentFragment();

    photos.forEach((photo) => {
      const photoCard = new PhotoCard(photo, this);
      const card = photoCard.create();
      fragment.appendChild(card);
    });

    this.container.appendChild(fragment);
    this.updateGridLayout();
  }

  clearGrid() {
    this.container.innerHTML = '';
  }

  showEmptyState() {
    // Clear first
    this.container.innerHTML = '';

    const errorState = utils.createElement('div', 'error-state');

    // Icon (safe - internal SVG)
    const iconDiv = utils.createElement('div', 'error-state-icon');
    iconDiv.innerHTML = window.iconHelper.getSemanticIcon('photo', { size: 64 });

    // Title (safe - i18n string)
    const title = utils.createElement(
      'div',
      'error-state-title',
      utils.t('ui.no_photos_found', 'No Photos Found')
    );

    // Message (safe - uses textContent for user query)
    const message = utils.createElement('div', 'error-state-message');
    if (this.currentQuery) {
      // Check if i18n has template support
      if (window.i18nManager) {
        const translatedMsg = window.i18nManager.t('messages.no_photos_match_search', {
          query: this.currentQuery,
        });
        message.textContent = translatedMsg;
      } else {
        // Fallback: safe concatenation
        message.textContent = `No photos match your search for "${this.currentQuery}"`;
      }
    } else {
      message.textContent = utils.t(
        'messages.no_photos_indexed',
        'No photos have been indexed yet'
      );
    }

    errorState.appendChild(iconDiv);
    errorState.appendChild(title);
    errorState.appendChild(message);

    // Refresh button (only if no query)
    if (!this.currentQuery) {
      const button = utils.createElement(
        'button',
        'error-state-button',
        utils.t('ui.refresh', 'Refresh')
      );
      button.onclick = () => window.location.reload();
      errorState.appendChild(button);
    }

    this.container.appendChild(errorState);
  }

  showErrorState(message) {
    // Clear first
    this.container.innerHTML = '';

    const errorState = utils.createElement('div', 'error-state');

    // Icon (safe - internal SVG)
    const iconDiv = utils.createElement('div', 'error-state-icon');
    iconDiv.innerHTML = window.iconHelper.getSemanticIcon('warning', { size: 64 });

    // Title (safe - i18n string)
    const title = utils.createElement(
      'div',
      'error-state-title',
      utils.t('errors.error_loading_photos', 'Error Loading Photos')
    );

    // Message (safe - uses textContent)
    const messageDiv = utils.createElement('div', 'error-state-message', message);

    // Try again button
    const button = utils.createElement(
      'button',
      'error-state-button',
      utils.t('ui.try_again', 'Try Again')
    );
    button.onclick = () => this.loadPhotos();

    errorState.appendChild(iconDiv);
    errorState.appendChild(title);
    errorState.appendChild(messageDiv);
    errorState.appendChild(button);

    this.container.appendChild(errorState);
  }

  updateLoadingState(loading) {
    const skeleton = this.container.querySelector('.loading-skeleton');
    if (loading && !skeleton && this.photos.length === 0) {
      this.container.innerHTML = `
                <div class="loading-skeleton">
                    ${Array(6)
                      .fill()
                      .map(() => '<div class="skeleton-item"></div>')
                      .join('')}
                </div>
            `;
    } else if (!loading && skeleton) {
      skeleton.remove();
    }
  }

  updateGridLayout() {
    // Grid layout is handled by CSS
  }

  // Public API
  refresh() {
    this.loadPhotos(this.currentQuery, this.currentFilters, true);
  }

  search(query) {
    // Disable semantic search mode when doing regular search
    this.semanticSearchMode = false;
    this.semanticSearchQuery = null;
    this.loadPhotos(query, this.currentFilters, true);
  }

  filter(filters) {
    // Disable semantic search mode when applying filters
    this.semanticSearchMode = false;
    this.semanticSearchQuery = null;
    this.loadPhotos(this.currentQuery, filters, true);
  }

  displayPhotos(photos) {
    this.photos = photos;
    this.clearGrid();
    this.renderPhotos(photos);
    this.infiniteScroll.updateLoadingIndicator();
  }

  getSelectedPhotos() {
    return this.photos.filter((photo) => {
      const card = this.container.querySelector(`[data-photo-id="${photo.hash_sha256}"]`);
      return card && card.classList.contains('selected');
    });
  }

  updatePhotoCard(updatedPhoto) {
    const index = this.photos.findIndex((p) => p.hash_sha256 === updatedPhoto.hash_sha256);
    if (index !== -1) {
      this.photos[index] = updatedPhoto;

      const oldCard = this.container.querySelector(`[data-photo-id="${updatedPhoto.hash_sha256}"]`);
      if (oldCard) {
        const photoCard = new PhotoCard(updatedPhoto, this);
        const newCard = photoCard.create();
        oldCard.replaceWith(newCard);
      }
    }
  }
}

// Make PhotoGrid available globally
window.PhotoGrid = PhotoGrid;

// Initialize global photo grid when DOM is ready
document.addEventListener('DOMContentLoaded', () => {
  const gridContainer = utils.$('#photo-grid');
  if (gridContainer) {
    window.photoGrid = new PhotoGrid(gridContainer);
  }
});
