// Photo Viewer Component

class PhotoViewer {
  constructor() {
    this.isOpen = false;
    this.currentPhoto = null;
    this.currentIndex = 0;
    this.photos = [];
    this.preloadedImages = new Map();
    this.zoomLevel = 1;
    this.maxZoom = 3;
    this.minZoom = 0.5;
    this.isDragging = false;
    this.dragStart = { x: 0, y: 0 };
    this.imagePosition = { x: 0, y: 0 };

    this.elements = {
      viewer: utils.$('#photo-viewer'),
      overlay: utils.$('.viewer-overlay'),
      content: utils.$('.viewer-content'),
      close: utils.$('.viewer-close'),
      prev: utils.$('.viewer-prev'),
      next: utils.$('.viewer-next'),
      image: utils.$('#viewer-image'),
      video: utils.$('#viewer-video'),
      sidebar: utils.$('.viewer-sidebar'),
      title: utils.$('#photo-title'),
      date: utils.$('#photo-date'),
      size: utils.$('#photo-size'),
      camera: utils.$('#photo-camera'),
      location: utils.$('#photo-location'),
      favoriteBtn: utils.$('.favorite-btn'),
      downloadBtn: utils.$('.download-btn'),
      metadataBtn: utils.$('.metadata-btn'),
      metadataContainer: utils.$('.photo-meta-full'),
      zoomIn: utils.$('.zoom-in'),
      zoomOut: utils.$('.zoom-out'),
      zoomFit: utils.$('.zoom-fit'),
      fullscreenBtn: utils.$('.fullscreen-btn'),
      infoToggle: utils.$('.info-toggle'),
    };

    this.init();
  }

  init() {
    this.bindEvents();
    this.setupKeyboardNavigation();
    this.setupTouchGestures();
  }

  bindEvents() {
    // Close button
    if (this.elements.close) {
      utils.on(this.elements.close, 'click', () => this.close());
    }

    // Overlay click to close
    if (this.elements.overlay) {
      utils.on(this.elements.overlay, 'click', () => this.close());
    }

    // Navigation buttons
    if (this.elements.prev) {
      utils.on(this.elements.prev, 'click', () => this.showPrevious());
    }

    if (this.elements.next) {
      utils.on(this.elements.next, 'click', () => this.showNext());
    }

    // Action buttons
    if (this.elements.favoriteBtn) {
      utils.on(this.elements.favoriteBtn, 'click', () => this.toggleFavorite());
    }

    if (this.elements.downloadBtn) {
      utils.on(this.elements.downloadBtn, 'click', () => this.downloadPhoto());
    }

    if (this.elements.metadataBtn) {
      utils.on(this.elements.metadataBtn, 'click', () => this.toggleMetadata());
    }

    // Zoom controls
    if (this.elements.zoomIn) {
      utils.on(this.elements.zoomIn, 'click', () => this.zoomIn());
    }

    if (this.elements.zoomOut) {
      utils.on(this.elements.zoomOut, 'click', () => this.zoomOut());
    }

    if (this.elements.zoomFit) {
      utils.on(this.elements.zoomFit, 'click', () => this.fitToScreen());
    }

    // Fullscreen button
    if (this.elements.fullscreenBtn) {
      utils.on(this.elements.fullscreenBtn, 'click', () => this.toggleFullscreen());
    }

    // Info toggle button (mobile sidebar toggle)
    if (this.elements.infoToggle) {
      utils.on(this.elements.infoToggle, 'click', () => this.toggleSidebar());
    }

    // Image interaction for dragging when zoomed
    if (this.elements.image) {
      utils.on(this.elements.image, 'mousedown', (e) => this.startDrag(e));
      utils.on(document, 'mousemove', (e) => this.drag(e));
      utils.on(document, 'mouseup', () => this.endDrag());
    }

    // Prevent click propagation on content
    if (this.elements.content) {
      utils.on(this.elements.content, 'click', (e) => e.stopPropagation());
    }
  }

  setupKeyboardNavigation() {
    utils.on(document, 'keydown', (e) => {
      if (!this.isOpen) return;

      switch (e.key) {
        case 'Escape':
          this.close();
          break;
        case 'ArrowLeft':
          e.preventDefault();
          this.showPrevious();
          break;
        case 'ArrowRight':
          e.preventDefault();
          this.showNext();
          break;
        case ' ':
          e.preventDefault();
          if (this.elements.video && !this.elements.video.paused) {
            this.elements.video.pause();
          } else if (this.elements.video && this.elements.video.paused) {
            this.elements.video.play();
          }
          break;
        case 'f':
          e.preventDefault();
          this.toggleFavorite();
          break;
        case 'd':
          e.preventDefault();
          this.downloadPhoto();
          break;
      }
    });
  }

