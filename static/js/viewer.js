// Photo Viewer Component
/* global ViewerControls, ViewerMetadata, GestureManager */

class PhotoViewer {
  constructor() {
    this.isOpen = false;
    this.currentPhoto = null;
    this.currentIndex = 0;
    this.photos = [];
    this.preloadedImages = new Map();

    this.elements = {
      viewer: utils.$('#photo-viewer'),
      overlay: utils.$('.viewer-overlay'),
      content: utils.$('.viewer-content'),
      close: utils.$('.viewer-close'),
      prev: utils.$('.viewer-prev'),
      next: utils.$('.viewer-next'),
      image: utils.$('#viewer-image'),
      video: utils.$('#viewer-video'),
      viewerLoading: utils.$('.viewer-loading-indicator'),
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
    };

    this.controls = new ViewerControls(this, this.elements);
    this.metadata = new ViewerMetadata(this.elements);
    this.gestureManager = null;

    this.init();
  }

  init() {
    this.bindEvents();
    this.setupKeyboardNavigation();
    this.setupTouchGestures();
  }

  bindEvents() {
    if (this.elements.close) {
      utils.on(this.elements.close, 'click', () => this.close());
    }

    if (this.elements.overlay) {
      utils.on(this.elements.overlay, 'click', () => this.close());
    }

    if (this.elements.prev) {
      utils.on(this.elements.prev, 'click', () => this.showPrevious());
    }

    if (this.elements.next) {
      utils.on(this.elements.next, 'click', () => this.showNext());
    }

    if (this.elements.favoriteBtn) {
      utils.on(this.elements.favoriteBtn, 'click', () => this.toggleFavorite());
    }

    if (this.elements.downloadBtn) {
      utils.on(this.elements.downloadBtn, 'click', () => this.downloadPhoto());
    }

    if (this.elements.metadataBtn) {
      utils.on(this.elements.metadataBtn, 'click', () => this.toggleInfo());
    }

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

    // Initialize GestureManager when available
    if (typeof GestureManager !== 'undefined') {
      this.initGestureManager();
    } else {
      // Wait for GestureManager to load
      setTimeout(() => this.setupTouchGestures(), 100);
    }
  }

  initGestureManager() {
    this.gestureManager = new GestureManager(this.elements.viewer, {
      enablePinch: true,
      enableSwipe: true,
      enableDoubleTap: true,
      enablePan: true,
    });

    // Pinch to zoom
    this.gestureManager.on('pinch', (data) => {
      const { scale, deltaScale, centerX, centerY, initialCenterX, initialCenterY } = data;

      if (!this.pinchStarted) {
        this.controls.startPinchZoom(scale, initialCenterX, initialCenterY);
        this.pinchStarted = true;
        // Disable transitions during gesture
        if (this.elements.image) this.elements.image.classList.add('gesture-active');
        if (this.elements.video) this.elements.video.classList.add('gesture-active');
      }

      this.controls.updatePinchZoom(scale, deltaScale, centerX, centerY);
    });

    this.gestureManager.on('pinchEnd', () => {
      this.controls.endPinchZoom();
      this.pinchStarted = false;
      // Re-enable transitions
      if (this.elements.image) this.elements.image.classList.remove('gesture-active');
      if (this.elements.video) this.elements.video.classList.remove('gesture-active');
    });

    // Double tap to zoom
    this.gestureManager.on('doubleTap', (data) => {
      const { x, y } = data;
      this.controls.doubleTapZoom(x, y);
    });

    // Swipe navigation
    this.gestureManager.on('swipe', (data) => {
      const { direction, velocity } = data;

      // Only allow swipe navigation when not zoomed
      if (this.controls.isZoomed()) {
        return;
      }

      if (direction === 'left') {
        this.showNext();
        this.triggerHapticFeedback('light');
      } else if (direction === 'right') {
        this.showPrevious();
        this.triggerHapticFeedback('light');
      } else if (direction === 'up') {
        // Swipe up could toggle info in the future
      } else if (direction === 'down') {
        // Swipe down to close (with threshold)
        if (velocity > 0.5) {
          this.close();
          this.triggerHapticFeedback('medium');
        }
      }
    });

    // Pan while zoomed
    this.gestureManager.on('pan', (data) => {
      const { deltaX, deltaY } = data;

      // Only allow pan when zoomed
      if (this.controls.isZoomed()) {
        this.gestureManager.enablePan();
        this.controls.updateTouchPan(deltaX, deltaY);
        // Disable transitions during pan
        if (this.elements.image) this.elements.image.classList.add('gesture-active');
      } else {
        this.gestureManager.disablePan();
      }
    });

    this.gestureManager.on('panEnd', (data) => {
      const { velocityX, velocityY } = data;

      // Re-enable transitions
      if (this.elements.image) this.elements.image.classList.remove('gesture-active');

      if (this.controls.isZoomed()) {
        this.controls.applyMomentum(velocityX, velocityY);
      }
    });
  }

