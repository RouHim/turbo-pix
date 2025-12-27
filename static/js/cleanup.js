// Cleanup Manager to handle deletion candidates

class CleanupManager {
  constructor() {
    this.container = document.getElementById('photo-grid');
    this.candidates = [];
    this.isActive = false;
  }

  async init() {
    // We don't have a specific container for cleanup, we reuse the main photo grid.
    // The App class handles view switching.
  }

  async loadAndRender() {
    this.isActive = true;
    this.container.innerHTML = '';

    // Show loading skeleton
    this.container.innerHTML = `
      <div class="loading-skeleton">
        <div class="skeleton-item"></div>
        <div class="skeleton-item"></div>
        <div class="skeleton-item"></div>
        <div class="skeleton-item"></div>
      </div>
    `;

    try {
      const response = await api.getCleanupCandidates();
      console.log('Cleanup response:', response);
      if (response && response.candidates) {
        console.log(`Found ${response.candidates.length} candidates`);
        this.candidates = response.candidates;
      } else {
        console.warn('Invalid response format', response);
        this.candidates = [];
      }
      this.render();
    } catch (e) {
      console.error('Failed to load cleanup candidates:', e);
      this.container.innerHTML = `<div class="error-message">Failed to load cleanup candidates: ${e.message}</div>`;
    }
  }

  render() {
    if (!this.isActive) return;
    this.container.innerHTML = '';

    if (this.candidates.length === 0) {
      this.container.innerHTML =
        '<div class="no-photos">No cleanup candidates found. Your library is clean!</div>';
      return;
    }

    // Add summary header
    const summary = utils.createElement(
      'div',
      'cleanup-summary',
      `Found ${this.candidates.length} candidates`
    );
    summary.style.gridColumn = '1 / -1';
    summary.style.padding = '20px';
    summary.style.textAlign = 'center';
    summary.style.color = 'var(--text-main)'; // Use main text color
    summary.style.fontSize = '1.2rem';
    summary.style.fontWeight = 'bold';
    summary.style.backgroundColor = 'var(--bg-secondary)';
    summary.style.marginBottom = '20px';
    summary.style.borderRadius = '8px';

    // Convert candidates to photos with attached cleanup metadata
    const photos = this.candidates.map((c) => {
      // The API returns flat fields: photo_hash, file_path, reason, score
      const p = {
        hash_sha256: c.photo_hash,
        file_path: c.file_path,
        // We might miss other metadata (width, height, date), but PhotoCard should handle it gracefully or we fetch it?
        // For cleanup view, we mostly need the image.
      };
      p.cleanupReason = c.reason;
      p.cleanupScore = c.score;
      return p;
    });

    // Create a fragment
    const fragment = document.createDocumentFragment();
    fragment.appendChild(summary);

    console.log('Rendering photos...');
    photos.forEach((photo, index) => {
      try {
        const card = new PhotoCard(photo, this);
        const element = card.create();
        fragment.appendChild(element);
        if (index < 3) console.log('Rendered card:', photo.hash_sha256);
      } catch (e) {
        console.error('Error rendering card for photo:', photo, e);
      }
    });

    this.container.appendChild(fragment);
    console.log('Appended fragment to container');

    // Add event listener for removal
    this.removeHandler = (e) => this.removePhoto(e.detail.hash);
    utils.on(window, 'cleanupCandidateRemoved', this.removeHandler);

    this.indexingHandler = (e) => this.handleIndexingStatus(e.detail);
    utils.on(window, 'indexingStatusChanged', this.indexingHandler);
  }

  handleIndexingStatus(status) {
    if (!this.isActive) return;

    if (status.phase === 'cleanup' && status.is_indexing) {
      this.container.innerHTML = `
              <div class="indexing-message-container" style="text-align: center; padding: 40px; color: var(--text-secondary);">
                  <div class="spinner" style="margin: 0 auto 20px;"></div>
                  <h3>${window.i18nManager.t('ui.indexing_cleanup')}</h3>
                  <p>This may take a moment...</p>
              </div>
          `;
    } else if (this.wasIndexingCleanup && !status.is_indexing) {
      // Cleanup phase finished
      this.loadAndRender();
    }

    this.wasIndexingCleanup = status.phase === 'cleanup' && status.is_indexing;
  }

  removePhoto(hash) {
    const card = this.container.querySelector(`.photo-card[data-photo-id="${hash}"]`);
    if (card) {
      card.remove();
    }
    this.candidates = this.candidates.filter((c) => c.photo_hash !== hash);
    if (this.candidates.length === 0) {
      this.container.innerHTML =
        '<div class="no-photos">No cleanup candidates found. Your library is clean!</div>';
    }
  }

  destroy() {
    this.isActive = false;
    if (this.removeHandler) {
      window.removeEventListener('cleanupCandidateRemoved', this.removeHandler);
    }
    if (this.indexingHandler) {
      window.removeEventListener('indexingStatusChanged', this.indexingHandler);
    }
  }
}

window.cleanupManager = new CleanupManager();