  setupTouchGestures() {
    if (!this.elements.viewer) return;

    utils.on(this.elements.viewer, 'touchstart', (e) => {
      utils.touchHandler.handleTouchStart(e);
    });

    utils.on(this.elements.viewer, 'touchend', (e) => {
      utils.touchHandler.handleTouchEnd(
        e,
        () => this.showNext(), // swipe left -> next
        () => this.showPrevious(), // swipe right -> previous
        () => this.close(), // swipe up -> close
        () => this.toggleSidebar() // swipe down -> toggle sidebar
      );
    });
  }

  async open(photo, allPhotos = []) {
    this.photos = allPhotos;
    this.currentIndex = this.photos.findIndex((p) => p.hash_sha256 === photo.hash_sha256);
    if (this.currentIndex === -1) {
      this.photos = [photo];
      this.currentIndex = 0;
    }

    this.currentPhoto = photo;
    this.isOpen = true;

    // Show viewer
    if (this.elements.viewer) {
      this.elements.viewer.classList.add('active', 'fade-in');
      document.body.style.overflow = 'hidden';
    }

    // Load and display photo
    await this.displayPhoto(photo);
    this.updateNavigation();
    this.updatePhotoInfo();

    // Preload adjacent photos
    this.preloadAdjacentPhotos();

    // Analytics
    utils.performance.mark('viewer-open');
  }

  close() {
    this.isOpen = false;

    if (this.elements.viewer) {
      this.elements.viewer.classList.remove('active', 'fade-in');
      document.body.style.overflow = '';
    }

    // Stop any playing video
    if (this.elements.video) {
      this.elements.video.pause();
    }

    // Clear preloaded images to free memory
    this.preloadedImages.clear();

    utils.performance.mark('viewer-close');
  }

  async showPrevious() {
    if (this.currentIndex > 0) {
      this.currentIndex--;
      await this.showPhotoAtIndex(this.currentIndex);
    }
  }

  async showNext() {
    if (this.currentIndex < this.photos.length - 1) {
      this.currentIndex++;
      await this.showPhotoAtIndex(this.currentIndex);
    }
  }

  async showPhotoAtIndex(index) {
    if (index < 0 || index >= this.photos.length) return;

    this.currentIndex = index;
    this.currentPhoto = this.photos[index];

    await this.displayPhoto(this.currentPhoto);
    this.updateNavigation();
    this.updatePhotoInfo();
    this.preloadAdjacentPhotos();
  }

  async displayPhoto(photo) {
    // Reset zoom when displaying new photo
    this.fitToScreen();

    // Hide detailed metadata when changing photos
    this.hideMetadata();

    utils.showLoading();

    try {
      const isVideo = this.isVideoFile(photo.filename);

      if (window.logger) {
        window.logger.info('Displaying photo', {
          component: 'PhotoViewer',
          photoHash: photo.hash_sha256,
          filename: photo.filename,
          isVideo,
        });
      }

      if (isVideo) {
        await this.displayVideo(photo);
      } else {
        await this.displayImage(photo);
      }
    } catch (error) {
      if (window.logger) {
        window.logger.error('Error displaying photo', error, {
          component: 'PhotoViewer',
          photoHash: photo.hash_sha256,
          filename: photo.filename,
        });
      } else {
        console.error('Error displaying photo:', error);
      }
      this.showError('Failed to load photo');
    } finally {
      utils.hideLoading();
    }
  }

  async displayImage(photo) {
    const imageUrl = utils.getPhotoUrl(photo.hash_sha256);

    // Check if already preloaded
    if (this.preloadedImages.has(photo.hash_sha256)) {
      const img = this.preloadedImages.get(photo.hash_sha256);
      this.showImage(img.src);
      return;
    }

    // Load image
    const img = new Image();
    img.onload = () => {
      this.preloadedImages.set(photo.hash_sha256, img);
      this.showImage(img.src);
    };
    img.onerror = () => {
      this.showError('Failed to load image');
    };
    img.src = imageUrl;
  }

  showImage(src) {
    if (this.elements.image && this.elements.video) {
      this.elements.image.src = src;
      this.elements.image.style.display = 'block';
      this.elements.video.style.display = 'none';
    }
  }

