// Collages view manager

class CollagesView {
  constructor() {
    this.container = null;
    this.collages = [];
  }

  initialize(container) {
    this.container = container || document.querySelector('.photo-grid');
  }

  async loadPendingCollages() {
    try {
      this.collages = await window.api.getPendingCollages();
      this.render();
    } catch (error) {
      console.error('Failed to load pending collages:', error);
      this.renderError();
    }
  }

  render() {
    if (!this.container) return;

    // Clear existing content
    this.container.innerHTML = '';

    if (this.collages.length === 0) {
      this.renderEmptyState();
      return;
    }

    // Use photo-grid class for consistent layout
    this.container.className = 'photo-grid';

    const fragment = document.createDocumentFragment();

    this.collages.forEach((collage) => {
      // Convert collage to PhotoCard-compatible format
      const collageAsPhoto = {
        hash_sha256: collage.id,
        thumbnail_path: `/api/collages/${collage.id}/image`,
        path: `/api/collages/${collage.id}/image`,
        filename: window.utils.formatCollageDate(collage.date),
        isCollage: true,
        collageId: collage.id,
        collageDate: collage.date,
        collagePhotoCount: collage.photo_count,
      };

      // Create PhotoCard instance
      const photoCard = new window.PhotoCard(collageAsPhoto, this);
      const card = photoCard.create();

      fragment.appendChild(card);
    });

    this.container.appendChild(fragment);

    if (window.iconHelper) {
      window.iconHelper.initializeIcons();
    }

    // Listen for collage events to remove from UI
    this.bindCollageEvents();
  }

  bindCollageEvents() {
    window.utils.on(window, 'collageAccepted', (e) => {
      this.removeCollage(e.detail.collageId);
    });

    window.utils.on(window, 'collageRejected', (e) => {
      this.removeCollage(e.detail.collageId);
    });
  }

  removeCollage(collageId) {
    this.collages = this.collages.filter((c) => c.id !== collageId);
    this.render();
  }

  renderEmptyState() {
    const emptyState = document.createElement('div');
    emptyState.className = 'empty-state';
    emptyState.innerHTML = `
      <div class="empty-state-icon">
        <svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1" stroke-linecap="round" stroke-linejoin="round">
          <rect x="3" y="3" width="18" height="18" rx="2" ry="2"></rect>
          <circle cx="8.5" cy="8.5" r="1.5"></circle>
          <polyline points="21 15 16 10 5 21"></polyline>
        </svg>
      </div>
      <p class="empty-state-message">${window.i18nManager.t('ui.no_pending_collages')}</p>
    `;
    this.container.appendChild(emptyState);
  }

  renderError() {
    const errorState = document.createElement('div');
    errorState.className = 'empty-state';
    errorState.innerHTML = `
      <div class="empty-state-icon">
        <svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="10"></circle>
          <line x1="12" y1="8" x2="12" y2="12"></line>
          <line x1="12" y1="16" x2="12.01" y2="16"></line>
        </svg>
      </div>
      <p class="empty-state-message">${window.i18nManager.t('ui.collages_load_failed')}</p>
    `;
    this.container.appendChild(errorState);
  }

  clear() {
    if (this.container) {
      this.container.innerHTML = '';
    }
    this.collages = [];
  }
}

// Create global instance
window.collagesView = new CollagesView();
