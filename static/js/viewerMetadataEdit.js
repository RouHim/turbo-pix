class MetadataEditor {
  constructor() {
    this.modal = document.getElementById('metadata-edit-modal');
    this.form = document.getElementById('metadata-edit-form');
    this.editBtn = document.getElementById('metadata-edit-btn');
    this.closeBtn = document.getElementById('metadata-edit-close');
    this.cancelBtn = document.getElementById('metadata-edit-cancel');
    this.saveBtn = document.getElementById('metadata-edit-save');
    this.errorEl = document.getElementById('metadata-edit-error');

    this.takenAtInput = document.getElementById('edit-taken-at');
    this.latitudeInput = document.getElementById('edit-latitude');
    this.longitudeInput = document.getElementById('edit-longitude');

    this.currentPhoto = null;

    this.bindEvents();
  }

  bindEvents() {
    this.editBtn?.addEventListener('click', () => this.openModal());
    this.closeBtn?.addEventListener('click', () => this.closeModal());
    this.cancelBtn?.addEventListener('click', () => this.closeModal());
    this.form?.addEventListener('submit', (e) => this.handleSubmit(e));

    // Close modal when clicking outside
    this.modal?.addEventListener('click', (e) => {
      if (e.target === this.modal) {
        this.closeModal();
      }
    });

    // Clear error when inputs change
    [this.takenAtInput, this.latitudeInput, this.longitudeInput].forEach((input) => {
      input?.addEventListener('input', () => this.clearError());
    });
  }

  setPhoto(photo) {
    this.currentPhoto = photo;
    // Show edit button only for supported image file types
    if (this.editBtn && photo) {
      const isSupported = photo.mime_type?.startsWith('image/');
      this.editBtn.style.display = isSupported ? 'block' : 'none';
      // Update feather icons if needed
      window.feather?.replace();
    }
  }

  openModal() {
    if (!this.currentPhoto || !this.modal) return;

    // Pre-fill form with current values
    this.populateForm();

    // Show modal
    this.modal.style.display = 'flex';
    document.body.style.overflow = 'hidden';
  }

  closeModal() {
    if (!this.modal) return;

    this.modal.style.display = 'none';
    document.body.style.overflow = '';
    this.clearError();
    this.form?.reset();
  }

  populateForm() {
    if (!this.currentPhoto) return;

    // Date Taken
    if (this.takenAtInput && this.currentPhoto.taken_at) {
      // Convert ISO 8601 to datetime-local format (YYYY-MM-DDTHH:MM)
      const date = new Date(this.currentPhoto.taken_at);
      const localDatetime = new Date(date.getTime() - date.getTimezoneOffset() * 60000)
        .toISOString()
        .slice(0, 16);
      this.takenAtInput.value = localDatetime;
    }

    // GPS coordinates
    const location = this.currentPhoto.metadata?.location || {};
    if (this.latitudeInput && location.latitude != null) {
      this.latitudeInput.value = location.latitude;
    }
    if (this.longitudeInput && location.longitude != null) {
      this.longitudeInput.value = location.longitude;
    }
  }

  async handleSubmit(e) {
    e.preventDefault();

    if (!this.currentPhoto) return;

    this.clearError();
    this.setSaveButtonLoading(true);

    try {
      const updates = this.getFormData();

      // Validate
      const validation = this.validateFormData(updates);
      if (!validation.valid) {
        this.showError(validation.message);
        this.setSaveButtonLoading(false);
        return;
      }

      // Call API
      const updatedPhoto = await window.api.updatePhotoMetadata(
        this.currentPhoto.hash_sha256,
        updates
      );

      // Success - update the current photo and refresh UI
      this.currentPhoto = updatedPhoto;
      if (window.appState) {
        window.appState.selectedPhoto = updatedPhoto;
      }

      // Update viewer's current photo and photos array
      if (window.photoViewer) {
        window.photoViewer.currentPhoto = updatedPhoto;

        // Update the photo in the photos array to keep it in sync
        const photos = window.photoViewer.photos;
        const index = photos.findIndex((p) => p.hash_sha256 === updatedPhoto.hash_sha256);
        if (index !== -1) {
          photos[index] = updatedPhoto;
        }
      }

      // Refresh metadata display
      if (window.photoViewer?.metadata) {
        window.photoViewer.metadata.updatePhotoInfo(updatedPhoto);
      }

      // Update photo card in grid
      if (window.photoGrid) {
        window.photoGrid.updatePhotoCard(updatedPhoto);
      }

      // Show success toast
      utils.showToast(
        'Success',
        window.i18nManager?.t('ui.metadata.edit_success') || 'Metadata updated successfully',
        'success',
        3000
      );

      this.closeModal();
    } catch (error) {
      console.error('Failed to update metadata:', error);

      // Try to extract a meaningful error message
      let errorMessage =
        window.i18nManager?.t('ui.metadata.edit_error') || 'Failed to update metadata';

      if (error.message) {
        // API errors come as "HTTP 500: error details"
        // Try to extract the meaningful part after the status code
        const match = error.message.match(/HTTP \d+: (.+)/);
        if (match && match[1]) {
          errorMessage = match[1];
        } else {
          errorMessage = error.message;
        }
      }

      this.showError(errorMessage);
    } finally {
      this.setSaveButtonLoading(false);
    }
  }

  getFormData() {
    const updates = {};

    // Date Taken - convert from datetime-local to ISO 8601 UTC
    if (this.takenAtInput?.value) {
      const localDate = new Date(this.takenAtInput.value);
      updates.taken_at = localDate.toISOString();
    }

    // GPS Coordinates
    const lat = this.latitudeInput?.value;
    const lng = this.longitudeInput?.value;

    if (lat !== '' && lat != null) {
      updates.latitude = parseFloat(lat);
    }
    if (lng !== '' && lng != null) {
      updates.longitude = parseFloat(lng);
    }

    return updates;
  }

  validateFormData(updates) {
    // GPS coordinates must be provided together
    const hasLat = updates.latitude != null;
    const hasLng = updates.longitude != null;

    if ((hasLat && !hasLng) || (!hasLat && hasLng)) {
      return {
        valid: false,
        message:
          window.i18nManager?.t('ui.metadata.edit_validation_gps_pair') ||
          'Both latitude and longitude must be provided together',
      };
    }

    // Validate GPS ranges
    if (hasLat && (updates.latitude < -90 || updates.latitude > 90)) {
      return {
        valid: false,
        message:
          window.i18nManager?.t('ui.metadata.edit_validation_gps') ||
          'GPS coordinates must be between -90/90 (lat) and -180/180 (lng)',
      };
    }

    if (hasLng && (updates.longitude < -180 || updates.longitude > 180)) {
      return {
        valid: false,
        message:
          window.i18nManager?.t('ui.metadata.edit_validation_gps') ||
          'GPS coordinates must be between -90/90 (lat) and -180/180 (lng)',
      };
    }

    return { valid: true };
  }

  showError(message) {
    if (this.errorEl) {
      this.errorEl.textContent = message;
      this.errorEl.style.display = 'block';
    }
  }

  clearError() {
    if (this.errorEl) {
      this.errorEl.textContent = '';
      this.errorEl.style.display = 'none';
    }
  }

  setSaveButtonLoading(loading) {
    if (!this.saveBtn) return;

    if (loading) {
      this.saveBtn.disabled = true;
      this.saveBtn.textContent = window.i18nManager?.t('ui.loading') || 'Saving...';
    } else {
      this.saveBtn.disabled = false;
      this.saveBtn.textContent = window.i18nManager?.t('ui.metadata.edit_save') || 'Save';
    }
  }
}

// Initialize when DOM is ready
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', () => {
    window.metadataEditor = new MetadataEditor();
  });
} else {
  window.metadataEditor = new MetadataEditor();
}