  async displayVideo(photo) {
    const videoUrl = utils.getVideoUrl(photo.hash_sha256);

    if (this.elements.video && this.elements.image) {
      // Force video reload by clearing src first to prevent browser caching issues
      this.elements.video.src = '';
      this.elements.video.load(); // Trigger reload

      // Now set the new source
      this.elements.video.src = videoUrl;
      this.elements.video.style.display = 'block';
      this.elements.image.style.display = 'none';

      // Auto-play if user preference allows
      const settings = api.getViewSettings();
      if (settings.autoPlay) {
        this.elements.video.play().catch(() => {
          // Auto-play failed, user interaction required
        });
      }
    }
  }

  updateNavigation() {
    if (this.elements.prev) {
      this.elements.prev.style.display = this.currentIndex > 0 ? 'block' : 'none';
    }

    if (this.elements.next) {
      this.elements.next.style.display =
        this.currentIndex < this.photos.length - 1 ? 'block' : 'none';
    }
  }

  updatePhotoInfo() {
    if (!this.currentPhoto) return;

    const photo = this.currentPhoto;
    const isVideo = this.isVideoFile(photo.filename);

    // Title
    if (this.elements.title) {
      this.elements.title.textContent =
        photo.filename || `Photo ${photo.hash_sha256.substring(0, 8)}`;
    }

    // Basic metadata (always visible)
    if (this.elements.date) {
      this.elements.date.textContent = photo.taken_at
        ? utils.formatDate(photo.taken_at)
        : 'Unknown';
    }

    if (this.elements.size) {
      const sizeText = photo.file_size ? utils.formatFileSize(photo.file_size) : 'Unknown';
      const dimensions = photo.width && photo.height ? ` • ${photo.width}×${photo.height}` : '';
      this.elements.size.textContent = sizeText + dimensions;
    }

    if (this.elements.camera) {
      const camera =
        photo.camera_make && photo.camera_model
          ? `${photo.camera_make} ${photo.camera_model}`
          : 'Unknown';
      this.elements.camera.textContent = camera;
    }

    if (this.elements.location) {
      const location =
        photo.latitude && photo.longitude
          ? `${photo.latitude.toFixed(6)}, ${photo.longitude.toFixed(6)}`
          : 'No location data';
      this.elements.location.textContent = location;
    }

    // Detailed metadata (for the expandable section)
    this.setMetaField('meta-filename', photo.filename);
    this.setMetaField(
      'meta-filesize',
      photo.file_size ? utils.formatFileSize(photo.file_size) : null
    );
    this.setMetaField(
      'meta-dimensions',
      photo.width && photo.height ? `${photo.width} × ${photo.height} px` : null
    );
    this.setMetaField('meta-type', photo.mime_type);
    this.setMetaField('meta-date-taken', photo.taken_at ? utils.formatDate(photo.taken_at) : null);
    this.setMetaField(
      'meta-date-modified',
      photo.date_modified ? utils.formatDate(photo.date_modified) : null
    );

    const hasCamera =
      photo.camera_make || photo.camera_model || photo.lens_make || photo.lens_model;
    this.toggleSection('camera-section', hasCamera);
    this.setMetaField('meta-camera-make', photo.camera_make);
    this.setMetaField('meta-camera-model', photo.camera_model);
    this.setMetaField('meta-lens-make', photo.lens_make);
    this.setMetaField('meta-lens-model', photo.lens_model);

    const hasSettings =
      photo.iso ||
      photo.aperture ||
      photo.shutter_speed ||
      photo.focal_length ||
      photo.exposure_mode ||
      photo.metering_mode ||
      photo.white_balance ||
      photo.flash_used !== null ||
      photo.orientation ||
      photo.color_space;
    this.toggleSection('settings-section', hasSettings);
    this.setMetaField('meta-iso', photo.iso ? `ISO ${photo.iso}` : null);
    this.setMetaField('meta-aperture', photo.aperture ? `f/${photo.aperture.toFixed(1)}` : null);
    this.setMetaField('meta-shutter', photo.shutter_speed);
    this.setMetaField(
      'meta-focal',
      photo.focal_length ? `${photo.focal_length.toFixed(0)} mm` : null
    );
    this.setMetaField('meta-exposure', photo.exposure_mode);
    this.setMetaField('meta-metering', photo.metering_mode);
    this.setMetaField('meta-wb', photo.white_balance);
    this.setMetaField(
      'meta-flash',
      photo.flash_used !== null ? (photo.flash_used ? 'Yes' : 'No') : null
    );
    this.setMetaField('meta-orientation', photo.orientation);
    this.setMetaField('meta-colorspace', photo.color_space);

    const hasLocation = photo.latitude || photo.longitude || photo.location_name;
    this.toggleSection('location-section', hasLocation);
    this.setMetaField(
      'meta-gps',
      photo.latitude && photo.longitude
        ? `${photo.latitude.toFixed(6)}, ${photo.longitude.toFixed(6)}`
        : null
    );
    this.setMetaField('meta-location-name', photo.location_name);

    this.toggleSection('video-section', isVideo);
    if (isVideo) {
      this.setMetaField(
        'meta-duration',
        photo.duration ? this.formatDuration(photo.duration) : null
      );
      this.setMetaField('meta-video-codec', photo.video_codec);
      this.setMetaField('meta-audio-codec', photo.audio_codec);
      this.setMetaField(
        'meta-framerate',
        photo.frame_rate ? `${photo.frame_rate.toFixed(2)} fps` : null
      );
      this.setMetaField('meta-bitrate', photo.bitrate ? `${photo.bitrate} kbps` : null);
    }

    // Update favorite button
    if (this.elements.favoriteBtn) {
      const isFavorite = api.isFavorite(photo);
      this.elements.favoriteBtn.classList.toggle('active', isFavorite);
      this.elements.favoriteBtn.title = isFavorite
        ? window.i18nManager
          ? window.i18nManager.t('ui.remove_from_favorites')
          : 'Remove from Favorites'
        : window.i18nManager
          ? window.i18nManager.t('ui.add_to_favorites')
          : 'Add to Favorites';
    }
  }

