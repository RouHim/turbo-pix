// Photo Grid Component

class PhotoGrid {
  constructor(container, options = {}) {
    this.container = container;
    this.options = {
      batchSize: 50,
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
  }

  async loadPhotos(query = null, filters = {}, reset = true) {
    if (this.loading) return;

    this.loading = true;
    this.updateLoadingState(true);

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
      this.loading = false;
      this.updateLoadingState(false);
      this.updateLoadMoreButton();
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
    card.dataset.photoId = photo.id;

    // Calculate aspect ratio for better layout
    const aspectRatio = photo.width && photo.height ? photo.width / photo.height : 1;
    const gridRowSpan = Math.ceil(200 / aspectRatio / 10);
    card.style.gridRowEnd = `span ${Math.max(gridRowSpan, 15)}`;

    // Create card with thumbnail URL for lazy loading
    card.innerHTML = `
            <div class="photo-card-image-container" data-src="${utils.getThumbnailUrl(photo.id, 'medium')}">
                <div class="photo-card-placeholder"></div>
            </div>
            <div class="photo-card-overlay">
                <div class="photo-card-title">${this.getPhotoTitle(photo)}</div>
                <div class="photo-card-meta">${this.getPhotoMeta(photo)}</div>
            </div>
            <div class="photo-card-actions">
                <button class="card-action-btn favorite-btn" title="Add to Favorites" data-action="favorite">
                    ${api.isFavorite(photo.id) ? '❤️' : '🤍'}
                </button>
                <button class="card-action-btn download-btn" title="Download" data-action="download">⬇️</button>
            </div>
        `;

    // Set up lazy loading
    const imageContainer = card.querySelector('.photo-card-image-container');
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
      this.toggleFavorite(photo.id, favoriteBtn);
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
          placeholder.innerHTML = '<div class="error-placeholder">❌</div>';
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
    return photo.filename || `Photo ${photo.id}`;
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

  toggleFavorite(photoId, button) {
    const isFavorite = api.isFavorite(photoId);

    if (isFavorite) {
      api.removeFromFavorites(photoId);
      button.textContent = '🤍';
      button.title = 'Add to Favorites';
      utils.showToast('Removed', 'Photo removed from favorites', 'info', 2000);
    } else {
      api.addToFavorites(photoId);
      button.textContent = '❤️';
      button.title = 'Remove from Favorites';
      utils.showToast('Added', 'Photo added to favorites', 'success', 2000);
    }

    // Emit event for other components
    utils.emit(window, 'favoriteToggled', { photoId, isFavorite: !isFavorite });
  }

  downloadPhoto(photo) {
    const link = utils.createElement('a');
    link.href = utils.getPhotoUrl(photo.id);
    link.download = photo.filename || `photo-${photo.id}`;
    link.click();

    utils.showToast('Download', 'Photo download started', 'info', 2000);
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
    this.container.innerHTML = `
            <div class="error-state">
                <div class="error-state-icon">📷</div>
                <div class="error-state-title">No Photos Found</div>
                <div class="error-state-message">
                    ${
                      this.currentQuery
                        ? `No photos match your search for "${this.currentQuery}"`
                        : 'No photos have been indexed yet'
                    }
                </div>
                ${
                  !this.currentQuery
                    ? `
                    <button class="error-state-button" onclick="window.location.reload()">
                        Refresh
                    </button>
                `
                    : ''
                }
            </div>
        `;
  }

  showErrorState(message) {
    this.container.innerHTML = `
            <div class="error-state">
                <div class="error-state-icon">⚠️</div>
                <div class="error-state-title">Error Loading Photos</div>
                <div class="error-state-message">${message}</div>
                <button class="error-state-button" onclick="photoGrid.loadPhotos()">
                    Try Again
                </button>
            </div>
        `;
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
    const loadMoreBtn = utils.$('#load-more-btn');

    if (loadMoreContainer && loadMoreBtn) {
      if (this.hasMore && this.photos.length > 0) {
        loadMoreContainer.style.display = 'flex';
        loadMoreBtn.disabled = this.loading;
        loadMoreBtn.textContent = this.loading ? 'Loading...' : 'Load More';
      } else {
        loadMoreContainer.style.display = 'none';
      }
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

  getSelectedPhotos() {
    return this.photos.filter((photo) => {
      const card = this.container.querySelector(`[data-photo-id="${photo.id}"]`);
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
