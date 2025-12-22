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

    // Create header with generate button
    const header = document.createElement('div');
    header.className = 'collage-header';

    const generateBtn = document.createElement('button');
    generateBtn.className = 'collage-btn collage-btn-generate';
    generateBtn.textContent = window.i18nManager.t('ui.generate_collages');
    generateBtn.onclick = () => this.generateCollages();

    header.appendChild(generateBtn);
    this.container.appendChild(header);

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
      date: this.formatCollageDate(collage.date),
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
    date.textContent = this.formatCollageDate(collage.date);

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
    acceptBtn.textContent = window.i18nManager.t('ui.accept_collage');
    acceptBtn.onclick = () => this.acceptCollage(collage.id);

    const rejectBtn = document.createElement('button');
    rejectBtn.className = 'collage-btn collage-btn-reject';
    rejectBtn.textContent = window.i18nManager.t('ui.reject_collage');
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
    try {
      await window.api.acceptCollage(collageId);

      // Remove from UI
      this.collages = this.collages.filter((c) => c.id !== collageId);
      this.render();

      // Show success notification
      window.toast?.show(window.i18nManager.t('notifications.collageAccepted'), 'success');
    } catch (error) {
      console.error('Failed to accept collage:', error);
      window.toast?.show(window.i18nManager.t('notifications.collageAcceptFailed'), 'error');
    }
  }

  async rejectCollage(collageId) {
    try {
      await window.api.rejectCollage(collageId);

      // Remove from UI
      this.collages = this.collages.filter((c) => c.id !== collageId);
      this.render();

      // Show success notification
      window.toast?.show(window.i18nManager.t('notifications.collageRejected'), 'success');
    } catch (error) {
      console.error('Failed to reject collage:', error);
      window.toast?.show(window.i18nManager.t('notifications.collageRejectFailed'), 'error');
    }
  }

  async generateCollages() {
    try {
      // Disable button during generation
      const generateBtn = this.container.querySelector('.collage-btn-generate');
      if (generateBtn) {
        generateBtn.disabled = true;
        generateBtn.textContent = window.i18nManager.t('ui.generating_collages');
      }

      const result = await window.api.generateCollages();

      // Reload collages
      await this.loadPendingCollages();

      // Show success notification
      window.toast?.show(
        window.i18nManager.t('notifications.collagesGenerated', { count: result.count }),
        'success'
      );
    } catch (error) {
      console.error('Failed to generate collages:', error);
      window.toast?.show(window.i18nManager.t('notifications.collageGenerateFailed'), 'error');

      // Re-enable button on error
      const generateBtn = this.container.querySelector('.collage-btn-generate');
      if (generateBtn) {
        generateBtn.disabled = false;
        generateBtn.textContent = window.i18nManager.t('ui.generate_collages');
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

  formatCollageDate(dateString) {
    if (!dateString || typeof dateString !== 'string') {
      return dateString;
    }

    const parts = dateString.split('-').map((value) => parseInt(value, 10));
    if (parts.length !== 3 || parts.some((value) => Number.isNaN(value))) {
      return dateString;
    }

    const [year, month, day] = parts;
    const monthKey = window.APP_CONSTANTS.MONTH_KEYS[month - 1];
    if (!monthKey) {
      return dateString;
    }

    const date = new Date(Date.UTC(year, month - 1, day));
    const weekdayKey = window.APP_CONSTANTS.WEEKDAY_KEYS[date.getUTCDay()];
    if (!weekdayKey) {
      return dateString;
    }

    const monthName = window.i18nManager
      ? window.i18nManager.t(`ui.months.${monthKey}`)
      : monthKey.charAt(0).toUpperCase() + monthKey.slice(1);
    const weekdayName = window.i18nManager
      ? window.i18nManager.t(`ui.weekdays.${weekdayKey}`)
      : weekdayKey.charAt(0).toUpperCase() + weekdayKey.slice(1);
    const locale = window.i18nManager?.getLocale?.() || 'en';

    if (locale === 'de') {
      return `${weekdayName}, ${day}. ${monthName} ${year}`;
    }

    return `${weekdayName}, ${monthName} ${day}, ${year}`;
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
