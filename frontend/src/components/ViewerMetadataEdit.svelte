<script>
  import { api } from '../lib/api.js';
  import { t } from '../lib/i18n.js';
  import { addToast } from '../lib/state.svelte.js';

  let {
    photo = null,
    onClose = () => {},
    onSaved = () => {},
  } = $props();

  let showModal = $state(false);
  let takenAt = $state('');
  let latitude = $state('');
  let longitude = $state('');
  let errorMessage = $state('');
  let saving = $state(false);

  function isFormatSupported(p) {
    if (!p?.mime_type) return false;
    const supported = ['image/jpeg', 'image/jpg', 'image/png'];
    return supported.includes(p.mime_type.toLowerCase());
  }

  function openModal() {
    if (!photo || !isFormatSupported(photo)) return;
    populateForm();
    showModal = true;
    document.body.style.overflow = 'hidden';
  }

  function closeModal() {
    showModal = false;
    document.body.style.overflow = '';
    errorMessage = '';
    onClose();
  }

  function populateForm() {
    if (!photo) return;
    errorMessage = '';

    if (photo.taken_at) {
      const date = new Date(photo.taken_at);
      const localDatetime = new Date(date.getTime() - date.getTimezoneOffset() * 60000)
        .toISOString()
        .slice(0, 16);
      takenAt = localDatetime;
    } else {
      takenAt = '';
    }

    const loc = photo.metadata?.location || {};
    latitude = loc.latitude != null ? String(loc.latitude) : '';
    longitude = loc.longitude != null ? String(loc.longitude) : '';
  }

  async function handleSubmit(e) {
    e.preventDefault();
    if (!photo) return;

    errorMessage = '';
    saving = true;

    try {
      const updates = {};

      if (takenAt) {
        const localDate = new Date(takenAt);
        updates.taken_at = localDate.toISOString();
      }

      const lat = latitude.trim();
      const lng = longitude.trim();
      const hasLat = lat !== '' && lat != null;
      const hasLng = lng !== '' && lng != null;

      if ((hasLat && !hasLng) || (!hasLat && hasLng)) {
        errorMessage = t('ui.metadata.edit_validation_gps_pair', {
          default: 'Both latitude and longitude must be provided together',
        });
        saving = false;
        return;
      }

      if (hasLat) {
        const latVal = parseFloat(lat);
        if (latVal < -90 || latVal > 90) {
          errorMessage = t('ui.metadata.edit_validation_gps', {
            default: 'GPS coordinates must be between -90/90 (lat) and -180/180 (lng)',
          });
          saving = false;
          return;
        }
        updates.latitude = latVal;
      }

      if (hasLng) {
        const lngVal = parseFloat(lng);
        if (lngVal < -180 || lngVal > 180) {
          errorMessage = t('ui.metadata.edit_validation_gps', {
            default: 'GPS coordinates must be between -90/90 (lat) and -180/180 (lng)',
          });
          saving = false;
          return;
        }
        updates.longitude = lngVal;
      }

      const updatedPhoto = await api.updatePhotoMetadata(photo.hash_sha256, updates);

      // Update photo refs
      if (onSaved) {
        onSaved(updatedPhoto);
      }

      addToast(
        t('ui.metadata.edit_success', { default: 'Metadata updated successfully' }),
        'success',
        3000
      );

      closeModal();
    } catch (error) {
      let msg = t('ui.metadata.edit_error', { default: 'Failed to update metadata' });
      if (error.message) {
        const match = error.message.match(/HTTP \d+: (.+)/);
        msg = match?.[1] || error.message;
      }
      errorMessage = msg;
    } finally {
      saving = false;
    }
  }

  function onOverlayClick(e) {
    if (e.target === e.currentTarget) closeModal();
  }

  // Expose open for parent
  export { openModal as open };
</script>

