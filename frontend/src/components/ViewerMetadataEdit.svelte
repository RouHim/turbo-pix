<script>
  import { api } from '../lib/api.js';
  import { get } from 'svelte/store';
  import { t } from '../lib/i18n.js';
  import { addToast } from '../lib/state.svelte.js';
  import { isMetadataEditable, isVideoFile } from '../lib/utils.js';
  import { METADATA_ERROR_KEYS } from '../lib/metadataErrors.js';
  import Icon from './Icon.svelte';

  const { photo = null, onClose = () => {}, onSaved = () => {} } = $props();

  let modalEl = $state(null);
  let showModal = $state(false);
  let editTargetHash = $state(null);
  let takenAt = $state('');
  // The `taken_at` value `populateForm` installed for the photo the form was
  // opened on. `taken_at` goes into the payload only when the input differs
  // from it: a save must never restate a field the user did not touch (the
  // backend rewrites every date carrier whenever it sees `taken_at`, so a
  // location-only save would otherwise move the file's capture time).
  let populatedTakenAt = '';
  // The same rule for the position. The backend calls the container's
  // location writer for ANY pair it is handed, so restating the coordinates
  // the form opened with would make a date-only save either refuse a
  // container that has no writable location carrier or rewrite location bytes
  // the user never looked at.
  let populatedLatitude = '';
  let populatedLongitude = '';
  let latitude = $state('');
  let longitude = $state('');
  let errorMessage = $state('');
  let saving = $state(false);
  // Bumped on every open/close/submit; a PATCH response from a superseded
  // session (modal closed and reopened for the same photo while in flight)
  // must not act on the fresh session.
  let saveToken = 0;
  // Set once the modal has actually been opened, so the focus-restore branch
  // below doesn't steal focus to the (hidden) edit button on initial mount.
  let wasOpen = false;

  // The editor serves photos and writable videos alike; only the heading differs.
  const videoTarget = $derived(photo ? isVideoFile(photo.filename) : false);

  function openModal() {
    if (!photo || !isMetadataEditable(photo)) return;
    saveToken++;
    // A request the PREVIOUS session left in flight does not own this form:
    // without this the reopened modal renders a disabled "Saving…" button and
    // swallows Enter until that unrelated request settles.
    saving = false;
    wasOpen = true;
    editTargetHash = photo.hash_sha256;
    populateForm();
    showModal = true;
    document.body.style.overflow = 'hidden';
  }

  function closeModal() {
    saveToken++;
    editTargetHash = null;
    showModal = false;
    document.body.style.overflow = '';
    errorMessage = '';
    // The session that request belonged to is over; whatever it still does
    // server-side, it may no longer hold this form's button.
    saving = false;
    onClose();
  }

  // Close the modal if the viewed photo changed while it was open (the
  // form targets the hash captured at open time).
  $effect(() => {
    if (showModal && photo?.hash_sha256 !== editTargetHash) {
      closeModal();
    }
  });

  // Escape closes the modal; the viewer's own keydown handler ignores
  // events originating inside #metadata-edit-modal (see PhotoViewer).
  $effect(() => {
    if (!showModal) return;
    function onKey(e) {
      if (e.key === 'Escape') {
        e.preventDefault();
        closeModal();
      } else if (e.key === 'Tab' && modalEl) {
        // Trap focus inside the modal so Tab/Shift+Tab never escapes it.
        const focusables = [
          ...modalEl.querySelectorAll(
            'button, input, select, textarea, [href], [tabindex]:not([tabindex="-1"] )'
          ),
        ].filter((el) => !el.disabled);
        if (focusables.length === 0) {
          e.preventDefault();
          return;
        }
        const first = focusables[0];
        const last = focusables[focusables.length - 1];
        if (e.shiftKey && document.activeElement === first) {
          e.preventDefault();
          last.focus();
        } else if (!e.shiftKey && document.activeElement === last) {
          e.preventDefault();
          first.focus();
        }
      }
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  // Move focus into the modal on open; restore it to the edit button on close.
  $effect(() => {
    if (showModal) {
      modalEl?.querySelector('input')?.focus();
    } else if (wasOpen) {
      wasOpen = false;
      document.getElementById('metadata-edit-btn')?.focus();
    }
  });

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
    // Remember what this open installed; an untouched date is left out of the
    // payload entirely (checked in buildUpdatesFromForm).
    populatedTakenAt = takenAt;

    const loc = photo.metadata?.location || {};
    latitude = loc.latitude != null ? String(loc.latitude) : '';
    longitude = loc.longitude != null ? String(loc.longitude) : '';
    // Remember what this open installed; an untouched position is left out of
    // the payload entirely (checked in buildUpdatesFromForm).
    populatedLatitude = latitude;
    populatedLongitude = longitude;
  }

  /**
   * The number a coordinate input stands for, or `null` when it holds nothing
   * parseable. The populated baseline goes through this too, so the
   * comparison in `buildUpdatesFromForm` is numeric on both sides: a
   * `<input type="number">` may hand back a normalized spelling of what was
   * installed ("52.5" as "52.50"), which is the same value, not an edit.
   */
  function coordinateValue(input) {
    const parsed = parseFloat(String(input ?? '').trim());
    return Number.isNaN(parsed) ? null : parsed;
  }

  /**
   * Builds the metadata update payload from the form, validating GPS pairing
   * and ranges. Returns { updates } on success or { error } on validation
   * failure (translated message for the form).
   */
  function buildUpdatesFromForm() {
    const updates = {};

    // Only an edited date goes into the payload: restating the value the form
    // was opened with would make the backend rewrite the file's capture date
    // (truncated to whole minutes) on an unrelated save. The comparison is on
    // the instant, so the input's own spelling (with or without seconds) never
    // counts as an edit.
    const takenAtMs = takenAt ? new Date(takenAt).getTime() : null;
    const populatedMs = populatedTakenAt ? new Date(populatedTakenAt).getTime() : null;
    if (takenAtMs !== null && takenAtMs !== populatedMs) {
      updates.taken_at = new Date(takenAtMs).toISOString();
    }

    const lat = String(latitude ?? '').trim();
    const lng = String(longitude ?? '').trim();
    const hasLat = lat !== '';
    const hasLng = lng !== '';

    if ((hasLat && !hasLng) || (!hasLat && hasLng)) {
      return {
        error: get(t)('ui.metadata.edit_validation_gps_pair', {
          default: 'Both latitude and longitude must be provided together',
        }),
      };
    }

    const latVal = hasLat ? parseFloat(lat) : null;
    const lngVal = hasLng ? parseFloat(lng) : null;

    if (hasLat && (latVal < -90 || latVal > 90)) {
      return {
        error: get(t)('ui.metadata.edit_validation_gps', {
          default: 'GPS coordinates must be between -90/90 (lat) and -180/180 (lng)',
        }),
      };
    }

    if (hasLng && (lngVal < -180 || lngVal > 180)) {
      return {
        error: get(t)('ui.metadata.edit_validation_gps', {
          default: 'GPS coordinates must be between -90/90 (lat) and -180/180 (lng)',
        }),
      };
    }

    // The position is all-or-nothing, like every other save: the backend
    // refuses a half pair, and a request that carries one of the two would be
    // answered with a refusal the user cannot act on. So the pair goes in as a
    // unit — and only when it differs from the value the form opened with.
    // Restating an untouched position is the location half of the same bug the
    // date half above avoids: the container's location writer runs for any
    // pair it is handed, so a date-only save on a geotagged video would either
    // be refused for a missing location carrier or rewrite location bytes the
    // user never looked at.
    const positionChanged =
      (hasLat && latVal !== coordinateValue(populatedLatitude)) ||
      (hasLng && lngVal !== coordinateValue(populatedLongitude));
    if (positionChanged) {
      updates.latitude = latVal;
      updates.longitude = lngVal;
    }

    return { updates };
  }

  /**
   * True when the user cleared a field the photo previously had a value for
   * (date taken / GPS). The backend cannot clear these fields, so such a save
   * must fail honestly instead of silently no-oping with a success toast.
   */
  function formHasClearedFields() {
    if (!photo) return false;
    const hadTakenAt = Boolean(photo.taken_at);
    const hadLat = photo.metadata?.location?.latitude != null;
    const hadLng = photo.metadata?.location?.longitude != null;
    const hasTakenAt = String(takenAt ?? '').trim() !== '';
    const hasLat = String(latitude ?? '').trim() !== '';
    const hasLng = String(longitude ?? '').trim() !== '';
    return (hadTakenAt && !hasTakenAt) || (hadLat && !hasLat) || (hadLng && !hasLng);
  }

  /**
   * True while `token`/`targetHash` still describe the session the form on
   * screen was opened for. The modal can be closed (Escape / overlay / X) and
   * reopened for the same photo, or the viewer can navigate to another one,
   * while a PATCH is in flight — that response describes a form which is no
   * longer there and must not act on it, neither on success nor on refusal.
   */
  function isCurrentSession(token, targetHash) {
    return photo?.hash_sha256 === targetHash && token === saveToken;
  }

  async function handleSubmit(e) {
    e.preventDefault();
    // Re-entry guard: the Save button is disabled while saving, but Enter in a
    // text field can still re-trigger submit — skip duplicate PATCHes.
    if (saving) return;
    if (!photo) return;

    errorMessage = '';
    saving = true;
    // Claim the session for this attempt before the first `await`, so the
    // failure path below can be guarded by the same token as the success path.
    const token = ++saveToken;
    const targetHash = editTargetHash;

    try {
      // Clearing a previously-set field cannot be saved by the backend; keep
      // the modal open with an honest error instead of a fake success toast.
      if (formHasClearedFields()) {
        errorMessage = get(t)('messages.metadata_clear_unsupported', {
          default: 'Clearing metadata fields is not supported',
        });
        return;
      }

      const { updates, error } = buildUpdatesFromForm();
      if (error) {
        errorMessage = error;
        return;
      }

      // Nothing changed: don't PATCH and don't claim success.
      if (Object.keys(updates).length === 0) {
        closeModal();
        return;
      }

      const updatedPhoto = await api.updatePhotoMetadata(targetHash, updates);

      // A response from a superseded session must not overwrite the photo now
      // on screen, must not raise a success toast for it, and must not close
      // the form the user is now looking at.
      if (!isCurrentSession(token, targetHash)) return;

      // Update photo refs
      if (onSaved) {
        onSaved(updatedPhoto);
      }

      addToast(
        get(t)('ui.metadata.edit_success', { default: 'Metadata updated successfully' }),
        '',
        'success',
        3000
      );

      closeModal();
    } catch (error) {
      // The same staleness guard the success path uses: a refusal arriving
      // after the modal was closed or reopened belongs to a form nobody is
      // looking at, and painting it here would report the CURRENT session's
      // fields as rejected when they were never submitted.
      if (!isCurrentSession(token, targetHash)) return;

      // A refusal the backend identified by code gets its translated message;
      // anything else (network failure, unexpected shape) keeps the raw text.
      const refusalKey = error?.errorCode ? METADATA_ERROR_KEYS[error.errorCode] : undefined;
      if (refusalKey) {
        errorMessage = get(t)(refusalKey);
        return;
      }
      let msg = get(t)('ui.metadata.edit_error', { default: 'Failed to update metadata' });
      if (error.message) {
        const match = error.message.match(/HTTP \d+: (.+)/);
        msg = match?.[1] || error.message;
      }
      errorMessage = msg;
    } finally {
      // Only the request that owns the session on screen may clear the
      // button's in-flight state. A superseded one settling after the modal
      // was reopened would otherwise unblock — or, mid-save, re-block — a form
      // it knows nothing about; its own session already cleared the flag when
      // that session ended (openModal/closeModal).
      if (isCurrentSession(token, targetHash)) {
        saving = false;
      }
    }
  }

  function onOverlayClick(e) {
    if (e.target === e.currentTarget) closeModal();
  }

  // Expose open/close for parent
  export { openModal as open };

  export function close() {
    closeModal();
  }
</script>

{#if showModal}
  <div
    id="metadata-edit-modal"
    class="modal"
    bind:this={modalEl}
    aria-labelledby="metadata-edit-title"
    onclick={onOverlayClick}
    onkeydown={(e) => {
      // Escape closes the modal; stopPropagation so the window-level handler
      // (Escape fallback + Tab trap) never double-fires closeModal.
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        closeModal();
      }
    }}
    role="dialog"
    aria-modal="true"
    tabindex="-1"
  >
    <div class="modal-content">
      <div class="modal-header">
        <h2 id="metadata-edit-title">
          {#if videoTarget}
            {$t('ui.metadata.edit_modal_title_video', { default: 'Edit Video Metadata' })}
          {:else}
            {$t('ui.metadata.edit_modal_title', { default: 'Edit Photo Metadata' })}
          {/if}
        </h2>
        <button
          type="button"
          id="metadata-edit-close"
          class="close-button"
          aria-label={$t('ui.metadata.close', { default: 'Close' })}
          onclick={closeModal}
        >
          <Icon name="x" width={20} height={20} />
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
            oninput={() => {
              errorMessage = '';
            }}
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
              placeholder={$t('ui.metadata.edit_latitude_placeholder', { default: '-90 to 90' })}
              bind:value={latitude}
              oninput={() => {
                errorMessage = '';
              }}
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
              placeholder={$t('ui.metadata.edit_longitude_placeholder', { default: '-180 to 180' })}
              bind:value={longitude}
              oninput={() => {
                errorMessage = '';
              }}
            />
          </div>
        </div>
        {#if errorMessage}
          <div id="metadata-edit-error" class="error-message" style="display: block" role="alert">
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
          <button type="submit" id="metadata-edit-save" class="btn-primary" disabled={saving}>
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
    -webkit-backdrop-filter: blur(16px);
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
