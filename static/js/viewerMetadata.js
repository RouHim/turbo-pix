class ViewerMetadata {
  constructor(elements) {
    this.elements = elements;
  }

  updatePhotoInfo(photo) {
    if (!photo) return;

    const isVideo = this.isVideoFile(photo.filename);

    this.updateBasicInfo(photo);
    this.updateDetailedMetadata(photo, isVideo);
  }

  updateBasicInfo(photo) {
    const unknownText = window.i18nManager ? window.i18nManager.t('ui.unknown') : 'Unknown';
    const noLocationText = window.i18nManager
      ? window.i18nManager.t('ui.no_location_data')
      : 'No location data';

    if (this.elements.title) {
      this.elements.title.textContent =
        photo.filename || `Photo ${photo.hash_sha256.substring(0, 8)}`;
    }

    if (this.elements.date) {
      this.elements.date.textContent = photo.taken_at
        ? utils.formatDate(photo.taken_at)
        : unknownText;
    }

    if (this.elements.size) {
      const sizeText = photo.file_size ? utils.formatFileSize(photo.file_size) : unknownText;
      const dimensions = photo.width && photo.height ? ` • ${photo.width}×${photo.height}` : '';
      this.elements.size.textContent = sizeText + dimensions;
    }

    if (this.elements.camera) {
      const camera =
        photo.camera_make && photo.camera_model
          ? `${photo.camera_make} ${photo.camera_model}`
          : unknownText;
      this.elements.camera.textContent = camera;
    }

    if (this.elements.location) {
      const location =
        photo.latitude && photo.longitude
          ? `${photo.latitude.toFixed(6)}, ${photo.longitude.toFixed(6)}`
          : noLocationText;
      this.elements.location.textContent = location;
    }
  }

  updateDetailedMetadata(photo, isVideo) {
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

    this.updateCameraSection(photo);
    this.updateSettingsSection(photo);
    this.updateLocationSection(photo);
    this.updateVideoSection(photo, isVideo);
  }

  updateCameraSection(photo) {
    const hasCamera =
      photo.camera_make || photo.camera_model || photo.lens_make || photo.lens_model;
    this.toggleSection('camera-section', hasCamera);
    this.setMetaField('meta-camera-make', photo.camera_make);
    this.setMetaField('meta-camera-model', photo.camera_model);
    this.setMetaField('meta-lens-make', photo.lens_make);
    this.setMetaField('meta-lens-model', photo.lens_model);
  }

  updateSettingsSection(photo) {
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
    const yesText = window.i18nManager ? window.i18nManager.t('ui.yes') : 'Yes';
    const noText = window.i18nManager ? window.i18nManager.t('ui.no') : 'No';

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
      photo.flash_used !== null ? (photo.flash_used ? yesText : noText) : null
    );
    this.setMetaField('meta-orientation', photo.orientation);
    this.setMetaField('meta-colorspace', photo.color_space);
  }

  updateLocationSection(photo) {
    const hasLocation = photo.latitude || photo.longitude || photo.location_name;
    this.toggleSection('location-section', hasLocation);
    this.setMetaField(
      'meta-gps',
      photo.latitude && photo.longitude
        ? `${photo.latitude.toFixed(6)}, ${photo.longitude.toFixed(6)}`
        : null
    );
    this.setMetaField('meta-location-name', photo.location_name);
  }

  updateVideoSection(photo, isVideo) {
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
  }

  updateFavoriteButton(photo) {
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

    if (this.elements.metadataBtn) {
      this.elements.metadataBtn.classList.add('active');
    }
  }

  hideMetadata() {
    if (this.elements.metadataContainer) {
      this.elements.metadataContainer.style.display = 'none';
    }

    if (this.elements.metadataBtn) {
      this.elements.metadataBtn.classList.remove('active');
    }
  }

  isVideoFile(filename) {
    if (!filename) return false;
    const ext = filename.toLowerCase().substring(filename.lastIndexOf('.'));
    return window.APP_CONSTANTS.VIDEO_EXTENSIONS.includes(ext);
  }
}

window.ViewerMetadata = ViewerMetadata;
