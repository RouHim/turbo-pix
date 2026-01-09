class PhotoCard {
  constructor(photo, grid) {
    this.photo = photo;
    this.grid = grid;
  }

  create() {
    const card = utils.createElement('div', 'photo-card');
    card.dataset.photoId = this.photo.hash_sha256;

    const isVideo = this.photo.metadata?.video?.codec != null;

    const imageContainer = utils.createElement('div', 'photo-card-image-container');

    // BlurHash placeholder for progressive loading
    if (this.photo.blurhash && window.blurhash) {
      try {
        const canvas = window.blurhash.createCanvas(this.photo.blurhash, 32, 32, 1);
        canvas.className = 'photo-card-blurhash';
        imageContainer.appendChild(canvas);
      } catch (e) {
        console.warn('Failed to decode blurhash:', e);
      }
    }

    // Responsive images with WebP and JPEG fallback
    const picture = utils.createElement('picture');
    let img;

    if (this.photo.isCollage) {
      // For collages, use direct image path (no responsive images)
      img = utils.createElement('img', 'photo-card-image');
      img.src = this.photo.thumbnail_path || this.photo.path;
      img.alt = this.getTitle();
      img.loading = 'lazy';
      img.decoding = 'async';
    } else {
      // For photos, use responsive images with WebP and JPEG fallback
      const webpSource = utils.createElement('source');
      webpSource.type = 'image/webp';
      webpSource.srcset = `${utils.getThumbnailUrl(this.photo, 'small')}&format=webp 200w, ${utils.getThumbnailUrl(this.photo, 'medium')}&format=webp 400w, ${utils.getThumbnailUrl(this.photo, 'large')}&format=webp 800w`;
      webpSource.sizes = '(max-width: 640px) 200px, (max-width: 1024px) 400px, 800px';
      picture.appendChild(webpSource);

      // JPEG fallback source with srcset
      const jpegSource = utils.createElement('source');
      jpegSource.type = 'image/jpeg';
      jpegSource.srcset = `${utils.getThumbnailUrl(this.photo, 'small')}&format=jpeg 200w, ${utils.getThumbnailUrl(this.photo, 'medium')}&format=jpeg 400w, ${utils.getThumbnailUrl(this.photo, 'large')}&format=jpeg 800w`;
      jpegSource.sizes = '(max-width: 640px) 200px, (max-width: 1024px) 400px, 800px';
      picture.appendChild(jpegSource);

      // img fallback for browsers that don't support picture
      img = utils.createElement('img', 'photo-card-image');
      img.src = `${utils.getThumbnailUrl(this.photo, 'medium')}&format=jpeg`;
      img.alt = this.getTitle();
      img.loading = 'lazy';
      img.decoding = 'async';
    }

    // Fade in image when loaded
    img.onload = () => {
      imageContainer.classList.add('image-loaded');
    };

    img.onerror = () => {
      // Show placeholder on error
      const placeholder = utils.createElement('div', 'photo-card-placeholder');
      // Remove the picture element (which contains the broken img)
      if (imageContainer.contains(picture)) {
        imageContainer.removeChild(picture);
      }
      imageContainer.appendChild(placeholder);

      // Also hide blurhash if present as it might be irrelevant/distracting if main image failed
      const blurhash = imageContainer.querySelector('.photo-card-blurhash');
      if (blurhash) blurhash.style.display = 'none';
    };

    picture.appendChild(img);
    imageContainer.appendChild(picture);

    if (isVideo) {
      const playIcon = utils.createElement('div', 'video-play-icon');
      imageContainer.appendChild(playIcon);
    }

    const overlay = utils.createElement('div', 'photo-card-overlay');
    const title = utils.createElement('div', 'photo-card-title', this.getTitle());
    const meta = utils.createElement('div', 'photo-card-meta', this.getMeta());
    overlay.appendChild(title);
    overlay.appendChild(meta);

    const actions = this.createActions();

    card.appendChild(imageContainer);
    card.appendChild(overlay);
    card.appendChild(actions);

    this.bindEvents(card);

    return card;
  }

  createActions() {
    const actions = utils.createElement('div', 'photo-card-actions');

    if (this.photo.isCollage) {
      // Accept Button (Green, check icon)
      const acceptBtn = utils.createElement('button', 'card-action-btn accept-btn');
      acceptBtn.title = utils.t('ui.accept_collage', 'Accept');
      acceptBtn.dataset.action = 'accept-collage';
      acceptBtn.innerHTML = window.iconHelper.getIcon('check', { size: 18 });
      acceptBtn.style.color = '#10b981'; // Green

      // Reject Button (Red, x icon)
      const rejectBtn = utils.createElement('button', 'card-action-btn reject-btn');
      rejectBtn.title = utils.t('ui.reject_collage', 'Reject');
      rejectBtn.dataset.action = 'reject-collage';
      rejectBtn.innerHTML = window.iconHelper.getIcon('x', { size: 18 });
      rejectBtn.style.color = '#ef4444'; // Red

      actions.appendChild(acceptBtn);
      actions.appendChild(rejectBtn);
    } else if (this.photo.housekeepingReason) {
      // Keep Button (Remove from list)
      const keepBtn = utils.createElement('button', 'card-action-btn keep-btn');
      keepBtn.title = 'Keep (Remove from housekeeping list)';
      keepBtn.dataset.action = 'keep';
      keepBtn.innerHTML = window.iconHelper.getIcon('check', { size: 18 });
      keepBtn.style.color = '#10b981'; // Green

      // Delete Button
      const deleteBtn = utils.createElement('button', 'card-action-btn delete-btn');
      deleteBtn.title = 'Delete Photo';
      deleteBtn.dataset.action = 'delete-housekeeping'; // specific action
      deleteBtn.innerHTML = window.iconHelper.getIcon('trash-2', { size: 18 });
      deleteBtn.style.color = '#ef4444'; // Red

      actions.appendChild(keepBtn);
      actions.appendChild(deleteBtn);
    } else {
      const favoriteBtn = utils.createElement(
        'button',
        `card-action-btn favorite-btn${this.photo.is_favorite ? ' active' : ''}`
      );
      favoriteBtn.title = utils.t('ui.add_to_favorites', 'Add to Favorites');
      favoriteBtn.dataset.action = 'favorite';
      favoriteBtn.innerHTML = window.iconHelper.getSemanticIcon('favorite', { size: 18 });

      const downloadBtn = utils.createElement('button', 'card-action-btn download-btn');
      downloadBtn.title = utils.t('ui.download', 'Download');
      downloadBtn.dataset.action = 'download';
      downloadBtn.innerHTML = window.iconHelper.getSemanticIcon('download', { size: 18 });

      actions.appendChild(favoriteBtn);
      actions.appendChild(downloadBtn);
    }

    return actions;
  }

  bindEvents(card) {
    utils.on(card, 'click', (e) => {
      if (!e.target.closest('.card-action-btn')) {
        this.openViewer();
      }
    });

    const favoriteBtn = card.querySelector('[data-action="favorite"]');
    const downloadBtn = card.querySelector('[data-action="download"]');
    const keepBtn = card.querySelector('[data-action="keep"]');
    const deleteHousekeepingBtn = card.querySelector('[data-action="delete-housekeeping"]');
    const acceptCollageBtn = card.querySelector('[data-action="accept-collage"]');
    const rejectCollageBtn = card.querySelector('[data-action="reject-collage"]');

    if (favoriteBtn) {
      utils.on(favoriteBtn, 'click', (e) => {
        e.stopPropagation();
        this.toggleFavorite(favoriteBtn);
      });
    }

    if (downloadBtn) {
      utils.on(downloadBtn, 'click', (e) => {
        e.stopPropagation();
        this.download();
      });
    }

    if (keepBtn) {
      utils.on(keepBtn, 'click', (e) => {
        e.stopPropagation();
        this.keepPhoto();
      });
    }

    if (deleteHousekeepingBtn) {
      utils.on(deleteHousekeepingBtn, 'click', (e) => {
        e.stopPropagation();
        this.deletePhoto();
      });
    }

    if (acceptCollageBtn) {
      utils.on(acceptCollageBtn, 'click', (e) => {
        e.stopPropagation();
        this.acceptCollage();
      });
    }

    if (rejectCollageBtn) {
      utils.on(rejectCollageBtn, 'click', (e) => {
        e.stopPropagation();
        this.rejectCollage();
      });
    }
  }

  getTitle() {
    return this.photo.filename || `Photo ${this.photo.hash_sha256.substring(0, 8)}`;
  }

  getMeta() {
    const parts = [];

    if (this.photo.taken_at) {
      const date = new Date(this.photo.taken_at);
      parts.push(date.toLocaleDateString());
    }

    const camera = this.photo.metadata?.camera || {};
    if (camera.make && camera.model) {
      parts.push(`${camera.make} ${camera.model}`);
    }

    if (this.photo.file_size) {
      parts.push(utils.formatFileSize(this.photo.file_size));
    }

    return parts.join(' • ');
  }

  async toggleFavorite(button) {
    const wasAlreadyFavorite = this.photo.is_favorite;
    const newFavoriteState = !wasAlreadyFavorite;

    button.classList.toggle('active', newFavoriteState);
    button.title = newFavoriteState
      ? utils.t('ui.remove_from_favorites', 'Remove from Favorites')
      : utils.t('ui.add_to_favorites', 'Add to Favorites');

    try {
      if (newFavoriteState) {
        await api.addToFavorites(this.photo.hash_sha256);
      } else {
        await api.removeFromFavorites(this.photo.hash_sha256);
      }

      this.photo.is_favorite = newFavoriteState;

      utils.showToast(
        newFavoriteState ? utils.t('ui.added', 'Added') : utils.t('ui.removed', 'Removed'),
        newFavoriteState
          ? utils.t('messages.photo_added_to_favorites', 'Photo added to favorites')
          : utils.t('messages.photo_removed_from_favorites', 'Photo removed from favorites'),
        'success',
        2000
      );

      utils.emit(window, 'favoriteToggled', {
        photoHash: this.photo.hash_sha256,
        isFavorite: newFavoriteState,
      });
    } catch (error) {
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

  download() {
    const link = utils.createElement('a');
    link.href = utils.getPhotoUrl(this.photo.hash_sha256);
    link.download = this.photo.filename || `photo-${this.photo.hash_sha256.substring(0, 8)}`;
    link.click();

    utils.showToast(
      utils.t('ui.download', 'Download'),
      utils.t('messages.photo_download_started', 'Photo download started'),
      'info',
      2000
    );
  }

  async keepPhoto() {
    try {
      await api.removeHousekeepingCandidate(this.photo.hash_sha256);
      utils.showToast('Kept', 'Photo removed from housekeeping candidates', 'success', 2000);
      // Remove card from UI
      if (this.grid && this.grid.removePhoto) {
        this.grid.removePhoto(this.photo.hash_sha256);
      } else {
        // Fallback if grid doesn't have removePhoto or we are just removing the card element
        // But PhotoGrid usually manages DOM.
        // We will implement removePhoto in HousekeepingManager's grid or similar.
        // For now, emit an event
        utils.emit(window, 'housekeepingCandidateRemoved', { hash: this.photo.hash_sha256 });
      }
    } catch (e) {
      console.error('Failed to keep photo:', e);
      utils.showToast('Error', 'Failed to keep photo', 'error');
    }
  }

  async deletePhoto() {
    if (!confirm('Are you sure you want to permanently delete this photo?')) return;

    try {
      await api.deletePhoto(this.photo.hash_sha256);
      utils.showToast('Deleted', 'Photo deleted permanently', 'success', 2000);
      utils.emit(window, 'housekeepingCandidateRemoved', { hash: this.photo.hash_sha256 });
    } catch (e) {
      console.error('Failed to delete photo:', e);
      utils.showToast('Error', 'Failed to delete photo', 'error');
    }
  }

  async acceptCollage() {
    try {
      await api.acceptCollage(this.photo.collageId);
      utils.showToast('Accepted', 'Collage accepted', 'success', 2000);
      utils.emit(window, 'collageAccepted', { collageId: this.photo.collageId });
    } catch (e) {
      console.error('Failed to accept collage:', e);
      utils.showToast('Error', 'Failed to accept collage', 'error');
    }
  }

  async rejectCollage() {
    const confirmMessage = utils.t(
      'messages.confirm_reject_collage',
      'Are you sure you want to reject this collage?'
    );
    if (!confirm(confirmMessage)) return;

    try {
      await api.rejectCollage(this.photo.collageId);
      utils.showToast('Rejected', 'Collage rejected', 'success', 2000);
      utils.emit(window, 'collageRejected', { collageId: this.photo.collageId });
    } catch (e) {
      console.error('Failed to reject collage:', e);
      utils.showToast('Error', 'Failed to reject collage', 'error');
    }
  }

  openViewer() {
    if (window.photoViewer) {
      window.photoViewer.open(this.photo, this.grid.photos);
    }
  }
}

window.PhotoCard = PhotoCard;
