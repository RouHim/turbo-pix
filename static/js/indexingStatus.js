// Indexing Status Manager
// Polls the backend for indexing status and displays a progress banner

class IndexingStatusManager {
  constructor() {
    this.pollInterval = null;
    this.pollFrequency = 5000; // 5 seconds
    this.banner = null;
    this.messageEl = null;
    this.progressEl = null;
    this.progressFillEl = null;
    this.isIndexing = false;
  }

  /**
   * Initializes the indexing status manager
   */
  init() {
    this.banner = utils.$('#indexing-banner');
    this.messageEl = utils.$('#indexing-message');
    this.progressEl = utils.$('#indexing-progress');
    this.progressFillEl = utils.$('#indexing-progress-fill');

    if (!this.banner) {
      console.warn('Indexing banner element not found');
      return;
    }

    // Start polling
    this.startPolling();
  }

  /**
   * Starts polling for indexing status
   */
  startPolling() {
    // Poll immediately
    this.checkStatus();

    // Then poll every 5 seconds
    this.pollInterval = setInterval(() => {
      this.checkStatus();
    }, this.pollFrequency);

    if (window.logger) {
      window.logger.debug('Indexing status polling started', {
        component: 'IndexingStatus',
        frequency: this.pollFrequency,
      });
    }
  }

  /**
   * Stops polling for indexing status
   */
  stopPolling() {
    if (this.pollInterval) {
      clearInterval(this.pollInterval);
      this.pollInterval = null;

      if (window.logger) {
        window.logger.debug('Indexing status polling stopped', {
          component: 'IndexingStatus',
        });
      }
    }
  }

  /**
   * Checks the indexing status from the API
   */
  async checkStatus() {
    try {
      const status = await window.api.getIndexingStatus();

      if (status.is_indexing) {
        this.showBanner(status);
        this.isIndexing = true;
      } else {
        // If we were indexing and now we're not, hide the banner
        if (this.isIndexing) {
          this.hideBanner();
          this.isIndexing = false;

          // Optionally reload photos when indexing completes
          if (window.photoGrid) {
            window.photoGrid.loadPhotos();
          }
        }
      }
    } catch (error) {
      if (window.logger) {
        window.logger.error('Failed to check indexing status', error, {
          component: 'IndexingStatus',
        });
      }
      // Don't show error to user, just log it
    }
  }

  /**
   * Shows the indexing banner with current status
   * @param {Object} status - Status object from API
   */
  showBanner(status) {
    if (!this.banner) return;

    const { phase, photos_total, photos_processed, photos_semantic_indexed, progress_percent } =
      status;

    // Update message based on phase
    let message = '';
    if (phase === 'metadata') {
      message = window.i18n?.t('ui.indexing_metadata') || 'Indexing metadata...';
    } else if (phase === 'semantic_vectors') {
      message = window.i18n?.t('ui.indexing_semantic') || 'Computing semantic vectors...';
    } else if (phase === 'collages') {
      message = window.i18n?.t('ui.indexing_collages') || 'Generating collages...';
    } else if (phase === 'housekeeping') {
      message =
        window.i18n?.t('ui.indexing_housekeeping') || 'Identifying housekeeping candidates...';
    } else {
      message = window.i18n?.t('ui.indexing_photos') || 'Indexing photos...';
    }

    // Update progress text
    let progressText = '';
    if (phase === 'metadata' && photos_total > 0) {
      progressText = `${photos_processed} / ${photos_total} (${Math.round(progress_percent)}%)`;
    } else if (phase === 'semantic_vectors' && photos_total > 0) {
      progressText = `${photos_semantic_indexed} / ${photos_total} (${Math.round(progress_percent)}%)`;
    }

    // Emit event for other components
    utils.emit(window, 'indexingStatusChanged', status);

    // Update DOM
    if (this.messageEl) {
      this.messageEl.textContent = message;
    }
    if (this.progressEl) {
      this.progressEl.textContent = progressText;
    }
    if (this.progressFillEl) {
      this.progressFillEl.style.width = `${progress_percent}%`;
    }

    // Show banner
    this.banner.style.display = 'block';

    // Replace Feather icons if available
    if (typeof feather !== 'undefined') {
      feather.replace();
    }
  }

  /**
   * Hides the indexing banner
   */
  hideBanner() {
    if (this.banner) {
      this.banner.style.display = 'none';
    }
  }

  /**
   * Housekeeping when destroying the manager
   */
  destroy() {
    this.stopPolling();
  }
}

// Create global instance
window.indexingStatus = new IndexingStatusManager();