{#if showModal}
  <div
    id="metadata-edit-modal"
    class="modal"
    style="display: flex"
    onclick={onOverlayClick}
    role="dialog"
    aria-modal="true"
  >
    <div class="modal-content">
      <div class="modal-header">
        <h2>{$t('ui.metadata.edit_modal_title', { default: 'Edit Photo Metadata' })}</h2>
        <button
          type="button"
          id="metadata-edit-close"
          class="close-button"
          aria-label="Close"
          onclick={closeModal}
        >
          &times;
        </button>
      </div>
      <form id="metadata-edit-form" onsubmit={handleSubmit}>
        <div class="form-group">
          <label for="edit-taken-at">
            {$t('ui.metadata.edit_date_label', { default: 'Date Taken' })}
          </label>
          <input
            type="datetime-local"
            id="edit-taken-at"
            name="taken_at"
            bind:value={takenAt}
            oninput={() => { errorMessage = ''; }}
          />
        </div>
        <div class="form-group-row">
          <div class="form-group">
            <label for="edit-latitude">
              {$t('ui.metadata.edit_latitude_label', { default: 'Latitude' })}
            </label>
            <input
              type="number"
              id="edit-latitude"
              name="latitude"
              step="any"
              min="-90"
              max="90"
              placeholder="-90 to 90"
              bind:value={latitude}
              oninput={() => { errorMessage = ''; }}
            />
          </div>
          <div class="form-group">
            <label for="edit-longitude">
              {$t('ui.metadata.edit_longitude_label', { default: 'Longitude' })}
            </label>
            <input
              type="number"
              id="edit-longitude"
              name="longitude"
              step="any"
              min="-180"
              max="180"
              placeholder="-180 to 180"
              bind:value={longitude}
              oninput={() => { errorMessage = ''; }}
            />
          </div>
        </div>
        {#if errorMessage}
          <div id="metadata-edit-error" class="error-message" style="display: block">
            {errorMessage}
          </div>
        {/if}
        <div class="modal-actions">
          <button
            type="button"
            id="metadata-edit-cancel"
            class="btn-secondary"
            onclick={closeModal}
          >
            {$t('ui.metadata.edit_cancel', { default: 'Cancel' })}
          </button>
          <button
            type="submit"
            id="metadata-edit-save"
            class="btn-primary"
            disabled={saving}
          >
            {saving
              ? $t('ui.loading', { default: 'Saving...' })
              : $t('ui.metadata.edit_save', { default: 'Save' })}
          </button>
        </div>
      </form>
    </div>
  </div>
{/if}

<style>
  .modal {
    display: flex;
    position: fixed;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    background: oklch(0% 0 0deg / 60%);
    z-index: 10000;
    align-items: center;
    justify-content: center;
    padding: 20px;
  }

  .modal-content {
    background: var(--glass-bg, oklch(100% 0 0deg / 90%));
    backdrop-filter: blur(16px);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-lg);
    max-width: 500px;
    width: 100%;
    max-height: 90vh;
    overflow-y: auto;
    animation: modal-entrance 0.3s ease-out;
  }

  @keyframes modal-entrance {
    from {
      opacity: 0;
      transform: scale(0.95) translateY(-20px);
    }
    to {
      opacity: 1;
      transform: scale(1) translateY(0);
    }
  }

  .modal-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 24px 24px 16px;
    border-bottom: 1px solid var(--divider-color);
  }

  .modal-header h2 {
    margin: 0;
    font-size: 20px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .close-button {
    background: none;
    border: none;
    font-size: 28px;
    line-height: 1;
    color: var(--text-muted);
    cursor: pointer;
    padding: 0;
    width: 32px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-sm);
    transition: var(--transition-fast);
  }

  .close-button:hover {
    background: var(--background-secondary);
    color: var(--text-primary);
  }

  .modal :global(form) {
    padding: 24px;
  }

  .form-group {
    margin-bottom: 20px;
  }

  .form-group :global(label) {
    display: block;
    margin-bottom: 8px;
    font-weight: 500;
    color: var(--text-primary);
    font-size: 14px;
  }

  .form-group :global(input) {
    width: 100%;
    padding: 10px 12px;
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-sm);
    background: var(--background-secondary);
    color: var(--text-primary);
    font-size: 14px;
    font-family: inherit;
    transition: var(--transition-fast);
    box-sizing: border-box;
  }

  .form-group :global(input:focus) {
    outline: none;
    border-color: var(--primary-color);
    box-shadow: 0 0 0 3px oklch(55% 0.08 250deg / 10%);
  }

  .error-message {
    padding: 12px;
    background: rgb(239 68 68 / 10%);
    border: 1px solid rgb(239 68 68 / 30%);
    border-radius: var(--radius-sm);
    color: #ef4444;
    font-size: 14px;
    margin-bottom: 16px;
  }

  .modal-actions {
    display: flex;
    gap: 12px;
    justify-content: flex-end;
    padding-top: 16px;
    border-top: 1px solid var(--divider-color);
  }

  .form-group-row {
    display: flex;
    gap: 12px;
  }

  .btn-primary,
  .btn-secondary {
    padding: 10px 20px;
    border-radius: var(--radius-sm);
    font-size: 14px;
    font-weight: 500;
    cursor: pointer;
    transition: var(--transition-fast);
    border: none;
    font-family: inherit;
  }

  .btn-primary {
    background: var(--primary-color);
    color: white;
  }

  .btn-primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-primary:hover:not(:disabled) {
    background: var(--primary-dark);
    transform: translateY(-1px);
    box-shadow: var(--shadow-medium);
  }

  .btn-secondary {
    background: var(--background-secondary);
    color: var(--text-primary);
  }

  .btn-secondary:hover {
    background: var(--divider-color);
  }
</style>
