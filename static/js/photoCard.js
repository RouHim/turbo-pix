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

    // WebP source with srcset for responsive images
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
    const img = utils.createElement('img', 'photo-card-image');
    img.src = `${utils.getThumbnailUrl(this.photo, 'medium')}&format=jpeg`;
    img.alt = this.getTitle();
    img.loading = 'lazy';
    img.decoding = 'async';

    // Fade in image when loaded
    img.onload = () => {
      imageContainer.classList.add('image-loaded');
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

    utils.on(favoriteBtn, 'click', (e) => {
      e.stopPropagation();
      this.toggleFavorite(favoriteBtn);
    });

    utils.on(downloadBtn, 'click', (e) => {
      e.stopPropagation();
      this.download();
    });
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

  openViewer() {
    if (window.photoViewer) {
      window.photoViewer.open(this.photo, this.grid.photos);
    }
  }
}

window.PhotoCard = PhotoCard;
