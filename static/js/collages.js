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

    // Create collage grid
    const collageGrid = document.createElement('div');
    collageGrid.className = 'collage-grid';

    this.collages.forEach((collage) => {
      const collageCard = this.createCollageCard(collage);
      collageGrid.appendChild(collageCard);
    });

    this.container.appendChild(collageGrid);

    if (window.iconHelper) {
      window.iconHelper.initializeIcons();
    }
  }

  createCollageCard(collage) {
    const card = document.createElement('div');
    card.className = 'collage-card';
    card.dataset.collageId = collage.id;

    // Collage image with thumbnail
    const imageContainer = document.createElement('div');
    imageContainer.className = 'collage-image-container';

    const img = document.createElement('img');
    img.className = 'collage-image';
    img.src = `/api/collages/${collage.id}/image`;
    img.alt = window.i18nManager.t('ui.collage_for', {
      date: window.utils.formatCollageDate(collage.date),
    });
    img.loading = 'lazy';

    imageContainer.appendChild(img);

    // Collage info
    const meta = document.createElement('div');
    meta.className = 'collage-meta';

    const info = document.createElement('div');
    info.className = 'collage-info';

    const date = document.createElement('div');
    date.className = 'collage-date';
    date.textContent = window.utils.formatCollageDate(collage.date);

    const photoCount = document.createElement('div');
    photoCount.className = 'collage-photo-count';
    photoCount.textContent = window.i18nManager.t('ui.collage_photos', {
      count: collage.photo_count,
    });

    info.appendChild(date);
    info.appendChild(photoCount);

    // Action buttons
    const actions = document.createElement('div');
    actions.className = 'collage-actions';

    const acceptBtn = document.createElement('button');
    acceptBtn.className = 'collage-btn collage-btn-accept';
    acceptBtn.dataset.icon = 'check';
    acceptBtn.dataset.iconSize = '20';
    acceptBtn.dataset.iconStrokeWidth = '2';
    acceptBtn.setAttribute('aria-label', window.i18nManager.t('ui.accept_collage'));
    acceptBtn.title = window.i18nManager.t('ui.accept_collage');
    acceptBtn.onclick = () => this.acceptCollage(collage.id);

    const rejectBtn = document.createElement('button');
    rejectBtn.className = 'collage-btn collage-btn-reject';
    rejectBtn.dataset.icon = 'x';
    rejectBtn.dataset.iconSize = '20';
    rejectBtn.dataset.iconStrokeWidth = '2';
    rejectBtn.setAttribute('aria-label', window.i18nManager.t('ui.reject_collage'));
    rejectBtn.title = window.i18nManager.t('ui.reject_collage');
    rejectBtn.onclick = () => this.rejectCollage(collage.id);

    actions.appendChild(acceptBtn);
    actions.appendChild(rejectBtn);

    meta.appendChild(info);
    meta.appendChild(actions);

    card.appendChild(imageContainer);
    card.appendChild(meta);

    return card;
  }

  async acceptCollage(collageId) {
    const card = document.querySelector(`[data-collage-id="${collageId}"]`);
    const acceptBtn = card?.querySelector('.collage-btn-accept');
    const rejectBtn = card?.querySelector('.collage-btn-reject');

    try {
      // Disable buttons and add loading state
      if (acceptBtn) {
        acceptBtn.disabled = true;
        acceptBtn.classList.add('loading');
      }
      if (rejectBtn) {
        rejectBtn.disabled = true;
      }

      await window.api.acceptCollage(collageId);

      // Remove from UI
      this.collages = this.collages.filter((c) => c.id !== collageId);
      this.render();

      // Show success notification
      window.toast?.show(window.i18nManager.t('notifications.collageAccepted'), 'success');
    } catch (error) {
      console.error('Failed to accept collage:', error);
      window.toast?.show(window.i18nManager.t('notifications.collageAcceptFailed'), 'error');

      // Re-enable buttons on error
      if (acceptBtn) {
        acceptBtn.disabled = false;
        acceptBtn.classList.remove('loading');
      }
      if (rejectBtn) {
        rejectBtn.disabled = false;
      }
    }
  }

  async rejectCollage(collageId) {
    const card = document.querySelector(`[data-collage-id="${collageId}"]`);
    const acceptBtn = card?.querySelector('.collage-btn-accept');
    const rejectBtn = card?.querySelector('.collage-btn-reject');

    try {
      // Disable buttons and add loading state
      if (acceptBtn) {
        acceptBtn.disabled = true;
      }
      if (rejectBtn) {
        rejectBtn.disabled = true;
        rejectBtn.classList.add('loading');
      }

      await window.api.rejectCollage(collageId);

      // Remove from UI
      this.collages = this.collages.filter((c) => c.id !== collageId);
      this.render();

      // Show success notification
      window.toast?.show(window.i18nManager.t('notifications.collageRejected'), 'success');
    } catch (error) {
      console.error('Failed to reject collage:', error);
      window.toast?.show(window.i18nManager.t('notifications.collageRejectFailed'), 'error');

      // Re-enable buttons on error
      if (acceptBtn) {
        acceptBtn.disabled = false;
      }
      if (rejectBtn) {
        rejectBtn.disabled = false;
        rejectBtn.classList.remove('loading');
      }
    }
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
