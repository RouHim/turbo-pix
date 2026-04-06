// Indexing Status Manager
// Polls the backend for indexing status and displays a progress banner

class IndexingStatusManager {
  constructor() {
    this.pollInterval = null;
    this.pollFrequencyIndexing = 1000;
    this.pollFrequencyIdle = 30000;
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

    if (window.logger) {
      window.logger.debug('Indexing status polling started', {
        component: 'IndexingStatus',
      });
    }
  }

  /**
   * Schedules the next poll based on current indexing state
   */
  scheduleNextPoll() {
    if (this.pollInterval) {
      clearTimeout(this.pollInterval);
    }

    const frequency = this.isIndexing ? this.pollFrequencyIndexing : this.pollFrequencyIdle;

    this.pollInterval = setTimeout(() => {
      this.checkStatus();
    }, frequency);
  }

  /**
   * Stops polling for indexing status
   */
  stopPolling() {
    if (this.pollInterval) {
      clearTimeout(this.pollInterval);
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
        } else {
          this.hideBanner();
        }
      }
    } catch (error) {
      if (window.logger) {
        window.logger.error('Failed to check indexing status', error, {
          component: 'IndexingStatus',
        });
      }
      // Don't show error to user, just log it
    } finally {
      this.scheduleNextPoll();
    }
  }

  /**
   * Shows the indexing banner with current status
   * @param {Object} status - Status object from API
   */
  showBanner(status) {
    if (!this.banner) return;

    const steps = this.banner.querySelectorAll('[data-step-id]');
    steps.forEach((step) => {
      step.setAttribute('data-step-state', 'pending');
      const progressBarEl = step.querySelector('.step-progress-bar');
      if (progressBarEl) {
        progressBarEl.style.width = '0%';
        progressBarEl.classList.remove('indeterminate');
      }
      const counterEl = step.querySelector('.step-counter');
      if (counterEl) counterEl.remove();
    });

    const { phases } = status;
    const activePhase = Array.isArray(phases)
      ? phases.find((phase) => phase.state === 'active') || null
      : null;

    if (phases && Array.isArray(phases)) {
      phases.forEach((phase) => {
        this.updatePhase(phase);
      });
    }

    this.updateHeader(activePhase);

    utils.emit(window, 'indexingStatusChanged', status);

    this.banner.style.display = 'block';

    if (typeof feather !== 'undefined') {
      feather.replace();
    }
  }

  updatePhase(phase) {
    const stepEl = this.banner.querySelector(`[data-step-id="${phase.id}"]`);
    if (!stepEl) return;

    stepEl.setAttribute('data-step-state', phase.state);

    const labelEl = stepEl.querySelector('.step-label');
    if (labelEl) {
      const labelKey = `ui.indexing_phase_${this.getPhaseTranslationKey(phase.id)}`;
      labelEl.textContent = this.getTranslation(labelKey, this.humanizePhaseId(phase.id));
      labelEl.removeAttribute('title');
    }

    const progressBarEl = stepEl.querySelector('.step-progress-bar');

    if (phase.kind === 'determinate' && phase.total > 0) {
      const percent = Math.min(100, Math.max(0, (phase.processed / phase.total) * 100));

      if (progressBarEl) {
        progressBarEl.style.width = `${percent}%`;
        progressBarEl.style.display = 'block';
      }
    } else if (phase.kind === 'indeterminate') {
      if (progressBarEl) {
        if (phase.state === 'active') {
          progressBarEl.style.width = '100%';
          progressBarEl.classList.add('indeterminate');
        } else {
          progressBarEl.style.width = '0%';
          progressBarEl.classList.remove('indeterminate');
        }
      }

      const counterEl = stepEl.querySelector('.step-counter');
      if (counterEl) counterEl.remove();
    } else {
      if (progressBarEl) {
        progressBarEl.style.width = '0%';
      }
      const counterEl = stepEl.querySelector('.step-counter');
      if (counterEl) counterEl.remove();
    }
  }

  updateHeader(activePhase) {
    if (!this.messageEl || !this.progressEl) return;

    if (!activePhase) {
      this.messageEl.textContent = this.getTranslation('ui.indexing_photos', 'Indexing photos...');
      this.progressEl.textContent = '';
      this.progressEl.removeAttribute('title');
      return;
    }

    const phaseKey = this.getPhaseTranslationKey(activePhase.id);
    const messageKey = `ui.indexing_status_${phaseKey}`;
    this.messageEl.textContent = this.getTranslation(
      messageKey,
      this.getTranslation('ui.indexing_photos', 'Indexing photos...')
    );

    const details = [];

    if (activePhase.kind === 'determinate' && activePhase.total > 0) {
      const counterTemplate = this.getTranslation(
        'ui.indexing_counter',
        '{{processed}} / {{total}}'
      );
      details.push(
        counterTemplate
          .replace('{{processed}}', activePhase.processed)
          .replace('{{total}}', activePhase.total)
      );
    }

    if (activePhase.current_item) {
      details.push(this.truncatePath(activePhase.current_item, 44));
      this.progressEl.title = activePhase.current_item;
    } else {
      this.progressEl.removeAttribute('title');
    }

    this.progressEl.textContent = details.join(' • ');
  }

  getPhaseTranslationKey(phaseId) {
    return phaseId;
  }

  getTranslation(key, fallback) {
    const translation = window.i18nManager?.getTranslationByKey?.(key);
    return translation || fallback;
  }

  humanizePhaseId(phaseId) {
    return phaseId
      .split('_')
      .map((part) => this.capitalize(part))
      .join(' ');
  }

  /**
   * Truncates a file path with ellipsis in the middle
   * @param {string} path - The path to truncate
   * @param {number} maxLength - Maximum length
   * @returns {string} Truncated path
   */
  truncatePath(path, maxLength = 30) {
    if (!path || path.length <= maxLength) return path;

    const parts = path.split('/');
    const filename = parts.pop();

    if (filename.length > maxLength) {
      return filename.substring(0, 10) + '...' + filename.substring(filename.length - 10);
    }

    return '.../' + filename;
  }

  /**
   * Capitalizes the first letter of a string
   * @param {string} str
   * @returns {string}
   */
  capitalize(str) {
    return str.charAt(0).toUpperCase() + str.slice(1);
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