  setMetaField(elementId, value) {
    const element = utils.$(`#${elementId}`);
    if (element) {
      element.textContent = value || '-';
      element.style.opacity = value ? '1' : '0.5';
    }
  }

  toggleSection(sectionId, show) {
    const section = utils.$(`#${sectionId}`);
    if (section) {
      section.style.display = show ? 'block' : 'none';
    }
  }

  formatDuration(seconds) {
    const hours = Math.floor(seconds / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    const secs = Math.floor(seconds % 60);

    if (hours > 0) {
      return `${hours}:${String(minutes).padStart(2, '0')}:${String(secs).padStart(2, '0')}`;
    }
    return `${minutes}:${String(secs).padStart(2, '0')}`;
  }

  preloadAdjacentPhotos() {
    const indices = [this.currentIndex - 1, this.currentIndex + 1];

    indices.forEach((index) => {
      if (index >= 0 && index < this.photos.length) {
        const photo = this.photos[index];
        if (!this.preloadedImages.has(photo.hash_sha256) && !this.isVideoFile(photo.filename)) {
          const img = new Image();
          img.onload = () => {
            this.preloadedImages.set(photo.hash_sha256, img);
          };
          img.src = utils.getPhotoUrl(photo.hash_sha256);
        }
      }
    });
  }

  async toggleFavorite() {
    if (!this.currentPhoto) return;

    const photoHash = this.currentPhoto.hash_sha256;
    const isFavorite = api.isFavorite(this.currentPhoto);

    try {
      if (isFavorite) {
        await api.removeFromFavorites(photoHash);
        this.currentPhoto.is_favorite = false;
        utils.showToast('Removed', 'Photo removed from favorites', 'info', 2000);
        if (window.logger) {
          window.logger.info('Photo removed from favorites', {
            component: 'PhotoViewer',
            photoHash,
            action: 'remove_favorite',
          });
        }
      } else {
        await api.addToFavorites(photoHash);
        this.currentPhoto.is_favorite = true;
        utils.showToast('Added', 'Photo added to favorites', 'success', 2000);
        if (window.logger) {
          window.logger.info('Photo added to favorites', {
            component: 'PhotoViewer',
            photoHash,
            action: 'add_favorite',
          });
        }
      }

      this.updatePhotoInfo();

      // Emit event for other components
      utils.emit(window, 'favoriteToggled', { photoHash, isFavorite: !isFavorite });
    } catch (error) {
      utils.showToast('Error', 'Failed to update favorite status', 'error', 2000);
      if (window.logger) {
        window.logger.error('Error toggling favorite', error, {
          component: 'PhotoViewer',
          photoHash,
        });
      }
    }
  }

  downloadPhoto() {
    if (!this.currentPhoto) return;

    const link = utils.createElement('a');
    link.href = utils.getPhotoUrl(this.currentPhoto.hash_sha256);
    link.download =
      this.currentPhoto.filename || `photo-${this.currentPhoto.hash_sha256.substring(0, 8)}`;
    link.click();

    utils.showToast('Download', 'Photo download started', 'info', 2000);
  }

  toggleSidebar() {
    if (this.elements.sidebar) {
      this.elements.sidebar.classList.toggle('show');
    }
  }

  showError(message) {
    if (this.elements.image) {
      this.elements.image.style.display = 'none';
    }
    if (this.elements.video) {
      this.elements.video.style.display = 'none';
    }

    utils.showToast('Error', message, 'error');
  }

  isVideoFile(filename) {
    if (!filename) return false;
    const videoExtensions = ['.mp4', '.mov', '.avi', '.mkv', '.webm', '.m4v'];
    const ext = filename.toLowerCase().substring(filename.lastIndexOf('.'));
    return videoExtensions.includes(ext);
  }

  zoomIn() {
    if (this.zoomLevel < this.maxZoom) {
      this.zoomLevel = Math.min(this.zoomLevel * 1.5, this.maxZoom);
      this.applyZoom();
    }
  }

  zoomOut() {
    if (this.zoomLevel > this.minZoom) {
      this.zoomLevel = Math.max(this.zoomLevel / 1.5, this.minZoom);
      this.applyZoom();
    }
  }

  fitToScreen() {
    this.zoomLevel = 1;
    this.imagePosition = { x: 0, y: 0 };
    this.applyZoom();
  }

  applyZoom() {
    if (!this.elements.image) return;

    const transform = `scale(${this.zoomLevel}) translate(${this.imagePosition.x}px, ${this.imagePosition.y}px)`;
    this.elements.image.style.transform = transform;

    // Update zoom class for cursor
    this.elements.image.classList.toggle('zoomed', this.zoomLevel > 1);
  }

  startDrag(e) {
    if (this.zoomLevel <= 1) return;

    e.preventDefault();
    this.isDragging = true;
    this.dragStart = {
      x: e.clientX - this.imagePosition.x,
      y: e.clientY - this.imagePosition.y,
    };
    this.elements.image.style.cursor = 'grabbing';
  }

  drag(e) {
    if (!this.isDragging || this.zoomLevel <= 1) return;

    e.preventDefault();
    this.imagePosition = {
      x: e.clientX - this.dragStart.x,
      y: e.clientY - this.dragStart.y,
    };
    this.applyZoom();
  }

  endDrag() {
    if (!this.isDragging) return;

    this.isDragging = false;
    if (this.elements.image) {
      this.elements.image.style.cursor = this.zoomLevel > 1 ? 'grab' : 'default';
    }
  }

  toggleFullscreen() {
    if (!document.fullscreenElement) {
      // Enter fullscreen
      if (this.elements.viewer) {
        if (this.elements.viewer.requestFullscreen) {
          this.elements.viewer.requestFullscreen();
        } else if (this.elements.viewer.mozRequestFullScreen) {
          // Firefox
          this.elements.viewer.mozRequestFullScreen();
        } else if (this.elements.viewer.webkitRequestFullscreen) {
          // Chrome, Safari and Opera
          this.elements.viewer.webkitRequestFullscreen();
        } else if (this.elements.viewer.msRequestFullscreen) {
          // IE/Edge
          this.elements.viewer.msRequestFullscreen();
        }
      }
    } else {
      // Exit fullscreen
      if (document.exitFullscreen) {
        document.exitFullscreen();
      } else if (document.mozCancelFullScreen) {
        // Firefox
        document.mozCancelFullScreen();
      } else if (document.webkitExitFullscreen) {
        // Chrome, Safari and Opera
        document.webkitExitFullscreen();
      } else if (document.msExitFullscreen) {
        // IE/Edge
        document.msExitFullscreen();
      }
    }
  }

  toggleMetadata() {
    if (!this.elements.metadataContainer) return;

    const isVisible = this.elements.metadataContainer.style.display !== 'none';

    if (isVisible) {
      this.hideMetadata();
    } else {
      this.showMetadata();
    }
  }

  showMetadata() {
    if (this.elements.metadataContainer) {
      this.elements.metadataContainer.style.display = 'block';
    }

    // Update button state
    if (this.elements.metadataBtn) {
      this.elements.metadataBtn.classList.add('active');
    }
  }

  hideMetadata() {
    if (this.elements.metadataContainer) {
      this.elements.metadataContainer.style.display = 'none';
    }

    // Update button state
    if (this.elements.metadataBtn) {
      this.elements.metadataBtn.classList.remove('active');
    }
  }

  // Public API
  getCurrentPhoto() {
    return this.currentPhoto;
  }

  getCurrentIndex() {
    return this.currentIndex;
  }

  getPhotos() {
    return this.photos;
  }
}

// Initialize global photo viewer when DOM is ready
function initGlobalPhotoViewer() {
  if (!window.photoViewer) {
    window.photoViewer = new PhotoViewer();
  }
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', initGlobalPhotoViewer);
} else {
  initGlobalPhotoViewer();
}