  triggerHapticFeedback(intensity = 'light') {
    if ('vibrate' in navigator) {
      const patterns = {
        light: 10,
        medium: 20,
        heavy: 50,
      };
      navigator.vibrate(patterns[intensity] || 10);
    }
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

    // Ensure sidebar is hidden on open
    if (this.elements.sidebar) {
      this.elements.sidebar.classList.remove('show');
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

    // Hide sidebar when closing viewer
    if (this.elements.sidebar) {
      this.elements.sidebar.classList.remove('show');
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
    this.controls.reset();

    // Hide current image/video immediately (no fade) and show viewer loading indicator
    if (this.elements.image) {
      this.elements.image.classList.remove('loaded');
      this.elements.image.style.display = 'none';
    }
    if (this.elements.video) {
      this.elements.video.pause();
      this.elements.video.classList.remove('loaded');
      this.elements.video.style.display = 'none';
    }
    if (this.elements.viewerLoading) {
      this.elements.viewerLoading.classList.add('show');
    }

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
      // Hide viewer loading indicator
      if (this.elements.viewerLoading) {
        this.elements.viewerLoading.classList.remove('show');
      }
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
      this.elements.image.classList.add('loaded');
      this.elements.video.style.display = 'none';
    }
  }

  async displayVideo(photo, forceTranscode = false) {
    if (!this.elements.video || !this.elements.image) {
      return;
    }

    // Check if video codec is HEVC and if browser supports it
    // Access codec from metadata JSON structure
    const videoCodec = photo.metadata?.video?.codec || '';
    const isHEVC = videoCodec.toLowerCase() === 'hevc' || videoCodec.toLowerCase() === 'h265';
    let needsTranscode = forceTranscode;

    if (isHEVC && !forceTranscode) {
      // Check browser HEVC support using Media Capabilities API
      const width = photo.width || 1920;
      const height = photo.height || 1080;
      const supportsHEVC = await utils.videoCodecSupport.supportsHEVC(width, height);

      if (window.logger) {
        window.logger.info('HEVC video detected', {
          component: 'PhotoViewer',
          photoHash: photo.hash_sha256,
          videoCodec,
          browserSupportsHEVC: supportsHEVC,
          width,
          height,
        });
      }

      needsTranscode = !supportsHEVC;
    }

    // Get video URL with optional transcoding
    const videoUrl = utils.getVideoUrl(photo.hash_sha256, { transcode: needsTranscode });

    // Check if transcoding failed by fetching headers with a minimal range request
    let transcodingFailed = false;
    if (needsTranscode) {
      try {
        const response = await fetch(videoUrl, {
          method: 'GET',
          headers: {
            Range: 'bytes=0-0', // Request just 1 byte to check headers
          },
        });
        const warningHeader = response.headers.get('X-Transcode-Warning');
        if (warningHeader && warningHeader.trim() !== '') {
          transcodingFailed = true;
          if (window.logger) {
            window.logger.info('HEVC transcoding failed on server', {
              component: 'PhotoViewer',
              photoHash: photo.hash_sha256,
              warning: warningHeader,
            });
          }
        }
      } catch (error) {
        if (window.logger) {
          window.logger.warn('Failed to check transcoding status', error);
        }
      }
    }

    // Force video reload by clearing src first to prevent browser caching issues
    this.elements.video.src = '';
    this.elements.video.load(); // Trigger reload

    // Remove old error handlers to prevent duplicates
    this.elements.video.onerror = null;

    // Add error handler for playback failures
    this.elements.video.onerror = async () => {
      if (window.logger) {
        window.logger.error('Video playback failed', null, {
          component: 'PhotoViewer',
          photoHash: photo.hash_sha256,
          videoCodec,
          needsTranscode,
          transcodingFailed,
          forceTranscode,
        });
      }

      // If HEVC playback failed and we haven't tried transcoding yet, retry with transcoding
      if (isHEVC && !needsTranscode && !forceTranscode) {
        if (window.logger) {
          window.logger.info('HEVC native playback failed, retrying with transcoding', {
            component: 'PhotoViewer',
            photoHash: photo.hash_sha256,
          });
        }

        utils.showToast(
          'Retrying',
          'Native HEVC playback failed, attempting transcoded version...',
          'info',
          3000
        );

        // Retry with forced transcoding
        await this.displayVideo(photo, true);
        return;
      }

      // Show clear error message
      if (transcodingFailed) {
        // HEVC transcoding failed on server
        const errorMessage = `⚠️ Video Cannot Play

This video uses HEVC (H.265) encoding, which your browser doesn't support.

The server attempted to convert it to a compatible format, but transcoding failed because ffmpeg with HEVC support is not installed on the server.

Server Administrator: Install ffmpeg with HEVC decoding support to enable playback.`;

        utils.showToast('Video Cannot Play', errorMessage, 'error', 12000);
        this.showError(errorMessage);
      } else {
        // Generic playback error
        const errorMessage =
          'Failed to load video. The video file may be corrupted or in an unsupported format.';
        utils.showToast('Playback Error', errorMessage, 'error', 6000);
        this.showError(errorMessage);
      }
    };

    // Now set the new source
    this.elements.video.src = videoUrl;
    this.elements.video.style.display = 'block';
    this.elements.video.classList.add('loaded');
    this.elements.image.style.display = 'none';

    // Auto-play if user preference allows
    const settings = api.getViewSettings();
    if (settings.autoPlay) {
      this.elements.video.play().catch(() => {
        // Auto-play failed, user interaction required
      });
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

    this.metadata.updatePhotoInfo(this.currentPhoto);
    this.metadata.updateFavoriteButton(this.currentPhoto);

    // Update metadata editor with current photo
    if (window.metadataEditor) {
      window.metadataEditor.setPhoto(this.currentPhoto);
    }
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

      this.metadata.updateFavoriteButton(this.currentPhoto);

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

  toggleInfo() {
    // Toggle the sidebar on all devices
    this.toggleSidebar();
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
    const ext = filename.toLowerCase().substring(filename.lastIndexOf('.'));
    return window.APP_CONSTANTS.VIDEO_EXTENSIONS.includes(ext);
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
