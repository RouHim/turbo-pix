// Photo Grid Component

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
    this.observer = null;
    this.currentQuery = null;
    this.currentFilters = {};
    this.loadingStartTime = null; // Track when loading started

    this.init();
  }

  init() {
    this.setupIntersectionObserver();
    this.setupLoadMoreButton();
    this.bindEvents();
  }

  setupIntersectionObserver() {
    this.observer = new IntersectionObserver(
      (entries) => {
        entries.forEach((entry) => {
          if (entry.isIntersecting) {
            this.loadImageForCard(entry.target);
          }
        });
      },
      {
        rootMargin: '50px',
        threshold: 0.1,
      }
    );
  }

  setupLoadMoreButton() {
    const loadMoreBtn = utils.$('#load-more-btn');
    if (loadMoreBtn) {
      utils.on(loadMoreBtn, 'click', () => this.loadMore());
    }
  }

  bindEvents() {
    utils.on(
      window,
      'resize',
      utils.throttle(() => {
        this.updateGridLayout();
      }, 250)
    );

    // Infinite scroll - listen to main-content scroll since it's the scrollable container
    const scrollContainer = utils.$('.main-content');
    if (scrollContainer) {
      utils.on(
        scrollContainer,
        'scroll',
        utils.throttle(() => {
          this.checkScrollPosition();
        }, 250)
      );
    }
  }

  checkScrollPosition() {
    // Don't trigger if already loading or no more photos
    if (this.loading || !this.hasMore) return;

    // Get the scrollable container
    const scrollContainer = utils.$('.main-content');
    if (!scrollContainer) return;

    // Calculate distance from bottom using the container's scroll properties
    const scrollTop = scrollContainer.scrollTop;
    const containerHeight = scrollContainer.clientHeight;
    const scrollHeight = scrollContainer.scrollHeight;
    const distanceFromBottom = scrollHeight - (scrollTop + containerHeight);

    // Debug logging
    if (window.logger) {
      window.logger.debug('Scroll position check', {
        scrollTop,
        containerHeight,
        scrollHeight,
        distanceFromBottom,
        loading: this.loading,
        hasMore: this.hasMore,
      });
    }

    // Trigger load more when within 800px of bottom (increased threshold to catch bottom edge)
    // Use <= to catch exact bottom position
    if (distanceFromBottom <= 800) {
      if (window.logger) {
        window.logger.info('Infinite scroll triggered', {
          distanceFromBottom,
        });
      }
      this.loadMore();
    }
  }

  /**
   * Loads photos with pagination and filtering
   * @param {string|null} query - Search query
   * @param {Object} filters - Filter parameters (sort, order, year, month, etc.)
   * @param {boolean} reset - Whether to reset pagination and clear existing photos
   * @returns {Promise<void>}
   */
  async loadPhotos(query = null, filters = {}, reset = true) {
    if (this.loading) return;

    this.loading = true;
    this.loadingStartTime = Date.now(); // Record when loading started
    this.updateLoadingState(true);
    this.updateLoadMoreButton(); // Show loading indicator immediately

    try {
      if (reset) {
        this.photos = [];
        this.currentPage = 1;
        this.hasMore = true;
        this.currentQuery = query;
        this.currentFilters = filters;
        this.clearGrid();
      }

      const params = {
        page: this.currentPage,
        limit: this.options.batchSize,
        query: this.currentQuery,
        ...this.currentFilters,
      };

      utils.performance.mark('photos-load-start');
      const response = await api.getPhotos(params);
      utils.performance.mark('photos-load-end');
      utils.performance.measure('photos-load', 'photos-load-start', 'photos-load-end');

      if (response.photos && response.photos.length > 0) {
        this.photos.push(...response.photos);
        this.renderPhotos(response.photos);
        this.currentPage++;
        this.hasMore = response.photos.length === this.options.batchSize;

        if (window.logger) {
          window.logger.info('Photos loaded successfully', {
            component: 'PhotoGrid',
            photosCount: response.photos.length,
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
        this.updateLoadMoreButton();

        // Check scroll position again after loading completes
        // This handles the case where user is still at bottom after photos load
        // Use requestAnimationFrame to ensure DOM has updated
        window.requestAnimationFrame(() => {
          setTimeout(() => {
            this.checkScrollPosition();
          }, 50);
        });
      }, remainingTime);
    }
  }

  async loadMore() {
    if (!this.hasMore || this.loading) return;
    await this.loadPhotos(this.currentQuery, this.currentFilters, false);
  }

  renderPhotos(photos) {
    const fragment = document.createDocumentFragment();

    photos.forEach((photo) => {
      const card = this.createPhotoCard(photo);
      fragment.appendChild(card);
    });

    this.container.appendChild(fragment);
    this.updateGridLayout();
  }

  createPhotoCard(photo) {
    const card = utils.createElement('div', 'photo-card');
    card.dataset.photoId = photo.hash_sha256;

    // Check if this is a video
    const isVideo = photo.video_codec != null;

    // Create image container with safe data attribute
    const imageContainer = utils.createElement('div', 'photo-card-image-container');
    imageContainer.dataset.src = utils.getThumbnailUrl(photo, 'medium');

    // Add placeholder
    const placeholder = utils.createElement('div', 'photo-card-placeholder');
    imageContainer.appendChild(placeholder);

    // Add video play icon if needed
    if (isVideo) {
      const playIcon = utils.createElement('div', 'video-play-icon');
      imageContainer.appendChild(playIcon);
    }

    // Create overlay with SAFE text content
    const overlay = utils.createElement('div', 'photo-card-overlay');
    const title = utils.createElement('div', 'photo-card-title', this.getPhotoTitle(photo));
    const meta = utils.createElement('div', 'photo-card-meta', this.getPhotoMeta(photo));
    overlay.appendChild(title);
    overlay.appendChild(meta);

    // Create actions with buttons
    const actions = utils.createElement('div', 'photo-card-actions');

    // Favorite button
    const favoriteBtn = utils.createElement('button',
      `card-action-btn favorite-btn${photo.is_favorite ? ' active' : ''}`);
    favoriteBtn.title = utils.t('ui.add_to_favorites', 'Add to Favorites');
    favoriteBtn.dataset.action = 'favorite';
    // Icon is safe - comes from internal SVG generator
    favoriteBtn.innerHTML = window.iconHelper.getSemanticIcon('favorite', { size: 18 });

    // Download button
    const downloadBtn = utils.createElement('button', 'card-action-btn download-btn');
    downloadBtn.title = utils.t('ui.download', 'Download');
    downloadBtn.dataset.action = 'download';
    // Icon is safe - comes from internal SVG generator
    downloadBtn.innerHTML = window.iconHelper.getSemanticIcon('download', { size: 18 });

    actions.appendChild(favoriteBtn);
    actions.appendChild(downloadBtn);

    // Assemble card
    card.appendChild(imageContainer);
    card.appendChild(overlay);
    card.appendChild(actions);

    // Set up lazy loading
    this.observer.observe(imageContainer);

    // Bind events
    this.bindCardEvents(card, photo);

    return card;
  }

  bindCardEvents(card, photo) {
    // Click to open viewer
    utils.on(card, 'click', (e) => {
      if (!e.target.closest('.card-action-btn')) {
        this.openPhotoViewer(photo, this.photos);
      }
    });

    // Action buttons
    const favoriteBtn = card.querySelector('[data-action="favorite"]');
    const downloadBtn = card.querySelector('[data-action="download"]');

    utils.on(favoriteBtn, 'click', (e) => {
      e.stopPropagation();
      this.toggleFavorite(photo, favoriteBtn);
    });

    utils.on(downloadBtn, 'click', (e) => {
      e.stopPropagation();
      this.downloadPhoto(photo);
    });
  }

  async loadImageForCard(container) {
    const src = container.dataset.src;
    if (!src || container.dataset.loaded) return;

    try {
      // Create image directly instead of using utils.createLazyImage() to avoid intersection observer conflicts
      const img = document.createElement('img');
      img.src = src;
      img.alt = '';
      img.className = 'photo-card-image';

      img.onload = () => {
        const placeholder = container.querySelector('.photo-card-placeholder');
        if (placeholder) {
          container.replaceChild(img, placeholder);
          container.dataset.loaded = 'true';
        }
      };

      img.onerror = () => {
        const placeholder = container.querySelector('.photo-card-placeholder');
        if (placeholder) {
          placeholder.innerHTML = `<div class="error-placeholder">${window.iconHelper.getSemanticIcon('error', { size: 24 })}</div>`;
        }

        if (window.logger) {
          window.logger.warn('Failed to load image', {
            component: 'PhotoGrid',
            src,
            photoId: container.dataset.photoId,
          });
        }
      };
    } catch (error) {
      console.error('Error loading image:', error);
    }
  }

  getPhotoTitle(photo) {
    return photo.filename || `Photo ${photo.hash_sha256.substring(0, 8)}`;
  }

  getPhotoMeta(photo) {
    const parts = [];

    if (photo.taken_at) {
      const date = new Date(photo.taken_at);
      parts.push(date.toLocaleDateString());
    }

    if (photo.camera_make && photo.camera_model) {
      parts.push(`${photo.camera_make} ${photo.camera_model}`);
    }

    if (photo.file_size) {
      parts.push(utils.formatFileSize(photo.file_size));
    }

    return parts.join(' • ');
  }

  /**
   * Toggles the favorite status of a photo
   * @param {Object} photo - The photo object
   * @param {HTMLElement} button - The favorite button element
   * @returns {Promise<void>}
   */
  async toggleFavorite(photo, button) {
    const wasAlreadyFavorite = photo.is_favorite;
    const newFavoriteState = !wasAlreadyFavorite;

    // Optimistically update UI
    button.classList.toggle('active', newFavoriteState);
    button.title = newFavoriteState
      ? utils.t('ui.remove_from_favorites', 'Remove from Favorites')
      : utils.t('ui.add_to_favorites', 'Add to Favorites');

    try {
      // Call backend API
      if (newFavoriteState) {
        await api.addToFavorites(photo.hash_sha256);
      } else {
        await api.removeFromFavorites(photo.hash_sha256);
      }

      // Update photo object
      photo.is_favorite = newFavoriteState;

      // Show success message
      utils.showToast(
        newFavoriteState ? utils.t('ui.added', 'Added') : utils.t('ui.removed', 'Removed'),
        newFavoriteState
          ? utils.t('messages.photo_added_to_favorites', 'Photo added to favorites')
          : utils.t('messages.photo_removed_from_favorites', 'Photo removed from favorites'),
        'success',
        2000
      );

      // Emit event for other components
      utils.emit(window, 'favoriteToggled', {
        photoHash: photo.hash_sha256,
        isFavorite: newFavoriteState,
      });
    } catch (error) {
      // Revert UI on error
      button.classList.toggle('active', wasAlreadyFavorite);
      button.title = wasAlreadyFavorite
        ? utils.t('ui.remove_from_favorites', 'Remove from Favorites')
        : utils.t('ui.add_to_favorites', 'Add to Favorites');

      console.error('Error toggling favorite:', error);
      utils.showToast(
        utils.t('ui.error', 'Error'),
        utils.t('messages.error_updating_favorite', 'Error updating favorite status'),
        'error',
        3000
      );
    }
  }

  downloadPhoto(photo) {
    const link = utils.createElement('a');
    link.href = utils.getPhotoUrl(photo.hash_sha256);
    link.download = photo.filename || `photo-${photo.hash_sha256.substring(0, 8)}`;
    link.click();

    utils.showToast(
      utils.t('ui.download', 'Download'),
      utils.t('messages.photo_download_started', 'Photo download started'),
      'info',
      2000
    );
  }

  openPhotoViewer(photo, allPhotos) {
    if (window.photoViewer) {
      window.photoViewer.open(photo, allPhotos);
    }
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
    const title = utils.createElement('div', 'error-state-title',
      utils.t('ui.no_photos_found', 'No Photos Found'));

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
      message.textContent = utils.t('messages.no_photos_indexed', 'No photos have been indexed yet');
    }

    errorState.appendChild(iconDiv);
    errorState.appendChild(title);
    errorState.appendChild(message);

    // Refresh button (only if no query)
    if (!this.currentQuery) {
      const button = utils.createElement('button', 'error-state-button',
        utils.t('ui.refresh', 'Refresh'));
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
    const title = utils.createElement('div', 'error-state-title',
      utils.t('errors.error_loading_photos', 'Error Loading Photos'));

    // Message (safe - uses textContent)
    const messageDiv = utils.createElement('div', 'error-state-message', message);

    // Try again button
    const button = utils.createElement('button', 'error-state-button',
      utils.t('ui.try_again', 'Try Again'));
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

  updateLoadMoreButton() {
    const loadMoreContainer = utils.$('#load-more-container');

    if (!loadMoreContainer) return;

    // Show loading indicator when loading more photos (but not on initial load)
    if (this.loading && this.photos.length > 0) {
      loadMoreContainer.style.display = 'flex';
      loadMoreContainer.innerHTML = `
        <div class="infinite-scroll-loading">
          <div class="dot-wave">
            <div class="dot-wave-dot"></div>
            <div class="dot-wave-dot"></div>
            <div class="dot-wave-dot"></div>
          </div>
        </div>
      `;
    } else if (!this.loading && !this.hasMore && this.photos.length > 0) {
      // Show "end of results" indicator - just dots without animation
      loadMoreContainer.style.display = 'flex';
      loadMoreContainer.innerHTML = `
        <div class="infinite-scroll-end">
          <div class="end-dots">
            <div class="end-dot"></div>
            <div class="end-dot"></div>
            <div class="end-dot"></div>
          </div>
        </div>
      `;
    } else {
      // Hide when not loading and has more
      loadMoreContainer.style.display = 'none';
    }
  }

  updateGridLayout() {
    // Dynamic grid sizing based on container width
    const containerWidth = this.container.offsetWidth;
    const minCardWidth = 200;
    const gap = 24;
    const columns = Math.floor((containerWidth + gap) / (minCardWidth + gap));

    this.container.style.gridTemplateColumns = `repeat(${columns}, 1fr)`;
  }

  // Public API
  refresh() {
    this.loadPhotos(this.currentQuery, this.currentFilters, true);
  }

  search(query) {
    this.loadPhotos(query, this.currentFilters, true);
  }

  filter(filters) {
    this.loadPhotos(this.currentQuery, filters, true);
  }

  displayPhotos(photos) {
    this.photos = photos;
    this.clearGrid();
    this.renderPhotos(photos);
    this.updateLoadMoreButton();
  }

  getSelectedPhotos() {
    return this.photos.filter((photo) => {
      const card = this.container.querySelector(`[data-photo-id="${photo.hash_sha256}"]`);
      return card && card.classList.contains('selected');
    });
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
